//! Realtime path (spike S7 decision, ADR-45): the store pushes every committed event into a `tokio::broadcast`
//! channel; each subscriber tracks the last `seq` it delivered and, on a gap, a lag or a fresh start, reads the
//! missing range back from the event log. Delivery is therefore exactly-once and in order even though the
//! broadcast channel itself may drop messages. The UI always resyncs from the log; the bus is only a doorbell.

use std::collections::VecDeque;
use std::sync::Arc;

use agenium_core::{EventRecord, EventSink};
use tokio::sync::broadcast;

/// Read access to the event log (implemented over the store).
pub trait EventSource: Send + Sync {
    /// Events with `seq > after`, oldest first, at most `limit`.
    fn since(&self, after: u64, limit: usize) -> Result<Vec<EventRecord>, String>;
}

#[derive(Clone)]
pub struct Bus {
    tx: broadcast::Sender<EventRecord>,
}

impl Bus {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity.max(1));
        Self { tx }
    }

    /// A subscriber that delivers every event with `seq > after`.
    pub fn subscribe(&self, source: Arc<dyn EventSource>, after: u64) -> Subscriber {
        Subscriber {
            rx: self.tx.subscribe(),
            source,
            last: after,
            pending: VecDeque::new(),
            batch: 512,
        }
    }
}

impl Default for Bus {
    fn default() -> Self {
        Self::new(1024)
    }
}

impl EventSink for Bus {
    fn emit(&self, e: &EventRecord) {
        // No receivers is fine: subscribers catch up from the log.
        let _ = self.tx.send(e.clone());
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum RecvError {
    /// The bus was dropped and the log has nothing newer.
    Closed,
    /// The event log could not be read.
    Source(String),
}

pub struct Subscriber {
    rx: broadcast::Receiver<EventRecord>,
    source: Arc<dyn EventSource>,
    last: u64,
    pending: VecDeque<EventRecord>,
    batch: usize,
}

impl Subscriber {
    pub fn last_seq(&self) -> u64 {
        self.last
    }

    fn deliver(&mut self, e: EventRecord) -> EventRecord {
        self.last = e.seq;
        e
    }

    /// Fills `pending` from the log; returns how many events were found.
    fn resync(&mut self) -> Result<usize, RecvError> {
        let evs = self
            .source
            .since(self.last, self.batch)
            .map_err(RecvError::Source)?;
        let n = evs.len();
        self.pending.extend(evs);
        Ok(n)
    }

    /// Next event in `seq` order, exactly once.
    pub async fn recv(&mut self) -> Result<EventRecord, RecvError> {
        loop {
            if let Some(e) = self.pending.pop_front() {
                if e.seq > self.last {
                    return Ok(self.deliver(e));
                }
                continue;
            }
            match self.rx.recv().await {
                Ok(e) if e.seq <= self.last => continue,
                Ok(e) if e.seq == self.last + 1 => return Ok(self.deliver(e)),
                // A gap (events committed before we subscribed, or dropped): read the range from the log.
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {
                    self.resync()?;
                }
                Err(broadcast::error::RecvError::Closed) => {
                    if self.resync()? == 0 {
                        return Err(RecvError::Closed);
                    }
                }
            }
        }
    }

    /// Delivers whatever is already in the log beyond `last_seq` without waiting (startup catch-up).
    pub fn catch_up(&mut self) -> Result<Vec<EventRecord>, RecvError> {
        let mut out = Vec::new();
        loop {
            if self.resync()? == 0 && self.pending.is_empty() {
                return Ok(out);
            }
            while let Some(e) = self.pending.pop_front() {
                if e.seq > self.last {
                    out.push(self.deliver(e));
                }
            }
        }
    }
}

//! Minimal Server-Sent Events decoder (WHATWG rules the vendor streams need: `event:`, `data:` multi-line,
//! comments, CRLF/LF, blank-line dispatch). Fed raw byte chunks; yields complete events.

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

#[derive(Default)]
pub struct SseDecoder {
    buf: Vec<u8>,
    event: Option<String>,
    data: Vec<String>,
}

impl SseDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds bytes; returns the events completed by them.
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            let mut line: Vec<u8> = self.buf.drain(..=pos).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            let line = String::from_utf8_lossy(&line).into_owned();
            if line.is_empty() {
                if !self.data.is_empty() || self.event.is_some() {
                    out.push(SseEvent {
                        event: self.event.take(),
                        data: std::mem::take(&mut self.data).join("\n"),
                    });
                }
                continue;
            }
            if line.starts_with(':') {
                continue;
            }
            let (field, value) = match line.split_once(':') {
                Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
                None => (line.as_str(), ""),
            };
            match field {
                "event" => self.event = Some(value.to_owned()),
                "data" => self.data.push(value.to_owned()),
                _ => {}
            }
        }
        out
    }

    /// Flushes a final event not terminated by a blank line (some servers omit it before closing).
    pub fn finish(&mut self) -> Option<SseEvent> {
        if !self.buf.is_empty() {
            let rest = std::mem::take(&mut self.buf);
            let mut evs = self.feed(&[&rest[..], b"\n"].concat());
            if let Some(e) = evs.pop() {
                return Some(e);
            }
        }
        (!self.data.is_empty()).then(|| SseEvent {
            event: self.event.take(),
            data: std::mem::take(&mut self.data).join("\n"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_split_chunks_and_multiline() {
        let mut d = SseDecoder::new();
        let mut evs = d.feed(b"event: message_start\r\ndata: {\"a\":");
        assert!(evs.is_empty());
        evs.extend(d.feed(b"1}\r\n\r\n: ping\n\ndata: line1\ndata: line2\n\n"));
        assert_eq!(
            evs,
            vec![
                SseEvent {
                    event: Some("message_start".into()),
                    data: "{\"a\":1}".into()
                },
                SseEvent {
                    event: None,
                    data: "line1\nline2".into()
                },
            ]
        );
        assert!(d.feed(b"data: [DONE]").is_empty());
        assert_eq!(
            d.finish(),
            Some(SseEvent {
                event: None,
                data: "[DONE]".into()
            })
        );
        assert_eq!(d.finish(), None);
    }
}

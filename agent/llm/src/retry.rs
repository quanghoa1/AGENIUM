//! Retry policy (decision 15): base 1 s doubling, cap 30 s, jitter, at most 8 attempts or 5 minutes; honours
//! `retry-after` up to [`MAX_PROVIDER_DELAY`]. A dropped stream resends the identical frozen request (history is append-only so it still hits
//! the cache). Exhausting the policy makes the agent Offline; quota errors make it QuotaExhausted.

use std::future::Future;
use std::time::{Duration, Instant};

use crate::error::{classify, Classified, LlmError};

/// The longest `retry-after` that is waited out (learned from pi: hour-long waits stalled runs). A provider that asks
/// for longer is not called again now: the agent goes Offline and the background probe resumes it later.
pub const MAX_PROVIDER_DELAY: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    pub base: Duration,
    pub cap: Duration,
    pub max_attempts: u32,
    pub max_total: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            base: Duration::from_secs(1),
            cap: Duration::from_secs(30),
            max_attempts: 8,
            max_total: Duration::from_secs(300),
        }
    }
}

/// Why a call finally failed.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CallError {
    #[error("{0}")]
    Fatal(LlmError),
    #[error("quota or credit exhausted: {0}")]
    Quota(LlmError),
    /// Retries exhausted: the agent goes Offline and a background probe resumes it.
    #[error("provider unreachable after {attempts} attempts: {last}")]
    Offline { attempts: u32, last: LlmError },
}

impl RetryPolicy {
    /// Exponential delay for `attempt` (0-based) with jitter in [exp/2, exp], at least `retry_after`.
    pub fn delay(&self, attempt: u32, retry_after: Option<Duration>, jitter01: f64) -> Duration {
        let exp = self
            .base
            .saturating_mul(1u32 << attempt.min(16))
            .min(self.cap);
        let lo = exp / 2;
        let jittered = lo + (exp - lo).mul_f64(jitter01.clamp(0.0, 1.0));
        match retry_after {
            Some(ra) => ra.min(MAX_PROVIDER_DELAY).max(jittered),
            None => jittered,
        }
    }

    /// Runs `op` until success, a non-retryable error, or the policy is exhausted. `sleep` is injectable so tests
    /// do not wait; `on_retry(attempt, delay, &err)` lets the caller log/notify.
    pub async fn run<T, F, Fut, S, SFut>(
        &self,
        mut op: F,
        mut sleep: S,
        mut on_retry: impl FnMut(u32, Duration, &LlmError),
    ) -> Result<T, CallError>
    where
        F: FnMut(u32) -> Fut,
        Fut: Future<Output = Result<T, LlmError>>,
        S: FnMut(Duration) -> SFut,
        SFut: Future<Output = ()>,
    {
        let start = Instant::now();
        let mut slept = Duration::ZERO;
        let mut attempt = 0u32;
        loop {
            match op(attempt).await {
                Ok(v) => return Ok(v),
                Err(e) => match classify(&e) {
                    Classified::Fatal => return Err(CallError::Fatal(e)),
                    Classified::Quota => return Err(CallError::Quota(e)),
                    Classified::Retry(Some(ra)) if ra > MAX_PROVIDER_DELAY => {
                        return Err(CallError::Offline {
                            attempts: attempt + 1,
                            last: e,
                        });
                    }
                    Classified::Retry(ra) => {
                        let d = self.delay(attempt, ra, rand::random::<f64>());
                        let elapsed = start.elapsed().max(slept);
                        if attempt + 1 >= self.max_attempts || elapsed + d > self.max_total {
                            return Err(CallError::Offline {
                                attempts: attempt + 1,
                                last: e,
                            });
                        }
                        on_retry(attempt, d, &e);
                        sleep(d).await;
                        slept += d;
                        attempt += 1;
                    }
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn http(status: u16) -> LlmError {
        LlmError::Http {
            status,
            kind: String::new(),
            message: String::new(),
            quota: false,
            retry_after: None,
        }
    }

    #[test]
    fn delays_double_cap_and_respect_retry_after() {
        let p = RetryPolicy::default();
        assert_eq!(p.delay(0, None, 1.0), Duration::from_secs(1));
        assert_eq!(p.delay(0, None, 0.0), Duration::from_millis(500));
        assert_eq!(p.delay(3, None, 1.0), Duration::from_secs(8));
        assert_eq!(p.delay(10, None, 1.0), Duration::from_secs(30), "capped");
        assert_eq!(
            p.delay(0, Some(Duration::from_secs(12)), 1.0),
            Duration::from_secs(12)
        );
        assert_eq!(
            p.delay(0, Some(Duration::from_secs(3600)), 1.0),
            MAX_PROVIDER_DELAY,
            "a provider cannot stall a run for an hour"
        );
    }

    fn block_on<F: Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(f)
    }

    #[test]
    fn retries_then_succeeds_and_gives_up_after_eight() {
        let slept = RefCell::new(Vec::new());
        let p = RetryPolicy::default();
        let r = block_on(p.run(
            |a| async move {
                if a < 2 {
                    Err(http(529))
                } else {
                    Ok(a)
                }
            },
            |d| {
                slept.borrow_mut().push(d);
                async {}
            },
            |_, _, _| {},
        ));
        assert_eq!(r, Ok(2));
        assert_eq!(slept.borrow().len(), 2);

        let calls = RefCell::new(0u32);
        let r: Result<(), _> = block_on(p.run(
            |_| {
                *calls.borrow_mut() += 1;
                async { Err(LlmError::Network("reset".into())) }
            },
            |_| async {},
            |_, _, _| {},
        ));
        assert!(
            matches!(r, Err(CallError::Offline { attempts: 8, .. })),
            "{r:?}"
        );
        assert_eq!(*calls.borrow(), 8);
    }

    #[test]
    fn total_time_budget_and_non_retryable() {
        // A 4-minute retry-after is not waited out: Offline at once (the background probe resumes the agent).
        let p = RetryPolicy::default();
        let ra = LlmError::Http {
            status: 429,
            kind: String::new(),
            message: String::new(),
            quota: false,
            retry_after: Some(Duration::from_secs(240)),
        };
        let r: Result<(), _> = block_on(p.run(
            |_| {
                let e = ra.clone();
                async move { Err(e) }
            },
            |_| async {},
            |_, _, _| {},
        ));
        assert!(
            matches!(r, Err(CallError::Offline { attempts: 1, .. })),
            "{r:?}"
        );
        let r: Result<(), _> =
            block_on(p.run(|_| async { Err(http(401)) }, |_| async {}, |_, _, _| {}));
        assert!(matches!(r, Err(CallError::Fatal(_))));
        let r: Result<(), _> =
            block_on(p.run(|_| async { Err(http(402)) }, |_| async {}, |_, _, _| {}));
        assert!(matches!(r, Err(CallError::Quota(_))));
    }
}

//! Asking the judge: the keys in use take turns, one request each (a key that runs out of quota or fails is set aside
//! and the request goes again with the next one, `health`), one reused client per key, answers looked up in the cache first, the billed usage kept, and every ask in
//! the activity log the Settings page shows. In shadow mode the uses ask and record what they would do, and change
//! nothing (ADR-127): the offload of a use is measured on real runs before it is turned on.

use std::collections::{BTreeMap, HashMap};

use agenium_llm::systemone::{Answer, Judge, Judging, Question};
use serde::{Deserialize, Serialize};

use super::health::stop_for;
use crate::app::App;
use crate::{OrchError, Result};

/// The judge's clients by endpoint, model and key value: each key's connections are reused.
pub type KeptJudge = HashMap<(String, String, String), Judge>;

/// The judge's activity kept: the newest this many asks.
const ACTIVITY_KEEP: usize = 300;

/// One time the judge was asked: what for, about which ticket, what it answered (or why it could not).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JudgeActivity {
    pub at: String,
    /// `skill_hint`, `memory_gate`, `review_assist`, `skill_quality`, `skill_contradiction`, `done_check`,
    /// `trim_output`, `find_code` or `test`.
    #[serde(rename = "use")]
    pub use_: String,
    #[serde(default)]
    pub ticket: Option<String>,
    pub outcome: String,
    pub ok: bool,
    pub ms: u64,
    /// What the request was billed, as the API reported it (none for an answer from the cache).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<serde_json::Value>,
    /// Answered from the cache: no request was made.
    #[serde(default)]
    pub cached: bool,
    /// Shadow mode: recorded what the use would have done; nothing was changed.
    #[serde(default)]
    pub shadow: bool,
    /// The name of the key the request was made with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

impl JudgeActivity {
    pub(super) fn now(use_: &str, ticket: Option<&str>, ms: u64) -> Self {
        Self {
            at: agenium_core::now_iso(),
            use_: use_.to_owned(),
            ticket: ticket.map(str::to_owned),
            outcome: String::new(),
            ok: false,
            ms,
            usage: None,
            cached: false,
            shadow: false,
            key: None,
        }
    }
}

/// Which key to ask with.
pub(super) enum Pick<'a> {
    /// The first key in use (to know whether the judge can be asked).
    First,
    /// A key by name (the user re-enabling or testing it).
    Named(&'a str),
}

/// The answers to one ask.
pub(crate) struct Reply {
    pub answers: BTreeMap<String, Answer>,
    pub ms: u64,
    pub usage: Option<serde_json::Value>,
    /// Every answer came from the cache.
    pub cached: bool,
    /// The name of the key the request was made with (`None` from the cache).
    pub key: Option<String>,
}

impl App {
    /// The judge with the first key in use, or `None` when it is not configured or no key is in use (whether the judge
    /// can be asked now; the requests themselves take the keys in turn).
    pub fn judge(&self) -> Result<Option<Judge>> {
        Ok(self.judge_with_key(Some(Pick::First))?.map(|(_, j)| j))
    }

    /// The judge with a key — the next in turn, the first in use, or the one named — and the key's name ("local" for a
    /// local endpoint without keys). `None` when the judge is not configured or no key is in use.
    pub(super) fn judge_with_key(&self, pick: Option<Pick<'_>>) -> Result<Option<(String, Judge)>> {
        let j = self.config().judge;
        if !j.enabled() {
            return Ok(None);
        }
        let in_use = self.keys_in_use();
        let keys = j.key_list();
        let turn = |n: usize| {
            self.judge_turn
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                % n.max(1)
        };
        let name = match pick {
            Some(Pick::Named(n)) => Some(n.to_owned()),
            Some(Pick::First) => in_use.first().cloned(),
            None => (!in_use.is_empty()).then(|| in_use[turn(in_use.len())].clone()),
        };
        let pick = name.and_then(|n| keys.iter().find(|k| k.name == n));
        let (name, value) = match pick {
            Some(k) => {
                let v = self
                    .secret(&k.secret_ref)?
                    .ok_or_else(|| {
                        OrchError::NotFound(format!("judge key {:?} has no value", k.name))
                    })?
                    .expose_str()
                    .map_err(|e| OrchError::Invalid(e.to_string()))?
                    .to_owned();
                (k.name.clone(), v)
            }
            // A local endpoint needs no key.
            None if keys.is_empty() && agenium_config::config::local_endpoint(&j.base_url) => {
                ("local".to_owned(), String::new())
            }
            None => return Ok(None),
        };
        // One client per endpoint, model and key: its connections are reused (a new one per call paid the TLS
        // handshake every time, about 1.5 s against 0.3 s warm).
        let id = (j.base_url.clone(), j.model.clone(), value);
        let mut kept = self.judge_client.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(judge) = kept.get(&id) {
            return Ok(Some((name, judge.clone())));
        }
        let judge =
            Judge::new(&id.0, &id.2, &id.1).map_err(|e| OrchError::Invalid(e.to_string()))?;
        kept.insert(id, judge.clone());
        Ok(Some((name, judge)))
    }

    /// The uses act on what the judge says; `false` in shadow mode (they record what they would do instead).
    pub(crate) fn judge_acts(&self) -> bool {
        !self.config().judge.shadow
    }

    /// Asks the judge for `use_`: answers already given to the same question about the same state come from the cache,
    /// the rest in one request with the first key in use. A key out of quota or rejected is set aside (the user is told
    /// which) and the request goes again with the next key. Every text leaves through the redactor first — tool output,
    /// commands and reports can hold a token (jevgrep drops credential-looking files before anything is sent; here every
    /// use goes through this one place). Any other failure returns `None` (fail-open) and is recorded.
    pub(crate) async fn ask(&self, j: &Judging, use_: &str, ticket: Option<&str>) -> Option<Reply> {
        let j = &self.redacted(j);
        let (_, first) = self.judge_with_key(Some(Pick::First)).ok().flatten()?;
        let answerer = first.answerer();
        let mut answers = self.cached_answers(&answerer, j);
        let mut rest = j.clone();
        rest.questions.retain(|id, _| !answers.contains_key(id));
        if rest.questions.is_empty() {
            return Some(Reply {
                answers,
                ms: 0,
                usage: None,
                cached: true,
                key: None,
            });
        }
        let started = std::time::Instant::now();
        loop {
            let Some((key, judge)) = self.judge_with_key(None).ok().flatten() else {
                let mut a = JudgeActivity::now(use_, ticket, started.elapsed().as_millis() as u64);
                a.outcome = "failed: no Jev key left in use".into();
                self.log_activity(a);
                return None;
            };
            let r = judge.ask_full(&rest).await;
            let ms = started.elapsed().as_millis() as u64;
            match r {
                Ok(asked) => {
                    // A key back from a rest that answers is clear again (its failures no longer count).
                    if self.judge_health().keys_off.contains_key(&key) {
                        let _ = self.clear_key_aside(Some(&key));
                    }
                    self.keep_answers(&answerer, &rest, &asked.answers);
                    answers.extend(asked.answers);
                    return Some(Reply {
                        answers,
                        ms,
                        usage: asked.usage,
                        cached: false,
                        key: Some(key),
                    });
                }
                Err(e) => {
                    let mut a = JudgeActivity::now(use_, ticket, ms);
                    a.outcome = format!("failed: {e}");
                    a.key = Some(key.clone());
                    self.log_activity(a);
                    match stop_for(&e) {
                        // Set the key aside (out of quota: until the user re-enables it; failing: for a rest) and go
                        // again with the next key in turn.
                        Some(stop) if key != "local" => {
                            if self.set_key_aside(&key, stop, &e).is_err() {
                                return None;
                            }
                        }
                        _ => {
                            tracing::warn!(error = %e, use_, "judge call failed");
                            return None;
                        }
                    }
                }
            }
        }
    }

    /// `j` with every text in its state and questions passed through the redactor.
    fn redacted(&self, j: &Judging) -> Judging {
        fn walk(r: &agenium_secrets::Redactor, v: &mut serde_json::Value) {
            match v {
                serde_json::Value::String(s) => *s = r.redact(s),
                serde_json::Value::Array(a) => a.iter_mut().for_each(|x| walk(r, x)),
                serde_json::Value::Object(o) => o.values_mut().for_each(|x| walk(r, x)),
                _ => {}
            }
        }
        let r = self.redactor();
        let mut out = j.clone();
        out.state.values_mut().for_each(|v| walk(r, v));
        for q in out.questions.values_mut() {
            match q {
                Question::Noul { instructions } => *instructions = r.redact(instructions),
                Question::Choice {
                    instructions,
                    criteria,
                } => {
                    *instructions = r.redact(instructions);
                    criteria.values_mut().for_each(|c| *c = r.redact(c));
                }
            }
        }
        out
    }

    /// Records what an ask was for and what its answer meant (marked when in shadow mode or from the cache).
    pub(crate) fn record(&self, use_: &str, ticket: Option<&str>, outcome: String, reply: &Reply) {
        let mut a = JudgeActivity::now(use_, ticket, reply.ms);
        a.shadow = !self.judge_acts();
        a.outcome = if a.shadow {
            format!("[shadow] {outcome}")
        } else {
            outcome
        };
        a.ok = true;
        a.key = reply.key.clone();
        a.usage = reply.usage.clone();
        a.cached = reply.cached;
        self.log_activity(a);
    }

    /// Appends one ask to the judge's activity (best effort: the activity is for the user to see, never a gate).
    pub(super) fn log_activity(&self, a: JudgeActivity) {
        use std::io::Write;
        let path = self.layout().judge_activity();
        let Ok(line) = serde_json::to_string(&a) else {
            return;
        };
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            let _ = writeln!(f, "{line}");
        }
        if let Ok(text) = std::fs::read_to_string(&path) {
            let lines: Vec<&str> = text.lines().collect();
            if lines.len() > 2 * ACTIVITY_KEEP {
                let _ = std::fs::write(
                    &path,
                    lines[lines.len() - ACTIVITY_KEEP..].join("\n") + "\n",
                );
            }
        }
    }

    /// The judge's newest asks, newest first.
    pub fn judge_activity(&self, limit: usize) -> Vec<JudgeActivity> {
        let text = std::fs::read_to_string(self.layout().judge_activity()).unwrap_or_default();
        text.lines()
            .rev()
            .filter_map(|l| serde_json::from_str(l).ok())
            .take(limit)
            .collect()
    }
}

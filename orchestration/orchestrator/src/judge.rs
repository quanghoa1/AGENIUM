//! The optional TypeSafe "Jev" judge (ADR-75, ADR-81): a cheap, calibrated second opinion, off until `[judge]` is
//! set. Agenium never uses it as an agent's model, and it runs in two modes — with Jev and without. Without Jev (no
//! config, or after a failure) every judge call simply returns nothing and the work proceeds on the agent's own
//! judgment, so Agenium always works. A call that fails on the key or the quota turns the judge off and asks the
//! user to re-enter the key and re-enable it; a run error does not.
//!
//! Its wired use is an independent evidence read before a review run (`review_assist`): one batched request, a Noul
//! per acceptance criterion, a token-light first opinion for the Lead's expensive model.

use std::collections::BTreeMap;

use agenium_core::{DomainEvent, Ticket};
use agenium_llm::systemone::{Answer, Judge, Judging, Question};
use agenium_llm::LlmError;
use serde::{Deserialize, Serialize};

use crate::app::App;
use crate::{OrchError, Result};

/// A Noul at or above this reads as "met", at or below `1 - this` as "not met", between as "unsure" (decision 35).
const MET: f64 = 0.7;

/// Whether `url` is plain HTTP to this machine (localhost, 127.0.0.1 or [::1], any port): the only endpoint the judge
/// may use without TLS. The host is compared whole, so `http://localhost.example.com` or `http://localhost@host` is not
/// local.
fn local_http(url: &str) -> bool {
    let Some(rest) = url
        .get(..7)
        .filter(|s| s.eq_ignore_ascii_case("http://"))
        .map(|_| &url[7..])
    else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.contains('@') {
        return false;
    }
    let host = match authority.strip_prefix('[') {
        Some(v6) => match v6.split_once(']') {
            Some((h, port)) if port.is_empty() || port.starts_with(':') => h,
            _ => return false,
        },
        None => authority.split(':').next().unwrap_or_default(),
    };
    ["localhost", "127.0.0.1", "::1"]
        .iter()
        .any(|l| host.eq_ignore_ascii_case(l))
}

/// Vault name of the judge's API key.
pub const JUDGE_SECRET: &str = "judge/typesafe";

/// The judge's activity kept: the newest this many asks.
const ACTIVITY_KEEP: usize = 300;

/// One time the judge was asked: what for, about which ticket, what it answered (or why it could not).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JudgeActivity {
    pub at: String,
    /// `skill_hint`, `memory_gate`, `review_assist`, `skill_quality`, `skill_contradiction` or `test`.
    #[serde(rename = "use")]
    pub use_: String,
    #[serde(default)]
    pub ticket: Option<String>,
    pub outcome: String,
    pub ok: bool,
    pub ms: u64,
}

/// The judge as the user sets it in the app: endpoint, model, the key (kept as is when not given) and what it may do.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct JudgeInput {
    pub base_url: String,
    pub model: String,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub clear_key: bool,
    #[serde(default)]
    pub review_assist: bool,
    #[serde(default)]
    pub skill_hint: bool,
    #[serde(default)]
    pub memory_gate: bool,
    #[serde(default)]
    pub skill_quality: bool,
}

/// Whether the judge is usable now and, if not, why (persisted in `<root>/judge-health.json`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JudgeHealth {
    /// `true` when a key/quota failure turned the judge off until the user re-enables it.
    pub disabled: bool,
    pub reason: Option<String>,
    pub at: Option<String>,
}

/// A key/quota failure disables the judge (the user must act); a one-off run error does not.
fn is_credential_or_quota(e: &LlmError) -> bool {
    match e {
        // systemone maps 401/403/400/422 to Config and 429 to a "rate limited" Network error.
        LlmError::Config(m) => {
            let m = m.to_lowercase();
            m.contains("key") || m.contains("subscription") || m.contains("rejected")
        }
        LlmError::Network(m) => m.to_lowercase().contains("rate limited"),
        _ => false,
    }
}

impl App {
    pub fn judge_health(&self) -> JudgeHealth {
        std::fs::read_to_string(self.layout().judge_health())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn set_judge_health(&self, h: &JudgeHealth) -> Result<()> {
        let path = self.layout().judge_health();
        std::fs::write(
            &path,
            serde_json::to_string_pretty(h).map_err(|e| OrchError::Invalid(e.to_string()))?,
        )
        .map_err(|e| OrchError::Invalid(format!("{}: {e}", path.display())))
    }

    /// Asks the judge for `use_` and records the ask (`record` adds what the answer meant). A key/quota failure
    /// disables the judge; any failure returns `None` (fail-open).
    async fn ask(
        &self,
        judge: &Judge,
        j: &Judging,
        use_: &str,
        ticket: Option<&str>,
    ) -> Option<(BTreeMap<String, Answer>, u64)> {
        let started = std::time::Instant::now();
        let r = judge.ask(j).await;
        let ms = started.elapsed().as_millis() as u64;
        match r {
            Ok(a) => Some((a, ms)),
            Err(e) => {
                if is_credential_or_quota(&e) {
                    let _ = self.disable_judge(e.to_string());
                } else {
                    tracing::warn!(error = %e, use_, "judge call failed");
                }
                self.record(use_, ticket, format!("failed: {e}"), false, ms);
                None
            }
        }
    }

    /// Appends one ask to the judge's activity (best effort: the activity is for the user to see, never a gate).
    fn record(&self, use_: &str, ticket: Option<&str>, outcome: String, ok: bool, ms: u64) {
        use std::io::Write;
        let a = JudgeActivity {
            at: agenium_core::now_iso(),
            use_: use_.to_owned(),
            ticket: ticket.map(str::to_owned),
            outcome,
            ok,
            ms,
        };
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

    /// Turns the judge off after a key/quota failure and tells the user to re-enter the key and re-enable it.
    fn disable_judge(&self, reason: String) -> Result<()> {
        if self.judge_health().disabled {
            return Ok(());
        }
        self.set_judge_health(&JudgeHealth {
            disabled: true,
            reason: Some(reason.clone()),
            at: Some(agenium_core::now_iso()),
        })?;
        self.store().commit(None, "system", |w| -> Result<()> {
            w.emit(DomainEvent::NeedsUser {
                ticket: None,
                reason: format!("the TypeSafe judge was turned off ({reason}); re-enter its key and re-enable it, or leave it off"),
            })?;
            Ok(())
        })?;
        Ok(())
    }

    /// The configured judge, or `None` when `[judge]` is off or a failure disabled it. `Err` only when it is on,
    /// enabled, but misconfigured (no key).
    pub fn judge(&self) -> Result<Option<Judge>> {
        let j = self.config().judge;
        if !j.enabled() || self.judge_health().disabled {
            return Ok(None);
        }
        let key = if j.api_key_ref.is_empty() {
            String::new()
        } else {
            self.secret(&j.api_key_ref)?
                .ok_or_else(|| {
                    OrchError::NotFound(format!("judge key {:?} is not set", j.api_key_ref))
                })?
                .expose_str()
                .map_err(|e| OrchError::Invalid(e.to_string()))?
                .to_owned()
        };
        Judge::new(&j.base_url, &key, &j.model)
            .map(Some)
            .map_err(|e| OrchError::Invalid(e.to_string()))
    }

    /// Re-enables the judge and checks it with a trivial request; clears the disabled state on success. For the user
    /// after re-entering the key.
    /// Sets the judge (ADR-75): an empty endpoint turns it off. A new key goes to the vault, never to the config; a
    /// change clears an earlier failure so the judge is tried again.
    pub fn set_judge(&self, j: &JudgeInput) -> Result<agenium_config::JudgeSection> {
        let base_url = j.base_url.trim().trim_end_matches('/').to_owned();
        let local = local_http(&base_url);
        if !base_url.is_empty() && !base_url.starts_with("https://") && !local {
            return Err(OrchError::Invalid(format!(
                "judge endpoint {base_url:?}: use https://"
            )));
        }
        let old = self.config().judge;
        let key_ref = match (&j.api_key, j.clear_key) {
            (Some(k), _) if !k.trim().is_empty() => {
                self.secrets().set(JUDGE_SECRET, k.trim().as_bytes())?;
                JUDGE_SECRET.to_owned()
            }
            (_, true) => {
                let _ = self.secrets().delete(JUDGE_SECRET);
                String::new()
            }
            _ => old.api_key_ref.clone(),
        };
        let next = agenium_config::JudgeSection {
            base_url,
            model: j.model.trim().to_owned(),
            api_key_ref: key_ref,
            review_assist: j.review_assist,
            skill_hint: j.skill_hint,
            memory_gate: j.memory_gate,
            skill_quality: j.skill_quality,
        };
        if next.enabled() && next.api_key_ref.is_empty() && !local {
            return Err(OrchError::Invalid("the judge needs its API key".into()));
        }
        self.update_config(|c| c.judge = next.clone())?;
        self.set_judge_health(&JudgeHealth::default())?;
        Ok(next)
    }

    pub async fn enable_judge(&self) -> Result<()> {
        self.set_judge_health(&JudgeHealth::default())?;
        let Some(judge) = self.judge()? else {
            return Err(OrchError::Invalid(
                "the judge is not configured: set [judge] base_url, model and the key first".into(),
            ));
        };
        let probe = Judging::default()
            .state_text("x", "ok")
            .noul("q", "Does `x` say ok?");
        let started = std::time::Instant::now();
        let r = judge.ask(&probe).await;
        let ms = started.elapsed().as_millis() as u64;
        match r {
            Ok(_) => {
                self.record("test", None, "answered the test question".into(), true, ms);
                Ok(())
            }
            Err(e) => {
                self.record("test", None, format!("failed: {e}"), false, ms);
                self.set_judge_health(&JudgeHealth {
                    disabled: true,
                    reason: Some(e.to_string()),
                    at: Some(agenium_core::now_iso()),
                })?;
                Err(OrchError::Invalid(format!("the judge still fails: {e}")))
            }
        }
    }

    /// Before a work run, lets the judge pick the skill that best fits the ticket from `skills` (name, description),
    /// so the worker loads the right one without reading them all (ADR-82). `None` when the judge is off/disabled,
    /// `skill_hint` is off, there are fewer than two skills, or the pick is "none"/low-confidence/failed. A
    /// key/quota failure disables the judge.
    pub async fn skill_hint(&self, t: &Ticket, skills: &[(String, String)]) -> Option<String> {
        if !self.config().judge.skill_hint || skills.len() < 2 {
            return None;
        }
        let judge = match self.judge() {
            Ok(Some(j)) => j,
            _ => return None,
        };
        let mut criteria: std::collections::BTreeMap<String, String> = skills
            .iter()
            .map(|(name, desc)| (name.clone(), desc.chars().take(200).collect()))
            .collect();
        criteria.insert("none".into(), "no skill fits this ticket".into());
        let mut j = Judging::default().state_text(
            "ticket",
            &format!(
                "{}\n{}",
                t.title,
                t.description.chars().take(800).collect::<String>()
            ),
        );
        j.questions.insert(
            "skill".into(),
            Question::Choice {
                instructions: "Which skill best fits the work described in `ticket`?".into(),
                criteria,
            },
        );
        let key = t.key.to_string();
        let (answers, ms) = self.ask(&judge, &j, "skill_hint", Some(&key)).await?;
        let a = answers.get("skill");
        let pick = a.and_then(|a| a.choice.as_deref()).unwrap_or("none");
        let conf = a.and_then(|a| a.confidence);
        let pct = conf
            .map(|c| format!(" ({:.0}%)", c * 100.0))
            .unwrap_or_default();
        if pick == "none" || conf.is_some_and(|c| c < MET) {
            let outcome = format!(
                "no skill suggested (answer: {pick}{pct}) among {}",
                skills.len()
            );
            self.record("skill_hint", Some(&key), outcome, true, ms);
            return None;
        }
        self.record(
            "skill_hint",
            Some(&key),
            format!("suggested `{pick}`{pct} among {} skills", skills.len()),
            true,
            ms,
        );
        Some(format!(
            "\nSuggested skill (TypeSafe Jev, a hint — load it only if it fits): `{pick}`. Call load_skill to read it before the work it covers.\n"
        ))
    }

    /// After a ticket closes, a cheap read of whether the `episode` holds anything worth remembering, so the costly
    /// extraction model can be skipped on routine work (ADR-85). `Some(false)` only when the judge is confident
    /// nothing is worth keeping; `Some(true)` when it is; `None` when the judge is off/disabled, `memory_gate` is off,
    /// the verdict is unsure, or the call failed — the caller then extracts as usual. A key/quota failure disables it.
    pub async fn worth_remembering(&self, episode: &str) -> Option<bool> {
        if !self.config().judge.memory_gate {
            return None;
        }
        let judge = match self.judge() {
            Ok(Some(j)) => j,
            _ => return None,
        };
        let j = Judging::default().state_text("episode", episode).noul(
            "worth",
            "Does `episode` contain anything worth remembering for future tickets — a durable fact, preference, \
             convention, path/command, or (if it names a signal like a reopen or correction) a lesson — as opposed to \
             only routine work with nothing new?",
        );
        let (answers, ms) = self.ask(&judge, &j, "memory_gate", None).await?;
        let p = answers.get("worth").and_then(|a| a.noul)?;
        let (verdict, outcome) = if p >= MET {
            (Some(true), "worth remembering: memory is extracted")
        } else if p <= 1.0 - MET {
            (Some(false), "nothing worth remembering: extraction skipped")
        } else {
            (None, "unsure: memory is extracted")
        };
        self.record(
            "memory_gate",
            None,
            format!("{outcome} ({:.0}%)", p * 100.0),
            true,
            ms,
        );
        verdict
    }

    /// A calibrated read of a skill `body` against the methodology principles (ADR-90): one job, actionable steps, a
    /// proof that demands evidence, and no long embedded code that goes stale. Returns a human-readable note per
    /// principle (e.g. "proof demands evidence: weak (41%)"). `None` when the judge is off/disabled, `skill_quality`
    /// is off, or the call failed — the deterministic shape check (`lint_skill`) still runs regardless. A key/quota
    /// failure disables the judge.
    pub async fn skill_quality(&self, body: &str) -> Option<Vec<String>> {
        if !self.config().judge.skill_quality {
            return None;
        }
        let judge = match self.judge() {
            Ok(Some(j)) => j,
            _ => return None,
        };
        let qs: [(&str, &str); 4] = [
            ("one job", "Does `skill` describe exactly one job, not several bundled together?"),
            ("actionable steps", "Are the steps in `skill` concrete and actionable rather than vague?"),
            ("proof demands evidence", "Does `skill` require concrete evidence (a proof) before the work may be called done?"),
            ("no stale code", "Is `skill` free of long embedded code blocks that would go out of date?"),
        ];
        let mut j = Judging::default().state_text("skill", body);
        for (id, q) in &qs {
            j = j.noul(id, q);
        }
        let (answers, ms) = self.ask(&judge, &j, "skill_quality", None).await?;
        let notes: Vec<String> = qs
            .iter()
            .filter_map(|(id, _)| {
                let p = answers.get(*id).and_then(|a| a.noul)?;
                let verdict = if p >= MET {
                    "yes"
                } else if p <= 1.0 - MET {
                    "no"
                } else {
                    "weak"
                };
                Some(format!("{id}: {verdict} ({:.0}%)", p * 100.0))
            })
            .collect();
        self.record("skill_quality", None, notes.join("; "), true, ms);
        (!notes.is_empty()).then_some(notes)
    }

    /// The judge's read of whether a new or changed skill `text` contradicts the existing skills closest to it
    /// (ADR-92): one Noul per related skill — "do they give contradictory instructions for the same situation?".
    /// Returns the names it confidently reads as contradicting (at or above `MET`). `None` when the judge is
    /// off/disabled, `skill_quality` is off, there is nothing related, or the call failed — the deterministic checks
    /// (one subject, explicit opposite directives) still apply. A key/quota failure disables the judge.
    pub async fn skill_contradiction(
        &self,
        text: &str,
        related: &[(String, String)],
    ) -> Option<Vec<String>> {
        if !self.config().judge.skill_quality || related.is_empty() {
            return None;
        }
        let judge = match self.judge() {
            Ok(Some(j)) => j,
            _ => return None,
        };
        let cap = |s: &str| s.chars().take(6000).collect::<String>();
        let mut j = Judging::default().state_text("new_skill", &cap(text));
        for (i, (_, guidance)) in related.iter().enumerate() {
            let key = format!("existing_{i}");
            j = j.state_text(&key, &cap(guidance)).noul(
                &format!("c{i}"),
                &format!(
                    "Do `new_skill` and `{key}` give contradictory instructions for the same situation (one says to do \
                     something the other says not to do)?"
                ),
            );
        }
        let (answers, ms) = self.ask(&judge, &j, "skill_contradiction", None).await?;
        let found: Vec<String> = related
            .iter()
            .enumerate()
            .filter(|(i, _)| {
                answers
                    .get(&format!("c{i}"))
                    .and_then(|a| a.noul)
                    .is_some_and(|p| p >= MET)
            })
            .map(|(_, (name, _))| name.clone())
            .collect();
        let outcome = if found.is_empty() {
            format!("no contradiction with {} related skill(s)", related.len())
        } else {
            format!("contradicts {}", found.join(", "))
        };
        self.record("skill_contradiction", None, outcome, true, ms);
        Some(found)
    }

    /// An independent first read of `report` against `t`'s acceptance criteria, for the review task (ADR-75).
    /// `None` when the judge is off/disabled, `review_assist` is off, there are no criteria, or the call failed —
    /// the review then proceeds on the Lead's own read. A key/quota failure disables the judge.
    pub async fn review_evidence(&self, t: &Ticket, report: &str) -> Option<String> {
        if !self.config().judge.review_assist || t.acceptance.is_empty() {
            return None;
        }
        let judge = match self.judge() {
            Ok(Some(j)) => j,
            Ok(None) => return None,
            Err(e) => {
                tracing::warn!(error = %e, "judge not available");
                return None;
            }
        };
        let mut j = Judging::default().state_text("report", report);
        for ac in &t.acceptance {
            j = j.noul(
                &ac.id,
                &format!("Does the work described in `report` provide evidence that this criterion is met: {}", ac.text),
            );
        }
        let key = t.key.to_string();
        let (answers, ms) = self.ask(&judge, &j, "review_assist", Some(&key)).await?;
        let mut lines = Vec::new();
        for ac in &t.acceptance {
            if let Some(p) = answers.get(&ac.id).and_then(|a| a.noul) {
                let verdict = if p >= MET {
                    "evidence found"
                } else if p <= 1.0 - MET {
                    "no evidence"
                } else {
                    "unclear"
                };
                lines.push(format!("- {}: {verdict} ({:.0}%)", ac.id, p * 100.0));
            }
        }
        let outcome = if lines.is_empty() {
            "no criterion answered".to_owned()
        } else {
            lines
                .iter()
                .map(|l| l.trim_start_matches("- "))
                .collect::<Vec<_>>()
                .join("; ")
        };
        self.record("review_assist", Some(&key), outcome, true, ms);
        if lines.is_empty() {
            return None;
        }
        Some(format!(
            "\n## Independent evidence read (TypeSafe Jev, calibrated — a first opinion, verify it yourself)\n{}\n",
            lines.join("\n")
        ))
    }
}

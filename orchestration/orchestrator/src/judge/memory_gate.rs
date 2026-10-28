//! Memory gate (ADR-85): after a ticket closes, whether the episode holds anything worth remembering, so the costly
//! extraction model can be skipped on routine work. It only ever skips; it never decides what is learned.

use agenium_llm::systemone::Judging;

use super::MET;
use crate::app::App;

impl App {
    /// `Some(false)` only when the judge is confident nothing in `episode` is worth keeping; `Some(true)` when it is;
    /// `None` when the judge is off/disabled, `memory_gate` is off, the verdict is unsure, the call failed, or in
    /// shadow mode — the caller then extracts as usual. A key/quota failure disables the judge.
    pub async fn worth_remembering(&self, episode: &str) -> Option<bool> {
        if !self.config().judge.memory_gate {
            return None;
        }
        self.judge().ok().flatten()?;
        let j = Judging::default().state_text("episode", episode).noul(
            "worth",
            "Does `episode` contain anything worth remembering for future tickets — a durable fact, preference, \
             convention, path/command, or (if it names a signal like a reopen or correction) a lesson — as opposed to \
             only routine work with nothing new?",
        );
        let reply = self.ask(&j, "memory_gate", None).await?;
        let p = reply.answers.get("worth").and_then(|a| a.noul)?;
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
            &reply,
        );
        verdict.filter(|_| self.judge_acts())
    }
}

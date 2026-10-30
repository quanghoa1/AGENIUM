//! Settling an agent's memory (ADR-145): after every episode it learned from, by its vectors and words (free); with the
//! judge once it learned [`SETTLE_EVERY`] facts since the judge last read, and when the user tidies its memory. Folds
//! facts that say the same, puts contradictions to the user; the judge reads only the pairs the vectors cannot tell,
//! when `memory_settle` is on.

use agenium_core::AgentId;
use agenium_memory::settle::{settle, PairJudge, SettleOptions, SettleReport};
use agenium_store::AgentStore;

use crate::app::App;
use crate::Result;

/// New facts after which the judge reads the pairs the vectors cannot tell (ADR-145: four tickets learn about ten).
pub const SETTLE_EVERY: i64 = 10;
/// The newest fact id the judge last read up to (agent meta).
const SETTLED_UPTO: &str = "settled_upto";
/// The newest fact id any pass saw (agent meta).
const SEEN_UPTO: &str = "settle_seen_upto";
/// The embedding model the pass's similarity bands were measured on (ADR-145). Agenium's memory uses it (ADR-47); the
/// hashing stand-in used while it is not installed reads other similarities, so the pass waits for the model.
pub const SETTLE_MODEL: &str = "bge-small-en-v1.5";

/// Whether the judge's pass is due: `newest` is the newest fact id, `upto` the newest it last read up to.
fn due(newest: i64, upto: i64) -> bool {
    newest - upto >= SETTLE_EVERY
}

impl App {
    /// One settling pass over `agent`'s memory with the judge (`dry_run`: what it would do, nothing changed). `None`
    /// when the embedding model is not installed (the pass needs it).
    pub async fn settle_memory(
        &self,
        agent: AgentId,
        dry_run: bool,
    ) -> Result<Option<SettleReport>> {
        let e = self.embedder();
        let mem = self.agent_store(agent, e.dim(), e.model_id())?;
        self.settle_store(&mem, dry_run, true).await
    }

    /// A pass; with `judged`, the judge reads (when on) and the judge's counter restarts.
    async fn settle_store(
        &self,
        mem: &AgentStore,
        dry_run: bool,
        judged: bool,
    ) -> Result<Option<SettleReport>> {
        if self.embedder().model_id() != SETTLE_MODEL {
            tracing::info!(
                model = self.embedder().model_id(),
                "memory not settled: the pass needs {SETTLE_MODEL}"
            );
            return Ok(None);
        }
        let judge = self.settle_judge().filter(|_| judged);
        let opts = SettleOptions {
            dry_run,
            ..Default::default()
        };
        let newest = mem.live(1)?.first().map_or(0, |f| f.id);
        let report = settle(mem, judge.as_ref().map(|j| j as &dyn PairJudge), opts)
            .await
            .map_err(|e| crate::OrchError::Invalid(e.to_string()))?;
        if !dry_run {
            mem.set_meta(SEEN_UPTO, &newest.to_string())?;
            if judged {
                mem.set_meta(SETTLED_UPTO, &newest.to_string())?;
            }
        }
        Ok(Some(report))
    }

    /// After an episode was learned: a free pass (vectors and words) when there is anything new, with the judge when
    /// [`SETTLE_EVERY`] facts came since it last read. `None` when nothing new, without the embedding model, or failed
    /// (logged).
    pub async fn settle_when_due(&self, mem: &AgentStore) -> Option<SettleReport> {
        let newest = mem.live(1).ok()?.first().map_or(0, |f| f.id);
        let meta = |k: &str| -> i64 {
            mem.meta(k)
                .ok()
                .flatten()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0)
        };
        if newest <= meta(SEEN_UPTO) {
            return None;
        }
        match self
            .settle_store(mem, false, due(newest, meta(SETTLED_UPTO)))
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "settling the memory failed");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_judge_reads_every_ten_new_facts() {
        assert!(!super::due(9, 0));
        assert!(super::due(10, 0));
        assert!(!super::due(19, 10));
        assert!(super::due(25, 10));
    }
}

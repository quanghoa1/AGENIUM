//! Review assist (ADR-75): an independent first read of a worker's report against the ticket's acceptance criteria,
//! put in front of the Lead's review. The Lead still decides. Its questions are also asked ahead, with the done check
//! at submit time (one request), and the review run then finds the answers in the cache.

use agenium_core::forms::WorkReport;
use agenium_core::Ticket;
use agenium_llm::systemone::Judging;

use super::MET;
use crate::app::App;

/// The report as the judge reads it (the same text at submit and at review, so a prefetched answer is found).
pub fn report_text(r: &WorkReport) -> String {
    format!(
        "{}\n{}",
        r.summary,
        r.acceptance
            .iter()
            .map(|a| format!("{}: {}", a.ac_id, a.evidence))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

/// One Noul per acceptance criterion: does the work described in `report` show it is met?
pub(super) fn review_questions(t: &Ticket, report: &str) -> Judging {
    let mut j = Judging::default().state_text("report", report);
    for ac in &t.acceptance {
        j = j.noul(
            &ac.id,
            &format!(
                "Does the work described in `report` provide evidence that this criterion is met: {}",
                ac.text
            ),
        );
    }
    j
}

impl App {
    /// The first read for the review task. `None` when the judge is off/disabled, `review_assist` is off, there are
    /// no criteria, the call failed, or in shadow mode — the review then proceeds on the Lead's own read. A key/quota
    /// failure disables the judge.
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
        let j = review_questions(t, report);
        let key = t.key.to_string();
        let reply = self.ask(&judge, &j, "review_assist", Some(&key)).await?;
        let mut lines = Vec::new();
        for ac in &t.acceptance {
            if let Some(p) = reply.answers.get(&ac.id).and_then(|a| a.noul) {
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
        self.record("review_assist", Some(&key), outcome, &reply);
        if lines.is_empty() || !self.judge_acts() {
            return None;
        }
        Some(format!(
            "\n## Independent evidence read (TypeSafe Jev, calibrated — a first opinion, verify it yourself)\n{}\n",
            lines.join("\n")
        ))
    }
}

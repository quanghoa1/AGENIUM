//! The optional TypeSafe "Jev" judge (ADR-75, ADR-81, ADR-126, ADR-127): a cheap, calibrated second opinion, off until
//! `[judge]` is set. Agenium never uses it as an agent's model, and it runs in two modes — with Jev and without.
//! Without Jev (no config, or after a failure) every judge call simply returns nothing and the work proceeds on the
//! agent's own judgment, so Agenium always works. A rejected key or an exhausted quota turns the judge off and asks
//! the user to re-enter the key and re-enable it; anything else does not.
//!
//! One file per concern, so a fault in one use is found and fixed there:
//! - [`settings`]: the judge as the user sets it (endpoint, key in the vault, which uses are on), the test call;
//! - [`health`]: when a failure turns it off;
//! - [`client`]: asking (one reused client, the answer cache, billed usage, shadow mode) and the activity log;
//! - [`cache`]: answers kept by question and the state it reads;
//! - the uses: [`skill_hint`], [`memory_gate`], [`skill_quality`] (with contradictions), [`review_assist`],
//!   [`done_check`], [`trim`].

mod cache;
mod client;
pub mod done_check;
mod health;
mod memory_gate;
mod review_assist;
mod settings;
mod skill_hint;
mod skill_quality;
pub mod trim;

pub use client::{JudgeActivity, KeptJudge};
pub use health::JudgeHealth;
pub use review_assist::report_text;
pub use settings::{JudgeInput, JUDGE_SECRET};

/// A Noul at or above this reads as "met", at or below `1 - this` as "not met", between as "unsure" (decision 35).
const MET: f64 = 0.7;

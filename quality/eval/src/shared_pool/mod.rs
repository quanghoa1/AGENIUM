//! Shared-pool feedback (audit M3, 2026-10-31): another agent's lesson in the shared pool (ADR-87) that misleads the
//! work is named by the learning, weighs on the pool's copy only, and leaves the pool after two contexts. Two scenarios:
//!
//! - `attribution` (`--shared-pool attribution --cases N --seed S`): the extraction alone, on a real model, over random
//!   cases. Each shows the learning the agent's own lessons and shared ones (pool ids drawn so they often equal the
//!   agent's), and a reopened episode whose cause is one of them — or neither: a lesson shown but not followed, or a
//!   mistake no lesson made. Graded per case: the culprit named, nothing else; the error that matters most is a shared
//!   lesson's blame landing on the agent's own lesson (`cross`).
//! - `loop` (`--shared-pool loop [--keep <new dir>]`): the whole app with real workers (`--provider` or `--cli`; a CLI
//!   agent extracts with `--extract`). The pool holds another agent's lesson that is wrong for these projects; two Solo
//!   agents, in two projects, each work a ticket, which the user then reopens — on that lesson when the work followed
//!   it, else on something else — and each learns from it. Checked: the lesson reached the work run and was logged in
//!   the pool's usage, the blame followed the work, the copy left the pool exactly when it misled both tickets (and
//!   recall stops finding it), and neither the lesson it was copied from nor the agents' own lessons changed. The
//!   extraction prompt's "MEMORIES SHOWN" and the model's `misleading` answer are kept per ticket.

mod attribution;
mod full_loop;

pub use attribution::attribution;
pub use full_loop::full_loop;

pub const ID: &str = "shared-pool";

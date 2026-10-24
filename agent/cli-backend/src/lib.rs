//! CliBackend (decision 3c / 30, milestone M2.5): run a vendor agent CLI headless (JSON event stream) as an
//! Agenium agent. Agenium only *detects* installed CLIs — version and login state — and never logs in, never writes
//! the user's global CLI configuration and reuses the user's own authentication read-only.
//!
//! This module currently provides detection (`detect`); driving runs comes next.

pub mod detect;

pub use detect::{detect_all, CliInfo, CliKind, Login};

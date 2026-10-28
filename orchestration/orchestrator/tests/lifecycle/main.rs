#[path = "../common/mod.rs"]
mod common;

use agenium_core::forms::*;
use agenium_core::*;
use agenium_orchestrator::*;
use agenium_store::core::{comments, events, ops, workspaces};
use common::*;

mod merge;
mod review;
mod scope;

fn approve(k: &TicketKey) -> ReviewDecision {
    ReviewDecision {
        ticket_id: k.clone(),
        decision: ReviewVerdict::Approve,
        ac_feedback: vec![],
        required_changes: vec![],
    }
}

fn cancel(k: &str) -> Intent {
    Intent::CancelTicket(CancelTicket {
        ticket_id: key(k),
        reason: "No longer needed".into(),
    })
}

//! A CLI agent's conversation shown again as text that would not fit its model's window (ADR-155): the agent moved
//! to a model with a smaller window (a new CLI session, since a session resumes only on the model that wrote it), or
//! the CLI could not resume. The older part is folded into notes piece by piece by the agent's own model, the way the
//! API loop folds it, before the run starts; the ticket's work order and the recent steps stay as they are.

use agenium_backend::{estimate_tokens, pieces, WorkOrder, COMPACTED, FOLD};
use agenium_core::*;
use agenium_llm::{ChatRequest, Message, Role};

use super::Runtime;

/// Share of the window a conversation shown as text may take before it is folded.
const FIT_PERCENT: u64 = 70;

/// Share of the window the recent steps kept verbatim may take.
const KEEP_PERCENT: u64 = 15;

/// Tokens the notes may take, and the reply that rewrites them (each fold call carries the notes so far).
const NOTES_TOKENS: u64 = 8_000;

fn tokens(messages: &[Message], system: &WorkOrder) -> u64 {
    estimate_tokens(&ChatRequest {
        model: String::new(),
        system: system.system.clone(),
        messages: messages.to_vec(),
        tools: vec![],
        effort: Effort::HIGH,
        max_tokens: 0,
        cache_key: None,
        long_cache: false,
        tools_off: false,
    })
}

/// Where the fold stops: the recent steps start there, and the last message (the turn's instruction) is never folded,
/// even when it alone outgrows the budget. `None` when there is nothing before it to fold.
fn fold_end(order: &WorkOrder, window: u64) -> Option<usize> {
    let end = recent_start(order, window * KEEP_PERCENT / 100)
        .min(order.messages.len().saturating_sub(1));
    (end > 1).then_some(end)
}

/// The notes as the next fold call and the folded order carry them: at most `NOTES_TOKENS`.
fn capped(notes: &str) -> String {
    notes.chars().take((NOTES_TOKENS * 4) as usize).collect()
}

/// Where the recent steps start: the earliest assistant turn from which the rest takes at most `budget` tokens.
fn recent_start(order: &WorkOrder, budget: u64) -> usize {
    let mut start = order.messages.len();
    for i in (2..order.messages.len()).rev() {
        if tokens(
            &order.messages[i..],
            &WorkOrder {
                system: vec![],
                messages: vec![],
            },
        ) > budget
        {
            break;
        }
        if order.messages[i].role == Role::Assistant {
            start = i;
        }
    }
    start
}

impl Runtime {
    /// `order` as the CLI will be shown it, fitted to `window`: unchanged when it fits, or when the CLI resumes its
    /// own session (it is then sent only what is new and keeps its own context).
    pub(super) async fn fit_cli_order(
        &self,
        agent: &agenium_store::core::agents::Agent,
        session: Option<(
            &dyn agenium_cli_backend::Driver,
            &agenium_cli_backend::run::SessionStore,
        )>,
        order: WorkOrder,
        window: u64,
    ) -> WorkOrder {
        let label = format!("{}:{}", agent.model.provider, agent.model.model);
        if window == 0
            || order.messages.len() < 3
            || session.is_some_and(|(d, s)| s.would_resume(d, &label))
            || tokens(&order.messages, &order) <= window * FIT_PERCENT / 100
        {
            return order;
        }
        let Some(split) = fold_end(&order, window) else {
            return order;
        };
        let head = tokens(&order.messages[..1], &order);
        if head > window * FIT_PERCENT / 100 {
            // The first message (the work order) is kept whole: nothing folded after it would make the run fit.
            tracing::warn!(agent = %agent.name, head, window, "the first message alone outgrows the model's window");
            return order;
        }
        // The first message, the notes so far and the reply that rewrites them are in every fold call.
        let fixed = head + 2 * NOTES_TOKENS;
        let room = window.saturating_sub(fixed) * 60 / 100;
        let parts = pieces(&order.messages[1..split], (room * 4) as usize);
        let total = parts.len();
        let mut notes = String::new();
        for (k, piece) in parts.iter().enumerate() {
            let ask = format!(
                "{FOLD}\n\n<notes>\n{}\n</notes>\n\n<conversation piece=\"{} of {total}\">\n{piece}</conversation>",
                notes.trim(),
                k + 1
            );
            match self
                .side_chat(
                    "fold",
                    None,
                    agent.backend,
                    &agent.model,
                    "You keep an agent's working notes. Answer with the notes only.",
                    vec![order.messages[0].clone(), Message::user_text(ask)],
                    NOTES_TOKENS as u32,
                )
                .await
            {
                Ok(a) if !a.text.trim().is_empty() => notes = capped(&a.text),
                Ok(_) => return order,
                Err(e) => {
                    tracing::warn!(error = %e, "the conversation could not be folded for the CLI");
                    return order;
                }
            }
        }
        tracing::info!(pieces = total, agent = %agent.name, "CLI conversation folded to fit its model's window");
        let mut messages = vec![order.messages[0].clone()];
        messages.push(Message::user_text(format!(
            "{COMPACTED}\n\n{}",
            notes.trim()
        )));
        messages.extend(order.messages[split..].iter().cloned());
        let folded = WorkOrder {
            system: order.system,
            messages,
        };
        let size = tokens(&folded.messages, &folded);
        if size > window {
            tracing::warn!(agent = %agent.name, size, window, "the folded conversation is still over the model's window");
        }
        folded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order(sizes: &[(Role, usize)]) -> WorkOrder {
        WorkOrder {
            system: vec![],
            messages: sizes
                .iter()
                .map(|(r, n)| {
                    let t = "x".repeat(*n);
                    if *r == Role::User {
                        Message::user_text(t)
                    } else {
                        Message::assistant_text(t)
                    }
                })
                .collect(),
        }
    }

    #[test]
    fn the_last_message_is_never_folded() {
        // The last answer alone outgrows the kept share (15 % of 32 000 tokens): the fold still stops before the
        // user's new message instead of taking it with the rest.
        let o = order(&[
            (Role::User, 4_000),
            (Role::Assistant, 10_000),
            (Role::User, 400),
            (Role::Assistant, 25_000),
            (Role::User, 200),
        ]);
        assert_eq!(fold_end(&o, 32_000), Some(4));
        // Three messages ending with the instruction: only the middle one may be folded.
        let o = order(&[
            (Role::User, 4_000),
            (Role::Assistant, 90_000),
            (Role::User, 200),
        ]);
        assert_eq!(fold_end(&o, 32_000), Some(2));
        // Nothing before the instruction but the first message: nothing to fold.
        let o = order(&[(Role::User, 4_000), (Role::User, 200)]);
        assert_eq!(fold_end(&o, 32_000), None);
        assert_eq!(
            capped(&"n".repeat(100_000)).len(),
            (NOTES_TOKENS * 4) as usize
        );
    }
}

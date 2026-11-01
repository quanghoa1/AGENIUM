//! ADR-155: a conversation resumed on a model with a smaller window is folded to fit before anything is sent.

use super::*;
use std::sync::Mutex;

#[derive(Default)]
struct Rec {
    replaced: Mutex<Option<usize>>,
    context: Mutex<Vec<(u64, u64)>>,
}

impl Observer for Rec {
    fn context(&self, used: u64, window: u64) {
        self.context.lock().unwrap().push((used, window));
    }
    fn replaced(&self, messages: &[Message]) {
        *self.replaced.lock().unwrap() = Some(messages.len());
    }
}

/// The agent worked on a 1M-token model; it now runs on one with a 20 000-token window. The resumed conversation
/// (about 60 000 tokens) is folded piece by piece, every notes request fitting the small window, the recent steps
/// kept; the stored conversation is replaced, and the run goes on to its report.
#[tokio::test]
async fn a_conversation_too_big_for_the_new_model_is_folded_to_fit() {
    let (_t, ctx, tools, proto) = harness();
    let mut order = order();
    for i in 0..60 {
        let text = format!("step {i}: {}", "worked on the cart module ".repeat(160));
        order.messages.push(if i % 2 == 0 {
            Message {
                role: Role::Assistant,
                parts: vec![Part::Text { text }],
            }
        } else {
            Message::user_text(text)
        });
    }
    order
        .messages
        .push(Message::user_text("Your run resumes. step 60: keep going"));
    let window = 20_000u64;
    let before = order.messages.len();
    let p = ScriptedProvider::new("m");
    let folds = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    for _ in 0..40 {
        let folds = folds.clone();
        p.push(Box::new(move |req: &ChatRequest| {
            assert!(
                estimate_tokens(req) <= window,
                "every request fits the window: {}",
                estimate_tokens(req)
            );
            let last = req.messages.last().unwrap().text();
            if last.contains("<conversation piece") {
                folds.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                return Ok(agenium_llm::mock::calls_response(
                    "## Goal\nFinish the cart.\n## Progress (Done / In progress / Blocked)\nDone: steps so far.",
                    vec![],
                ));
            }
            // After the fold: the notes in place of the older part, the work order and the recent steps kept.
            let all: String = req.messages.iter().map(|m| m.text()).collect();
            assert!(all.contains("[Context compacted") && all.contains("Finish the cart"));
            assert!(all.contains("# Ticket AGN-1") && all.contains("step 60: keep going"));
            Ok(agenium_llm::mock::calls_response(
                "",
                vec![("submit_report", json!({"ticket_id": "AGN-1"}))],
            ))
        }));
    }
    let rec = Rec::default();
    let mut c = cfg();
    c.context_window = window;
    let mut r = run(&p, &ctx, &tools, &proto, c);
    r.observer = &rec;
    let report = r.run(&order).await;
    assert_eq!(report.outcome, Outcome::Completed, "{:?}", report.outcome);
    assert!(folds.load(std::sync::atomic::Ordering::SeqCst) >= 3);
    let kept = rec
        .replaced
        .lock()
        .unwrap()
        .expect("the stored conversation is replaced");
    assert!(kept < before, "{kept} of {before}");
    // The gauge: the estimate before the fold was over the window, the one after it under.
    let ctx = rec.context.lock().unwrap().clone();
    assert!(ctx[0].0 > window && ctx[0].1 == window, "{ctx:?}");
    assert!(ctx[1].0 < window * 85 / 100, "{ctx:?}");
}

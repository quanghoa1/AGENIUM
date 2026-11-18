//! ADR-164: in an API run a command still going at its timeout goes on as a job the loop waits for, and a report
//! waits for the agent's checklist.

use super::*;

fn slow(ctx: &ToolCtx) -> &'static str {
    match ctx.executor.shell() {
        agenium_sandbox::Shell::Cmd => "ping -n 5 127.0.0.1 > nul & echo done",
        agenium_sandbox::Shell::PowerShell(_) => "Start-Sleep -Seconds 4; echo done",
        agenium_sandbox::Shell::Bash(_) => "sleep 4; echo done",
    }
}

fn texts(m: &Message) -> String {
    m.parts
        .iter()
        .map(|p| match p {
            Part::Text { text } => text.clone(),
            Part::ToolResult { content, .. } => content.clone(),
            _ => String::new(),
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_command_past_its_timeout_goes_on_and_its_end_comes_to_the_agent() {
    let (_t, mut ctx, tools, proto) = harness();
    ctx.awaits_commands = true;
    let cmd = slow(&ctx);
    let p = ScriptedProvider::new("m");
    p.push_calls(
        "",
        vec![("shell", json!({"command": cmd, "timeout_ms": 1000}))],
    );
    // A report while it runs is refused; ending the turn waits for it.
    p.push_calls("", vec![("submit_report", json!({"ticket_id": "AGN-1"}))]);
    p.push_calls("I wait for the tests.", vec![]);
    p.push_calls(
        "",
        vec![("submit_report", json!({"ticket_id": "AGN-1", "n": 2}))],
    );
    let r = run(&p, &ctx, &tools, &proto, cfg()).run(&order()).await;
    assert_eq!(r.outcome, Outcome::Completed);
    let reqs = p.requests();
    let first = texts(reqs[1].messages.last().unwrap());
    assert!(
        first.contains("goes on as job 1") && first.contains("time limit 600 s"),
        "{first}"
    );
    let refused = texts(reqs[2].messages.last().unwrap());
    assert!(refused.contains("commands_running"), "{refused}");
    let ended = texts(reqs[3].messages.last().unwrap());
    assert!(
        ended.contains("has ended: exit code 0") && ended.contains("done"),
        "the end, not a nudge: {ended}"
    );
    assert_eq!(
        proto.calls.lock().unwrap().len(),
        1,
        "one report reached the protocol"
    );
}

#[tokio::test]
async fn a_report_with_the_checklist_unfinished_goes_back_twice_then_through() {
    let (_t, ctx, tools, proto) = harness();
    let p = ScriptedProvider::new("m");
    p.push_calls(
        "",
        vec![(
            "todo",
            json!({"items": [{"text": "write it", "status": "done"}, {"text": "run the tests", "status": "pending"}]}),
        )],
    );
    for n in 0..3 {
        p.push_calls(
            "",
            vec![("submit_report", json!({"ticket_id": "AGN-1", "n": n}))],
        );
    }
    let r = run(&p, &ctx, &tools, &proto, cfg()).run(&order()).await;
    assert_eq!(r.outcome, Outcome::Completed);
    let reqs = p.requests();
    let refused = texts(reqs[2].messages.last().unwrap());
    assert!(
        refused.contains("unfinished_todo")
            && refused.contains("- run the tests")
            && !refused.contains("write it")
            && refused.contains("refusal 1 of 2"),
        "{refused}"
    );
    assert!(texts(reqs[3].messages.last().unwrap()).contains("refusal 2 of 2"));
    assert_eq!(
        proto.calls.lock().unwrap().len(),
        1,
        "the third went through"
    );

    // A finished checklist is no reason to refuse.
    let (_t, ctx, tools, proto) = harness();
    let p = ScriptedProvider::new("m");
    p.push_calls(
        "",
        vec![(
            "todo",
            json!({"items": [{"text": "write it", "status": "done"}]}),
        )],
    );
    p.push_calls("", vec![("submit_report", json!({"ticket_id": "AGN-1"}))]);
    let r = run(&p, &ctx, &tools, &proto, cfg()).run(&order()).await;
    assert_eq!((r.outcome, r.steps), (Outcome::Completed, 2));
}

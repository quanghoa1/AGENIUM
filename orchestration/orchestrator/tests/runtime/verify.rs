//! A check backs a criterion only for the files it ran on (ADR-148): a worker that changes a file with a shell command
//! after its check, no edit tool involved, cannot have the check approved on its word.

use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_file_changed_by_a_shell_command_after_the_check_makes_it_stale() {
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(AutoApprove));
    let (project, _agent) = solo_with_tickets(&app, &["PI-1"]);
    let check = "bash -c \"test -s notes.txt\"";

    // --- work run: write, check, change the file with a shell command, report the check
    p.push_calls(
        "",
        vec![(
            "write_file",
            json!({"path": "notes.txt", "content": "hello\n"}),
        )],
    );
    p.push_calls("", vec![("shell", json!({ "command": check }))]);
    p.push_calls(
        "",
        vec![("shell", json!({"command": "echo bye > notes.txt"}))],
    );
    let mut rep = report("AGN-1");
    rep.acceptance[0].evidence = format!("`{check}` exits 0");
    p.push_calls(
        "",
        vec![("submit_report", serde_json::to_value(&rep).unwrap())],
    );
    // --- review run: an approval on the report's word is refused; with the reviewer's own check it goes through
    p.push_calls(
        "",
        vec![(
            "review_ticket",
            json!({"ticket_id": "AGN-1", "decision": "Approve"}),
        )],
    );
    p.push(Box::new(|req| {
        let last = req.messages.last().unwrap();
        let refused = matches!(&last.parts[0], agenium_llm::Part::ToolResult { content, .. } if content.contains("ran only before the last change"));
        assert!(refused, "an approval on a stale check is refused");
        Ok(calls_response(
            "",
            vec![(
                "review_ticket",
                json!({"ticket_id": "AGN-1", "decision": "Approve", "ac_feedback": [
                    {"ac_id": "AC-1", "feedback": "Read notes.txt in the worktree: it is not empty."}
                ]}),
            )],
        ))
    }));
    p.push_calls(
        "",
        vec![(
            "close_ticket",
            json!({"ticket_id": "AGN-1", "reason": "Met, merged"}),
        )],
    );
    rt.drive(project, 5).await.unwrap();
    assert_eq!(p.remaining(), 0, "every scripted turn was used");
    let t = app.ticket(project, &key("AGN-1")).unwrap();
    assert_eq!(t.status, TicketStatus::Closed);

    // The check's log entry carries the tree it ran on; the shell write does not (it is no check).
    let logs = app
        .store()
        .read(|c| {
            let mut out = Vec::new();
            for r in runs::for_ticket(c, t.id)? {
                out.extend(runs::log_entries(c, r.id)?);
            }
            Ok(out)
        })
        .unwrap();
    let shells: Vec<_> = logs
        .iter()
        .filter(|e| e.kind == "tool" && e.data["tool"] == "shell")
        .collect();
    assert_eq!(shells.len(), 2);
    assert!(shells[0].data["tree"].is_string(), "{:?}", shells[0].data);
    assert!(shells[1].data.get("tree").is_none());
}

//! ADR-162, rules learned from babysitter: a run after a reopen diagnoses before it fixes; the project's pre-merge
//! check runs when the worker submits, and its failure goes back to the worker in the same run (capped).

use super::*;
use agenium_core::comment::CommentBody;
use agenium_store::core::comments;

const DIAGNOSE: &str = "## Before you change code again";

fn lead_ctx(project: ProjectId, agent: AgentId) -> agenium_orchestrator::CallCtx {
    agenium_orchestrator::CallCtx {
        project,
        actor: Actor::Lead(agent),
        intent_id: None,
        run: None,
    }
}

fn system_texts(app: &agenium_orchestrator::App, project: ProjectId, k: &str) -> Vec<String> {
    let t = app.ticket(project, &key(k)).unwrap();
    app.store()
        .read(|c| comments::for_ticket(c, t.id))
        .unwrap()
        .into_iter()
        .filter_map(|c| match c.body {
            CommentBody::System { text } => Some(text),
            _ => None,
        })
        .collect()
}

fn last_tool_result(req: &agenium_llm::ChatRequest) -> String {
    match &req.messages.last().unwrap().parts[0] {
        agenium_llm::Part::ToolResult { content, .. } => content.clone(),
        other => panic!("not a tool result: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_run_after_a_reopen_is_told_to_diagnose_first() {
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(AutoApprove));
    let (project, agent) = solo_with_tickets(&app, &["PI-1"]);

    // First run: no diagnosis asked.
    p.push(Box::new(|req| {
        let t = req.messages[0].text();
        assert!(
            !t.contains(DIAGNOSE),
            "a first run is not told to diagnose: {t}"
        );
        Ok(calls_response(
            "",
            vec![(
                "submit_report",
                serde_json::to_value(report("AGN-1")).unwrap(),
            )],
        ))
    }));
    assert_eq!(
        rt.run_ticket(project, &key("AGN-1")).await.unwrap().outcome,
        "Completed"
    );

    let reopen = Intent::parse(
        "review_ticket",
        json!({"ticket_id": "AGN-1", "decision": "Reopen",
               "ac_feedback": [{"ac_id": "AC-1", "feedback": "running the command still prints the old text"}]}),
        RoleKind::Lead,
        true,
    )
    .unwrap();
    app.apply(&lead_ctx(project, agent), reopen).unwrap();

    // The run after the reopen: told to reproduce, weigh causes and cite evidence, and reports it.
    p.push(Box::new(|req| {
        let t = req.messages[0].text();
        assert!(
            t.contains(DIAGNOSE) && t.contains("2-3 candidate causes"),
            "{t}"
        );
        let mut rep = report("AGN-1");
        rep.diagnosis = vec![
            "reproduced: the command printed HELLO; ruled out the cache (cleared, same output)"
                .into(),
        ];
        Ok(calls_response(
            "",
            vec![("submit_report", serde_json::to_value(&rep).unwrap())],
        ))
    }));
    assert_eq!(
        rt.run_ticket(project, &key("AGN-1")).await.unwrap().outcome,
        "Completed"
    );
    assert_eq!(p.remaining(), 0);
    let t = app.ticket(project, &key("AGN-1")).unwrap();
    let last = app
        .store()
        .read(|c| comments::for_ticket(c, t.id))
        .unwrap()
        .into_iter()
        .rev()
        .find_map(|c| match c.body {
            CommentBody::Report(r) => Some(r),
            _ => None,
        })
        .unwrap();
    assert_eq!(last.diagnosis.len(), 1, "the report carries the diagnosis");
}

/// Passes once the worktree holds `fixed.txt`; `ready: false` stands for a check that cannot run unattended.
struct FileCheck {
    ready: bool,
}
impl agenium_vcs::CheckRunner for FileCheck {
    fn run(&self, worktree: &std::path::Path) -> agenium_vcs::CheckOutcome {
        let ok = worktree.join("fixed.txt").exists();
        agenium_vcs::CheckOutcome {
            ok,
            log: if ok {
                "1 passed".into()
            } else {
                "FAILED test_real_path: expected 'hello'".into()
            },
        }
    }
    fn ready(&self) -> Result<(), String> {
        if self.ready {
            Ok(())
        } else {
            Err("no sandbox backend".into())
        }
    }
}
impl agenium_orchestrator::CheckFactory for FileCheck {
    fn for_project(
        &self,
        _: &agenium_store::core::projects::Project,
        _: &agenium_orchestrator::manifest::ProjectManifest,
        _: bool,
    ) -> Arc<dyn agenium_vcs::CheckRunner> {
        Arc::new(FileCheck { ready: self.ready })
    }
}

/// A solo project with one assigned ticket and `[checks] pre_merge` set, its checks from [`FileCheck`].
fn checked_project(
    f: &Fixture,
    p: &Arc<ScriptedProvider>,
    ready: bool,
) -> (Arc<agenium_orchestrator::App>, Runtime, ProjectId) {
    let (app, rt) = runtime_checked(
        f,
        p.clone(),
        Arc::new(AutoApprove),
        Arc::new(FileCheck { ready }),
    );
    let (project, _) = solo_with_tickets(&app, &["PI-1"]);
    let dir = std::path::PathBuf::from(app.project(project).unwrap().path);
    let mut m = agenium_orchestrator::manifest::ProjectManifest::load(&dir)
        .unwrap()
        .unwrap();
    m.checks.pre_merge = "python -m pytest".into();
    m.save(&dir).unwrap();
    (app, rt, project)
}

fn submit(summary: &str) -> (&'static str, serde_json::Value) {
    let mut rep = report("AGN-1");
    rep.summary = summary.into();
    ("submit_report", serde_json::to_value(&rep).unwrap())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failing_check_refuses_the_submit_until_the_fix() {
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt, project) = checked_project(&f, &p, true);
    p.push_calls(
        "",
        vec![("write_file", json!({"path": "work.txt", "content": "x\n"}))],
    );
    p.push_calls("", vec![submit("done")]);
    p.push(Box::new(|req| {
        let r = last_tool_result(req);
        assert!(
            r.contains("check_failed") || r.contains("pre-merge check"),
            "{r}"
        );
        assert!(
            r.contains("FAILED test_real_path") && r.contains("refusal 1 of 2"),
            "{r}"
        );
        Ok(calls_response(
            "",
            vec![(
                "write_file",
                json!({"path": "fixed.txt", "content": "ok\n"}),
            )],
        ))
    }));
    p.push_calls("", vec![submit("fixed")]);
    assert_eq!(
        rt.run_ticket(project, &key("AGN-1")).await.unwrap().outcome,
        "Completed"
    );
    assert_eq!(p.remaining(), 0, "every scripted turn was used");
    assert_eq!(
        app.ticket(project, &key("AGN-1")).unwrap().status,
        TicketStatus::Resolved
    );
    assert!(system_texts(&app, project, "AGN-1")
        .iter()
        .all(|t| !t.contains("failing pre-merge check")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn after_two_refusals_the_report_goes_through_marked_failing() {
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt, project) = checked_project(&f, &p, true);
    p.push_calls(
        "",
        vec![("write_file", json!({"path": "work.txt", "content": "x\n"}))],
    );
    p.push_calls("", vec![submit("first")]);
    p.push_calls("", vec![submit("second")]);
    p.push(Box::new(|req| {
        assert!(last_tool_result(req).contains("refusal 2 of 2"));
        Ok(calls_response("", vec![submit("third")]))
    }));
    assert_eq!(
        rt.run_ticket(project, &key("AGN-1")).await.unwrap().outcome,
        "Completed"
    );
    assert_eq!(p.remaining(), 0);
    assert_eq!(
        app.ticket(project, &key("AGN-1")).unwrap().status,
        TicketStatus::Resolved
    );
    let notes = system_texts(&app, project, "AGN-1");
    assert!(
        notes.iter().any(|t| t.contains("Submitted with a failing pre-merge check") && t.contains("FAILED")),
        "{notes:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_check_that_cannot_run_does_not_block_the_submit() {
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt, project) = checked_project(&f, &p, false);
    p.push_calls(
        "",
        vec![("write_file", json!({"path": "work.txt", "content": "x\n"}))],
    );
    p.push_calls("", vec![submit("done")]);
    assert_eq!(
        rt.run_ticket(project, &key("AGN-1")).await.unwrap().outcome,
        "Completed"
    );
    assert_eq!(
        app.ticket(project, &key("AGN-1")).unwrap().status,
        TicketStatus::Resolved
    );
    let notes = system_texts(&app, project, "AGN-1");
    assert!(
        notes
            .iter()
            .any(|t| t.contains("did not run at submit") && t.contains("no sandbox backend")),
        "{notes:?}"
    );
}

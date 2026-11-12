//! ADR-161: a command that leaves the sandbox goes to the user (who may allow it always for the project), runs at once
//! when the user chose to approve these themselves, and never under "sandbox only, never ask".

use super::*;

/// Allows always, keeping what it was asked.
#[derive(Default)]
struct Always(std::sync::Mutex<Vec<String>>);

#[async_trait]
impl Approver for Always {
    async fn approve_command(
        &self,
        _: ProjectId,
        _: Option<&TicketKey>,
        _: &str,
        command: &str,
        _: &std::path::Path,
    ) -> Approval {
        self.0.lock().unwrap().push(command.to_owned());
        Approval::AllowAlways
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_command_leaves_the_sandbox_as_the_user_decides() {
    use agenium_config::ApprovalPolicy;
    use agenium_orchestrator::runtime::bridge::RunGate;
    use agenium_tools::{CommandGate, GateDecision};
    let f = fixture();
    let (app, _rt) = runtime(
        &f,
        Arc::new(ScriptedProvider::new("mock")),
        Arc::new(Denier),
    );
    let (project, _) = solo(&app);
    let gate = |approver: Arc<dyn Approver>| RunGate {
        app: app.clone(),
        project,
        ticket: None,
        agent: "ada".into(),
        approver,
    };
    let cwd = f.root.clone();
    let why = "the sandbox blocked it: docker: Operation not permitted";

    // The user is asked, with what the sandbox blocked; "always" lets it out next time without asking.
    let always = Arc::new(Always::default());
    assert_eq!(
        gate(always.clone()).escalate("docker ps", &cwd, why).await,
        GateDecision::Allow
    );
    let asked = always.0.lock().unwrap().clone();
    assert_eq!(asked.len(), 1);
    assert!(
        asked[0].contains("outside the sandbox")
            && asked[0].contains(why)
            && asked[0].ends_with("docker ps"),
        "{asked:?}"
    );
    assert_eq!(
        gate(Arc::new(Denier))
            .escalate("docker ps", &cwd, why)
            .await,
        GateDecision::Allow,
        "allowed always for the project"
    );
    let d = gate(Arc::new(Denier))
        .escalate("docker compose up", &cwd, why)
        .await;
    assert!(
        matches!(&d, GateDecision::Deny(r) if r.contains("not now")),
        "another command is asked: {d:?}"
    );

    // Approved by the user's standing choice: nobody is asked.
    app.update_config(|c| c.sandbox.auto_escalate = true)
        .unwrap();
    assert_eq!(
        gate(Arc::new(Denier))
            .escalate("docker compose up", &cwd, why)
            .await,
        GateDecision::Allow
    );

    // "Sandbox only, never ask": nothing leaves it, automatic approval or not.
    app.update_config(|c| c.sandbox.approval = ApprovalPolicy::Never)
        .unwrap();
    let d = gate(Arc::new(Always::default()))
        .escalate("docker ps", &cwd, why)
        .await;
    assert!(
        matches!(&d, GateDecision::Deny(r) if r.contains("never ask")),
        "{d:?}"
    );
}

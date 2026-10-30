//! The Lead: the spec and the plan, roles and ownership, permissions, the inbox.

use super::*;

#[test]
/// ADR-63: the team's spec waits for the user (the agreement); the plan built from it is the Lead's.
fn team_spec_waits_for_the_user_and_the_plan_is_the_leads() {
    let t = team();
    let f = &t.f;
    let lead = Actor::Lead(t.lead);
    let e = f
        .app
        .apply(&ctx(f, lead), Intent::CreatePlan(plan(&[("PI-1", &[])])))
        .unwrap_err();
    assert!(e.to_string().contains("not approved"), "{e}");
    f.app.approve_spec(f.project.id, 1).unwrap();
    // The Lead takes no tickets: an item for its own role could never be worked on.
    let mut own = plan(&[("PI-1", &[])]);
    own.items[0].role = "lead".into();
    let e = f
        .app
        .apply(&ctx(f, lead), Intent::CreatePlan(own))
        .unwrap_err();
    assert!(
        e.to_string().contains("the Lead's own role") && e.to_string().contains("(api, web)"),
        "{e}"
    );
    let out = f
        .app
        .apply(&ctx(f, lead), Intent::CreatePlan(plan(&[("PI-1", &[])])))
        .unwrap();
    assert_eq!(out.result["approved"], true);
    f.app
        .apply(
            &ctx(f, lead),
            Intent::CreateTicket(new_ticket("PI-1", "x", &[], &[])),
        )
        .unwrap();
    // The worker's task carries the requirements its ticket serves (it cannot read the spec itself).
    let tk = f.app.ticket(f.project.id, &key("AGN-1")).unwrap();
    let req = f.app.ticket_requirements(f.project.id, &tk).unwrap();
    assert!(
        req.contains("Goals: Greet users")
            && req.contains("- REQ-1: A greeting file exists")
            && req.contains("  - AC-1: hello.txt says hello"),
        "{req}"
    );
    // A plan revision that only adds an item is minor: the Lead approves it itself.
    let mut p2 = plan(&[("PI-1", &[]), ("PI-2", &["PI-1"])]);
    let rev = agenium_core::plan::PlanRevision {
        base_version: 1,
        reason: "split work".into(),
        milestones: p2.milestones.clone(),
        items: p2.items.clone(),
    };
    let out = f.app.apply(&ctx(f, lead), Intent::RevisePlan(rev)).unwrap();
    assert_eq!(
        (
            out.result["approved"].clone(),
            out.result["scope_change"].clone()
        ),
        (serde_json::json!(true), serde_json::json!(false))
    );
    // Changing acceptance criteria is a scope change, but inside the agreed spec: the Lead approves it (ADR-63).
    p2.items[0].acceptance[0].text = "different".into();
    let rev = agenium_core::plan::PlanRevision {
        base_version: 2,
        reason: "change AC".into(),
        milestones: p2.milestones,
        items: p2.items,
    };
    let out = f.app.apply(&ctx(f, lead), Intent::RevisePlan(rev)).unwrap();
    assert_eq!(
        (
            out.result["approved"].clone(),
            out.result["scope_change"].clone()
        ),
        (serde_json::json!(true), serde_json::json!(true))
    );
}

#[test]
fn roles_and_ownership_are_enforced() {
    let t = team();
    planned(&t);
    let f = &t.f;
    let (lead, web, api) = (
        Actor::Lead(t.lead),
        Actor::Worker(t.web),
        Actor::Worker(t.api),
    );
    // Workers cannot use Lead tools, even when the role check of the tool catalogue is bypassed.
    let e = f
        .app
        .apply(
            &ctx(f, web),
            Intent::CreateTicket(new_ticket("PI-1", "x", &[], &[])),
        )
        .unwrap_err();
    assert!(matches!(e, OrchError::Denied(_)), "{e}");
    let mut nt = new_ticket("PI-1", "Web page", &[], &[]);
    nt.required_role = Some("web".into());
    nt.required_tags = vec!["web".into()];
    let out = f
        .app
        .apply(&ctx(f, lead), Intent::CreateTicket(nt))
        .unwrap();
    let k: TicketKey = serde_json::from_value(out.result["ticket_id"].clone()).unwrap();
    // Role/tag matching on assignment.
    let e = f
        .app
        .apply(
            &ctx(f, lead),
            Intent::AssignTicket(AssignTicket {
                ticket_id: k.clone(),
                worker: t.api,
                note: None,
            }),
        )
        .unwrap_err();
    assert!(e.to_string().contains("requires role web"), "{e}");
    let e = f
        .app
        .apply(
            &ctx(f, lead),
            Intent::AssignTicket(AssignTicket {
                ticket_id: k.clone(),
                worker: t.lead,
                note: None,
            }),
        )
        .unwrap_err();
    assert!(e.to_string().contains("not a worker"), "{e}");
    f.app
        .apply(
            &ctx(f, lead),
            Intent::AssignTicket(AssignTicket {
                ticket_id: k.clone(),
                worker: t.web,
                note: Some("Start with AC-1".into()),
            }),
        )
        .unwrap();
    f.app.start_run(f.project.id, &k, "mock", "m").unwrap();
    // Another worker cannot act on it.
    let e = f
        .app
        .apply(&ctx(f, api), Intent::SubmitReport(report(k.as_str())))
        .unwrap_err();
    assert!(matches!(e, OrchError::Denied(_)), "{e}");
    // The assignee's report must cover every acceptance criterion.
    let mut r = report(k.as_str());
    r.acceptance[0].ac_id = "AC-9".into();
    let e = f
        .app
        .apply(&ctx(f, web), Intent::SubmitReport(r))
        .unwrap_err();
    assert!(e.to_string().contains("missing"), "{e}");
    // Workers see only the Lead; the roster is a Lead tool.
    assert!(f
        .app
        .apply(&ctx(f, web), Intent::GetTeam(Default::default()))
        .is_err());
    let roster = f
        .app
        .apply(&ctx(f, lead), Intent::GetTeam(Default::default()))
        .unwrap();
    assert_eq!(roster.result["members"].as_array().unwrap().len(), 3);
    assert_eq!(roster.result["members"][1]["current_ticket"], k.as_str());
    // A user can always act as Lead.
    let e = f
        .app
        .apply(
            &ctx(f, Actor::System),
            Intent::CloseTicket(CloseTicket {
                ticket_id: k.clone(),
                reason: "nope nope".into(),
            }),
        )
        .unwrap_err();
    assert!(matches!(e, OrchError::Denied(_)));
}

#[test]
fn permissions_lead_inside_allowlist_user_outside_and_always_allow() {
    let t = team();
    planned(&t);
    let f = &t.f;
    let (lead, web) = (Actor::Lead(t.lead), Actor::Worker(t.web));
    let k = started(f, t.web, "PI-1", "Needs network", &[], &[]);
    let run = f.app.start_run(f.project.id, &k, "mock", "m").unwrap();
    let req = |domains: &[&str]| PermissionRequest {
        ticket_id: k.clone(),
        kind: PermissionKind::Network,
        command: Some("cargo add oauth2".into()),
        domains: domains.iter().map(|s| s.to_string()).collect(),
        paths: vec![],
        reason: "cargo add oauth2".into(),
    };
    let wctx = CallCtx {
        run: Some(run.run),
        ..ctx(f, web)
    };
    let inside = f
        .app
        .apply(&wctx, Intent::RequestPermission(req(&["crates.io"])))
        .unwrap();
    assert_eq!(
        (inside.run, inside.result["decided_by"].clone()),
        (RunEffect::Park, serde_json::json!("lead"))
    );
    assert_eq!(ticket(f, k.as_str()).status, TicketStatus::Pending);
    let rid: CommentId = serde_json::from_value(inside.result["request_id"].clone()).unwrap();
    f.app
        .apply(
            &ctx(f, lead),
            Intent::ApprovePermission(ApprovePermission {
                ticket_id: k.clone(),
                request_id: rid,
                decision: PermissionDecision::Approve,
                note: None,
            }),
        )
        .unwrap();
    assert_eq!(ticket(f, k.as_str()).status, TicketStatus::InProgress);
    let grants = f
        .app
        .store()
        .read(|c| runs::active_grants(c, f.project.id, Some(ticket(f, k.as_str()).id)))
        .unwrap();
    assert_eq!((grants.len(), grants[0].scope.as_str()), (1, "run"));

    // Outside the allowlist the Lead is refused; the user decides and may grant always-allow.
    f.app
        .finish_run(run.run, "Parked", 1, &UsageSummary::default())
        .unwrap();
    let run2 = f.app.start_run(f.project.id, &k, "mock", "m").unwrap();
    assert!(run2.continuation);
    let wctx = CallCtx {
        run: Some(run2.run),
        ..ctx(f, web)
    };
    let outside = f
        .app
        .apply(&wctx, Intent::RequestPermission(req(&["github.com"])))
        .unwrap();
    assert_eq!(outside.result["decided_by"], "user");
    let rid: CommentId = serde_json::from_value(outside.result["request_id"].clone()).unwrap();
    let e = f
        .app
        .apply(
            &ctx(f, lead),
            Intent::ApprovePermission(ApprovePermission {
                ticket_id: k.clone(),
                request_id: rid,
                decision: PermissionDecision::Approve,
                note: None,
            }),
        )
        .unwrap_err();
    assert!(e.to_string().contains("only the user"), "{e}");
    f.app
        .decide_permission(
            f.project.id,
            &k,
            rid,
            PermissionDecision::Approve,
            true,
            None,
        )
        .unwrap();
    assert!(f
        .app
        .project(f.project.id)
        .unwrap()
        .allowlist
        .contains(&"github.com".to_string()));
    let grants = f
        .app
        .store()
        .read(|c| runs::active_grants(c, f.project.id, None))
        .unwrap();
    assert!(grants
        .iter()
        .any(|g| g.scope == "project" && g.granted_by == "user"));
    assert_eq!(ticket(f, k.as_str()).status, TicketStatus::InProgress);

    // Run grants last while the ticket lives (Resolved may be reopened) and end with it.
    write(&worktree(f, k.as_str()), "deliverables/x", "x");
    f.app
        .apply(&ctx(f, web), Intent::SubmitReport(report(k.as_str())))
        .unwrap();
    let grants = || {
        f.app
            .store()
            .read(|c| runs::active_grants(c, f.project.id, Some(ticket(f, k.as_str()).id)))
            .unwrap()
    };
    assert!(
        grants().iter().any(|g| g.scope == "run"),
        "kept while Resolved"
    );
    f.app
        .apply(
            &ctx(f, lead),
            Intent::CancelTicket(CancelTicket {
                ticket_id: k.clone(),
                reason: "The dependency is not needed".into(),
                replaced_by: None,
            }),
        )
        .unwrap();
    assert!(
        grants().iter().all(|g| g.scope == "project"),
        "run-scoped grants revoked"
    );
}

#[test]
fn lead_inbox_order_sla_and_escalation() {
    let t = team();
    planned(&t);
    let f = &t.f;
    let (lead, web) = (Actor::Lead(t.lead), Actor::Worker(t.web));
    let k = started(f, t.web, "PI-1", "Needs answers", &[], &[]);
    let run = f.app.start_run(f.project.id, &k, "mock", "m").unwrap();
    // A fresh intent id per call (the same id would replay the first result).
    let wctx = || CallCtx {
        run: Some(run.run),
        ..ctx(f, web)
    };
    let ask = |blocking: bool, text: &str| -> CommentId {
        let out = f
            .app
            .apply(
                &wctx(),
                Intent::AskQuestion(Question {
                    ticket_id: k.clone(),
                    category: QuestionCategory::Requirement,
                    question: text.into(),
                    context: String::new(),
                    options: vec![],
                    recommendation: None,
                    blocking,
                    urgency: Urgency::High,
                }),
            )
            .unwrap();
        serde_json::from_value(out.result["comment_id"].clone()).unwrap()
    };
    ask(false, "Should the page be dark by default?");
    let blocking = ask(true, "Which port does the dev server use?");
    f.app
        .apply(
            &wctx(),
            Intent::RequestPermission(PermissionRequest {
                ticket_id: k.clone(),
                kind: PermissionKind::Network,
                command: Some("cargo add oauth2".into()),
                domains: vec!["crates.io".into()],
                paths: vec![],
                reason: "cargo add oauth2".into(),
            }),
        )
        .unwrap();
    // Most urgent first; PI-2 of the approved plan has no ticket yet.
    let kinds: Vec<_> = f
        .app
        .lead_inbox(f.project.id)
        .unwrap()
        .into_iter()
        .map(|i| i.kind)
        .collect();
    use agenium_orchestrator::inbox::InboxKind as K;
    assert_eq!(
        kinds,
        [K::BlockingQuestion, K::Permission, K::Plan, K::Question]
    );

    // SLA: once, only for blocking questions, only after the configured minutes (default 10).
    let now = chrono::Utc::now();
    assert_eq!(f.app.check_sla(f.project.id, now).unwrap(), 0);
    let later = now + chrono::Duration::minutes(11);
    assert_eq!(f.app.check_sla(f.project.id, later).unwrap(), 1);
    assert_eq!(
        f.app.check_sla(f.project.id, later).unwrap(),
        0,
        "noted once"
    );
    let cs = f
        .app
        .store()
        .read(|c| agenium_store::core::comments::for_ticket(c, ticket(f, k.as_str()).id))
        .unwrap();
    assert!(cs
        .iter()
        .any(|c| c.reply_to == Some(blocking) && c.kind == CommentKind::System));
    // Past its SLA the question goes to the user too, while the Lead still has it (it was hidden from the user).
    let for_user: Vec<_> = f
        .app
        .needs_you(f.project.id)
        .unwrap()
        .into_iter()
        .filter_map(|n| n.comment)
        .collect();
    assert_eq!(for_user, [blocking]);
    assert!(f
        .app
        .lead_inbox(f.project.id)
        .unwrap()
        .iter()
        .any(|i| i.comment == Some(blocking)));

    // The Lead escalates: the question stays open for the user and leaves the inbox; the user answers it.
    let esc = |actor: Actor| {
        f.app.apply(
            &ctx(f, actor),
            Intent::AnswerQuestion(AnswerQuestion {
                ticket_id: k.clone(),
                reply_to: blocking,
                answer: "Only you know the deployment port.".into(),
                chosen_option: None,
                escalate: true,
            }),
        )
    };
    assert!(esc(Actor::User).is_err(), "the user answers, not escalates");
    assert_eq!(esc(lead).unwrap().result["escalated"], true);
    let kinds: Vec<_> = f
        .app
        .lead_inbox(f.project.id)
        .unwrap()
        .into_iter()
        .map(|i| i.kind)
        .collect();
    assert_eq!(kinds, [K::Permission, K::Plan, K::Question]);
    assert_eq!(ticket(f, k.as_str()).status, TicketStatus::Pending);
    // In a team the user answers the Lead, never the worker (ADR-122): the answer becomes a message to the Lead and
    // nothing lands on the ticket until the Lead answers there.
    let before = f
        .app
        .store()
        .read(|c| agenium_store::core::comments::for_ticket(c, ticket(f, k.as_str()).id))
        .unwrap()
        .len();
    let routed = f
        .app
        .route_user_answer(f.project.id, &k, blocking, "8080", None)
        .unwrap();
    let agenium_orchestrator::engine::UserAnswer::ToLead(msg) = routed else {
        panic!("a team's answer goes to the Lead: {routed:?}");
    };
    assert!(
        msg.contains("8080") && msg.contains(&blocking.to_string()) && msg.contains(k.as_str()),
        "{msg}"
    );
    assert_eq!(
        f.app
            .store()
            .read(|c| agenium_store::core::comments::for_ticket(c, ticket(f, k.as_str()).id))
            .unwrap()
            .len(),
        before
    );
    f.app
        .apply(
            &ctx(f, lead),
            Intent::AnswerQuestion(AnswerQuestion {
                ticket_id: k.clone(),
                reply_to: blocking,
                answer: "The user says: 8080.".into(),
                chosen_option: None,
                escalate: false,
            }),
        )
        .unwrap();
    // The Lead's answer to an escalated question is the user's decision for the next tickets (ADR-120).
    let decided = f
        .app
        .store()
        .read(|c| agenium_store::core::comments::user_answers(c, f.project.id, 8))
        .unwrap();
    assert_eq!(decided.len(), 1);
    assert!(matches!(&decided[0].0.body, CommentBody::Answer(a) if a.resolution.contains("8080")));
    // Still Pending: the permission request is blocking too.
    assert_eq!(ticket(f, k.as_str()).status, TicketStatus::Pending);
}

mod common;

use agenium_core::forms::*;
use agenium_core::*;
use agenium_orchestrator::*;
use agenium_store::core::runs;
use common::*;

struct Team {
    f: Fixture,
    lead: AgentId,
    web: AgentId,
    api: AgentId,
}

fn team() -> Team {
    let f = fixture();
    // Project network allowlist (manifest) so the Lead can approve requests inside it.
    let mut m = f.app.manifest(&f.project).unwrap();
    m.network.allowlist = vec!["crates.io".into(), "*.npmjs.org".into()];
    m.save(std::path::Path::new(&f.project.path)).unwrap();
    let (_, members) = f
        .app
        .create_team(
            f.project.id,
            "web team",
            &agent_spec("lead", "lead", RoleKind::Lead, &["planning"]),
            &[
                agent_spec("web-1", "web", RoleKind::Worker, &["web"]),
                agent_spec("api-1", "api", RoleKind::Worker, &["rust"]),
            ],
        )
        .unwrap();
    let (lead, web, api) = (members[0].id, members[1].id, members[2].id);
    f.app
        .apply(&ctx(&f, Actor::Lead(lead)), Intent::ProposeSpec(spec()))
        .unwrap();
    Team { f, lead, web, api }
}

#[test]
fn team_spec_and_plan_wait_for_the_user() {
    let t = team();
    let f = &t.f;
    let lead = Actor::Lead(t.lead);
    let e = f
        .app
        .apply(&ctx(f, lead), Intent::CreatePlan(plan(&[("PI-1", &[])])))
        .unwrap_err();
    assert!(e.to_string().contains("not approved"), "{e}");
    f.app.approve_spec(f.project.id, 1).unwrap();
    let out = f
        .app
        .apply(&ctx(f, lead), Intent::CreatePlan(plan(&[("PI-1", &[])])))
        .unwrap();
    assert_eq!(out.result["approved"], false);
    let e = f
        .app
        .apply(
            &ctx(f, lead),
            Intent::CreateTicket(new_ticket("PI-1", "x", &[], &[])),
        )
        .unwrap_err();
    assert!(e.to_string().contains("no approved plan"), "{e}");
    f.app.approve_plan(f.project.id, 1).unwrap();
    f.app
        .apply(
            &ctx(f, lead),
            Intent::CreateTicket(new_ticket("PI-1", "x", &[], &[])),
        )
        .unwrap();
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
    // Changing acceptance criteria is a scope change: the user must approve.
    p2.items[0].acceptance[0].text = "different".into();
    let rev = agenium_core::plan::PlanRevision {
        base_version: 2,
        reason: "change AC".into(),
        milestones: p2.milestones,
        items: p2.items,
    };
    let out = f.app.apply(&ctx(f, lead), Intent::RevisePlan(rev)).unwrap();
    assert_eq!(out.result["approved"], false);
}

fn planned(t: &Team) {
    t.f.app.approve_spec(t.f.project.id, 1).unwrap();
    t.f.app
        .apply(
            &ctx(&t.f, Actor::Lead(t.lead)),
            Intent::CreatePlan(plan(&[("PI-1", &[]), ("PI-2", &[])])),
        )
        .unwrap();
    t.f.app.approve_plan(t.f.project.id, 1).unwrap();
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

    // Run grants end when the ticket is resolved.
    write(&worktree(f, k.as_str()), "deliverables/x", "x");
    f.app
        .apply(&ctx(f, web), Intent::SubmitReport(report(k.as_str())))
        .unwrap();
    let grants = f
        .app
        .store()
        .read(|c| runs::active_grants(c, f.project.id, Some(ticket(f, k.as_str()).id)))
        .unwrap();
    assert!(
        grants.iter().all(|g| g.scope == "project"),
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
    f.app
        .user_answer(f.project.id, &k, blocking, "8080", None)
        .unwrap();
    // Still Pending: the permission request is blocking too.
    assert_eq!(ticket(f, k.as_str()).status, TicketStatus::Pending);
}

#[test]
fn team_from_template_and_write_scope() {
    let f = fixture();
    let out = f
        .app
        .create_team_from_template(f.project.id, "pentest", &model(), None, "acme")
        .unwrap();
    let names: Vec<&str> = out.agents.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        ["acme-lead", "acme-web", "acme-mobile", "acme-recon"]
    );
    assert_eq!(
        f.app.manifest(&f.project).unwrap().project.template,
        "pentest@1"
    );
    let roles = f.app.layout().roles();
    assert!(roles.join("pentest-web").join("ROLE.md").is_file());
    assert_eq!(
        f.app.role_allowed_paths("pentest-web"),
        ["deliverables/web/**", "artifacts/web/**"]
    );
    assert!(
        f.app
            .create_team_from_template(f.project.id, "coding", &model(), None, "again")
            .is_err(),
        "one team per project"
    );

    let (team, members) = f.app.team_of_project(f.project.id).unwrap().unwrap();
    let lead = Actor::Lead(team.lead.unwrap());
    let web = members.iter().find(|a| a.name == "acme-web").unwrap().id;
    f.app
        .apply(&ctx(&f, lead), Intent::ProposeSpec(spec()))
        .unwrap();
    f.app.approve_spec(f.project.id, 1).unwrap();
    f.app
        .apply(&ctx(&f, lead), Intent::CreatePlan(plan(&[("PI-1", &[])])))
        .unwrap();
    f.app.approve_plan(f.project.id, 1).unwrap();
    let k = started(&f, web, "PI-1", "Test the login page", &[], &[]);
    let run = f.app.start_run(f.project.id, &k, "mock", "m").unwrap();
    let wt = worktree(&f, k.as_str());
    write(&wt, "deliverables/web/login.md", "finding");
    // A write outside the role's scope (e.g. by a shell command) blocks the report.
    write(&wt, "deliverables/mobile/app.md", "not mine");
    let mut r = report(k.as_str());
    r.extension = serde_json::json!({"findings": []});
    let submit = |r: &WorkReport| {
        f.app.apply(
            &CallCtx {
                run: Some(run.run),
                ..ctx(&f, Actor::Worker(web))
            },
            Intent::SubmitReport(r.clone()),
        )
    };
    let e = submit(&r).unwrap_err().to_string();
    assert!(
        e.contains("deliverables/mobile/app.md") && !e.contains("login.md"),
        "{e}"
    );
    // The user grants the path; the report goes through.
    let req = f
        .app
        .apply(
            &CallCtx {
                run: Some(run.run),
                ..ctx(&f, Actor::Worker(web))
            },
            Intent::RequestPermission(PermissionRequest {
                ticket_id: k.clone(),
                kind: PermissionKind::Path,
                command: None,
                domains: vec![],
                paths: vec!["deliverables/mobile/app.md".into()],
                reason: "the login finding also affects the mobile client".into(),
            }),
        )
        .unwrap();
    assert_eq!(req.result["decided_by"], "user");
    let rid: CommentId = serde_json::from_value(req.result["request_id"].clone()).unwrap();
    f.app
        .decide_permission(
            f.project.id,
            &k,
            rid,
            PermissionDecision::Approve,
            false,
            None,
        )
        .unwrap();
    assert_eq!(ticket(&f, k.as_str()).status, TicketStatus::InProgress);
    submit(&r).unwrap();
    assert_eq!(ticket(&f, k.as_str()).status, TicketStatus::Resolved);
}

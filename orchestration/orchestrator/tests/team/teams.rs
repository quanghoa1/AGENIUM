//! Teams: templates, names, the planner, what waits for the user, the Lead slot, prices and usage.

use super::*;

#[test]
fn team_from_template_and_write_scope() {
    let f = fixture();
    let out = f
        .app
        .create_team_from_template(f.project.id, "pentest", &model(), None, None, "acme")
        .unwrap();
    let names: Vec<&str> = out.agents.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        [
            "acme-lead",
            "acme-planner",
            "acme-web",
            "acme-mobile",
            "acme-app",
            "acme-reverser",
            "acme-recon"
        ]
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
            .create_team_from_template(f.project.id, "coding", &model(), None, None, "again")
            .is_err(),
        "one team per project"
    );

    let (team, members) = f.app.team_of_project(f.project.id).unwrap().unwrap();
    let lead = Actor::Lead(team.lead.unwrap());
    let web = members.iter().find(|a| a.name == "acme-web").unwrap().id;
    let planner = f.app.planner(f.project.id).unwrap().unwrap().id;
    f.app
        .apply(&ctx(&f, lead), Intent::ProposeSpec(spec()))
        .unwrap();
    f.app.approve_spec(f.project.id, 1).unwrap();
    let p = plan(&[("PI-1", &[])]);
    f.app
        .apply(
            &ctx(&f, Actor::Worker(planner)),
            Intent::SubmitPlanDraft(agenium_core::plan::PlanDraft {
                spec_version: 1,
                milestones: p.milestones.clone(),
                items: p.items.clone(),
                rationale: "One item.".into(),
            }),
        )
        .unwrap();
    f.app.apply(&ctx(&f, lead), Intent::CreatePlan(p)).unwrap();
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

    // The grant lasts while the ticket lives: reopened, the file it allowed is on the branch and still the ticket's.
    f.app
        .apply(
            &ctx(&f, lead),
            Intent::ReviewTicket(ReviewDecision {
                ticket_id: k.clone(),
                decision: ReviewVerdict::Reopen,
                ac_feedback: vec![AcFeedback {
                    ac_id: "AC-1".into(),
                    feedback: "Give the finding its severity".into(),
                }],
                required_changes: vec![],
            }),
        )
        .unwrap();
    f.app
        .finish_run(run.run, "Completed", 1, &UsageSummary::default())
        .unwrap();
    let run2 = f.app.start_run(f.project.id, &k, "mock", "m").unwrap();
    write(&wt, "deliverables/web/login.md", "finding, severity high");
    f.app
        .apply(
            &CallCtx {
                run: Some(run2.run),
                ..ctx(&f, Actor::Worker(web))
            },
            Intent::SubmitReport(r.clone()),
        )
        .unwrap();
    assert_eq!(ticket(&f, k.as_str()).status, TicketStatus::Resolved);
}

/// Without a prefix a template names its agents after their roles; names stay unique across teams (ignoring case),
/// so a taken name gets the next free number and a name the user types twice is refused.
#[test]
fn agent_names_are_role_names_and_unique() {
    let f = fixture();
    let out = f
        .app
        .create_team_from_template(f.project.id, "pentest", &model(), None, None, "")
        .unwrap();
    let names: Vec<&str> = out.agents.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        ["lead", "planner", "web", "mobile", "app", "reverser", "recon"]
    );
    let project = |name: &str| {
        f.app
            .create_project(&agenium_orchestrator::CreateProject {
                name: name.into(),
                path: None,
                key_prefix: None,
                template: None,
                folder: None,
                repo: None,
            })
            .unwrap()
            .id
    };
    let second = project("second");
    let out = f
        .app
        .create_team_from_template(second, "research", &model(), None, None, "")
        .unwrap();
    let names: Vec<&str> = out.agents.iter().map(|(n, _)| n.as_str()).collect();
    assert!(
        names.contains(&"lead-2") && !names.contains(&"lead"),
        "{names:?}"
    );
    let third = project("third");
    let solo = |name: &str| {
        f.app.create_solo(
            third,
            &agenium_orchestrator::CreateAgent {
                name: name.into(),
                role: "solo".into(),
                role_kind: RoleKind::Lead,
                tags: vec![],
                model: model(),
            },
        )
    };
    let e = solo("Web").unwrap_err().to_string();
    assert!(e.contains("already exists"), "{e}");
    solo("ada").unwrap();
}

/// ADR-59: the planner drafts plan v1 once the spec is approved; the Lead reviews the draft and creates the plan;
/// the planner never takes tickets.
#[test]
fn planner_drafts_and_the_lead_creates_the_plan() {
    use agenium_core::plan::PlanDraft;
    use agenium_orchestrator::inbox::InboxKind as K;
    let f = fixture();
    f.app
        .create_team_from_template(f.project.id, "coding", &model(), None, None, "acme")
        .unwrap();
    let (team, members) = f.app.team_of_project(f.project.id).unwrap().unwrap();
    let lead = Actor::Lead(team.lead.unwrap());
    let id = |n: &str| members.iter().find(|a| a.name == n).unwrap().id;
    let (planner, backend) = (id("acme-planner"), id("acme-backend"));
    assert_eq!(f.app.planner(f.project.id).unwrap().unwrap().id, planner);
    let kinds = || -> Vec<K> {
        f.app
            .lead_inbox(f.project.id)
            .unwrap()
            .into_iter()
            .map(|i| i.kind)
            .collect()
    };

    f.app
        .apply(&ctx(&f, lead), Intent::ProposeSpec(spec()))
        .unwrap();
    assert!(kinds().is_empty(), "the spec waits for the user");
    f.app.approve_spec(f.project.id, 1).unwrap();
    assert!(kinds().is_empty(), "the planner drafts first");

    let p = plan(&[("PI-1", &[]), ("PI-2", &["PI-1"])]);
    // A Lead asked in chat does not plan around the planner (ADR-116).
    let early = f
        .app
        .apply(&ctx(&f, lead), Intent::CreatePlan(p.clone()))
        .unwrap_err()
        .to_string();
    assert!(early.contains("acme-planner drafts plan v1"), "{early}");
    let draft = |v: u32| PlanDraft {
        spec_version: v,
        milestones: p.milestones.clone(),
        items: p.items.clone(),
        rationale: "Backend first: the page needs its API; PI-2 depends on PI-1 for that reason."
            .into(),
    };
    let submit =
        |actor: Actor, d: PlanDraft| f.app.apply(&ctx(&f, actor), Intent::SubmitPlanDraft(d));
    assert!(
        submit(Actor::Worker(backend), draft(1)).is_err(),
        "only the planner drafts"
    );
    assert!(
        submit(Actor::Worker(planner), draft(2)).is_err(),
        "only for the approved spec"
    );
    assert_eq!(
        submit(Actor::Worker(planner), draft(1)).unwrap().run,
        RunEffect::End
    );

    let inbox = f.app.lead_inbox(f.project.id).unwrap();
    assert_eq!(
        inbox.iter().map(|i| i.kind).collect::<Vec<_>>(),
        [K::Planning]
    );
    assert!(inbox[0].text.contains("Backend first") && inbox[0].text.contains("create_plan"));
    // The Lead owns the plan: it creates it (here unchanged); the planner is done.
    f.app
        .apply(&ctx(&f, lead), Intent::CreatePlan(p.clone()))
        .unwrap();
    assert_eq!(
        kinds(),
        [K::Plan],
        "the Lead's plan is approved: its tickets come next"
    );
    assert!(
        submit(Actor::Worker(planner), draft(1)).is_err(),
        "no drafts once a plan exists"
    );
    assert_eq!(kinds(), [K::Plan]);

    let out = f
        .app
        .apply(
            &ctx(&f, lead),
            Intent::CreateTicket(new_ticket("PI-1", "API", &[], &[])),
        )
        .unwrap();
    let k: TicketKey = serde_json::from_value(out.result["ticket_id"].clone()).unwrap();
    let assign = |w: AgentId| {
        f.app.apply(
            &ctx(&f, lead),
            Intent::AssignTicket(AssignTicket {
                ticket_id: k.clone(),
                worker: w,
                note: None,
            }),
        )
    };
    assert!(assign(planner).unwrap_err().to_string().contains("planner"));
    assign(backend).unwrap();
    let roster = f
        .app
        .apply(&ctx(&f, lead), Intent::GetTeam(Default::default()))
        .unwrap()
        .result;
    let flag = |n: &str| {
        roster["members"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["name"] == n)
            .unwrap()["planner"]
            .clone()
    };
    assert_eq!(
        (flag("acme-planner"), flag("acme-backend")),
        (serde_json::json!(true), serde_json::json!(false))
    );
}

/// Without a planner, an approved spec without a plan is an inbox item for the Lead.
#[test]
fn approved_spec_without_planner_asks_the_lead_for_a_plan() {
    let t = team();
    let f = &t.f;
    f.app.approve_spec(f.project.id, 1).unwrap();
    let inbox = f.app.lead_inbox(f.project.id).unwrap();
    assert_eq!(inbox.len(), 1);
    assert_eq!(
        inbox[0].kind,
        agenium_orchestrator::inbox::InboxKind::Planning
    );
    assert!(inbox[0].text.contains("create_plan"));
}

#[test]
fn needs_you_lists_what_waits_for_the_user() {
    use agenium_orchestrator::views::NeedKind as N;
    let t = team();
    let f = &t.f;
    let kinds = || -> Vec<N> {
        f.app
            .needs_you(f.project.id)
            .unwrap()
            .into_iter()
            .map(|n| n.kind)
            .collect()
    };
    assert_eq!(kinds(), [N::SpecApproval]);
    f.app.approve_spec(f.project.id, 1).unwrap();
    f.app
        .apply(
            &ctx(f, Actor::Lead(t.lead)),
            Intent::CreatePlan(plan(&[("PI-1", &[])])),
        )
        .unwrap();
    assert!(
        kinds().is_empty(),
        "the plan of an agreed spec waits for nobody (ADR-63)"
    );
    let k = started(f, t.web, "PI-1", "Needs things", &[], &[]);
    let run = f.app.start_run(f.project.id, &k, "mock", "m").unwrap();
    let wctx = || CallCtx {
        run: Some(run.run),
        ..ctx(f, Actor::Worker(t.web))
    };
    // Outside the allowlist: the user decides. Inside: the Lead (not listed).
    for domain in ["github.com", "crates.io"] {
        f.app
            .apply(
                &wctx(),
                Intent::RequestPermission(PermissionRequest {
                    ticket_id: k.clone(),
                    kind: PermissionKind::Network,
                    command: None,
                    domains: vec![domain.into()],
                    paths: vec![],
                    reason: format!("fetch from {domain}"),
                }),
            )
            .unwrap();
    }
    assert_eq!(kinds(), [N::Permission]);
    f.app
        .set_agent_status(t.api, AgentStatus::QuotaExhausted)
        .unwrap();
    let all = f.app.needs_you(f.project.id).unwrap();
    assert_eq!(all[0].kind, N::AgentBlocked, "blocked agents come first");
    assert_eq!(all[0].agent.as_deref(), Some("api-1"));
    assert!(all[1].text.contains("github.com"));
}

#[test]
fn the_user_puts_another_member_in_the_lead_slot() {
    let t = team();
    planned(&t);
    let f = &t.f;
    let k = started(f, t.web, "PI-1", "Web page", &[], &[]);
    let e = f
        .app
        .set_team_lead(f.project.id, t.web)
        .unwrap_err()
        .to_string();
    assert!(
        e.contains(k.as_str()),
        "open tickets must be reassigned first: {e}"
    );
    f.app.set_team_lead(f.project.id, t.api).unwrap();
    let (team, members) = f.app.team_of_project(f.project.id).unwrap().unwrap();
    assert_eq!(team.lead, Some(t.api));
    let kind = |id| members.iter().find(|m| m.id == id).unwrap().role_kind;
    assert_eq!(
        (kind(t.api), kind(t.lead)),
        (RoleKind::Lead, RoleKind::Worker)
    );
    // The new Lead acts as Lead; the old one is a worker now.
    let out = f.app.apply(
        &ctx(f, Actor::Lead(t.api)),
        Intent::GetTeam(Default::default()),
    );
    assert!(out.is_ok());
    let e = f.app.apply(
        &ctx(f, Actor::Lead(t.lead)),
        Intent::GetTeam(Default::default()),
    );
    assert!(e.is_err());
}

/// Per-agent MCP keys: the Lead gives a member its own key (the system key stays the server's value), get_team
/// shows which members have one, other teams and workers cannot, logs mask the key.
#[test]
fn lead_sets_a_member_mcp_key() {
    let t = team();
    let f = &t.f;
    f.app
        .put_mcp_server(
            serde_json::from_value(serde_json::json!({
                "id": "crm", "url": "https://crm.example.com/mcp",
                "headers": {"Authorization": "Bearer lead-system-key-0001"}
            }))
            .unwrap(),
        )
        .unwrap();
    f.app.toggle_agent_mcp(t.web, "crm", true).unwrap();
    let set = |actor, agent, value: Option<&str>| {
        f.app.apply(
            &ctx(f, actor),
            Intent::SetMemberMcpKey(SetMemberMcpKey {
                agent,
                server: "crm".into(),
                key: "Authorization".into(),
                value: value.map(str::to_owned),
            }),
        )
    };
    assert!(
        set(Actor::Worker(t.web), t.web, Some("Bearer x")).is_err(),
        "workers cannot"
    );
    assert!(
        set(Actor::Lead(t.lead), t.api, Some("Bearer x")).is_err(),
        "api-1 does not use crm"
    );
    let out = set(Actor::Lead(t.lead), t.web, Some("Bearer web-own-key-0002")).unwrap();
    assert_eq!(out.result["uses"], "its own key");
    let keys = f.app.agent_mcp_keys(t.web).unwrap();
    assert_eq!(keys[0].own.as_deref(), Some("Bearer web-own-key-0002"));
    assert_eq!(
        keys[0].system.as_deref(),
        Some("Bearer lead-system-key-0001")
    );
    let roster = f
        .app
        .apply(
            &ctx(f, Actor::Lead(t.lead)),
            Intent::GetTeam(Default::default()),
        )
        .unwrap();
    let web = roster.result["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == "web-1")
        .unwrap()
        .clone();
    assert_eq!(web["mcp"][0]["keys"][0]["own_key"], true);
    assert!(
        !roster.result.to_string().contains("web-own-key"),
        "values never reach the Lead's roster"
    );
    assert!(!f
        .app
        .redactor()
        .redact("web-own-key-0002")
        .contains("web-own-key"));
    set(Actor::Lead(t.lead), t.web, None).unwrap();
    assert_eq!(f.app.agent_mcp_keys(t.web).unwrap()[0].own, None);
}

/// ADR-63: a Solo agent's inbox carries what moves the work on (plan items without tickets), so the team loop
/// takes a Solo project to the end too; its questions still go to the user.
#[test]
fn a_solo_inbox_has_the_work_left_to_plan() {
    let f = fixture();
    let a = solo_with_plan(&f, &[("PI-1", &[]), ("PI-2", &[])]);
    let inbox = f.app.lead_inbox(f.project.id).unwrap();
    assert_eq!(
        inbox.iter().map(|i| i.kind).collect::<Vec<_>>(),
        [agenium_orchestrator::inbox::InboxKind::Plan]
    );
    assert!(inbox[0].text.contains("PI-1") && inbox[0].text.contains("PI-2"));
    // An item whose only ticket the Lead cancelled is neither done nor in work. It once came back to the Lead (ADR-64,
    // found on a real run); it now waits for the user in Needs you (ADR-147: put to the Lead every round, a Lead
    // re-created work the user had dropped), and the cancel told the Lead to carry the work on if it is needed.
    let lead = Actor::Lead(a.id);
    for (item, title) in [("PI-1", "one"), ("PI-2", "two")] {
        f.app
            .apply(
                &ctx(&f, lead),
                Intent::CreateTicket(new_ticket(item, title, &[], &[])),
            )
            .unwrap();
    }
    let out = f
        .app
        .apply(
            &ctx(&f, lead),
            Intent::CancelTicket(CancelTicket {
                ticket_id: key("AGN-2"),
                reason: "the Lead cannot take it".into(),
                replaced_by: None,
            }),
        )
        .unwrap();
    assert!(out.result["note"]
        .as_str()
        .is_some_and(|n| n.contains("a new ticket of PI-2 carries it on")));
    let inbox = f.app.lead_inbox(f.project.id).unwrap();
    assert!(
        !inbox
            .iter()
            .any(|i| i.kind == agenium_orchestrator::inbox::InboxKind::Plan),
        "{inbox:?}"
    );
    assert!(f
        .app
        .needs_you(f.project.id)
        .unwrap()
        .iter()
        .any(|n| n.kind == agenium_orchestrator::views::NeedKind::CancelledWork));
}

/// Runs of a model without a price cost 0 until the price catalogue knows the model; storing the catalogue prices
/// them, matched to the configured provider by its URL. The model list shows which models are free.
#[test]
fn the_price_catalogue_prices_earlier_runs() {
    use agenium_llm::pricing::Catalogue;
    let t = team();
    let f = &t.f;
    f.app
        .update_config(|c| {
            c.providers.push(agenium_config::ProviderConfig {
                body: vec![],
                effort_field: None,
                headers: Default::default(),
                id: "go".into(),
                kind: agenium_config::ProviderKind::OpenaiCompat,
                base_url: Some("https://opencode.ai/zen/go/v1".into()),
                api_key_ref: String::new(),
                max_concurrency: 1,
                models: vec![],
                model_caps: vec![],
            })
        })
        .unwrap();
    planned(&t);
    let k = started(f, t.web, "PI-1", "Price me", &[], &[]);
    let run = f
        .app
        .start_run(f.project.id, &k, "go", "deepseek-v4.1-flash")
        .unwrap();
    // Half of the input was served from the cache: it is billed once, at the cache price.
    let usage = UsageSummary {
        tokens_in: 2_000_000,
        tokens_out: 1_000_000,
        cache_read: 1_000_000,
        ..Default::default()
    };
    f.app.finish_run(run.run, "Completed", 3, &usage).unwrap();
    assert!(f.app.price_view().providers["go"].is_empty());
    let c = Catalogue::from_models_dev(
        &serde_json::json!({"opencode-go": {"api": "https://opencode.ai/inference/go/openai/v1", "models": {
            "deepseek-v4.1-flash": {"cost": {"input": 0.15, "output": 0.6, "cache_read": 0.015}},
            "beam-free": {"cost": {"input": 0, "output": 0}}
        }}}),
        "test",
        &now_iso(),
    );
    assert_eq!(f.app.save_price_catalogue(&c).unwrap(), 1);
    let r = f.app.store().read(|c| runs::get(c, run.run)).unwrap();
    assert!(
        (r.usage.estimated_cost_usd - 0.765).abs() < 1e-9,
        "{:?}",
        r.usage
    );
    assert_eq!(f.app.price_unpriced_runs().unwrap(), 0, "priced once");
    let view = f.app.price_view();
    assert!(view.providers["go"]["beam-free"].is_free());
    assert!(!f.app.price_catalogue_stale());
}

/// A vendor's reading is kept per vendor; a failed reading keeps the earlier windows with its error. Rows follow
/// the CLIs installed and the OpenCode Go providers configured.
#[test]
fn vendor_usage_keeps_the_last_good_reading() {
    use agenium_llm::vendor_usage::{UsageWindow, VendorUsage};
    let f = fixture();
    let good = VendorUsage {
        windows: vec![UsageWindow {
            window: "5h".into(),
            used_percent: 42.0,
            resets_at: None,
        }],
        ..VendorUsage::new("codex", "2026-10-25T00:00:00Z")
    };
    f.app.record_vendor_usage(good).unwrap();
    f.app
        .record_vendor_usage(VendorUsage {
            error: Some("codex app-server did not answer in 40 s".into()),
            ..VendorUsage::new("codex", "2026-10-25T00:15:00Z")
        })
        .unwrap();
    let codex = agenium_cli_backend::CliInfo {
        kind: agenium_cli_backend::CliKind::Codex,
        path: Some("codex".into()),
        version: Some("0.163.0".into()),
        login: agenium_cli_backend::Login::LoggedIn,
        compatible: true,
        note: None,
    };
    let rows = f.app.vendor_usage(&[codex]);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].windows[0].used_percent, 42.0);
    assert_eq!(
        rows[0].updated_at, "2026-10-25T00:00:00Z",
        "the good reading's time"
    );
    assert!(rows[0].error.as_deref().unwrap().contains("did not answer"));
}

/// ADR-152: a team from a template can run each role on its own model (the template's `default_model`) when this
/// machine runs it (an API provider in the config, an agent CLI found); otherwise the role takes the team's model.
/// The Lead's model picked on the form wins over its role's.
#[test]
fn a_team_from_a_template_runs_each_role_on_its_own_usable_model() {
    use agenium_orchestrator::team_templates::TeamModels;
    let f = fixture();
    f.app
        .update_config(|c| {
            c.providers.push(agenium_config::ProviderConfig {
                body: vec![],
                effort_field: None,
                headers: Default::default(),
                id: "go".into(),
                kind: agenium_config::ProviderKind::OpenaiCompat,
                base_url: Some("https://opencode.ai/zen/go/v1".into()),
                api_key_ref: String::new(),
                max_concurrency: 1,
                models: vec![],
                model_caps: vec![],
            })
        })
        .unwrap();
    let listed = |id: &str| agenium_llm::ModelInfo {
        id: id.into(),
        ..Default::default()
    };
    f.app
        .record_models(
            &agenium_orchestrator::model_cache::cli_key("claude"),
            Ok(vec![listed("haiku")]),
        )
        .unwrap();
    let mut tpl = f.app.team_template("coding").unwrap();
    tpl.id = "per-role".into();
    let lead = tpl
        .roles
        .iter()
        .position(|r| r.kind == RoleKind::Lead)
        .unwrap();
    let workers: Vec<usize> = (0..tpl.roles.len()).filter(|i| *i != lead).collect();
    tpl.roles[lead].default_model = Some("go/deepseek-v4.1-flash".into());
    tpl.roles[workers[0]].default_model = Some("go/deepseek-v4.1-flash".into());
    tpl.roles[workers[1]].default_model = Some("claude/haiku".into());
    tpl.roles[workers[2]].default_model = Some("codex/gpt-6-luna".into()); // Codex not found here
    f.app.save_team_template(&tpl).unwrap();
    assert!(f.app.usable_model("go/deepseek-v4.1-flash").is_some());
    assert_eq!(
        f.app.usable_model("claude/haiku").map(|(b, _)| b),
        Some(BackendKind::Cli)
    );
    assert!(f.app.usable_model("codex/gpt-6-luna").is_none());
    assert!(f.app.usable_model("nonsense").is_none());

    let picked = ModelBinding {
        provider: "mock".into(),
        model: "lead-pick".into(),
        effort: Effort::MEDIUM,
        max_tokens: None,
    };
    let out = f
        .app
        .create_team_with_models(
            f.project.id,
            "per-role",
            &TeamModels {
                model: (BackendKind::Api, model()),
                lead: Some((BackendKind::Api, picked.clone())),
                planner: None,
                role_models: true,
            },
            "",
        )
        .unwrap();
    let agents = f.app.agents().unwrap();
    let of_role = |i: usize| {
        let role = tpl.role_dir(&tpl.roles[i].id);
        agents.iter().find(|a| a.role == role).unwrap().clone()
    };
    assert_eq!(of_role(lead).model, picked, "the form's Lead model wins");
    assert_eq!(of_role(workers[0]).model.model, "deepseek-v4.1-flash");
    assert_eq!(of_role(workers[0]).backend, BackendKind::Api);
    assert_eq!(of_role(workers[1]).model.provider, "claude");
    assert_eq!(of_role(workers[1]).backend, BackendKind::Cli);
    assert_eq!(
        of_role(workers[2]).model,
        model(),
        "not usable here: the team's"
    );
    assert!(
        out.model_notes
            .iter()
            .any(|n| n.contains("codex/gpt-6-luna is not usable")),
        "{:?}",
        out.model_notes
    );
}

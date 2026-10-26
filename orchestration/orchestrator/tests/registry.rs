//! ADR-61: agents created on their own into the pool, teams composed from them (Lead suggested, user decides), the
//! system skill library with per-agent switches, MCP switches, and providers entered by the user.

mod common;

use agenium_config::ProviderKind;
use agenium_core::*;
use agenium_orchestrator::registry::*;
use agenium_orchestrator::CallCtx;
use common::*;

fn provider(f: &Fixture) {
    f.app
        .put_provider(&ProviderInput {
            id: "local".into(),
            kind: ProviderKind::OpenaiCompat,
            base_url: Some("http://127.0.0.1:9/v1".into()),
            api_key: None,
            clear_key: false,
            max_concurrency: 2,
            models: vec!["qwen3:8b".into()],
        })
        .unwrap();
}

fn binding(model: &str) -> ModelBinding {
    ModelBinding {
        provider: "local".into(),
        model: model.into(),
        effort: Effort::Medium,
        max_tokens: None,
    }
}

fn new_agent(f: &Fixture, name: &str, role: &str) -> agenium_store::core::agents::Agent {
    f.app
        .create_agent(&NewAgent {
            name: name.into(),
            role: role.into(),
            tags: None,
            model: binding("qwen3:8b"),
            backend: BackendKind::Api,
            skills: vec![],
            mcp: vec![],
        })
        .unwrap()
}

fn write_skill(dir: &std::path::Path, name: &str, description: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(
        dir.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: {description}\n---\nInstructions for {name}.\n"),
    )
    .unwrap();
}

#[test]
fn roles_are_template_presets_or_the_users_own() {
    let f = fixture();
    let roles = f.app.roles().unwrap();
    let lead = roles.iter().find(|r| r.id == "coding-lead").unwrap();
    assert!(lead.lead && !lead.exists && lead.template.starts_with("coding@"));
    assert!(roles.iter().any(|r| r.id == "coding-planner" && r.planner));

    let mine = f
        .app
        .put_role(&RoleDef {
            id: "data-analyst".into(),
            prompt: "You analyse CSV data and write short reports.".into(),
            tags: vec!["data".into()],
            allowed_paths: vec!["reports/**".into()],
            planner: false,
            lead: false,
        })
        .unwrap();
    assert!(mine.exists && mine.template.is_empty());
    assert_eq!(mine.tags, vec!["data"]);
    assert!(f.app.role_allowed_paths("data-analyst") == vec!["reports/**"]);
    for bad in [
        RoleDef {
            id: "Bad Id".into(),
            ..role_def("x")
        },
        RoleDef {
            prompt: " ".into(),
            ..role_def("empty")
        },
        RoleDef {
            allowed_paths: vec!["../out/**".into()],
            ..role_def("escape")
        },
        RoleDef {
            planner: true,
            lead: true,
            ..role_def("both")
        },
    ] {
        assert!(f.app.put_role(&bad).is_err(), "{bad:?}");
    }

    // A role in use stays.
    provider(&f);
    new_agent(&f, "dana", "data-analyst");
    assert!(f.app.remove_role("data-analyst").is_err());
}

fn role_def(id: &str) -> RoleDef {
    RoleDef {
        id: id.into(),
        prompt: "Does things.".into(),
        tags: vec![],
        allowed_paths: vec![],
        planner: false,
        lead: false,
    }
}

#[test]
fn agents_start_in_the_pool_with_their_own_model() {
    let f = fixture();
    // No provider yet: refused.
    let e = f
        .app
        .create_agent(&NewAgent {
            name: "ada".into(),
            role: "coding-backend".into(),
            tags: None,
            model: binding("qwen3:8b"),
            backend: BackendKind::Api,
            skills: vec![],
            mcp: vec![],
        })
        .unwrap_err();
    assert!(e.to_string().contains("not configured"), "{e}");
    provider(&f);
    let ada = new_agent(&f, "ada", "coding-backend");
    assert_eq!(ada.team, None);
    assert_eq!(ada.role_kind, RoleKind::Worker);
    assert_eq!(ada.tags, vec!["backend", "api", "db"], "the role's tags");
    assert_eq!(ada.model.model, "qwen3:8b");
    // The preset role was written when the first agent took it.
    assert!(f
        .app
        .layout()
        .roles()
        .join("coding-backend/ROLE.md")
        .is_file());
    // A CLI agent names the CLI program.
    let cli = f
        .app
        .create_agent(&NewAgent {
            name: "cody".into(),
            role: "coding-frontend".into(),
            tags: Some(vec!["ui".into()]),
            model: ModelBinding {
                provider: "codex".into(),
                model: "gpt-6-luna".into(),
                effort: Effort::Medium,
                max_tokens: None,
            },
            backend: BackendKind::Cli,
            skills: vec![],
            mcp: vec![],
        })
        .unwrap();
    assert_eq!(cli.backend, BackendKind::Cli);
    assert_eq!(cli.tags, vec!["ui"]);
    // Names are unique; unknown skills and MCP servers are refused.
    assert!(f
        .app
        .create_agent(&NewAgent {
            name: "ada".into(),
            ..spec_of(&ada)
        })
        .is_err());
    assert!(f
        .app
        .create_agent(&NewAgent {
            name: "x".into(),
            skills: vec!["nope".into()],
            ..spec_of(&ada)
        })
        .is_err());
    assert!(f
        .app
        .create_agent(&NewAgent {
            name: "y".into(),
            mcp: vec![agenium_mcp::Binding {
                server: "nope".into(),
                credentials: Default::default(),
                keys: Default::default(),
                tools: None
            }],
            ..spec_of(&ada)
        })
        .is_err());
    // The role can change later (tags follow the role unless given).
    f.app
        .set_agent_role(ada.id, "coding-frontend", None)
        .unwrap();
    let ada = f.app.agent(ada.id).unwrap();
    assert_eq!(ada.role, "coding-frontend");
    assert_eq!(ada.tags, vec!["frontend", "web", "ui"]);
}

fn spec_of(a: &agenium_store::core::agents::Agent) -> NewAgent {
    NewAgent {
        name: a.name.clone(),
        role: a.role.clone(),
        tags: None,
        model: a.model.clone(),
        backend: a.backend,
        skills: vec![],
        mcp: vec![],
    }
}

#[test]
fn teams_are_composed_from_pool_agents_and_the_lead_is_only_suggested() {
    let f = fixture();
    provider(&f);
    let lea = new_agent(&f, "lea", "coding-lead");
    let pat = new_agent(&f, "pat", "coding-planner");
    let bob = new_agent(&f, "bob", "coding-backend");
    let fay = new_agent(&f, "fay", "coding-frontend");
    let pia = new_agent(&f, "pia", "coding-planner");

    // Suggestion: the role meant to lead; planners are never suggested.
    let s = f
        .app
        .suggest_lead(&[bob.id, lea.id, pat.id, fay.id])
        .unwrap()
        .unwrap();
    assert_eq!(s.agent, lea.id);
    assert!(
        s.reasons.iter().any(|r| r.contains("meant to lead")),
        "{s:?}"
    );
    assert_eq!(f.app.suggest_lead(&[pat.id]).unwrap(), None);

    // The user may still pick someone else as Lead.
    let bad = |t: NewTeam| f.app.compose_team(&t).unwrap_err().to_string();
    let base = NewTeam {
        name: "web team".into(),
        project: Some(f.project.id),
        lead: bob.id,
        members: vec![lea.id, pat.id, fay.id],
        solo: false,
    };
    assert!(bad(NewTeam {
        lead: pat.id,
        members: vec![bob.id],
        ..base.clone()
    })
    .contains("planner"));
    assert!(bad(NewTeam {
        members: vec![pat.id, pia.id],
        ..base.clone()
    })
    .contains("at most one planner"));
    assert!(bad(NewTeam {
        members: vec![],
        ..base.clone()
    })
    .contains("at least one worker"));
    assert!(bad(NewTeam {
        members: vec![bob.id],
        ..base.clone()
    })
    .contains("twice"));
    assert!(bad(NewTeam {
        solo: true,
        ..base.clone()
    })
    .contains("Solo"));

    let team = f.app.compose_team(&base).unwrap();
    assert_eq!(team.lead, Some(bob.id));
    assert_eq!(team.project, Some(f.project.id));
    let (_, members) = f.app.team(team.id).unwrap();
    assert_eq!(members.len(), 4);
    assert_eq!(f.app.agent(bob.id).unwrap().role_kind, RoleKind::Lead);
    assert_eq!(f.app.agent(lea.id).unwrap().role_kind, RoleKind::Worker);
    assert_eq!(f.app.planner(f.project.id).unwrap().unwrap().id, pat.id);
    // The project is the team's: it can now be driven like any team (decision 25).
    assert_eq!(
        f.app.team_of_project(f.project.id).unwrap().unwrap().0.id,
        team.id
    );

    // An agent is in one team at a time; a project has one team.
    let gus = new_agent(&f, "gus", "coding-backend");
    let e = f
        .app
        .compose_team(&NewTeam {
            name: "other".into(),
            project: None,
            lead: lea.id,
            members: vec![gus.id],
            solo: false,
        })
        .unwrap_err();
    assert!(e.to_string().contains("already in a team"), "{e}");
    let e = f
        .app
        .compose_team(&NewTeam {
            name: "solo".into(),
            project: Some(f.project.id),
            lead: gus.id,
            members: vec![],
            solo: true,
        })
        .unwrap_err();
    assert!(e.to_string().contains("one team per project"), "{e}");

    // Membership changes: a pool agent joins as a worker, a second planner is refused, the Lead cannot leave.
    f.app.add_team_member(team.id, gus.id).unwrap();
    assert!(f.app.add_team_member(team.id, pia.id).is_err());
    assert!(f.app.remove_team_member(team.id, bob.id).is_err());
    f.app.remove_team_member(team.id, gus.id).unwrap();
    assert_eq!(f.app.agent(gus.id).unwrap().team, None);
    // The user swaps the Lead: the old Lead becomes a worker.
    f.app.set_lead_of_team(team.id, lea.id).unwrap();
    assert_eq!(f.app.agent(lea.id).unwrap().role_kind, RoleKind::Lead);
    assert_eq!(f.app.agent(bob.id).unwrap().role_kind, RoleKind::Worker);

    // A worker holding an open ticket stays; a team with open tickets is not dissolved.
    f.app
        .apply(&ctx(&f, Actor::Lead(lea.id)), Intent::ProposeSpec(spec()))
        .unwrap();
    f.app.approve_spec(f.project.id, 1).unwrap();
    f.app
        .apply(
            &ctx(&f, Actor::Lead(lea.id)),
            Intent::CreatePlan(plan(&[("PI-1", &[])])),
        )
        .unwrap();
    let out = f
        .app
        .apply(
            &ctx(&f, Actor::Lead(lea.id)),
            Intent::CreateTicket(new_ticket("PI-1", "API", &[], &[])),
        )
        .unwrap();
    let k: TicketKey = serde_json::from_value(out.result["ticket_id"].clone()).unwrap();
    f.app
        .apply(
            &ctx(&f, Actor::Lead(lea.id)),
            Intent::AssignTicket(agenium_core::forms::AssignTicket {
                ticket_id: k.clone(),
                worker: fay.id,
                note: None,
            }),
        )
        .unwrap();
    let e = f.app.remove_team_member(team.id, fay.id).unwrap_err();
    assert!(e.to_string().contains(&k.to_string()), "{e}");
    let e = f.app.dissolve_team(team.id).unwrap_err();
    assert!(e.to_string().contains("open tickets"), "{e}");
    let e = f.app.set_team_project(team.id, None).unwrap_err();
    assert!(e.to_string().contains("open tickets"), "{e}");
    f.app
        .apply(
            &CallCtx::user(f.project.id),
            Intent::CancelTicket(agenium_core::forms::CancelTicket {
                ticket_id: k.clone(),
                reason: "not needed any more".into(),
            }),
        )
        .unwrap();

    // Dissolved: everyone back in the pool, the project free again.
    f.app.dissolve_team(team.id).unwrap();
    for a in [lea.id, pat.id, bob.id, fay.id] {
        let a = f.app.agent(a).unwrap();
        assert_eq!(a.team, None);
        assert_eq!(a.role_kind, RoleKind::Worker);
    }
    assert!(f.app.team_of_project(f.project.id).unwrap().is_none());

    // A Solo from the pool, then pointed at the project.
    let solo = f
        .app
        .compose_team(&NewTeam {
            name: "gus (solo)".into(),
            project: None,
            lead: gus.id,
            members: vec![],
            solo: true,
        })
        .unwrap();
    assert!(solo.solo && solo.project.is_none());
    f.app.set_team_project(solo.id, Some(f.project.id)).unwrap();
    assert_eq!(
        f.app.team_of_project(f.project.id).unwrap().unwrap().0.id,
        solo.id
    );
}

#[test]
fn library_skills_and_mcp_servers_are_switched_per_agent() {
    let f = fixture();
    provider(&f);
    let src = f.root.join("incoming");
    write_skill(
        &src.join("lead-team"),
        "lead-team",
        "How to lead an agent team: plans, reviews.",
    );
    write_skill(&src.join("csv-report"), "csv-report", "Write CSV reports.");
    f.app
        .import_library_skill(&src.join("lead-team"), false)
        .unwrap();
    f.app
        .import_library_skill(&src.join("csv-report"), false)
        .unwrap();
    assert!(
        f.app
            .import_library_skill(&src.join("csv-report"), false)
            .is_err(),
        "exists"
    );
    // The two imported skills are in the library (alongside the seeded starter skills, ADR-88).
    let lib = f.app.skill_registry().library();
    let names = lib.names();
    assert!(names.contains(&"csv-report") && names.contains(&"lead-team"));

    let ana = new_agent(&f, "ana", "coding-backend");
    let max = new_agent(&f, "max", "coding-backend");
    // Off by default; switched on per agent.
    let set = f
        .app
        .skill_registry()
        .for_agent(&ana.id.to_string(), &ana.role, &ana.skills);
    assert!(set.get("csv-report").is_none());
    f.app
        .toggle_agent_skill(ana.id, "csv-report", true)
        .unwrap();
    f.app.toggle_agent_skill(max.id, "lead-team", true).unwrap();
    let ana = f.app.agent(ana.id).unwrap();
    assert_eq!(ana.skills, vec!["csv-report"]);
    let set = f
        .app
        .skill_registry()
        .for_agent(&ana.id.to_string(), &ana.role, &ana.skills);
    assert_eq!(
        set.get("csv-report").unwrap().meta.origin,
        agenium_skills::Origin::Library
    );
    assert!(f.app.toggle_agent_skill(ana.id, "nope", true).is_err());

    // A lead-related skill makes the agent the suggested Lead.
    let s = f.app.suggest_lead(&[ana.id, max.id]).unwrap().unwrap();
    assert_eq!(s.agent, max.id);
    assert!(s.reasons.iter().any(|r| r.contains("lead-team")), "{s:?}");

    // Removal: refused while enabled, unless forced (the agents lose it).
    assert!(f.app.remove_library_skill("csv-report", false).is_err());
    f.app.remove_library_skill("csv-report", true).unwrap();
    assert!(f.app.agent(ana.id).unwrap().skills.is_empty());
    let lib = f.app.skill_registry().library();
    let names = lib.names();
    assert!(!names.contains(&"csv-report") && names.contains(&"lead-team"));

    // MCP servers of the system, switched per agent.
    f.app
        .put_mcp_server(agenium_config::McpServerConfig {
            id: "files".into(),
            command: Some("mcp-files".into()),
            args: vec![],
            env: Default::default(),
            url: None,
            headers: Default::default(),
            timeout_secs: 60,
            oauth: None,
        })
        .unwrap();
    f.app.toggle_agent_mcp(ana.id, "files", true).unwrap();
    f.app.toggle_agent_mcp(ana.id, "files", true).unwrap();
    let b = agenium_mcp::parse_bindings(&f.app.agent(ana.id).unwrap().mcp).unwrap();
    assert_eq!(b.len(), 1, "switching on twice binds once");
    assert!(f.app.toggle_agent_mcp(ana.id, "nope", true).is_err());
    assert!(f.app.remove_mcp_server("files").is_err(), "in use");
    f.app.remove_mcp_server_force("files").unwrap();
    assert!(
        agenium_mcp::parse_bindings(&f.app.agent(ana.id).unwrap().mcp)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn providers_keep_their_keys_in_the_vault() {
    let f = fixture();
    let key = "sk-test-0123456789abcdefghij";
    f.app
        .put_provider(&ProviderInput {
            id: "deepseek".into(),
            kind: ProviderKind::OpenaiCompat,
            base_url: Some("https://api.deepseek.example/v1".into()),
            api_key: Some(key.into()),
            clear_key: false,
            max_concurrency: 4,
            models: vec![],
        })
        .unwrap();
    let cfg = f.app.config();
    let pc = cfg.provider("deepseek").unwrap();
    assert_eq!(pc.api_key_ref, "provider/deepseek");
    let text = std::fs::read_to_string(f.app.layout().config()).unwrap();
    assert!(!text.contains(key), "no key in config.toml");
    assert_eq!(
        f.app
            .secret("provider/deepseek")
            .unwrap()
            .unwrap()
            .expose_str()
            .unwrap(),
        key
    );
    // Editing without a key keeps the stored one.
    f.app
        .put_provider(&ProviderInput {
            id: "deepseek".into(),
            kind: ProviderKind::OpenaiCompat,
            base_url: Some("https://api.deepseek.example/v1".into()),
            api_key: None,
            clear_key: false,
            max_concurrency: 8,
            models: vec![],
        })
        .unwrap();
    let cfg = f.app.config();
    assert_eq!(
        cfg.provider("deepseek").unwrap().api_key_ref,
        "provider/deepseek"
    );
    assert_eq!(cfg.provider("deepseek").unwrap().max_concurrency, 8);

    // Checks: a key for Anthropic/Gemini, a base URL for OpenAI-compatible endpoints, valid ids.
    let bad = |p: ProviderInput| f.app.put_provider(&p).is_err();
    let p = |id: &str, kind, url: Option<&str>| ProviderInput {
        id: id.into(),
        kind,
        base_url: url.map(str::to_owned),
        api_key: None,
        clear_key: false,
        max_concurrency: 4,
        models: vec![],
    };
    assert!(bad(p("claude", ProviderKind::Anthropic, None)));
    assert!(bad(p("local", ProviderKind::OpenaiCompat, None)));
    assert!(bad(p(
        "Local Box",
        ProviderKind::OpenaiCompat,
        Some("http://x/v1")
    )));
    assert!(bad(p("x", ProviderKind::OpenaiCompat, Some("ftp://x"))));

    // In use: refused; after the agent switches model: removed with its key.
    let ag = f
        .app
        .create_agent(&NewAgent {
            name: "dee".into(),
            role: "coding-backend".into(),
            tags: None,
            model: ModelBinding {
                provider: "deepseek".into(),
                model: "deepseek-v4.1-flash".into(),
                effort: Effort::Medium,
                max_tokens: None,
            },
            backend: BackendKind::Api,
            skills: vec![],
            mcp: vec![],
        })
        .unwrap();
    assert!(f.app.remove_provider("deepseek").is_err());
    provider(&f);
    f.app.set_agent_model(ag.id, &binding("qwen3:8b")).unwrap();
    f.app.remove_provider("deepseek").unwrap();
    assert!(f.app.config().provider("deepseek").is_none());
    assert!(f.app.secret("provider/deepseek").unwrap().is_none());
}

/// A project is a folder name (the app decides where it lives) or a git repository cloned there.
#[test]
fn projects_from_a_folder_name_or_a_repository() {
    let f = fixture();
    let p = f
        .app
        .create_project(&agenium_orchestrator::app::CreateProject {
            name: "shop".into(),
            path: None,
            key_prefix: Some("SHP".into()),
            template: None,
            folder: Some("shop-site".into()),
            repo: None,
        })
        .unwrap();
    assert!(p.path.ends_with("shop-site"), "{}", p.path);
    let bad = agenium_orchestrator::app::CreateProject {
        name: "x".into(),
        path: None,
        key_prefix: None,
        template: None,
        folder: Some("../outside".into()),
        repo: None,
    };
    assert!(f.app.create_project(&bad).is_err());

    // A repository with one commit on "trunk", cloned by the app.
    let src = f.root.join("src-repo");
    std::fs::create_dir_all(&src).unwrap();
    let git = |args: &[&str]| {
        let ok = std::process::Command::new("git")
            .arg("-C")
            .arg(&src)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    };
    git(&["init", "-q", "-b", "trunk"]);
    std::fs::write(src.join("README.md"), "hello").unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "first"]);
    assert!(agenium_orchestrator::app::is_repo_url(
        "https://github.com/o/r.git"
    ));
    assert!(!agenium_orchestrator::app::is_repo_url("my-folder"));
    assert_eq!(
        agenium_orchestrator::app::repo_folder("git@github.com:o/site.git"),
        "site"
    );
    let p = f
        .app
        .create_project(&agenium_orchestrator::app::CreateProject {
            name: "cloned".into(),
            path: None,
            key_prefix: Some("CLN".into()),
            template: None,
            folder: None,
            repo: Some(src.to_string_lossy().into_owned()),
        })
        .unwrap();
    assert!(std::path::Path::new(&p.path).join("README.md").is_file());
    assert_eq!(p.main_branch, "trunk");
}

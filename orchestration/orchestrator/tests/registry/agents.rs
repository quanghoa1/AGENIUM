//! Roles, the agent pool and teams composed from it.

use super::*;

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
                effort: Effort::MEDIUM,
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
    // The team has a planner: its draft comes first (ADR-116).
    let p = plan(&[("PI-1", &[])]);
    f.app
        .apply(
            &ctx(&f, Actor::Worker(pat.id)),
            Intent::SubmitPlanDraft(agenium_core::plan::PlanDraft {
                spec_version: 1,
                milestones: p.milestones.clone(),
                items: p.items.clone(),
                rationale: "One item.".into(),
            }),
        )
        .unwrap();
    f.app
        .apply(&ctx(&f, Actor::Lead(lea.id)), Intent::CreatePlan(p))
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
                replaced_by: None,
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

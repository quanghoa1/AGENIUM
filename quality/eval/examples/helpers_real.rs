//! Real check of the drafting helpers on a vendor CLI (ADR-151, ADR-157; 2026-11-17), through the app's own calls:
//! 1. the team designer, given library and plugin skills, answers a skill question in seconds, live, from what it was
//!    given (no file hunting), and gives roles plugin skills by their entry;
//! 2. asked for a team like the built-in `coding`, it names it in `based_on` and keeps a new id;
//! 3. the draft saved over `coding` replaces it and Back to built-in brings it back; a team made from the first draft
//!    gives the role's agents the plugin skill alone;
//! 4. an agent's copy of a plugin skill in conflict with the vendor's update is merged by the helper's draft, which keeps
//!    the vendor's change and the agent's lesson, and is accepted.
//!
//! cargo run -p agenium-eval --example helpers_real -- claude [model]      (default haiku; the CLI's own login)

use std::sync::{Arc, Mutex};
use std::time::Instant;

use agenium_core::agent::{BackendKind, Effort, RoleKind};
use agenium_core::forms::ProposeSkillUpdate;
use agenium_core::*;
use agenium_llm::ChatEvent;
use agenium_orchestrator::team_design::ChatTurn;

fn check(fails: &mut u32, name: &str, ok: bool, seen: &str) {
    if !ok {
        *fails += 1;
    }
    println!(
        "{} {name}{}",
        if ok { "PASS" } else { "FAIL" },
        if seen.is_empty() {
            String::new()
        } else {
            format!(": {seen}")
        }
    );
}

fn skill(dir: &std::path::Path, name: &str, desc: &str, body: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir.join(name))?;
    std::fs::write(
        dir.join(name).join("SKILL.md"),
        format!("---\nname: {name}\ndescription: {desc}\n---\n{body}"),
    )
}

/// The vendor's plugin folder: `pentest` with three skills; `web-pentest`'s body is `web`.
fn vendor(dir: &std::path::Path, version: &str, web: &str) -> std::io::Result<()> {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir.join(".claude-plugin"))?;
    std::fs::write(
        dir.join(".claude-plugin/plugin.json"),
        format!(r#"{{"name":"pentest","version":"{version}","description":"Pentest skills."}}"#),
    )?;
    let s = dir.join("skills");
    skill(
        &s,
        "web-pentest",
        "Use when testing a web application: OWASP WSTG checks, auth, injection, SSRF.",
        web,
    )?;
    skill(
        &s,
        "recon-enumeration",
        "Use to map a target's hosts, subdomains, ports and services before testing.",
        "# Recon\n\n1. Enumerate subdomains.\n",
    )?;
    skill(
        &s,
        "reverse-engineering",
        "Use to reverse a binary or an APK to find logic flaws and secrets.",
        "# Reverse\n\n1. Decompile.\n",
    )
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cli = args.first().cloned().unwrap_or_else(|| "claude".into());
    let model = args.get(1).cloned().unwrap_or_else(|| "haiku".into());
    let tmp = tempfile::tempdir()?;
    let (app, _) = agenium_orchestrator::App::open(
        &tmp.path().join("root"),
        agenium_orchestrator::AppOptions {
            keyring: Arc::new(agenium_secrets::MemoryKeyring::default()),
            checks: Arc::new(agenium_orchestrator::NoCheckFactory),
        },
    )?;
    let app = Arc::new(app);
    app.update_config(|c| c.memory.learn = false)?;
    let rt = agenium_orchestrator::runtime::Runtime::new(
        app.clone(),
        Arc::new(agenium_orchestrator::runtime::StaticProviders(vec![])),
        Arc::new(agenium_orchestrator::runtime::AutoApprove),
    );
    let binding = ModelBinding {
        provider: cli.clone(),
        model: model.clone(),
        effort: Effort::LOW,
        max_tokens: None,
    };
    let mut fails = 0;

    // The library and a plugin.
    let lib = tmp.path().join("lib");
    skill(
        &lib,
        "report-writer",
        "Use to write a pentest report with CVSS scores and remediation for each finding.",
        "# Report\n\n1. One finding per section.\n",
    )?;
    skill(
        &lib,
        "burp-workflow",
        "Use to drive Burp Suite: proxy, repeater, intruder for web requests.",
        "# Burp\n\n1. Proxy the app.\n",
    )?;
    for s in ["report-writer", "burp-workflow"] {
        app.import_library_skill(&lib.join(s), false)?;
    }
    let repo = tmp.path().join("vendor-pentest");
    vendor(
        &repo,
        "1.0",
        "# Web pentest\n\n1. Map the app and its APIs.\n2. Test inputs.\n",
    )?;
    let install = |app: Arc<agenium_orchestrator::App>, repo: std::path::PathBuf| async move {
        let p = app
            .preview_plugin(&repo.display().to_string(), None, None)
            .await?;
        app.install_plugin(
            &p.plugin.source,
            &p.plugin.reference,
            None,
            &p.plugin.commit,
            None,
        )
        .await
    };
    install(app.clone(), repo.clone()).await?;
    println!("{cli}:{model}, library 2 skills, plugin pentest 3 skills");

    // 1. Skills for each role, live.
    let pieces = Arc::new(Mutex::new((0usize, None::<f64>)));
    let start = Instant::now();
    let seen = pieces.clone();
    let live = move |e: &ChatEvent| {
        if let ChatEvent::TextDelta(_) = e {
            let mut p = seen.lock().unwrap();
            p.0 += 1;
            p.1.get_or_insert(start.elapsed().as_secs_f64());
        }
    };
    let ask1 = "Tôi cần 1 team pentest web gồm lead, recon và web tester. Tôi vừa thêm vài skill cho hệ thống của chúng ta, bạn xem qua rồi đề xuất skill cho đúng với mỗi role. Đề xuất luôn, đừng hỏi lại.";
    let d1 = rt
        .design_team_live(
            BackendKind::Cli,
            &binding,
            &[ChatTurn {
                role: "user".into(),
                text: ask1.into(),
            }],
            &[],
            None,
            &live,
        )
        .await?;
    let took = start.elapsed().as_secs_f64();
    let (n, first) = *pieces.lock().unwrap();
    println!(
        "1. {took:.0} s, {n} pieces (first at {:.1} s); reply: {}",
        first.unwrap_or(0.0),
        d1.reply
            .replace('\n', " ")
            .chars()
            .take(220)
            .collect::<String>()
    );
    let draft1 = d1.draft.clone();
    let skills1: Vec<String> = draft1
        .iter()
        .flat_map(|d| d.template.roles.iter().flat_map(|r| r.skills.clone()))
        .collect();
    println!("   role skills: {skills1:?}");
    check(
        &mut fails,
        "the designer answers within 2 minutes, from what it was given",
        took < 120.0 && draft1.is_some(),
        &format!("{took:.0} s"),
    );
    check(
        &mut fails,
        "its answer comes in pieces as it is written",
        n > 1,
        &format!("{n} pieces"),
    );
    let lower = d1.reply.to_lowercase();
    check(
        &mut fails,
        "it does not say it cannot read the skills",
        ![
            "cannot read",
            "can't read",
            "không đọc được",
            "không thể đọc",
            "không có công cụ",
        ]
        .iter()
        .any(|w| lower.contains(w)),
        "",
    );
    check(
        &mut fails,
        "it gives roles plugin skills by their entry",
        skills1.iter().any(|s| s.starts_with("plugin:pentest/")),
        "",
    );

    // 2. A team like the built-in `coding`.
    let ask2 = "Tôi muốn một team giống template coding có sẵn, nhưng thêm 1 role QA chuyên viết test. Đề xuất luôn, đừng hỏi lại.";
    let t2 = Instant::now();
    let d2 = rt
        .design_team(
            BackendKind::Cli,
            &binding,
            &[ChatTurn {
                role: "user".into(),
                text: ask2.into(),
            }],
            &[],
            None,
        )
        .await?;
    let draft2 = d2.draft.clone();
    println!(
        "2. {:.0} s; based_on {:?}, id {:?}, roles {:?}",
        t2.elapsed().as_secs_f64(),
        draft2.as_ref().and_then(|d| d.based_on.clone()),
        draft2.as_ref().map(|d| d.template.id.clone()),
        draft2.as_ref().map(|d| d
            .template
            .roles
            .iter()
            .map(|r| r.id.clone())
            .collect::<Vec<_>>())
    );
    check(
        &mut fails,
        "a team like coding names it in based_on",
        draft2.as_ref().and_then(|d| d.based_on.as_deref()) == Some("coding"),
        "",
    );
    check(
        &mut fails,
        "and keeps a new id",
        draft2.as_ref().is_some_and(|d| d.template.id != "coding"),
        "",
    );

    // 3. Saved over coding, then the built-in back; a team from the first draft.
    if let Some(d) = &draft2 {
        let mut over = d.template.clone();
        over.id = "coding".into();
        app.save_team_template(&over)?;
        let (builtin, custom) = app.template_origin("coding");
        check(
            &mut fails,
            "saved over coding, it replaces the built-in",
            builtin && custom && app.team_template("coding")?.name == over.name,
            "",
        );
        app.remove_team_template("coding")?;
        check(
            &mut fails,
            "Back to built-in brings it back",
            !app.template_origin("coding").1,
            "",
        );
    }
    let project = app
        .create_project(&agenium_orchestrator::CreateProject {
            name: "pentest work".into(),
            path: None,
            key_prefix: None,
            template: None,
            folder: None,
            repo: None,
        })?
        .id;
    if let Some(d) = &draft1 {
        app.save_team_template(&d.template)?;
        let made =
            app.create_team_from_template(project, &d.template.id, &binding, None, None, "pt")?;
        let agents = app.agents()?;
        let given: Vec<String> = agents
            .iter()
            .filter(|a| a.skills.iter().any(|s| s.starts_with("plugin:pentest/")))
            .map(|a| format!("{} {:?}", a.name, a.skills))
            .collect();
        println!(
            "3. team from the draft: missing {:?}; {given:?}",
            made.missing_skills
        );
        check(
            &mut fails,
            "a team from the draft gives its agents the plugin skills alone",
            !given.is_empty() && made.missing_skills.is_empty(),
            "",
        );
    }

    // 4. The helper merges an agent's copy in conflict with the vendor's update.
    // A project of its own: one team per project (decision 25).
    let solo_project = app
        .create_project(&agenium_orchestrator::CreateProject {
            name: "solo work".into(),
            path: None,
            key_prefix: None,
            template: None,
            folder: None,
            repo: None,
        })?
        .id;
    let (_, ada) = app.create_solo(
        solo_project,
        &agenium_orchestrator::CreateAgent {
            name: "ada".into(),
            role: "solo".into(),
            role_kind: RoleKind::Lead,
            tags: vec!["pentest".into()],
            model: binding.clone(),
        },
    )?;
    app.set_agent_backend(ada.id, BackendKind::Cli, &binding)?;
    app.toggle_agent_skill(ada.id, "plugin:pentest/web-pentest", true)?;
    let e = app.embedder();
    let mem = app.agent_store(ada.id, e.dim(), e.model_id())?;
    let lesson = mem.add(
        &agenium_store::NewFact {
            kind: agenium_store::FactKind::Lesson,
            cue: Some("starting a web pentest".into()),
            ..agenium_store::NewFact::fact(
                "Confirm the written scope before testing.",
                "ticket:PT-1",
            )
        },
        None,
    )?;
    for key in ["CTX-1", "CTX-2"] {
        let ctx = closed_ticket(&app, solo_project, key)?;
        mem.validate_in(lesson, &ctx)?;
    }
    app.apply_skill_update(
        ada.id,
        &ProposeSkillUpdate {
            skill: "web-pentest".into(),
            patch: String::new(),
            content: Some("# Web pentest\n\n1. Map the app and its APIs.\n2. Test inputs.\n3. Confirm the written scope first.\n".into()),
            reason: "a test ran out of scope".into(),
            lesson,
        },
    )?;
    vendor(&repo, "2.0", "# Web pentest\n\n1. Map the app and its APIs.\n2. Test inputs.\n3. Report each finding with a PoC.\n")?;
    let updated = install(app.clone(), repo.clone()).await?;
    let conflicts = updated.refreshed.map(|r| r.conflicts).unwrap_or_default();
    println!("4. update: conflicts {conflicts:?}");
    check(
        &mut fails,
        "the vendor's update leaves the conflicting copy for the user",
        conflicts.iter().any(|c| c.ends_with("web-pentest")),
        "",
    );
    let t4 = Instant::now();
    let m = rt
        .draft_overlay_merge(ada.id, "web-pentest", BackendKind::Cli, &binding)
        .await?;
    println!(
        "   merge drafted in {:.0} s:\n{}",
        t4.elapsed().as_secs_f64(),
        m.merged.trim()
    );
    check(
        &mut fails,
        "the helper's merge keeps the vendor's change and the agent's lesson",
        m.merged.starts_with("---")
            && m.merged.contains("Report each finding")
            && m.merged.contains("Confirm the written scope"),
        "",
    );
    app.accept_overlay_merge(ada.id, "web-pentest", &m.merged)?;
    let v = app.skill_overlays(ada.id)?;
    check(
        &mut fails,
        "accepted, the copy is on the new version",
        v.first()
            .is_some_and(|o| !o.stale && o.base == "plugin:pentest"),
        "",
    );

    println!(
        "{}",
        if fails == 0 {
            "ALL PASS".to_owned()
        } else {
            format!("{fails} FAILED")
        }
    );
    std::process::exit(if fails == 0 { 0 } else { 1 });
}

/// A closed ticket of `project`, as the context a lesson held in (ADR-118).
fn closed_ticket(
    app: &agenium_orchestrator::App,
    project: ProjectId,
    key: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    use agenium_store::core::tickets;
    let key_parsed: TicketKey = key.parse()?;
    app.store().commit(
        None,
        "check",
        |w| -> std::result::Result<(), agenium_store::StoreError> {
            let mut t = tickets::insert(
                w.tx,
                &tickets::TicketInsert {
                    id: TicketId::new(),
                    key: key_parsed.clone(),
                    project,
                    parent: None,
                    kind: TicketKind::Task,
                    title: "earlier work".into(),
                    description: String::new(),
                    acceptance: vec![AcceptanceCriterion {
                        id: "AC-1".into(),
                        text: "done".into(),
                    }],
                    plan_item: "PI-0".into(),
                    priority: 2,
                    depends_on: vec![],
                    required_role: None,
                    required_tags: vec![],
                    scope_paths: vec![],
                    report_template: "default".into(),
                    created_at: "2026-11-17T00:00:00.000Z".into(),
                },
            )?;
            t.status = TicketStatus::Closed;
            tickets::update(w.tx, &t)
        },
    )?;
    Ok(format!("{project}/ticket:{key}"))
}

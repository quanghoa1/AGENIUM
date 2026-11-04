//! ADR-157: a plugin from a vendor's folder, shown before it is written, on or off whole per agent, its skills
//! read-only, updated whole, removed with its switches.

use super::*;

fn plugin_repo(dir: &std::path::Path, version: &str, skills: &[&str]) {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir.join(".claude-plugin")).unwrap();
    std::fs::write(
        dir.join(".claude-plugin/plugin.json"),
        format!(r#"{{"name":"superpowers","version":"{version}","description":"Core skills."}}"#),
    )
    .unwrap();
    for s in skills {
        write_skill(
            &dir.join("skills").join(s),
            s,
            &format!("Use {s} {version}."),
        );
    }
    std::fs::write(
        dir.join(".mcp.json"),
        r#"{"mcpServers":{"docs":{"command":"npx","args":["-y","docs-mcp"]}}}"#,
    )
    .unwrap();
    std::fs::create_dir_all(dir.join("hooks")).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plugin_is_installed_switched_whole_updated_and_removed() {
    let f = fixture();
    provider(&f);
    let repo = f.root.join("vendor-superpowers");
    plugin_repo(&repo, "1.0", &["brainstorming", "writing-plans"]);
    let src = repo.display().to_string();

    // A preview writes nothing.
    let p = f.app.preview_plugin(&src, None).await.unwrap();
    assert_eq!(p.plugin.name, "superpowers");
    assert_eq!(
        p.plugin.skills.keys().collect::<Vec<_>>(),
        ["brainstorming", "writing-plans"]
    );
    assert_eq!(p.plugin.mcp[0].id, "superpowers-docs");
    // An empty hooks folder brings no hook.
    assert!(p.plugin.hooks.is_empty());
    assert!(f.app.plugins().unwrap().is_empty());
    f.app
        .install_plugin(
            &p.plugin.source,
            &p.plugin.reference,
            None,
            &p.plugin.commit,
        )
        .await
        .unwrap();
    assert_eq!(f.app.plugins().unwrap().len(), 1);
    // Agenium's manifest in Claude Code's plugin.json format, with what it took; no hooks; the install recorded.
    let dir = f.app.skill_registry().plugins_dir();
    assert!(dir
        .join("superpowers/skills/brainstorming/SKILL.md")
        .is_file());
    for k in ["hooks", ".claude-plugin", ".mcp.json"] {
        assert!(!dir.join("superpowers").join(k).exists(), "{k}");
    }
    let m: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("superpowers/agenium-plugin.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(m["name"], "superpowers");
    assert_eq!(m["version"], "1.0");
    assert_eq!(m["skills"], "./skills/");
    assert_eq!(m["mcpServers"]["docs"]["command"], "npx");
    assert!(m.get("hooks").is_none());
    let installed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("installed_plugins.json")).unwrap())
            .unwrap();
    let key = format!("superpowers@{src}");
    assert_eq!(
        installed["plugins"][&key][0]["gitCommitSha"], "local",
        "{installed}"
    );
    assert_eq!(installed["version"], 2);

    // Off until the agent has it on, then every skill and server at once.
    let ana = new_agent(&f, "ana", "coding-backend");
    let reg = f.app.skill_registry();
    assert!(reg
        .for_agent(&ana.id.to_string(), &ana.role, &ana.skills)
        .get("brainstorming")
        .is_none());
    f.app
        .toggle_agent_plugin(ana.id, "superpowers", true)
        .unwrap();
    let ana = f.app.agent(ana.id).unwrap();
    assert_eq!(ana.skills, ["plugin:superpowers"]);
    let set = reg.for_agent(&ana.id.to_string(), &ana.role, &ana.skills);
    for s in ["brainstorming", "writing-plans"] {
        assert_eq!(
            set.get(s).unwrap().meta.origin,
            agenium_skills::Origin::Plugin
        );
    }
    let (bindings, servers) = f.app.plugin_mcp(&ana.skills);
    assert_eq!(
        (bindings[0].server.as_str(), servers[0].id.as_str()),
        ("superpowers-docs", "superpowers-docs")
    );
    assert!(f.app.toggle_agent_plugin(ana.id, "nope", true).is_err());

    // Read-only: the agent's learning cannot change a plugin's skill.
    let refused = f.app.preview_skill_update(
        ana.id,
        &agenium_core::forms::ProposeSkillUpdate {
            skill: "brainstorming".into(),
            patch: String::new(),
            content: Some("Changed.".into()),
            reason: "r".into(),
            lesson: 1,
        },
    );
    assert!(refused.is_err());

    // An update shows what changes and replaces the whole set.
    plugin_repo(&repo, "2.0", &["brainstorming", "executing-plans"]);
    let u = f.app.preview_plugin_update("superpowers").await.unwrap();
    assert_eq!(u.skills_added, ["executing-plans"]);
    assert_eq!(u.skills_changed, ["brainstorming"]);
    assert_eq!(u.skills_removed, ["writing-plans"]);
    assert_eq!(u.agents, ["ana"]);
    f.app
        .install_plugin(
            &u.plugin.source,
            &u.plugin.reference,
            None,
            &u.plugin.commit,
        )
        .await
        .unwrap();
    let set = reg.for_agent(&ana.id.to_string(), &ana.role, &ana.skills);
    assert!(set.get("writing-plans").is_none() && set.get("executing-plans").is_some());
    assert_eq!(
        f.app.plugin("superpowers").unwrap().version.as_deref(),
        Some("2.0")
    );

    // Removed: gone, and off for the agent.
    f.app.remove_plugin("superpowers").unwrap();
    assert!(f.app.plugins().unwrap().is_empty());
    assert!(f.app.agent(ana.id).unwrap().skills.is_empty());
    assert!(f.app.remove_plugin("superpowers").is_err());
}

/// A plugin with one SessionStart hook whose script reads Claude Code's input on stdin.
fn hooked_repo(dir: &std::path::Path) {
    std::fs::create_dir_all(dir.join(".claude-plugin")).unwrap();
    std::fs::write(
        dir.join(".claude-plugin/plugin.json"),
        r#"{"name":"hooky","version":"1.0","description":"A hook."}"#,
    )
    .unwrap();
    write_skill(
        &dir.join("skills/hooky-rules"),
        "hooky-rules",
        "Hooky's rules.",
    );
    std::fs::create_dir_all(dir.join("hooks")).unwrap();
    std::fs::write(
        dir.join("hooks/hooks.json"),
        r#"{"hooks":{
            "SessionStart":[{"matcher":"startup","hooks":[{"type":"command",
                "command":"bash \"${CLAUDE_PLUGIN_ROOT}/hooks/start.sh\"",
                "powershell":"& \"${CLAUDE_PLUGIN_ROOT}/hooks/start.ps1\""}]}],
            "Stop":[{"hooks":[{"type":"command","command":"echo bye"}]}]}}"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("hooks/start.sh"),
        "read -r line\ncase \"$line\" in *SessionStart*) echo \"Hooky says: check the rules.\";; esac\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("hooks/start.ps1"),
        "$j = (@($input) -join '') | ConvertFrom-Json\nif ($j.hook_event_name -eq 'SessionStart') { 'Hooky says: check the rules.' }\n",
    )
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plugins_hooks_run_in_the_sandbox_and_leave_nothing_once_removed() {
    let f = fixture();
    provider(&f);
    let repo = f.root.join("vendor-hooky");
    hooked_repo(&repo);
    let p = f
        .app
        .preview_plugin(&repo.display().to_string(), None)
        .await
        .unwrap();
    assert_eq!(p.plugin.hooks.len(), 1, "{:?}", p.plugin.hooks);
    assert!(p.plugin.hooks[0].starts_with("SessionStart: bash"));
    assert!(
        p.plugin.skipped.iter().any(|s| s.starts_with("hook Stop")),
        "{:?}",
        p.plugin.skipped
    );
    f.app
        .install_plugin(
            &p.plugin.source,
            &p.plugin.reference,
            None,
            &p.plugin.commit,
        )
        .await
        .unwrap();
    let dir = f.app.skill_registry().plugins_dir().join("hooky");
    // What was skipped stays with the install.
    let kept = f.app.plugin("hooky").unwrap();
    assert!(kept.skipped.iter().any(|s| s.starts_with("hook Stop")) && kept.hooks.len() == 1);
    assert!(dir.join("hooks/start.sh").is_file() && dir.join("hooks/start.ps1").is_file());

    let ana = new_agent(&f, "ana", "coding-backend");
    // Off: no hook runs for the agent.
    assert!(!f.app.has_plugin_hooks(&ana.skills, "SessionStart"));
    f.app.toggle_agent_plugin(ana.id, "hooky", true).unwrap();
    let ana = f.app.agent(ana.id).unwrap();
    assert!(f.app.has_plugin_hooks(&ana.skills, "SessionStart"));
    let project = f.root.join("work");
    std::fs::create_dir_all(&project).unwrap();
    let runs = f
        .app
        .run_plugin_hooks(
            &ana.skills,
            "SessionStart",
            "startup",
            serde_json::json!({"source": "startup"}),
            "R1",
            &project,
        )
        .await;
    assert_eq!(runs.len(), 1);
    if agenium_sandbox::detect().backend.is_some() {
        assert!(runs[0].note.ok, "{:?}", runs[0].note);
        assert_eq!(
            runs[0].context.as_deref(),
            Some("Hooky says: check the rules.")
        );
    } else {
        assert!(!runs[0].note.ok && runs[0].context.is_none());
    }
    // A matcher that does not take the source runs nothing.
    assert!(f
        .app
        .run_plugin_hooks(
            &ana.skills,
            "SessionStart",
            "resume",
            serde_json::json!({}),
            "R2",
            &project
        )
        .await
        .is_empty());

    // Switched off: the agent's kept CLI sessions go (they carry what the hooks said); removed: every file goes.
    let sessions = f.app.cli_sessions(ana.id).join("T-1");
    std::fs::create_dir_all(&sessions).unwrap();
    f.app.toggle_agent_plugin(ana.id, "hooky", false).unwrap();
    assert!(!sessions.exists());
    f.app.toggle_agent_plugin(ana.id, "hooky", true).unwrap();
    f.app.remove_plugin("hooky").unwrap();
    assert!(!dir.exists());
    let left: Vec<_> = std::fs::read_dir(f.app.skill_registry().plugins_dir())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left, ["installed_plugins.json"]);
    let ana = f.app.agent(ana.id).unwrap();
    assert!(ana.skills.is_empty());
    assert!(f
        .app
        .run_plugin_hooks(
            &["plugin:hooky".into()],
            "SessionStart",
            "startup",
            serde_json::json!({}),
            "R3",
            &project
        )
        .await
        .is_empty());
}

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
    assert!(
        p.plugin.skipped.iter().any(|s| s.starts_with("hooks")),
        "{:?}",
        p.plugin.skipped
    );
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
    assert!(f
        .app
        .plugin("superpowers")
        .unwrap()
        .skipped
        .iter()
        .any(|s| s.starts_with("hooks")));

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

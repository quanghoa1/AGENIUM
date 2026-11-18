use super::*;

/// Updates captured from devin 3000.11.3 (`swe-2-max`, 2026-11-04), shortened.
#[test]
fn reads_the_acp_updates() {
    let ev = |s: &str| Devin.parse(&serde_json::from_str(s).unwrap());
    assert_eq!(
        ev(r#"{"type":"session","id":"flax-poppyseed"}"#),
        [Event::Session("flax-poppyseed".into())]
    );
    assert_eq!(
        ev(
            r#"{"type":"update","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"DONE"}}}"#
        ),
        [Event::Text("DONE".into())]
    );
    let call = ev(
        r#"{"type":"update","update":{"sessionUpdate":"tool_call","toolCallId":"exec:0#1","title":"Ran echo","kind":"execute","rawInput":{"command":"echo hi > cmd.txt","workdir":"w"},"_meta":{"cognition.ai/inferenceToolName":"exec"}}}"#,
    );
    match &call[0] {
        Event::Tool {
            name, input, done, ..
        } => {
            assert_eq!(name, "exec");
            assert!(!done);
            assert_eq!(
                Devin.command(name, input).as_deref(),
                Some("echo hi > cmd.txt")
            );
        }
        e => panic!("{e:?}"),
    }
    assert!(ev(r#"{"type":"update","update":{"sessionUpdate":"tool_call_update","toolCallId":"exec:0#1","status":"in_progress"}}"#).is_empty());
    // A command's output while it runs, its exit code with it (captured from devin 3000.11.234 in the sandbox).
    assert_eq!(
        ev(
            r#"{"type":"update","update":{"_meta":{"cognition.ai/inferenceToolName":"exec","terminal_exit":{"exit_code":1}},"content":[{"content":{"text":"boom","type":"text"},"type":"content"}],"sessionUpdate":"tool_call_update","status":"in_progress","toolCallId":"exec:0#2"}}"#
        ),
        [Event::ToolOutput {
            id: "exec:0#2".into(),
            text: "boom".into(),
            error: true
        }]
    );
    // What a call returned (captured from devin 3000.11.234): its content's text, before the end of the step.
    assert_eq!(
        ev(
            r#"{"type":"update","update":{"sessionUpdate":"tool_call_update","toolCallId":"m:1","status":"completed","content":[{"type":"content","content":{"type":"text","text":"a\nb"}}],"_meta":{"cognition.ai/inferenceToolName":"mcp__agenium__list_dir"}}}"#
        )[0],
        Event::ToolOutput {
            id: "m:1".into(),
            text: "a\nb".into(),
            error: false
        }
    );
    assert!(matches!(
        &ev(
            r#"{"type":"update","update":{"sessionUpdate":"tool_call_update","toolCallId":"exec:0#1","status":"completed","_meta":{"cognition.ai/inferenceToolName":"exec"}}}"#
        )[0],
        Event::Tool { done: true, .. }
    ));
    match &ev(
        r#"{"type":"turn","result":{"stopReason":"end_turn","usage":{"inputTokens":18000,"outputTokens":30,"cachedReadTokens":17900}}}"#,
    )[0]
    {
        Event::TurnEnd { usage, error, .. } => {
            assert_eq!(
                (usage.input, usage.cache_read, usage.output),
                (100, 17_900, 30)
            );
            assert!(error.is_none());
        }
        e => panic!("{e:?}"),
    }
    assert!(Devin.is_agenium_call("mcp__agenium__submit_report", &Value::Null, "agenium"));
    assert!(!Devin.is_agenium_call("mcp__github__search", &Value::Null, "agenium"));
}

#[test]
fn files_are_read_and_written_in_the_worktree_only() {
    let w = std::env::temp_dir().join("agenium-devin-check").join("wt");
    let h = std::env::temp_dir()
        .join("agenium-devin-check")
        .join("home");
    let cx = |read_only| CheckCtx {
        worktree: &w,
        home: &h,
        mcp_name: "agenium",
        subagents: false,
        read_only,
    };
    let ask = |tool: &str, path: &Path| json!({"tool_name": tool, "tool_input": {"path": path}});
    let inside_file = w.join("src").join("a.rs");
    let outside = std::env::temp_dir().join("elsewhere.txt");
    assert_eq!(
        Devin.check(&ask("read", &inside_file), &cx(true)),
        Check::Allow
    );
    assert_eq!(
        Devin.check(&ask("write", &inside_file), &cx(false)),
        Check::Allow
    );
    assert!(matches!(
        Devin.check(&ask("write", &inside_file), &cx(true)),
        Check::Deny(_)
    ));
    assert!(matches!(
        Devin.check(&ask("write", &outside), &cx(false)),
        Check::Deny(_)
    ));
    assert!(matches!(
        Devin.check(&ask("read", &outside), &cx(false)),
        Check::Deny(_)
    ));
    assert!(matches!(
        Devin.check(&ask("write", &w.join(LOCAL_MCP)), &cx(false)),
        Check::Deny(_)
    ));
    let mcp = |n: &str| json!({"tool_name": n, "tool_input": {}});
    assert_eq!(
        Devin.check(&mcp("mcp__agenium__submit_report"), &cx(false)),
        Check::Allow
    );
    assert!(matches!(
        Devin.check(&mcp("mcp__github__search"), &cx(false)),
        Check::Deny(_)
    ));
}

#[test]
fn a_model_is_its_name_and_its_effort_apart() {
    let m = parse_models(
            r#"{"families":[{"family_uid":"swe-2","variants":[
                {"model_uid":"swe-2-high","label":"SWE-2 High","max_context_tokens":262000},
                {"model_uid":"swe-2-medium","label":"SWE-2 Medium","max_context_tokens":262000},
                {"model_uid":"swe-2-max","label":"SWE-2 Max","max_context_tokens":262000},
                {"model_uid":"swe-2-medium-lightning","label":"SWE-2 Medium Lightning"},
                {"model_uid":"swe-2-max-lightning","label":"SWE-2 Max Lightning"}]},
              {"family_uid":"adaptive","variants":[{"model_uid":"adaptive","label":"Adaptive"}]},
              {"family_uid":"glm-5.2","variants":[{"model_uid":"glm-5-2"},{"model_uid":"glm-5-2-max"},{"model_uid":"glm-5-2-max-1m"}]},
              {"family_uid":"swe-1.7","variants":[{"model_uid":"swe-1-7","label":"SWE-1.7 Max"},{"model_uid":"swe-1-7-medium","label":"SWE-1.7 Medium"}]},
              {"family_uid":"fusion","variants":[{"model_uid":"fusion-claude-opus-5-5-medium-sidekick-swe-2-medium"}]}]}"#,
        )
        .unwrap();
    let (m, variants) = m;
    let ids: Vec<&str> = m.iter().map(|x| x.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "swe-2",
            "swe-2-lightning",
            "adaptive",
            "glm-5-2",
            "glm-5-2-1m",
            "swe-1-7"
        ]
    );
    let names = |i: usize| {
        m[i].efforts
            .clone()
            .unwrap()
            .iter()
            .map(|e| e.as_str().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(names(0), ["medium", "high", "max"]);
    assert_eq!(m[0].display_name.as_deref(), Some("SWE-2"));
    assert_eq!(
        m[0].default_effort.as_ref().map(|e| e.as_str()),
        Some("high")
    );
    assert_eq!(m[0].context_window, Some(262_000));
    assert_eq!(names(1), ["medium", "max"]);
    assert!(names(2).is_empty());
    // The variant with no effort in its name, at the level its label names.
    assert_eq!(names(5), ["medium", "max"]);
    assert_eq!(
        variants[&("swe-1-7".to_owned(), "max".to_owned())],
        "swe-1-7"
    );
    assert_eq!(m[1].display_name.as_deref(), Some("SWE-2 Lightning"));
    assert_eq!(m[5].display_name.as_deref(), Some("SWE-1.7"));
    // Run as Devin names it.
    assert_eq!(variant("swe-2", Some(&Effort::MAX)), "swe-2-max");
    assert_eq!(
        variant("swe-2-lightning", Some(&Effort::HIGH)),
        "swe-2-high-lightning"
    );
    assert_eq!(variant("adaptive", None), "adaptive");
    assert_eq!(
        variant("swe-2-max", Some(&Effort::MEDIUM)),
        "swe-2-max",
        "an agent saved with the old name"
    );
    *VARIANTS.write().unwrap() = Some(variants);
    assert_eq!(variant("swe-1-7", Some(&Effort::MAX)), "swe-1-7");
    assert_eq!(variant("swe-1-7", Some(&Effort::MEDIUM)), "swe-1-7-medium");
    let a = Devin.args("swe-2", Some(&Effort::MAX), Path::new("/h"), None, false);
    assert_eq!(
        a[a.len() - 2..],
        ["--model".to_owned(), "swe-2-max".to_owned()]
    );
}

#[test]
fn the_run_sees_devins_login_but_not_the_users_plugins_or_config() {
    let read = Devin.read_roots(Path::new("/bin/devin"));
    let names: Vec<String> = read
        .iter()
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();
    for f in ["config.json", "mcp_config.json", "cli", "devin", "plugins"] {
        assert!(!names.iter().any(|n| n == f), "{f} must not be readable");
    }
    assert!(Devin
        .writable_roots()
        .iter()
        .all(|p| p.file_name().is_some_and(|n| n != "plugins")));
}

#[test]
fn the_users_servers_are_turned_off_and_the_rest_kept() {
    let v = local_mcp(
        Some(r#"{"mcpServers":{"mine":{"command":"x"}},"other":1}"#),
        &["github".into(), "memory".into()],
    );
    assert_eq!(v["mcpServers"]["github"], json!({"disabled": true}));
    assert_eq!(v["mcpServers"]["memory"], json!({"disabled": true}));
    assert_eq!(v["mcpServers"]["mine"]["command"], "x");
    assert_eq!(v["other"], 1);
    assert_eq!(
        local_mcp(Some("not json"), &["a".into()])["mcpServers"]["a"]["disabled"],
        true
    );
}

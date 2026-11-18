use super::*;

#[test]
fn the_level_picks_the_models_variant() {
    let model = |id: &str, e: Option<&Effort>| {
        let a = Agy.args(id, e, Path::new("/h"), None, false);
        let i = a.iter().position(|x| x == "--model").unwrap();
        assert!(!a.contains(&"--effort".to_owned()));
        a[i + 1].clone()
    };
    assert_eq!(
        model("gemini-3.8-flash", Some(&Effort::LOW)),
        "gemini-3.8-flash-low"
    );
    assert_eq!(
        model("gemini-3.8-flash-high", Some(&Effort::LOW)),
        "gemini-3.8-flash-high"
    );
    assert_eq!(model("gemini-4-argon", None), "gemini-4-argon");
    // A ticket's earlier conversation goes on (ADR-134); `-p=` stays last.
    let a = Agy.args("m", None, Path::new("/h"), Some("c1"), false);
    assert_eq!(&a[a.len() - 3..], ["--conversation", "c1", "-p="]);
    let g = group(parse_models(
            "x\ngemini-4-argon-medium\tA\ngemini-4-argon-high\tA\ngemini-4-argon-low\tA\ngpt-oss-120b-medium\tG\n",
        ));
    let names = |i: usize| {
        g[i].efforts
            .as_ref()
            .unwrap()
            .iter()
            .map(|e| e.as_str())
            .collect::<Vec<_>>()
            .join(",")
    };
    assert_eq!(
        (g[0].id.as_str(), names(0).as_str()),
        ("gemini-4-argon", "low,medium,high")
    );
    assert_eq!(
        (g[1].id.as_str(), names(1).as_str()),
        ("gpt-oss-120b", "medium")
    );
}

/// Lines captured from agy 1.4.4 (shortened).
#[test]
fn parses_the_stream() {
    let a = Agy;
    let ev = |s: &str| a.parse(&serde_json::from_str(s).unwrap());
    assert_eq!(
        ev(r#"{"event":"init","conversation_id":"c1","init":{"cwd":"x"}}"#),
        vec![Event::Session("c1".into())]
    );
    assert_eq!(
        ev(
            r#"{"event":"step_update","step_update":{"step_index":1,"state":"DONE","step_type":"agent_response","text_delta":"PONG"}}"#
        ),
        vec![Event::Text("PONG".into())]
    );
    // What a step returned (captured from agy 1.5.0): its output when done, its error when refused.
    let done = Agy.parse(&serde_json::json!({"event":"step_update","step_update":{"step_index":3,"state":"DONE","step_type":"tool","tool_name":"call_mcp_tool","tool_info":{"name":"call_mcp_tool","output":"agenium.toml","parameters":{}}}}));
    assert_eq!(
        done[0],
        Event::ToolOutput {
            id: "3".into(),
            text: "agenium.toml".into(),
            error: false
        }
    );
    let refused = Agy.parse(&serde_json::json!({"event":"step_update","step_update":{"step_index":2,"state":"ERROR","step_type":"tool","tool_name":"run_command","tool_info":{"error":{"message":"denied"},"parameters":{}}}}));
    assert_eq!(
        refused[0],
        Event::ToolOutput {
            id: "2".into(),
            text: "denied".into(),
            error: true
        }
    );
    assert_eq!(
        ev(
            r#"{"event":"step_update","step_update":{"step_index":4,"state":"ACTIVE","step_type":"tool","tool_name":"run_command","tool_info":{"name":"run_command","parameters":{"CommandLine":"echo hi"}}}}"#
        ),
        vec![Event::Tool {
            id: "4".into(),
            name: "run_command".into(),
            input: json!({"CommandLine": "echo hi"}),
            done: false
        }]
    );
    let end = ev(
        r#"{"event":"result","result":{"status":"SUCCESS","response":"","usage":{"input_tokens":23438,"output_tokens":165,"thinking_tokens":0,"cache_read_tokens":438,"total_tokens":23603},"denied_actions":[{"action":"command","display_name":"RunCommand"}]}}"#,
    );
    match &end[0] {
        Event::TurnEnd {
            usage,
            denied,
            error,
            ..
        } => {
            assert_eq!(
                (usage.input, usage.cache_read, usage.output),
                (23000, 438, 165)
            );
            assert_eq!(denied, &vec!["command".to_owned()]);
            assert!(error.is_none());
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn checks_tool_calls() {
    let wt = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let cx = CheckCtx {
        worktree: wt.path(),
        home: home.path(),
        mcp_name: "agenium",
        subagents: false,
        read_only: false,
    };
    let solo = CheckCtx {
        subagents: true,
        read_only: false,
        ..cx
    };
    // Team members never start agy's subagents; Solo agents may. agy's own scheduling, messaging and
    // interactive questions are withheld in both.
    let subagent = json!({"toolCall": {"name": "invoke_subagent", "args": {}}});
    assert_eq!(Agy.check(&subagent, &cx), Check::Deny(NO_SUBAGENTS.into()));
    assert_eq!(Agy.check(&subagent, &solo), Check::Allow);
    for name in [
        "schedule",
        "send_message",
        "generate_image",
        "ask_question",
        "search_marketplace",
    ] {
        let c = Agy.check(&json!({"toolCall": {"name": name, "args": {}}}), &solo);
        assert!(matches!(c, Check::Deny(_)), "{name}: {c:?}");
    }
    let call = |name: &str, args: Value| {
        Agy.check(&json!({"toolCall": {"name": name, "args": args}}), &cx)
    };
    // ADR-129: its own command tools are off; its own file and search tools do not reach the worktree, nor act on
    // it without a path.
    for name in ["run_command", "command_status", "send_command_input"] {
        assert_eq!(
            call(name, json!({"CommandLine": "cargo test"})),
            Check::Deny(own_tool_off(name))
        );
    }
    for name in [
        "write_to_file",
        "view_file",
        "list_dir",
        "grep_search",
        "find_by_name",
        "codebase_search",
        "replace_file_content",
        "multi_replace_file_content",
    ] {
        let c = call(name, json!({"Path": wt.path().join("a.txt")}));
        assert!(
            matches!(&c, Check::Deny(r) if r.contains("read_file")),
            "{name}: {c:?}"
        );
        assert_eq!(
            call(name, json!({"Query": "x"})),
            Check::Deny(own_tool_off(name))
        );
    }
    // Agenium's MCP resources are read by URI: not a file, not refused (2026-10-28: refused as a worktree path).
    assert_eq!(
        call(
            "read_resource",
            json!({"ServerName": "agenium", "Uri": "agenium://schemas/submit_report"})
        ),
        Check::Allow
    );
    assert!(matches!(
        call(
            "read_resource",
            json!({"ServerName": "figma", "Uri": "figma://x"})
        ),
        Check::Deny(_)
    ));
    // agy's other known tools (no file, no command) stay; one Agenium does not know is refused (audit M14).
    assert_eq!(
        call("task_boundary", json!({"TaskName": "x"})),
        Check::Allow
    );
    assert_eq!(
        call("notify_user", json!({"Message": "done"})),
        Check::Allow
    );
    for name in ["read_terminal", "trajectory_search", "exec_shell_v2"] {
        assert!(matches!(call(name, json!({})), Check::Deny(_)), "{name}");
    }
    // A move names its source as a path (it was read as text): one out of reach is refused.
    let outside = tempfile::tempdir().unwrap();
    assert!(matches!(
        call(
            "move_file",
            json!({"Source": outside.path().join("secret.txt"), "Destination": home.path().join("notes/x.txt")})
        ),
        Check::Deny(_)
    ));
    assert_eq!(
        call(
            "call_mcp_tool",
            json!({"ServerName": "agenium", "ToolName": "submit_report"})
        ),
        Check::Allow
    );
    assert!(matches!(
        call("call_mcp_tool", json!({"ServerName": "figma"})),
        Check::Deny(_)
    ));
    assert!(matches!(
        call("read_url_content", json!({"Url": "https://example.com"})),
        Check::Deny(_)
    ));
    assert!(matches!(
        call("browser_click_element", json!({})),
        Check::Deny(_)
    ));
}

/// Review of 2026-10-27: the hook script and agy's config in the per-run home, and the worktree's `.git`, were
/// writable by the model's file tools; the hook's call had no deadline under agy's hook timeout.
#[test]
fn agenium_files_and_dot_git_are_out_of_reach() {
    let wt = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let cx = CheckCtx {
        worktree: wt.path(),
        home: home.path(),
        mcp_name: "agenium",
        subagents: false,
        read_only: false,
    };
    let call = |name: &str, p: std::path::PathBuf| {
        Agy.check(
            &json!({"toolCall": {"name": name, "args": {"TargetFile": p}}}),
            &cx,
        )
    };
    let config = home.path().join(".gemini").join("config");
    for (name, p) in [
        ("write_to_file", config.join(hook_script().1)),
        ("replace_file_content", config.join("hooks.json")),
        ("view_file", config.join("mcp_config.json")),
        (
            "write_to_file",
            home.path().join(".gemini/antigravity-cli/settings.json"),
        ),
        ("write_to_file", wt.path().join(".git/hooks/post-commit")),
        ("some_new_tool", wt.path().join(".git/config")),
    ] {
        assert!(
            matches!(call(name, p.clone()), Check::Deny(_)),
            "{name} {}",
            p.display()
        );
    }
    assert!(matches!(
        call("view_file", wt.path().join(".git/HEAD")),
        Check::Deny(_)
    ));
    assert_eq!(
        call(
            "write_to_file",
            home.path().join(".gemini/antigravity/brain/c1/task.md")
        ),
        Check::Allow
    );
    assert!(hook_script().2.contains("--max-time 800"));
}

#[test]
fn lists_models() {
    // `agy models` on agy 1.4.7.
    let out = "Fetching available models...
gemini-4-argon-medium	Gemini 4 Argon (Medium)
gemini-3.8-flash-low	Gemini 3.8 Flash (Low)
";
    assert_eq!(
        parse_models(out),
        ["gemini-4-argon-medium", "gemini-3.8-flash-low"]
    );
}

#[test]
fn writes_an_isolated_home() {
    let home = tempfile::tempdir().unwrap();
    let env = Agy
        .prepare(&Setup {
            home: home.path(),
            worktree: Path::new("/wt"),
            mcp_name: "agenium",
            mcp_url: "http://127.0.0.1:1/mcp",
            mcp_authorization: "Bearer t",
            hook_url: "http://127.0.0.1:1/hook",
            subagents: false,
            model: "",
            run_for: std::time::Duration::from_secs(2700),
        })
        .unwrap();
    assert_eq!(env["USERPROFILE"], home.path().display().to_string());
    assert_eq!(env[HOOK_TOKEN_ENV], "t");
    let read = |p: &str| -> Value {
        serde_json::from_slice(&std::fs::read(home.path().join(p)).unwrap()).unwrap()
    };
    let mcp = read(".gemini/config/mcp_config.json");
    assert_eq!(mcp["mcpServers"].as_object().unwrap().len(), 1);
    assert_eq!(
        mcp["mcpServers"]["agenium"]["headers"]["Authorization"],
        "Bearer t"
    );
    let hooks = read(".gemini/config/hooks.json");
    let cmd = hooks["agenium-permissions"]["PreToolUse"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap();
    assert!(cmd.contains("agenium-hook") && !cmd.contains('"'), "{cmd}");
    let script =
        std::fs::read_to_string(home.path().join(".gemini/config").join(hook_script().1)).unwrap();
    assert!(script.contains("--fail-with-body") && script.contains(HOOK_URL_ENV));
    assert!(
        !script.contains("Bearer t"),
        "the token stays in the environment"
    );
    let s = read(".gemini/antigravity-cli/settings.json");
    assert_eq!(s["allowNonWorkspaceAccess"], false);
    assert!(s.get("permissions").is_none());
}

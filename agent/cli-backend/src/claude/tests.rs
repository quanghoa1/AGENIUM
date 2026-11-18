use super::*;

/// Lines captured from claude 2.1.310 (shortened).
/// ADR-129: a subagent's messages carry the id of the `Agent` call that started it; its steps come wrapped with it.
#[test]
fn a_subagents_steps_are_wrapped_with_the_call_that_started_it() {
    let c = Claude;
    let ev = |s: &str| c.parse(&serde_json::from_str(s).unwrap());
    assert_eq!(
        ev(
            r#"{"type":"assistant","parent_tool_use_id":"toolu_A","message":{"content":[{"type":"tool_use","id":"toolu_2","name":"mcp__agenium__grep","input":{"pattern":"x"}}]}}"#
        ),
        vec![Event::Subagent {
            parent: "toolu_A".into(),
            events: vec![Event::Tool {
                id: "toolu_2".into(),
                name: "mcp__agenium__grep".into(),
                input: json!({"pattern": "x"}),
                done: false
            }]
        }]
    );
    // The main agent's messages (no parent, or a null one) are not wrapped.
    assert!(matches!(
            ev(r#"{"type":"assistant","parent_tool_use_id":null,"message":{"content":[{"type":"text","text":"hi"}]}}"#)
                .as_slice(),
            [Event::Text(_)]
        ));
    assert!(c.starts_subagent("Agent") && c.starts_subagent("Task") && !c.starts_subagent("Read"));
}

/// `--include-partial-messages`: the main conversation's text and reasoning pieces as they are written; a
/// subagent's pieces and the stream's other events are left out (its whole messages still come).
#[test]
fn partial_messages_come_as_pieces() {
    let c = Claude;
    let ev = |s: &str| c.parse(&serde_json::from_str(s).unwrap());
    assert_eq!(
        ev(
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Team "}},"parent_tool_use_id":null,"session_id":"s1"}"#
        ),
        vec![Event::Partial("Team ".into())]
    );
    assert_eq!(
        ev(
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"The user wants"}},"parent_tool_use_id":null}"#
        ),
        vec![Event::Thinking("The user wants".into())]
    );
    assert!(ev(r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"x"}},"parent_tool_use_id":"toolu_9"}"#).is_empty());
    assert!(ev(r#"{"type":"stream_event","event":{"type":"message_start","message":{}},"parent_tool_use_id":null}"#).is_empty());
    assert!(ev(r#"{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"a\""}},"parent_tool_use_id":null}"#).is_empty());
}

#[test]
fn parses_the_stream() {
    let c = Claude;
    let ev = |s: &str| c.parse(&serde_json::from_str(s).unwrap());
    assert_eq!(
        ev(
            r#"{"type":"system","subtype":"init","session_id":"s1","mcp_servers":[{"name":"agenium","status":"connected"}]}"#
        ),
        vec![Event::Session("s1".into())]
    );
    assert_eq!(
        ev(
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"On it."},{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"git status"}}]}}"#
        ),
        vec![
            Event::Text("On it.\n".into()),
            Event::Tool {
                id: "toolu_1".into(),
                name: "Bash".into(),
                input: json!({"command": "git status"}),
                done: false
            }
        ]
    );
    assert_eq!(
        ev(
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"ok"}]}}"#
        ),
        vec![
            Event::ToolOutput {
                id: "toolu_1".into(),
                text: "ok".into(),
                error: false
            },
            Event::Tool {
                id: "toolu_1".into(),
                name: String::new(),
                input: Value::Null,
                done: true
            }
        ]
    );
    assert_eq!(
        stream::result_text(
            &serde_json::json!([{"type":"text","text":"a"},{"type":"tool_reference","tool_name":"mcp__agenium__create_ticket"}])
        ),
        "a\ntool: mcp__agenium__create_ticket"
    );
    match &ev(
        r#"{"type":"result","subtype":"success","is_error":false,"result":"Done.","usage":{"input_tokens":10,"cache_creation_input_tokens":7564,"cache_read_input_tokens":114296,"output_tokens":764,"output_tokens_details":{"thinking_tokens":226}}}"#,
    )[0]
    {
        Event::TurnEnd {
            usage,
            response,
            error,
            ..
        } => {
            assert_eq!(
                (
                    usage.input,
                    usage.cache_read,
                    usage.cache_write,
                    usage.output,
                    usage.reasoning
                ),
                (10, 114296, 7564, 764, 226)
            );
            assert_eq!(response, "Done.");
            assert!(error.is_none());
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        &ev(r#"{"type":"result","subtype":"error_max_turns","is_error":true,"result":""}"#)[0],
        Event::TurnEnd { error: Some(_), .. }
    ));
    // The subscription usage the CLI reports on its own (claude 2.1.310).
    match &ev(
        r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed","resetsAt":1792903200,"rateLimitType":"five_hour","unifiedWindows":{"five_hour":{"utilization":0.09,"resetsAt":1792903200,"status":"allowed"},"seven_day":{"utilization":0.18,"resetsAt":1793224800,"status":"allowed"}}}}"#,
    )[0]
    {
        Event::Limits(u) => {
            assert_eq!(u.vendor, "claude");
            assert_eq!(u.windows.len(), 2);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn checks_tool_calls_and_answers_like_claude_code() {
    let wt = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let cx = CheckCtx {
        worktree: wt.path(),
        home: home.path(),
        mcp_name: "agenium",
        subagents: false,
        read_only: false,
    };
    // Allowlist: subagents only for Solo; scheduling, notifications, plugins, worktrees... never.
    let task = json!({"tool_name": "Task", "tool_input": {"prompt": "x"}});
    assert_eq!(Claude.check(&task, &cx), Check::Deny(NO_SUBAGENTS.into()));
    // Its current name (Claude Code 2.1.312, a Solo run on 2026-10-28 was refused under it).
    let agent = json!({"tool_name": "Agent", "tool_input": {"prompt": "x"}});
    assert_eq!(Claude.check(&agent, &cx), Check::Deny(NO_SUBAGENTS.into()));
    let solo = CheckCtx {
        subagents: true,
        read_only: false,
        ..cx
    };
    assert_eq!(Claude.check(&task, &solo), Check::Allow);
    assert_eq!(Claude.check(&agent, &solo), Check::Allow);
    for name in [
        "CronCreate",
        "RemoteTrigger",
        "PushNotification",
        "PublishPlugin",
        "EnterWorktree",
        "Workflow",
        "Skill",
    ] {
        let c = Claude.check(&json!({"tool_name": name, "tool_input": {}}), &solo);
        assert!(matches!(c, Check::Deny(_)), "{name}: {c:?}");
    }
    let call = |name: &str, input: Value| {
        Claude.check(&json!({"tool_name": name, "tool_input": input}), &cx)
    };
    // ADR-129: its shells are off; its own file and search tools do not reach the worktree (the refusal names
    // Agenium's tools), nor act on it without a path.
    for name in ["Bash", "PowerShell", "NotebookEdit"] {
        assert_eq!(
            call(name, json!({"command": "cargo test"})),
            Check::Deny(own_tool_off(name))
        );
    }
    assert!(own_tool_off("Bash").contains("shell for every command"));
    for name in ["Write", "Read", "Edit", "Glob", "Grep"] {
        assert!(
            matches!(call(name, json!({"file_path": wt.path().join("a.txt")})), Check::Deny(r) if r.contains("read_file")),
            "{name}"
        );
        assert_eq!(
            call(name, json!({"pattern": "x"})),
            Check::Deny(own_tool_off(name))
        );
    }
    assert!(matches!(
        call("Edit", json!({"file_path": wt.path().join("../x.txt")})),
        Check::Deny(_)
    ));
    assert!(matches!(
        call(
            "Read",
            json!({"file_path": std::env::temp_dir().join("secret.txt")})
        ),
        Check::Deny(_)
    ));
    assert_eq!(call("mcp__agenium__submit_report", json!({})), Check::Allow);
    assert!(matches!(
        call("mcp__github__create_issue", json!({})),
        Check::Deny(_)
    ));
    assert!(matches!(
        call("WebFetch", json!({"url": "https://x"})),
        Check::Deny(_)
    ));
    // ADR-163: its search runs on Anthropic's servers.
    assert_eq!(call("WebSearch", json!({"query": "x"})), Check::Allow);
    assert!(matches!(
        call("ToolSearch", json!({"query": "agenium"})),
        Check::Deny(_)
    ));
    assert!(
        matches!(call("TodoWrite", json!({"todos": []})), Check::Deny(_)),
        "not in the allowlist"
    );
    assert_eq!(
        Claude.hook_answer(false, "no")["hookSpecificOutput"]["permissionDecision"],
        "deny"
    );
}

/// Review of 2026-10-27: the per-run home was open to the file tools, the hook script included (rewritten to
/// `exit 0`, every later shell call skips the gate), and a write into the worktree's `.git` (a hook runs on the host
/// at the next git call) was allowed.
#[test]
fn agenium_files_in_the_home_and_dot_git_are_out_of_reach() {
    let wt = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(wt.path().join(".git").join("hooks")).unwrap();
    let cx = CheckCtx {
        worktree: wt.path(),
        home: home.path(),
        mcp_name: "agenium",
        subagents: false,
        read_only: false,
    };
    let call = |name: &str, key: &str, p: &Path| {
        Claude.check(&json!({"tool_name": name, "tool_input": {key: p}}), &cx)
    };
    let (h, git) = (home.path(), wt.path().join(".git"));
    for (name, key, p) in [
        ("Write", "file_path", h.join(HOOK_FILE)),
        ("Edit", "file_path", h.join("settings.json")),
        ("Write", "file_path", h.join(MCP_FILE)),
        ("Read", "file_path", h.join(LOGIN)),
        ("Grep", "path", h.to_path_buf()),
        ("Write", "file_path", git.join("hooks").join("post-commit")),
        ("Edit", "file_path", wt.path().join("sub/.GIT/config")),
    ] {
        assert!(
            matches!(call(name, key, &p), Check::Deny(_)),
            "{name} {}",
            p.display()
        );
    }
    assert!(matches!(
        call("Read", "file_path", &git.join("HEAD")),
        Check::Deny(_)
    ));
    for (name, key, p) in [
        ("Write", "file_path", h.join("projects").join("notes.md")),
        ("Read", "file_path", h.join("projects/p/tool-results/1.txt")),
        ("Grep", "path", h.join("projects")),
    ] {
        assert_eq!(call(name, key, &p), Check::Allow, "{name} {}", p.display());
    }
}

fn files_below(dir: &Path) -> Vec<PathBuf> {
    let mut out = vec![];
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        if e.path().is_dir() {
            out.extend(files_below(&e.path()));
        } else {
            out.push(e.path());
        }
    }
    out
}

/// Review of 2026-10-27: the usage probe left its copy of the user's login (an OAuth refresh token) in the
/// persistent scratch folder; the hook's call to Agenium had no deadline under the CLI's hook timeout, past which
/// Claude Code goes on with the tool.
#[tokio::test]
async fn no_login_stays_behind_and_the_hook_ends_before_the_cli_gives_up() {
    let user = tempfile::tempdir().unwrap();
    std::fs::write(user.path().join(LOGIN), "{\"refresh\":\"r\"}").unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let legacy = scratch.path().join("claude-home");
    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::write(legacy.join(LOGIN), "left by an earlier version").unwrap();
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("CLAUDE_CONFIG_DIR", user.path());
    let probe = rate_limits(&scratch.path().join("no-such-claude"), scratch.path()).await;
    let env = Claude.prepare(&Setup {
        home: home.path(),
        worktree: Path::new("."),
        mcp_name: "agenium",
        mcp_url: "http://127.0.0.1:9/mcp",
        mcp_authorization: "Bearer t",
        hook_url: "http://127.0.0.1:9/hook",
        subagents: false,
        model: "",
        run_for: std::time::Duration::from_secs(2700),
    });
    std::env::remove_var("CLAUDE_CONFIG_DIR");
    assert!(probe.is_err());
    let left = files_below(scratch.path());
    assert!(left.is_empty(), "{left:?}");
    env.unwrap();
    let script = std::fs::read_to_string(home.path().join(HOOK_FILE)).unwrap();
    assert!(
        script.contains("--max-time 800") && script.contains("exit 2"),
        "{script}"
    );
    let settings: Value =
        serde_json::from_slice(&std::fs::read(home.path().join("settings.json")).unwrap()).unwrap();
    assert_eq!(
        settings["hooks"]["PreToolUse"][0]["hooks"][0]["timeout"],
        900
    );
    assert!(crate::driver::hook_deadline(&Claude) < std::time::Duration::from_secs(800));
}

#[test]
fn the_hook_command_quotes_a_path_with_a_space_or_a_quote() {
    assert_eq!(
        hook_command(Path::new("/tmp/John Smith/agenium-hook.sh")),
        "sh '/tmp/John Smith/agenium-hook.sh'"
    );
    assert_eq!(
        hook_command(Path::new("/tmp/O'Brien/h.sh")),
        r"sh '/tmp/O'\''Brien/h.sh'"
    );
}

#[test]
fn args_point_at_the_per_run_home() {
    let a = Claude.args("haiku", Some(&Effort::LOW), Path::new("/h"), None, false);
    let at = a.iter().position(|x| x == "--effort").unwrap();
    assert_eq!(a[at + 1], "low");
    let o = Claude.args(
        "claude-opus-4-5",
        Some(&Effort::MAX),
        Path::new("/h"),
        None,
        false,
    );
    let at = o.iter().position(|x| x == "--effort").unwrap();
    assert_eq!(o[at + 1], "high", "Opus 4.5 ends at high");
    assert!(!Claude
        .args("haiku", None, Path::new("/h"), None, false)
        .contains(&"--effort".to_owned()));
    assert!(a.contains(&"--strict-mcp-config".to_owned()));
    assert!(a.contains(&"--disable-slash-commands".to_owned()));
    let i = a.iter().position(|x| x == "--tools").unwrap();
    assert_eq!(a[i + 1], "Read,Write,Edit,Glob,Grep,WebSearch");
    let s = Claude.args("haiku", None, Path::new("/h"), None, true);
    let i = s.iter().position(|x| x == "--tools").unwrap();
    assert!(s[i + 1].ends_with(",Agent,Task,TaskOutput,SendMessage,ListAgents,TaskStop"));
    let i = a.iter().position(|x| x == "--setting-sources").unwrap();
    assert_eq!(a[i + 1], "user");
    let i = a.iter().position(|x| x == "--mcp-config").unwrap();
    assert!(a[i + 1].ends_with(MCP_FILE));
}

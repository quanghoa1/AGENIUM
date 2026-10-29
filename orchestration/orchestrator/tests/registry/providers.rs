//! Library skills, MCP servers, providers and their endpoints, projects.

use super::*;

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
            approval: None,
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
            headers: None,
            body: None,
            effort_field: None,
            model_caps: None,
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
            headers: None,
            body: None,
            effort_field: None,
            model_caps: None,
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
        headers: None,
        body: None,
        effort_field: None,
        model_caps: None,
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
    // A preset URL whose <placeholder> was not filled in (ADR-111).
    assert!(bad(p(
        "azure-openai",
        ProviderKind::OpenaiCompat,
        Some("https://<resource>.openai.azure.com/openai/v1")
    )));

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
                effort: Effort::MEDIUM,
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

/// ADR-107: every known vendor's preset builds an adapter (with a placeholder key), and its endpoint is the one the
/// adapter will call.
#[test]
fn every_vendor_preset_builds_an_adapter() {
    for pr in agenium_config::presets::PRESETS {
        let spec = agenium_llm::ProviderSpec {
            custom: Default::default(),
            id: pr.id.into(),
            kind: match pr.kind {
                agenium_config::ProviderKind::Anthropic => "anthropic",
                agenium_config::ProviderKind::Gemini => "gemini",
                _ => "openai_compat",
            }
            .into(),
            base_url: pr.base_url.map(str::to_owned),
            api_key: (!pr.key_env.is_empty()).then(|| "placeholder".to_owned()),
            max_concurrency: 2,
        };
        let p = agenium_llm::build_provider(&spec).unwrap_or_else(|e| panic!("{}: {e}", pr.id));
        assert_eq!(p.id(), pr.id);
    }
}

/// A custom endpoint's extra headers: a plain one is written as it is, a secret one goes to the vault behind a
/// placeholder (never into config.toml) and an empty value keeps it; per-model effort levels are kept and checked.
#[test]
fn custom_endpoint_headers_and_model_settings_are_stored() {
    let f = fixture();
    let put = |headers: Vec<HeaderInput>, caps: Option<Vec<agenium_config::ModelCaps>>| {
        f.app.put_provider(&ProviderInput {
            headers: Some(headers),
            body: None,
            effort_field: None,
            model_caps: caps,
            id: "gw".into(),
            kind: ProviderKind::OpenaiCompat,
            base_url: Some("https://gateway.example/v1".into()),
            api_key: Some("gw-key-123".into()),
            clear_key: false,
            max_concurrency: 2,
            models: vec!["house-model".into()],
        })
    };
    let h = |name: &str, value: Option<&str>, secret: bool| HeaderInput {
        name: name.into(),
        value: value.map(str::to_owned),
        secret,
    };
    let caps = agenium_config::ModelCaps {
        model: "house-model".into(),
        tools: Some(true),
        vision: None,
        context_window: Some(64_000),
        efforts: Some(vec!["low".into(), "high".into()]),
        default_effort: Some("high".into()),
    };
    let pc = put(
        vec![
            h("X-Tenant", Some("team-7"), false),
            h("X-Gateway-Token", Some("tok-s3cret"), true),
        ],
        Some(vec![caps.clone()]),
    )
    .unwrap();
    assert_eq!(pc.headers["X-Tenant"], "team-7");
    assert_eq!(
        pc.headers["X-Gateway-Token"],
        "${secret:provider/gw/header/x-gateway-token}"
    );
    assert_eq!(pc.model_caps, vec![caps.clone()]);
    let toml = std::fs::read_to_string(f.app.layout().config()).unwrap();
    assert!(!toml.contains("tok-s3cret"), "{toml}");
    assert!(f
        .app
        .secret("provider/gw/header/x-gateway-token")
        .unwrap()
        .is_some());
    // Saved again without the secret's value: the stored one stays; a header left out loses its secret.
    let pc = put(vec![h("X-Gateway-Token", None, true)], None).unwrap();
    assert_eq!(pc.headers.len(), 1);
    assert_eq!(pc.model_caps, vec![caps], "settings not sent are kept");
    put(vec![], None).unwrap();
    assert!(f
        .app
        .secret("provider/gw/header/x-gateway-token")
        .unwrap()
        .is_none());
    // A level with a space is not an effort level.
    let bad = agenium_config::ModelCaps {
        efforts: Some(vec!["very high".into()]),
        ..Default::default()
    };
    assert!(put(vec![], Some(vec![bad])).is_err());
}

/// Extra request fields of a custom endpoint: kept as typed when they follow the rules; Agenium's own fields, a
/// placeholder, an odd name, a reserved header or a header broken across lines are refused with the reason.
#[test]
fn extra_request_fields_follow_the_rules() {
    let f = fixture();
    let put = |body: Vec<(&str, &str, Option<&str>)>, effort: &str, headers: Vec<HeaderInput>| {
        f.app.put_provider(&ProviderInput {
            headers: Some(headers),
            body: Some(
                body.into_iter()
                    .map(|(p, v, m)| agenium_config::BodyField {
                        path: p.into(),
                        value: v.into(),
                        model: m.map(str::to_owned),
                    })
                    .collect(),
            ),
            effort_field: Some(effort.into()),
            model_caps: None,
            id: "gw".into(),
            kind: ProviderKind::OpenaiCompat,
            base_url: Some("https://gateway.example/v1".into()),
            api_key: Some("gw-key-123".into()),
            clear_key: false,
            max_concurrency: 2,
            models: vec!["house-model".into()],
        })
    };
    let pc = put(
        vec![
            ("temperature", "0.2", None),
            ("thinking.budget", " 2048 ", Some("house-model")),
        ],
        "reasoning.effort",
        vec![],
    )
    .unwrap();
    assert_eq!(pc.body.len(), 2);
    assert_eq!(pc.body[1].value, "2048");
    assert_eq!(pc.effort_field.as_deref(), Some("reasoning.effort"));
    for (path, value) in [
        ("model", "x"),
        ("messages", "[]"),
        ("stream", "false"),
        ("a b", "1"),
        ("x", "${secret:provider/gw}"),
    ] {
        let e = put(vec![(path, value, None)], "", vec![])
            .unwrap_err()
            .to_string();
        assert!(!e.is_empty(), "{path}");
    }
    assert!(
        put(vec![], "max_tokens", vec![]).is_err(),
        "the effort field cannot be one of Agenium's"
    );
    let header = |n: &str, v: &str| HeaderInput {
        name: n.into(),
        value: Some(v.into()),
        secret: false,
    };
    assert!(put(vec![], "", vec![header("Host", "evil.example")]).is_err());
    assert!(put(vec![], "", vec![header("X-A", "a\r\nX-Injected: 1")]).is_err());
    // An empty effort field goes back to Agenium's own.
    assert_eq!(put(vec![], "", vec![]).unwrap().effort_field, None);
}

/// End to end: what the user saved for a custom endpoint reaches it. A local server records one request made through
/// the configured provider: the user's headers (the secret one from the vault), the extra fields (the model's own
/// value winning) and the effort level at the user's field.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_custom_endpoint_receives_what_the_user_set() {
    use agenium_orchestrator::runtime::{ConfiguredProviders, ProviderSource};
    use std::io::{Read, Write};
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let seen = std::thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        let mut buf = Vec::new();
        let mut tmp = [0u8; 8192];
        loop {
            let n = s.read(&mut tmp).unwrap();
            buf.extend_from_slice(&tmp[..n]);
            let text = String::from_utf8_lossy(&buf).to_string();
            if let Some(h) = text.find("\r\n\r\n") {
                let len: usize = text[..h]
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().parse().unwrap())
                    })
                    .unwrap_or(0);
                if buf.len() >= h + 4 + len {
                    break;
                }
            }
        }
        let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
        let _ = s.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n{sse}").as_bytes());
        String::from_utf8_lossy(&buf).to_string()
    });
    let f = fixture();
    // The configured providers hold the app.
    let app = std::sync::Arc::new(open_app(
        &f.root.parent().unwrap().join("rt-root"),
        f.keyring.clone(),
        None,
    ));
    let field = |p: &str, v: &str, m: Option<&str>| agenium_config::BodyField {
        path: p.into(),
        value: v.into(),
        model: m.map(str::to_owned),
    };
    app.put_provider(&ProviderInput {
        headers: Some(vec![
            HeaderInput {
                name: "X-Tenant".into(),
                value: Some("team-7".into()),
                secret: false,
            },
            HeaderInput {
                name: "X-Gateway-Token".into(),
                value: Some("tok-s3cret".into()),
                secret: true,
            },
        ]),
        body: Some(vec![
            field("temperature", "0.2", None),
            field("temperature", "0.7", Some("house-model")),
            field("metadata", r#"{"team": "a"}"#, None),
        ]),
        effort_field: Some("reasoning.effort".into()),
        model_caps: Some(vec![agenium_config::ModelCaps {
            model: "house-model".into(),
            efforts: Some(vec!["low".into(), "high".into()]),
            ..Default::default()
        }]),
        id: "gw".into(),
        kind: ProviderKind::OpenaiCompat,
        base_url: Some(format!("http://127.0.0.1:{port}/v1")),
        api_key: Some("gw-key-123".into()),
        clear_key: false,
        max_concurrency: 1,
        models: vec!["house-model".into()],
    })
    .unwrap();
    let p = ConfiguredProviders::new(app.clone())
        .provider("gw")
        .unwrap();
    let req = agenium_llm::ChatRequest {
        model: "house-model".into(),
        system: vec![],
        messages: vec![agenium_llm::Message::user_text("hi")],
        tools: vec![],
        effort: Effort::HIGH,
        max_tokens: 100,
        cache_key: None,
        long_cache: false,
        tools_off: false,
    };
    let r = p.chat(&req, &|_| {}).await.unwrap();
    assert_eq!(r.message.text(), "ok");
    let raw = seen.join().unwrap();
    let lower = raw.to_ascii_lowercase();
    assert!(lower.contains("x-tenant: team-7"), "{raw}");
    assert!(
        lower.contains("x-gateway-token: tok-s3cret"),
        "the secret header is resolved from the vault"
    );
    let body: serde_json::Value =
        serde_json::from_str(&raw[raw.find("\r\n\r\n").unwrap() + 4..]).unwrap();
    assert_eq!(body["temperature"], 0.7);
    assert_eq!(body["metadata"]["team"], "a");
    assert_eq!(body["reasoning"]["effort"], "high");
    assert!(body.get("reasoning_effort").is_none());
    assert_eq!(body["model"], "house-model");
}

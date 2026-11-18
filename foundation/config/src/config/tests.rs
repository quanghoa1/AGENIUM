use super::*;

const EXAMPLE: &str = r#"
schema_version = 1
[app]
language = "vi"
# Removed setting (ADR-40 amendment): old files still load.
user_language = "vi"
[runtime]
max_parallel_agents = 2
[[providers]]
id = "anthropic"
kind = "anthropic"
api_key_ref = "provider/anthropic"
[[providers]]
id = "local-ollama"
kind = "openai_compat"
base_url = "http://localhost:11434/v1"
some_future_key = 1
[[providers.model_caps]]
model = "qwen3:8b"
tools = true
context_window = 32768
"#;

#[test]
fn partial_file_gets_defaults() {
    let c = Config::parse(EXAMPLE).unwrap();
    assert_eq!(c.app.language, "vi");
    assert_eq!(c.runtime.max_parallel_agents, 2);
    assert_eq!(c.runtime.max_steps_per_run, 200);
    assert_eq!(c.retry, RetrySection::default());
    assert_eq!(c.providers[1].max_concurrency, 4);
    let caps = c.providers[1].caps("qwen3:8b").unwrap();
    assert_eq!(
        (caps.tools, caps.vision, caps.context_window),
        (Some(true), None, Some(32768))
    );
    assert!(c.providers[0].caps("qwen3:8b").is_none());
    assert_eq!(c.secrets.backend, SecretsBackend::Vault);
    assert_eq!(c.sandbox.approval, ApprovalPolicy::Sandboxed);
    assert!(
        !c.sandbox.off && !c.sandbox.runs_unsandboxed(),
        "the sandbox is on by default (ADR-160)"
    );
    let mut off = c.sandbox.clone();
    off.off = true;
    assert!(
        off.runs_unsandboxed(),
        "turning the sandbox off runs commands without one"
    );
    assert!(
        c.sandbox.network,
        "sandboxed commands may reach the network by default (ADR-84)"
    );
}

#[test]
fn round_trip_and_checks() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("config.toml");
    let c = Config::parse(EXAMPLE).unwrap();
    c.save(&p).unwrap();
    assert_eq!(Config::load(&p).unwrap(), c);
    assert!(Config::parse("[app]\nlanguage = \"fr\"").is_err());
    assert!(Config::parse("schema_version = 99").is_err());
    assert!(Config::parse("[[providers]]\nid=\"x\"\nkind=\"openai_compat\"").is_err());
    assert!(Config::parse(
        "[[providers]]\nid=\"x\"\nkind=\"mock\"\n[[providers]]\nid=\"x\"\nkind=\"mock\""
    )
    .is_err());
}

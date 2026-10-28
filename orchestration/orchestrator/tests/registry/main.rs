//! ADR-61: agents created on their own into the pool, teams composed from them (Lead suggested, user decides), the
//! system skill library with per-agent switches, MCP switches, and providers entered by the user.

#[path = "../common/mod.rs"]
mod common;

use agenium_config::ProviderKind;
use agenium_core::*;
use agenium_orchestrator::registry::*;
use agenium_orchestrator::CallCtx;
use common::*;

mod agents;
mod providers;

fn provider(f: &Fixture) {
    f.app
        .put_provider(&ProviderInput {
            headers: None,
            body: None,
            effort_field: None,
            model_caps: None,
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
        effort: Effort::MEDIUM,
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

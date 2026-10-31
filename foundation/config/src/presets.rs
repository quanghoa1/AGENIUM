//! Known model vendors (ADR-107): the adapter and endpoint each one needs, so the user picks a name and pastes a key
//! instead of looking up base URLs. Endpoints as the vendors document them, cross-checked against the pi coding
//! agent's provider list (2026-10-26). Cloud vendors whose endpoint names the user's own resource (Azure OpenAI,
//! Amazon Bedrock, Cloudflare) carry a `<placeholder>` in the URL that the user replaces; a URL that still has one is
//! refused when saved. Google Vertex (hour-long OAuth tokens) and GitHub Copilot (its own sign-in) are not here.

use crate::ProviderKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderPreset {
    /// Suggested provider id (also the preset's name on the command line).
    pub id: &'static str,
    pub name: &'static str,
    pub kind: ProviderKind,
    /// `None` = the adapter's own default endpoint.
    pub base_url: Option<&'static str>,
    /// Environment variable that usually holds the key; empty = no key (a server on this machine).
    pub key_env: &'static str,
    pub note: &'static str,
}

const fn p(
    id: &'static str,
    name: &'static str,
    kind: ProviderKind,
    base_url: Option<&'static str>,
    key_env: &'static str,
    note: &'static str,
) -> ProviderPreset {
    ProviderPreset {
        id,
        name,
        kind,
        base_url,
        key_env,
        note,
    }
}

use ProviderKind::{Anthropic, Gemini, OpenaiCompat as Compat};

pub const PRESETS: &[ProviderPreset] = &[
    p(
        "anthropic",
        "Anthropic (Claude)",
        Anthropic,
        None,
        "ANTHROPIC_API_KEY",
        "",
    ),
    p(
        "openai",
        "OpenAI",
        Compat,
        Some("https://api.openai.com/v1"),
        "OPENAI_API_KEY",
        "",
    ),
    p(
        "google",
        "Google Gemini",
        Gemini,
        None,
        "GEMINI_API_KEY",
        "Google AI Studio key",
    ),
    p(
        "deepseek",
        "DeepSeek",
        Compat,
        Some("https://api.deepseek.com"),
        "DEEPSEEK_API_KEY",
        "",
    ),
    p(
        "openrouter",
        "OpenRouter",
        Compat,
        Some("https://openrouter.ai/api/v1"),
        "OPENROUTER_API_KEY",
        "many vendors behind one key",
    ),
    p(
        "opencode-zen",
        "OpenCode Zen",
        Compat,
        Some("https://opencode.ai/zen/v1"),
        "OPENCODE_API_KEY",
        "pay as you go: each request is paid from the account's credit (a Go subscription is not used here); its          free models answer only inside OpenCode",
    ),
    p(
        "opencode-go",
        "OpenCode Go",
        Compat,
        Some("https://opencode.ai/zen/go/v1"),
        "OPENCODE_API_KEY",
        "the OpenCode Go subscription: its models and its free ones (beam-free), from the same account key as Zen",
    ),
    p(
        "xai",
        "xAI (Grok)",
        Compat,
        Some("https://api.x.ai/v1"),
        "XAI_API_KEY",
        "",
    ),
    p(
        "mistral",
        "Mistral",
        Compat,
        Some("https://api.mistral.ai/v1"),
        "MISTRAL_API_KEY",
        "",
    ),
    p(
        "groq",
        "Groq",
        Compat,
        Some("https://api.groq.com/openai/v1"),
        "GROQ_API_KEY",
        "",
    ),
    p(
        "cerebras",
        "Cerebras",
        Compat,
        Some("https://api.cerebras.ai/v1"),
        "CEREBRAS_API_KEY",
        "",
    ),
    p(
        "together",
        "Together AI",
        Compat,
        Some("https://api.together.ai/v1"),
        "TOGETHER_API_KEY",
        "",
    ),
    p(
        "fireworks",
        "Fireworks AI",
        Compat,
        Some("https://api.fireworks.ai/inference/v1"),
        "FIREWORKS_API_KEY",
        "",
    ),
    p(
        "huggingface",
        "Hugging Face (router)",
        Compat,
        Some("https://router.huggingface.co/v1"),
        "HF_TOKEN",
        "",
    ),
    p(
        "nvidia",
        "NVIDIA NIM",
        Compat,
        Some("https://integrate.api.nvidia.com/v1"),
        "NVIDIA_API_KEY",
        "",
    ),
    p(
        "baseten",
        "Baseten",
        Compat,
        Some("https://inference.baseten.co/v1"),
        "BASETEN_API_KEY",
        "",
    ),
    p(
        "moonshotai",
        "Moonshot AI (Kimi)",
        Compat,
        Some("https://api.moonshot.ai/v1"),
        "MOONSHOT_API_KEY",
        "",
    ),
    p(
        "moonshotai-cn",
        "Moonshot AI (Kimi), China",
        Compat,
        Some("https://api.moonshot.cn/v1"),
        "MOONSHOT_API_KEY",
        "",
    ),
    p(
        "kimi-coding",
        "Kimi Coding plan",
        Compat,
        Some("https://api.kimi.com/coding/v1"),
        "KIMI_API_KEY",
        "",
    ),
    p(
        "zai",
        "Z.ai (GLM)",
        Compat,
        Some("https://api.z.ai/api/paas/v4"),
        "ZAI_API_KEY",
        "",
    ),
    p(
        "zai-coding",
        "Z.ai GLM Coding plan",
        Compat,
        Some("https://api.z.ai/api/coding/paas/v4"),
        "ZAI_API_KEY",
        "",
    ),
    p(
        "zai-coding-cn",
        "Zhipu GLM Coding plan, China",
        Compat,
        Some("https://open.bigmodel.cn/api/coding/paas/v4"),
        "ZAI_CODING_CN_API_KEY",
        "",
    ),
    p(
        "qwen",
        "Qwen (Alibaba Model Studio)",
        Compat,
        Some("https://dashscope-intl.aliyuncs.com/compatible-mode/v1"),
        "DASHSCOPE_API_KEY",
        "",
    ),
    p(
        "minimax",
        "MiniMax",
        Anthropic,
        Some("https://api.minimax.io/anthropic"),
        "MINIMAX_API_KEY",
        "Anthropic-compatible endpoint",
    ),
    p(
        "minimax-cn",
        "MiniMax, China",
        Anthropic,
        Some("https://api.minimaxi.com/anthropic"),
        "MINIMAX_CN_API_KEY",
        "Anthropic-compatible endpoint",
    ),
    p(
        "xiaomi",
        "Xiaomi MiMo",
        Compat,
        Some("https://api.xiaomimimo.com/v1"),
        "XIAOMI_API_KEY",
        "",
    ),
    p(
        "vercel-ai-gateway",
        "Vercel AI Gateway",
        Compat,
        Some("https://ai-gateway.vercel.sh/v1"),
        "AI_GATEWAY_API_KEY",
        "many vendors behind one key",
    ),
    p(
        "azure-openai",
        "Azure OpenAI",
        Compat,
        Some("https://<resource>.openai.azure.com/openai/v1"),
        "AZURE_OPENAI_API_KEY",
        "replace <resource> with your resource name; models are your deployment names",
    ),
    p(
        "bedrock",
        "Amazon Bedrock",
        Compat,
        Some("https://bedrock-mantle.<region>.api.aws/v1"),
        "AWS_BEARER_TOKEN_BEDROCK",
        "a Bedrock API key; replace <region> (us-east-1, ...)",
    ),
    p(
        "cloudflare",
        "Cloudflare Workers AI",
        Compat,
        Some("https://api.cloudflare.com/client/v4/accounts/<account-id>/ai/v1"),
        "CLOUDFLARE_API_TOKEN",
        "replace <account-id> with your Cloudflare account id",
    ),
    p(
        "ollama",
        "Ollama (this machine)",
        Compat,
        Some("http://localhost:11434/v1"),
        "",
        "start the server first",
    ),
    p(
        "lmstudio",
        "LM Studio (this machine)",
        Compat,
        Some("http://localhost:1234/v1"),
        "",
        "start the server first",
    ),
];

/// The preset named `id`.
pub fn preset(id: &str) -> Option<&'static ProviderPreset> {
    PRESETS.iter().find(|p| p.id == id)
}

/// The first `<placeholder>` left in a base URL (`<resource>`), which the user must replace before saving.
pub fn placeholder(url: &str) -> Option<&str> {
    let start = url.find('<')?;
    let end = url[start..].find('>')? + start;
    Some(&url[start..=end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_are_well_formed() {
        let mut ids = std::collections::HashSet::new();
        for p in PRESETS {
            assert!(ids.insert(p.id), "duplicate {}", p.id);
            assert!(crate::valid_mcp_id(p.id), "id {} must be a slug", p.id);
            if let Some(u) = p.base_url {
                let local = u.starts_with("http://localhost:");
                assert!(u.starts_with("https://") || local, "{u}");
                assert!(!u.ends_with('/'), "{u}");
                assert_eq!(
                    p.key_env.is_empty(),
                    local,
                    "{}: only local servers need no key",
                    p.id
                );
            }
            assert!(p.kind != ProviderKind::Mock);
        }
        assert_eq!(
            preset("groq").unwrap().base_url,
            Some("https://api.groq.com/openai/v1")
        );
        assert!(preset("nope").is_none());
        assert_eq!(
            placeholder(preset("azure-openai").unwrap().base_url.unwrap()),
            Some("<resource>")
        );
        assert_eq!(placeholder("https://api.openai.com/v1"), None);
    }
}

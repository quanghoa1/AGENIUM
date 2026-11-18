//! One tool call of a step: a protocol call, a work tool, or why it was not run.

use super::*;

impl Run<'_> {
    /// A work tool without side effects (never a protocol tool).
    pub(crate) fn read_only(&self, name: &str) -> bool {
        !self.protocol_names.iter().any(|n| n == name)
            && self.tools.iter().any(|t| t.name() == name && t.read_only())
    }

    pub(crate) async fn exec(
        &self,
        name: &str,
        args: serde_json::Value,
        call_id: &str,
    ) -> Executed {
        if let Some(raw) = args.get(agenium_llm::INVALID_JSON_KEY) {
            let preview: String = raw.as_str().unwrap_or_default().chars().take(200).collect();
            return (
                Err(ToolError::new(
                    "invalid_json",
                    format!("arguments are not valid JSON (got {preview:?}); send the call again"),
                )),
                Effect::Continue,
                false,
                vec![],
            );
        }
        // Harmless shape mistakes are repaired against the tool's schema (ADR-105).
        let args = match self.specs.iter().find(|s| s.name == name) {
            Some(spec) => agenium_tools::coerce::coerce(&spec.parameters, args),
            None => args,
        };
        if self.protocol_names.iter().any(|n| n == name) {
            // ADR-133, ADR-164: no report while a command still goes on or the agent's checklist is unfinished.
            if name == "submit_report" {
                if let Some(e) = agenium_tools::submit_refusal(self.ctx) {
                    return (Err(e), Effect::Continue, false, vec![]);
                }
            }
            return match self.protocol.call(name, args, call_id).await {
                Ok((serde_json::Value::String(text), eff)) => (Ok(text), eff, false, vec![]),
                Ok((v, eff)) => (Ok(v.to_string()), eff, false, vec![]),
                Err(e) => (Err(e), Effect::Continue, false, vec![]),
            };
        }
        match self.tools.iter().find(|t| t.name() == name) {
            Some(t) => match t.call(self.ctx, args).await {
                Ok(o) => (Ok(o.text), Effect::Continue, o.untrusted, o.images),
                Err(e) => (Err(e), Effect::Continue, false, vec![]),
            },
            None => (
                Err(ToolError::new(
                    "unknown_tool",
                    format!("there is no tool named {name}"),
                )),
                Effect::Continue,
                false,
                vec![],
            ),
        }
    }
}

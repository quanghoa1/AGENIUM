//! Built-in report templates (decision 3e: WorkReport = common core + a versioned, template-specific
//! extension validated by the orchestrator). Unknown templates accept any object until custom templates with
//! their own JSON Schema arrive (M4).

use serde_json::Value;

use crate::{OrchError, Result};

fn invalid(t: &str, msg: impl std::fmt::Display) -> OrchError {
    OrchError::Invalid(format!(
        "report extension does not match template {t}: {msg}"
    ))
}

fn uint(v: &Value, t: &str, field: &str) -> Result<u64> {
    v.as_u64()
        .ok_or_else(|| invalid(t, format!("{field} must be a non-negative integer")))
}

/// Model-facing description of a template's `extension` (added to the work order so reports validate first time).
pub fn describe(template: &str) -> &'static str {
    match template.split('@').next().unwrap_or_default() {
        "coding" => "submit_report.extension is optional. If you ran tests, use {\"tests\": {\"ran\": <integer>, \"passed\": <integer>}} (passed <= ran); otherwise send {}.",
        "pentest" => "submit_report.extension must be {\"findings\": [{\"title\": str, \"asset\": str, \"severity\": \"info|low|medium|high|critical\", ...}]} (an empty list is allowed).",
        "research" => "submit_report.extension must be {\"sources\": [ ... at least one source ... ]}.",
        _ => "submit_report.extension: any JSON object (may be {}).",
    }
}

/// Validates the `extension` of a WorkReport against the ticket's report template.
pub fn validate_extension(template: &str, ext: &Value) -> Result<()> {
    let obj = ext
        .as_object()
        .ok_or_else(|| invalid(template, "extension must be an object"))?;
    let name = template.split('@').next().unwrap_or_default();
    match name {
        // Coding: tests are optional (a template may require them later), but when present they must be sane.
        "coding" => {
            if let Some(tests) = obj.get("tests") {
                let ran = uint(&tests["ran"], template, "tests.ran")?;
                let passed = uint(&tests["passed"], template, "tests.passed")?;
                if passed > ran {
                    return Err(invalid(template, "tests.passed cannot exceed tests.ran"));
                }
            }
            Ok(())
        }
        // Pentest: findings registry (may be empty: "nothing found" is a result).
        "pentest" => {
            let findings = obj
                .get("findings")
                .and_then(Value::as_array)
                .ok_or_else(|| invalid(template, "findings (array) is required"))?;
            for (i, f) in findings.iter().enumerate() {
                for k in ["title", "asset"] {
                    if f.get(k)
                        .and_then(Value::as_str)
                        .is_none_or(|s| s.trim().is_empty())
                    {
                        return Err(invalid(template, format!("findings[{i}].{k} is required")));
                    }
                }
                let sev = f
                    .get("severity")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if !["info", "low", "medium", "high", "critical"].contains(&sev) {
                    return Err(invalid(
                        template,
                        format!("findings[{i}].severity must be info|low|medium|high|critical"),
                    ));
                }
            }
            Ok(())
        }
        // Research: every claim needs sources.
        "research" => {
            let sources = obj
                .get("sources")
                .and_then(Value::as_array)
                .ok_or_else(|| invalid(template, "sources (array) is required"))?;
            if sources.is_empty() {
                return Err(invalid(template, "at least one source is required"));
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn templates() {
        validate_extension("coding@1", &json!({})).unwrap();
        validate_extension("coding@1", &json!({"tests": {"ran": 12, "passed": 12}})).unwrap();
        assert!(
            validate_extension("coding@1", &json!({"tests": {"ran": 1, "passed": 2}})).is_err()
        );
        assert!(validate_extension("pentest@1", &json!({})).is_err());
        validate_extension("pentest@1", &json!({"findings": []})).unwrap();
        assert!(validate_extension(
            "pentest@1",
            &json!({"findings": [{"title": "SSRF", "asset": "api", "severity": "urgent"}]})
        )
        .is_err());
        assert!(validate_extension("research@1", &json!({"sources": []})).is_err());
        validate_extension("content@1", &json!({"anything": 1})).unwrap();
        assert!(validate_extension("coding@1", &json!([])).is_err());
    }
}

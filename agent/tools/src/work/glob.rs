//! `glob`: files of the worktree by pattern.

use super::*;

/// Find files by glob pattern, newest first.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GlobArgs {
    /// Glob such as `src/**/*.rs`.
    pub pattern: String,
    /// Directory to search from (default: the root).
    #[serde(default)]
    pub path: Option<String>,
}

pub struct GlobTool;

#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &'static str {
        "glob"
    }
    fn description(&self) -> &'static str {
        "Find files whose path matches a glob pattern (e.g. src/**/*.rs, **/test_*.py), newest first. Respects \
         .gitignore. Use it to find files by name; grep searches what they contain."
    }
    fn parameters(&self) -> serde_json::Value {
        args_schema::<GlobArgs>()
    }
    fn read_only(&self) -> bool {
        true
    }
    async fn call(&self, ctx: &ToolCtx, a: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let a: GlobArgs = args(a)?;
        let base = ctx.resolve(a.path.as_deref().unwrap_or("."), false)?;
        let g = globset::GlobBuilder::new(&a.pattern)
            .literal_separator(true)
            .build()
            .map_err(|e| ToolError::new("invalid_pattern", e.to_string()))?
            .compile_matcher();
        let mut hits: Vec<(SystemTime, String)> = Vec::new();
        for e in walker(&base, None).flatten() {
            if !e.file_type().is_some_and(|t| t.is_file()) {
                continue;
            }
            let rel = display(&base, e.path());
            if g.is_match(&rel) {
                let t = e
                    .metadata()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .unwrap_or(SystemTime::UNIX_EPOCH);
                hits.push((t, display(&ctx.root, e.path())));
            }
        }
        hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        let n = hits.len();
        let mut lines: Vec<String> = hits.into_iter().take(500).map(|(_, p)| p).collect();
        if n > 500 {
            lines.push(format!("[{} more files omitted]", n - 500));
        }
        Ok(ctx.finish_output(
            if lines.is_empty() {
                "no files matched".into()
            } else {
                lines.join("\n")
            },
            false,
        ))
    }
}

//! `read_file`: a text file of the worktree, lines numbered.

use super::*;

/// Read a text file of your worktree. Lines are numbered.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadArgs {
    /// Path relative to the worktree root.
    pub path: String,
    /// First line to read (1-based).
    #[serde(default)]
    pub offset: Option<usize>,
    /// Number of lines to read (default 2000).
    #[serde(default)]
    pub limit: Option<usize>,
}

pub struct ReadFile;

#[async_trait]
impl Tool for ReadFile {
    fn name(&self) -> &'static str {
        "read_file"
    }
    fn description(&self) -> &'static str {
        "Read a text file in your worktree (or a skill folder). Output lines are numbered. Read a file before editing it."
    }
    fn parameters(&self) -> serde_json::Value {
        args_schema::<ReadArgs>()
    }
    fn read_only(&self) -> bool {
        true
    }
    async fn call(&self, ctx: &ToolCtx, a: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let a: ReadArgs = args(a)?;
        let p = ctx.resolve(&a.path, false)?;
        let rel = display(&ctx.root, &p);
        let bytes = read_text(ctx, &p, &rel)?;
        let s = utf8(&bytes, &rel)?;
        ctx.record_read(&p, &bytes);
        let start = a.offset.unwrap_or(1).max(1);
        let limit = a.limit.unwrap_or(2000).max(1);
        let total = s.lines().count();
        let mut out = String::new();
        for (i, line) in s.lines().enumerate().skip(start - 1).take(limit) {
            out.push_str(&format!("{:>6}\t{}\n", i + 1, line));
        }
        if total == 0 {
            out.push_str("(empty file)\n");
        } else if start - 1 + limit < total {
            out.push_str(&format!(
                "[showing lines {start}-{} of {total}; use offset to read more]\n",
                start - 1 + limit
            ));
        }
        Ok(ctx.finish_output(out, false))
    }
}

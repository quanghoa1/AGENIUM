//! `list_dir`: a folder of the worktree, ignore rules applied.

use super::*;

/// List a directory (respects .gitignore).
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListArgs {
    /// Directory relative to the worktree root (default: the root).
    #[serde(default)]
    pub path: Option<String>,
    /// Depth 1-3 (default 1).
    #[serde(default)]
    pub depth: Option<usize>,
}

pub struct ListDir;

pub(super) fn walker(root: &Path, depth: Option<usize>) -> ignore::Walk {
    ignore::WalkBuilder::new(root)
        .max_depth(depth)
        .hidden(false)
        .git_ignore(true)
        .git_exclude(true)
        .parents(true)
        .require_git(false)
        .filter_entry(|e| e.file_name() != ".git" && e.file_name() != ".agenium")
        .build()
}

#[async_trait]
impl Tool for ListDir {
    fn name(&self) -> &'static str {
        "list_dir"
    }
    fn description(&self) -> &'static str {
        "List files and directories (directories end with '/'). Respects .gitignore."
    }
    fn parameters(&self) -> serde_json::Value {
        args_schema::<ListArgs>()
    }
    fn read_only(&self) -> bool {
        true
    }
    async fn call(&self, ctx: &ToolCtx, a: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let a: ListArgs = args(a)?;
        let p = ctx.resolve(a.path.as_deref().unwrap_or("."), false)?;
        if !p.is_dir() {
            return Err(ToolError::new(
                "not_a_directory",
                format!("{} is not a directory", display(&ctx.root, &p)),
            ));
        }
        let depth = a.depth.unwrap_or(1).clamp(1, 3);
        let mut entries: Vec<String> = Vec::new();
        for e in walker(&p, Some(depth)).flatten() {
            if e.path() == p {
                continue;
            }
            let mut rel = display(&p, e.path());
            if e.file_type().is_some_and(|t| t.is_dir()) {
                rel.push('/');
            }
            entries.push(rel);
            if entries.len() >= 1000 {
                entries.push("[... more entries omitted ...]".into());
                break;
            }
        }
        entries.sort();
        Ok(ctx.finish_output(
            if entries.is_empty() {
                "(empty)".into()
            } else {
                entries.join("\n")
            },
            false,
        ))
    }
}

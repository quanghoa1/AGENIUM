//! `grep`: lines of the worktree's files by regular expression.

use super::*;

/// Search file contents with a regular expression.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GrepArgs {
    /// Regular expression (Rust regex syntax).
    pub pattern: String,
    /// File or directory to search (default: the root).
    #[serde(default)]
    pub path: Option<String>,
    /// Only files whose path matches this glob, e.g. `*.rs`.
    #[serde(default)]
    pub glob: Option<String>,
    #[serde(default)]
    pub case_insensitive: bool,
    /// `content` (matching lines, default) or `files` (paths only).
    #[serde(default)]
    pub output: Option<String>,
}

pub struct GrepTool;

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &'static str {
        "grep"
    }
    fn description(&self) -> &'static str {
        "Search file contents with a regular expression (an exact name, a string, a pattern); returns \
         path:line:text, or only the paths with output = files. Respects .gitignore; narrow it with path or glob. \
         For where a behaviour lives when you do not know its names, find_code (when offered) answers better."
    }
    fn parameters(&self) -> serde_json::Value {
        args_schema::<GrepArgs>()
    }
    fn read_only(&self) -> bool {
        true
    }
    async fn call(&self, ctx: &ToolCtx, a: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let a: GrepArgs = args(a)?;
        let re = regex::RegexBuilder::new(&a.pattern)
            .case_insensitive(a.case_insensitive)
            .size_limit(1 << 22)
            .build()
            .map_err(|e| {
                // Byte patterns (ripgrep's `(?-u)\xEF\xBB\xBF`) do not apply: files are searched as UTF-8 text.
                let hint = if e.to_string().contains("invalid UTF-8") {
                    " (patterns match UTF-8 text: write a character, e.g. a byte-order mark as \\x{FEFF}, not its bytes)"
                } else {
                    ""
                };
                ToolError::new("invalid_pattern", format!("{e}{hint}"))
            })?;
        let filter = match &a.glob {
            Some(g) => Some(
                globset::Glob::new(g)
                    .map_err(|e| ToolError::new("invalid_pattern", e.to_string()))?
                    .compile_matcher(),
            ),
            None => None,
        };
        let base = ctx.resolve(a.path.as_deref().unwrap_or("."), false)?;
        let files_only = a.output.as_deref() == Some("files");
        let mut out: Vec<String> = Vec::new();
        let (mut matches, mut unread) = (0usize, 0usize);
        let files: Vec<PathBuf> = if base.is_file() {
            vec![base.clone()]
        } else {
            walker(&base, None)
                .flatten()
                .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
                .map(|e| e.into_path())
                .collect()
        };
        for f in files {
            let rel = display(&ctx.root, &f);
            if let Some(g) = &filter {
                let name = f
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if !g.is_match(&rel) && !g.is_match(&name) {
                    continue;
                }
            }
            // Too large to search, out of the roots or with other names (`real_read.rs`): skipped, and said.
            let bytes = match read_checked(ctx, &f, &rel, MAX_READ_BYTES) {
                Ok(b) => b,
                Err(e) if e.code == "out_of_scope" => {
                    unread += 1;
                    continue;
                }
                Err(_) => continue,
            };
            if bytes.iter().take(8192).any(|b| *b == 0) {
                continue;
            }
            let s = String::from_utf8_lossy(&bytes);
            let mut hit = false;
            for (i, line) in s.lines().enumerate() {
                if re.is_match(line) {
                    matches += 1;
                    hit = true;
                    if files_only {
                        break;
                    }
                    out.push(format!("{rel}:{}:{}", i + 1, line.trim_end()));
                }
            }
            if hit && files_only {
                out.push(rel);
            }
        }
        // Files grep may not read (a hard link, as a package manager's store makes): a "no matches" would mislead.
        let unread_note = (unread > 0).then(|| {
            format!(
                "[{unread} file(s) with other names (hard links) or leading outside the worktree were not searched: \
                 search them with shell if they matter]"
            )
        });
        if out.is_empty() {
            let mut text = "no matches".to_owned();
            if let Some(n) = unread_note {
                text = format!("{text}\n{n}");
            }
            return Ok(ToolOutput::text(text));
        }
        out.push(if files_only {
            format!("[{matches} matching files]")
        } else {
            format!("[{matches} matching lines]")
        });
        out.extend(unread_note);
        Ok(ctx.finish_output(out.join("\n"), false))
    }
}

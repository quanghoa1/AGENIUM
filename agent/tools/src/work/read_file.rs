//! `read_file`: a text file of the worktree, lines numbered. A document (PDF, Word, PowerPoint, Excel, EPUB, an old
//! Office file or an Outlook message) is read as Markdown (ADR-152): markitdown when installed and a sandbox works,
//! Agenium's own converters otherwise; it is read-only (the edit tools do not apply to it).

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
        "Read a text file in your worktree (or a skill folder): its lines, numbered, up to 2000 at a time (offset and \
         limit read further in a long file). A document (PDF, DOCX, PPTX, XLSX, EPUB) is read as Markdown. Read a file before editing or overwriting it; the edit tools refuse a \
         file not read in this run. To find which file holds something, search first (grep, glob) instead of reading \
         files one by one."
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
        let ext = p
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        let (s, header) = if DOCUMENTS.contains(&ext.as_str()) {
            let bytes = read_checked(ctx, &p, &rel, agenium_documents::MAX_BYTES as u64)?;
            let unsandboxed = matches!(ctx.policy.mode, agenium_sandbox::SandboxMode::FullAccess);
            let (text, by) = document_text(&rel, &bytes, unsandboxed).await?;
            (
                text,
                Some(format!(
                    "[{rel} read as Markdown by {by}; a converted copy, read-only]\n"
                )),
            )
        } else {
            let bytes = read_text(ctx, &p, &rel)?;
            let s = utf8(&bytes, &rel)?;
            ctx.record_read(&p, &bytes);
            (s, None)
        };
        let start = a.offset.unwrap_or(1).max(1);
        let limit = a.limit.unwrap_or(2000).max(1);
        let total = s.lines().count();
        let mut out = header.unwrap_or_default();
        for (i, line) in s.lines().enumerate().skip(start - 1).take(limit) {
            out.push_str(&format!("{:>6}\t{}\n", i + 1, line));
        }
        if total == 0 {
            out.push_str("(empty file)\n");
        // Saturating: a model's huge offset or limit overflowed (audit L4, 2026-10-31).
        } else if (start - 1).saturating_add(limit) < total {
            out.push_str(&format!(
                "[showing lines {start}-{} of {total}; use offset to read more]\n",
                (start - 1).saturating_add(limit)
            ));
        }
        Ok(ctx.finish_output(out, false))
    }
}

/// Files read as documents (by extension): binary formats a model cannot read as they are.
const DOCUMENTS: &[&str] = &[
    "pdf", "docx", "pptx", "xlsx", "epub", "doc", "xls", "ppt", "msg", "odt", "ods", "odp",
];

/// A document's Markdown and what read it: markitdown where it reads the format better (only inside a working sandbox),
/// Agenium's converters otherwise. `unsandboxed`: the run itself goes without a sandbox (the user's choice, ADR-117).
async fn document_text(
    name: &str,
    bytes: &[u8],
    unsandboxed: bool,
) -> Result<(String, &'static str), ToolError> {
    let own = agenium_documents::convert(name, bytes);
    if agenium_documents::better_with_markitdown(name) || own.is_err() {
        if let Some(m) = agenium_documents::markitdown::find() {
            if let Ok(c) = m.convert(name, bytes, unsandboxed).await {
                return Ok((c.text, "markitdown"));
            }
        }
    }
    own.map(|c| (c.text, "Agenium"))
        .map_err(|e| ToolError::new("unreadable_document", format!("{name}: {e}")))
}

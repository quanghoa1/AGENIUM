//! `edit_file` and `multi_edit`: exact replacements, checked against what was read.

use super::*;

#[derive(Deserialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct EditSpec {
    /// Exact text to replace; must occur exactly once unless replace_all is true.
    pub old_string: String,
    pub new_string: String,
    #[serde(default)]
    pub replace_all: bool,
}

/// Replace a unique piece of text in a file you have read.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditArgs {
    pub path: String,
    /// Exact text to replace; must occur exactly once unless replace_all is true.
    pub old_string: String,
    pub new_string: String,
    #[serde(default)]
    pub replace_all: bool,
}

/// Applies one edit to `doc`; `true` when the text was found only in the loose form (ADR-102).
pub(super) fn apply_edit(
    doc: &mut crate::edit_match::Doc,
    e: &EditSpec,
    rel: &str,
    idx: Option<usize>,
) -> Result<bool, ToolError> {
    let at = idx.map(|i| format!("edit {} ", i + 1)).unwrap_or_default();
    if e.old_string.is_empty() {
        return Err(ToolError::new(
            "invalid_arguments",
            format!("{at}old_string is empty; use write_file to create files"),
        ));
    }
    if e.old_string == e.new_string {
        return Err(ToolError::new(
            "invalid_arguments",
            format!("{at}old_string and new_string are identical"),
        ));
    }
    let (old, new) = (doc.input(&e.old_string), doc.input(&e.new_string));
    let (ranges, loose) = crate::edit_match::find(&doc.text, &old);
    match (ranges.len(), e.replace_all) {
        (0, _) => Err(ToolError::new("not_found", format!("{at}old_string not found in {rel}; read the file and copy the exact text"))),
        (1, _) | (_, true) => {
            doc.text = crate::edit_match::splice(&doc.text, &ranges, &new);
            Ok(loose)
        }
        (n, false) => Err(ToolError::new("not_unique", format!("{at}old_string occurs {n} times in {rel}; add surrounding context or set replace_all"))),
    }
}

pub(super) fn edit_files(
    ctx: &ToolCtx,
    path: &str,
    edits: &[EditSpec],
    multi: bool,
) -> Result<ToolOutput, ToolError> {
    let p = ctx.resolve(path, true)?;
    let rel = display(&ctx.root, &p);
    let bytes = read_text(&p, &rel)?;
    ctx.check_fresh(&p, &bytes, &rel)?;
    let mut doc = crate::edit_match::Doc::open(&utf8(&bytes, &rel)?);
    let mut loose = false;
    for (i, e) in edits.iter().enumerate() {
        loose |= apply_edit(&mut doc, e, &rel, multi.then_some(i))?;
    }
    let s = doc.save();
    write_file(&p, &rel, s.as_bytes())?;
    ctx.record_read(&p, s.as_bytes());
    Ok(ToolOutput::text(format!(
        "edited {rel} ({} change{}{})",
        edits.len(),
        if edits.len() == 1 { "" } else { "s" },
        if loose {
            "; matched after ignoring quote and dash styles and trailing spaces"
        } else {
            ""
        }
    )))
}

pub struct EditFile;

#[async_trait]
impl Tool for EditFile {
    fn name(&self) -> &'static str {
        "edit_file"
    }
    fn description(&self) -> &'static str {
        "Replace text in a file you have read: old_string must match exactly once (or set replace_all)."
    }
    fn parameters(&self) -> serde_json::Value {
        args_schema::<EditArgs>()
    }
    async fn call(&self, ctx: &ToolCtx, a: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let a: EditArgs = args(a)?;
        edit_files(
            ctx,
            &a.path,
            &[EditSpec {
                old_string: a.old_string,
                new_string: a.new_string,
                replace_all: a.replace_all,
            }],
            false,
        )
    }
}

/// Several replacements in one file, applied in order and atomically.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MultiEditArgs {
    pub path: String,
    #[schemars(length(min = 1))]
    pub edits: Vec<EditSpec>,
}

pub struct MultiEdit;

#[async_trait]
impl Tool for MultiEdit {
    fn name(&self) -> &'static str {
        "multi_edit"
    }
    fn description(&self) -> &'static str {
        "Apply several replacements to one file you have read, in order; nothing is written if any edit fails."
    }
    fn parameters(&self) -> serde_json::Value {
        args_schema::<MultiEditArgs>()
    }
    async fn call(&self, ctx: &ToolCtx, a: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let a: MultiEditArgs = args(a)?;
        if a.edits.is_empty() {
            return Err(ToolError::new("invalid_arguments", "edits is empty"));
        }
        edit_files(ctx, &a.path, &a.edits, true)
    }
}

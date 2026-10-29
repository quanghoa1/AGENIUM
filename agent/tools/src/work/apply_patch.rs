//! `apply_patch`: a patch of added, changed and deleted files.

use super::*;

/// Multi-file patch (add, update, move, delete).
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PatchArgs {
    /// Patch text from `*** Begin Patch` to `*** End Patch`.
    pub patch: String,
}

pub struct ApplyPatch;

#[async_trait]
impl Tool for ApplyPatch {
    fn name(&self) -> &'static str {
        "apply_patch"
    }
    fn description(&self) -> &'static str {
        "Edit several files atomically. Format: '*** Begin Patch', then per file '*** Add File: p' (lines '+...'), \
'*** Delete File: p', or '*** Update File: p' (optional '*** Move to: q') with hunks starting '@@' whose lines \
begin with ' ' (context), '-' (remove) or '+' (add); end with '*** End Patch'."
    }
    fn parameters(&self) -> serde_json::Value {
        args_schema::<PatchArgs>()
    }
    async fn call(&self, ctx: &ToolCtx, a: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let a: PatchArgs = args(a)?;
        let ops = parse(&a.patch).map_err(|e| ToolError::new("invalid_patch", e))?;
        // Compute every resulting file first (atomic).
        let mut writes: Vec<(PathBuf, String, Option<String>)> = Vec::new(); // (path, rel, Some(content) | None=delete)
        let mut summary = Vec::new();
        for op in &ops {
            match op {
                FileOp::Add { path, content } => {
                    let p = ctx.resolve(path, true)?;
                    let rel = display(&ctx.root, &p);
                    if p.exists() {
                        return Err(ToolError::new(
                            "exists",
                            format!("{rel} already exists; use Update File"),
                        ));
                    }
                    summary.push(format!("A {rel}"));
                    writes.push((p, rel, Some(content.clone())));
                }
                FileOp::Delete { path } => {
                    let p = ctx.resolve(path, true)?;
                    let rel = display(&ctx.root, &p);
                    if !p.is_file() {
                        return Err(ToolError::new("not_found", format!("{rel} does not exist")));
                    }
                    summary.push(format!("D {rel}"));
                    writes.push((p, rel, None));
                }
                FileOp::Update {
                    path,
                    move_to,
                    hunks,
                } => {
                    let p = ctx.resolve(path, true)?;
                    let rel = display(&ctx.root, &p);
                    let cur = utf8(&read_text(ctx, &p, &rel)?, &rel)?;
                    let new = if hunks.is_empty() {
                        cur
                    } else {
                        apply_hunks(&cur, hunks, &rel)
                            .map_err(|e| ToolError::new("hunk_failed", e))?
                    };
                    match move_to {
                        Some(m) => {
                            let q = ctx.resolve(m, true)?;
                            let qrel = display(&ctx.root, &q);
                            summary.push(format!("R {rel} -> {qrel}"));
                            writes.push((p, rel, None));
                            writes.push((q, qrel, Some(new)));
                        }
                        None => {
                            summary.push(format!("M {rel}"));
                            writes.push((p, rel, Some(new)));
                        }
                    }
                }
            }
        }
        // Write, restoring the originals if anything fails midway.
        let originals: Vec<(PathBuf, Option<Vec<u8>>)> = writes
            .iter()
            .map(|(p, _, _)| (p.clone(), std::fs::read(p).ok()))
            .collect();
        for (p, rel, content) in &writes {
            let r = match content {
                Some(c) => write_file(&ctx.root, p, rel, c.as_bytes()),
                None => super::real_remove::remove_file(&ctx.root, p, rel),
            };
            if let Err(e) = r {
                for (op, orig) in &originals {
                    match orig {
                        Some(b) => {
                            let _ = write_file(&ctx.root, op, "", b);
                        }
                        None => {
                            let _ = super::real_remove::remove_file(&ctx.root, op, "");
                        }
                    }
                }
                return Err(e);
            }
            if let Some(c) = content {
                ctx.record_read(p, c.as_bytes());
            }
        }
        Ok(ToolOutput::text(format!(
            "applied patch:\n{}",
            summary.join("\n")
        )))
    }
}

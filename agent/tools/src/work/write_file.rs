//! `write_file`: a whole file written.

use super::*;

/// Create or overwrite a file (overwriting requires reading it first).
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WriteArgs {
    pub path: String,
    pub content: String,
}

pub struct WriteFile;

#[async_trait]
impl Tool for WriteFile {
    fn name(&self) -> &'static str {
        "write_file"
    }
    fn description(&self) -> &'static str {
        "Create a file with the given content, or overwrite one you have read in this run (parent directories are \
         created). Use it for new files and full rewrites; to change part of an existing file use edit or \
         apply_patch, which keep the rest as it is."
    }
    fn parameters(&self) -> serde_json::Value {
        args_schema::<WriteArgs>()
    }
    async fn call(&self, ctx: &ToolCtx, a: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let a: WriteArgs = args(a)?;
        let p = ctx.resolve(&a.path, true)?;
        let rel = display(&ctx.root, &p);
        if p.is_dir() {
            return Err(ToolError::new(
                "is_directory",
                format!("{rel} is a directory"),
            ));
        }
        if crate::text::numbered_copy(&a.content) {
            return Err(crate::text::numbered_refusal(&rel));
        }
        let existed = p.exists();
        if existed {
            let cur = super::handle::read_regular(&p).map_err(|e| io_err(&rel, e))?;
            ctx.check_fresh(&p, &cur, &rel)?;
        }
        write_file(&ctx.root, &p, &rel, a.content.as_bytes())?;
        ctx.record_read(&p, a.content.as_bytes());
        Ok(ToolOutput::text(format!(
            "{} {rel} ({} bytes)",
            if existed { "overwrote" } else { "created" },
            a.content.len()
        )))
    }
}

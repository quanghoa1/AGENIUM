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
        "Create a file, or overwrite one you have read in this run. Parent directories are created."
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
        let existed = p.exists();
        if existed {
            let cur = std::fs::read(&p).map_err(|e| io_err(&rel, e))?;
            ctx.check_fresh(&p, &cur, &rel)?;
        }
        write_file(&p, &rel, a.content.as_bytes())?;
        ctx.record_read(&p, a.content.as_bytes());
        Ok(ToolOutput::text(format!(
            "{} {rel} ({} bytes)",
            if existed { "overwrote" } else { "created" },
            a.content.len()
        )))
    }
}

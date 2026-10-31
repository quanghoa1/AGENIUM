//! Tool implementations.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use agenium_core::protocol::args_schema;
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::patch::{apply_hunks, parse, FileOp};
use crate::paths::display;
use crate::{args, GateDecision, TodoItem, TodoStatus, Tool, ToolCtx, ToolError, ToolOutput};
use real_read::read_checked;
use real_write::write_file;

mod apply_patch;
mod edit;
mod fetch;
mod glob;
mod grep;
mod handle;
mod list_dir;
mod read_file;
mod real_read;
mod real_remove;
mod real_write;
mod shell;
mod shell_awaited;
mod shell_jobs;
mod todo;
mod view_image;
mod write_file;

pub use apply_patch::*;
pub use edit::*;
pub use fetch::*;
pub use glob::*;
pub use grep::*;
pub use list_dir::*;
pub use read_file::*;
pub(crate) use real_write::write_new;
pub use shell::*;
pub use shell_awaited::ended_message;
pub use shell_jobs::*;
pub use todo::*;
pub use view_image::*;
pub use write_file::*;

const MAX_READ_BYTES: u64 = 10 * 1024 * 1024;

fn io_err(rel: &str, e: std::io::Error) -> ToolError {
    match e.kind() {
        std::io::ErrorKind::NotFound => {
            ToolError::new("not_found", format!("{rel} does not exist"))
        }
        _ => ToolError::new("io_error", format!("{rel}: {e}")),
    }
}

fn read_text(ctx: &ToolCtx, p: &Path, rel: &str) -> Result<Vec<u8>, ToolError> {
    read_checked(ctx, p, rel, MAX_READ_BYTES)
}

fn utf8(bytes: &[u8], rel: &str) -> Result<String, ToolError> {
    if bytes.iter().take(8192).any(|b| *b == 0) {
        return Err(ToolError::new("binary_file", format!("{rel} looks binary")));
    }
    String::from_utf8(bytes.to_vec())
        .map_err(|_| ToolError::new("not_utf8", format!("{rel} is not valid UTF-8")))
}

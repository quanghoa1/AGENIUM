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
use real_write::write_file;

mod apply_patch;
mod edit;
mod fetch;
mod glob;
mod grep;
mod list_dir;
mod read_file;
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

fn read_text(p: &Path, rel: &str) -> Result<Vec<u8>, ToolError> {
    let meta = std::fs::metadata(p).map_err(|e| io_err(rel, e))?;
    if meta.is_dir() {
        return Err(ToolError::new(
            "is_directory",
            format!("{rel} is a directory; use list_dir"),
        ));
    }
    if meta.len() > MAX_READ_BYTES {
        return Err(ToolError::new(
            "too_large",
            format!("{rel} is {} bytes; use grep or read a range", meta.len()),
        ));
    }
    std::fs::read(p).map_err(|e| io_err(rel, e))
}

fn utf8(bytes: &[u8], rel: &str) -> Result<String, ToolError> {
    if bytes.iter().take(8192).any(|b| *b == 0) {
        return Err(ToolError::new("binary_file", format!("{rel} looks binary")));
    }
    String::from_utf8(bytes.to_vec())
        .map_err(|_| ToolError::new("not_utf8", format!("{rel} is not valid UTF-8")))
}

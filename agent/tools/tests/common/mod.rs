//! What the tool tests share: a test worktree and a tool call on it.
#![allow(dead_code)]

use std::path::Path;
use std::sync::Arc;

use agenium_tools::*;

pub fn ctx(root: &Path) -> ToolCtx {
    ToolCtx::new(root, "run1", Arc::new(AllowAll))
}

pub fn tool(name: &str) -> Arc<dyn Tool> {
    work_tools().into_iter().find(|t| t.name() == name).unwrap()
}

pub async fn call(
    c: &ToolCtx,
    name: &str,
    args: serde_json::Value,
) -> Result<ToolOutput, ToolError> {
    tool(name).call(c, args).await
}

/// The test worktree. Its commands' TEMP lies outside it, under the system TEMP, and goes with it.
pub struct Worktree {
    pub root: std::path::PathBuf,
    _tmp: tempfile::TempDir,
}

impl Drop for Worktree {
    fn drop(&mut self) {
        agenium_sandbox::remove_run_temp_dir(&self.root);
    }
}

pub fn setup() -> (Worktree, ToolCtx) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("wt");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/lib.rs"),
        "pub fn add(a: i32, b: i32) -> i32 {\n    a - b\n}\n// add\n",
    )
    .unwrap();
    std::fs::write(root.join("README.md"), "# demo\n").unwrap();
    std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::write(root.join("target/junk.rs"), "add").unwrap();
    let c = ctx(&root);
    (Worktree { root, _tmp: tmp }, c)
}

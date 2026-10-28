//! What a sandboxed command may reach: the policy, the read roots it adds, the network, the run's temp folder.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SandboxMode {
    ReadOnly,
    /// Write inside the writable roots (ticket worktree + tmp); network off unless granted.
    WorkspaceWrite,
    FullAccess,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxPolicy {
    pub mode: SandboxMode,
    pub writable_roots: Vec<PathBuf>,
    pub network: bool,
    /// Read-only roots beyond the platform folders (toolchains, the repository's `.git`, skill folders).
    #[serde(default)]
    pub read_roots: Vec<PathBuf>,
    /// Writable temporary directory outside the worktree (TEMP/TMP of the command); created on demand.
    #[serde(default)]
    pub temp_dir: Option<PathBuf>,
}

impl SandboxPolicy {
    /// Workspace-write policy for a ticket worktree: the worktree is writable, the toolchains and the
    /// repository's git directory (a worktree's git dir lives outside it) are readable, network off.
    pub fn workspace(root: &Path) -> SandboxPolicy {
        let mut read_roots = toolchain_read_roots();
        read_roots.extend(git_read_roots(root));
        SandboxPolicy {
            mode: SandboxMode::WorkspaceWrite,
            writable_roots: vec![root.to_path_buf()],
            network: false,
            read_roots,
            temp_dir: Some(run_temp_dir(root)),
        }
    }
}

/// Toolchain folders a sandboxed command may read (Windows: see `mxc::toolchain_read_roots`).
pub fn toolchain_read_roots() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        mxc::toolchain_read_roots()
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        unix::toolchain_read_roots()
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        Vec::new()
    }
}

/// The git directories a worktree needs: `.git` is a file `gitdir: <repo>/.git/worktrees/<name>` whose
/// `commondir` is the repository's `.git`.
///
/// That file is inside the writable root: an agent can rewrite it between runs (`gitdir: C:\Users\me` made the whole
/// profile readable). So the directory it names is only believed when git itself vouches for it: it lies outside the
/// worktree, its `gitdir` file points back to this worktree's `.git`, and it is `<common>/worktrees/<its name>` of
/// the repository its `commondir` names. The agent cannot write outside the worktree and its temp folders, so it can
/// only name the directory git made for this worktree; anything else gives no git read root (git then fails inside
/// the sandbox instead of reading elsewhere).
pub fn git_read_roots(root: &Path) -> Vec<PathBuf> {
    let canon = |p: &Path| std::fs::canonicalize(p).ok();
    let (Ok(text), Some(root)) = (std::fs::read_to_string(root.join(".git")), canon(root)) else {
        return Vec::new(); // a normal repository: .git is inside the writable root
    };
    let Some(gitdir) = text
        .trim()
        .strip_prefix("gitdir:")
        .and_then(|g| canon(&root.join(g.trim())))
    else {
        return Vec::new();
    };
    // Outside the worktree: inside it (`gitdir: .`, whose parent folder is `.agenium/worktrees`) the agent writes the
    // `gitdir` and `commondir` files itself.
    if gitdir.starts_with(&root) {
        return Vec::new();
    }
    let points_back = std::fs::read_to_string(gitdir.join("gitdir"))
        .ok()
        .and_then(|b| canon(&gitdir.join(b.trim())))
        .is_some_and(|b| b == root.join(".git"));
    let common = std::fs::read_to_string(gitdir.join("commondir"))
        .ok()
        .and_then(|c| canon(&gitdir.join(c.trim())));
    let Some(common) = common.filter(|c| {
        points_back
            && gitdir.file_name().is_some_and(|name| {
                canon(&c.join("worktrees").join(name)).as_deref() == Some(gitdir.as_path())
            })
    }) else {
        return Vec::new();
    };
    vec![PathBuf::from(
        common
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .to_owned(),
    )]
}

/// Whether sandboxed commands asked to have the network get it on this machine; `Err` says why not (Linux without a
/// private network: they run without one, ADR-95).
pub fn network_available() -> Result<(), String> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        unix::private_network()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Ok(())
    }
}

/// What `[sandbox] network = true` gives sandboxed commands on this machine, in one line (`agenium doctor`).
pub fn network_report() -> String {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        match unix::private_network() {
            Err(why) => {
                format!("commands run WITHOUT network, and CLI agents cannot run sandboxed: {why}")
            }
            Ok(()) => "private network (outbound only; no inbound, no host loopback)".into(),
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        "network through the container's own policy (Windows MXC)".into()
    }
}

/// Per-worktree temporary directory outside the worktree (so auto-commit never sees it): the TEMP/TMP of the
/// worktree's commands. Keyed by the worktree path, spelled either way (separators; case on Windows).
pub fn run_temp_dir(root: &Path) -> PathBuf {
    let mut key = root.to_string_lossy().replace('\\', "/");
    key = key
        .trim_start_matches("//?/")
        .trim_end_matches('/')
        .to_owned();
    if cfg!(windows) {
        key = key.to_lowercase();
    }
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in key.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
    }
    std::env::temp_dir()
        .join("agenium")
        .join(format!("{h:016x}"))
}

/// Deletes the worktree's temporary directory when the worktree goes away. Builds and package managers fill it, and
/// it lives under the system TEMP (on Windows usually the small system drive); nothing removed it before, so every
/// ticket left one behind. Best effort: a file still open is left for the next removal.
pub fn remove_run_temp_dir(root: &Path) {
    let dir = run_temp_dir(root);
    if dir.exists() {
        let _ = std::fs::remove_dir_all(&dir);
    }
}

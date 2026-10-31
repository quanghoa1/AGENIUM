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

/// In a worktree's temporary directory: the worktree it belongs to, so [`sweep_run_temp_dirs`] can tell when it is gone.
const OWNER_FILE: &str = ".agenium-worktree";

/// Creates the policy's temporary directory and records the worktree it belongs to (the first writable root). A
/// worktree removed by Agenium takes its directory with it ([`remove_run_temp_dir`]); one that went another way (a
/// root deleted whole, a test's or a helper's scratch folder, a crash) is found by the sweep from this record.
pub fn prepare_temp_dir(policy: &SandboxPolicy) -> std::io::Result<()> {
    let Some(t) = &policy.temp_dir else {
        return Ok(());
    };
    std::fs::create_dir_all(t)?;
    let owner = t.join(OWNER_FILE);
    if let (false, Some(root)) = (owner.exists(), policy.writable_roots.first()) {
        std::fs::write(owner, root.to_string_lossy().as_bytes())?;
    }
    Ok(())
}

/// Removes the worktrees' temporary directories (under the system TEMP) whose worktree is gone, and those without a
/// record (made before records existed) untouched for `unowned_after`. Only Agenium's own directories (16 hex digits)
/// are looked at, and only they are removed: the record is never followed. Best effort; returns what it removed.
pub fn sweep_run_temp_dirs(unowned_after: std::time::Duration) -> Vec<PathBuf> {
    sweep_in(&std::env::temp_dir().join("agenium"), unowned_after)
}

fn sweep_in(base: &Path, unowned_after: std::time::Duration) -> Vec<PathBuf> {
    let mut removed = Vec::new();
    let Ok(rd) = std::fs::read_dir(base) else {
        return removed;
    };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let ours = name.len() == 16 && name.chars().all(|c| c.is_ascii_hexdigit());
        if !ours || !e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let dir = e.path();
        let gone = match std::fs::read_to_string(dir.join(OWNER_FILE)) {
            Ok(owner) => !Path::new(owner.trim()).exists(),
            Err(_) => std::fs::metadata(&dir)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age >= unowned_after),
        };
        if gone && std::fs::remove_dir_all(&dir).is_ok() {
            removed.push(dir);
        }
    }
    removed
}

#[cfg(test)]
mod sweep_tests {
    use super::*;

    #[test]
    fn a_temp_dir_whose_worktree_is_gone_is_swept_and_nothing_else() {
        let base = tempfile::tempdir().unwrap();
        let wt = tempfile::tempdir().unwrap();
        let gone = wt.path().join("gone");
        std::fs::create_dir_all(&gone).unwrap();
        let at = |n: &str| base.path().join(n);
        let policy = |root: &Path, t: PathBuf| SandboxPolicy {
            temp_dir: Some(t),
            ..SandboxPolicy::workspace(root)
        };
        prepare_temp_dir(&policy(wt.path(), at("00000000000000aa"))).unwrap();
        prepare_temp_dir(&policy(&gone, at("00000000000000bb"))).unwrap();
        // The record is written once: a later run does not rewrite it.
        prepare_temp_dir(&policy(wt.path(), at("00000000000000bb"))).unwrap();
        std::fs::create_dir_all(at("00000000000000cc")).unwrap(); // no record, fresh
        std::fs::create_dir_all(at("not-ours")).unwrap();
        std::fs::remove_dir_all(&gone).unwrap();
        let removed = sweep_in(base.path(), std::time::Duration::from_secs(3600));
        assert_eq!(removed, vec![at("00000000000000bb")]);
        assert!(at("00000000000000aa").exists(), "its worktree is there");
        assert!(at("00000000000000cc").exists(), "no record, but too young");
        assert!(at("not-ours").exists());
        // Without a record and old enough (here: any age), it goes.
        assert_eq!(
            sweep_in(base.path(), std::time::Duration::ZERO),
            vec![at("00000000000000cc")]
        );
    }
}

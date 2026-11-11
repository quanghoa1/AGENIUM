//! Making a project (ADR-159): in the app's projects folder (a new folder or a clone), or in any folder the user picks:
//! an empty one, one holding their files (made a repository, the files its first commit) or a repository (taken as
//! it is, on its current branch; only Agenium's own files are committed, the user's work in progress stays theirs).
//! Beside it, reference folders: read by the project's agents, never changed (an SDK, docs, another codebase).

use std::path::{Path, PathBuf};

use agenium_core::events::DomainEvent;
use agenium_core::ProjectId;
use agenium_store::core::projects::{self, Reference};
use agenium_store::StoreError;
use agenium_vcs::Repo;
use serde::Serialize;

use crate::app::{canonical, repo_folder, App, CreateProject, PROJECT_DIRS};
use crate::manifest::{ProjectManifest, ProjectSection};
use crate::{OrchError, Result};

/// Written into a picked folder of files that has no `.gitignore`, so its first commit leaves out what is rebuilt.
const GITIGNORE: &str = "# Added by Agenium: what is built or downloaded again stays out of git.\n\
node_modules/\ntarget/\ndist/\nbuild/\nout/\n.venv/\nvenv/\n__pycache__/\n.gradle/\n.idea/\n.vs/\n*.log\n";

/// What a folder the user picked holds, for the form to say what creating a project there does.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FolderInfo {
    pub path: String,
    pub exists: bool,
    pub empty: bool,
    /// The repository it is: its current branch (`None`: no branch checked out) and whether it has changes.
    pub repo: Option<RepoInfo>,
    /// The repository it is inside, when it is a folder of a larger one (not allowed: pick that one).
    pub inside: Option<String>,
    /// Files and bytes its first commit takes (a folder of files that is not a repository yet), `.gitignore` aside.
    pub files: u64,
    pub bytes: u64,
    /// An Agenium project already (`agenium.toml`): it opens as it is.
    pub project: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RepoInfo {
    pub branch: Option<String>,
    pub changes: bool,
}

impl App {
    /// Creates a project: folder + standard layout + `agenium.toml` + git repository (`git init` when needed,
    /// first commit), then registers it in `core.db`. An existing folder with `agenium.toml` is adopted.
    pub fn create_project(&self, req: &CreateProject) -> Result<projects::Project> {
        self.create_project_on(req, None)
    }

    /// [`App::create_project`] with the branch the team works on (ADR-159): `None`, the repository's own (the default
    /// branch of a clone, the branch checked out in a picked repository); `Some(name)`, a new branch for Agenium. A
    /// clone checks it out (the folder is Agenium's); a picked repository keeps the user's branch checked out and gets
    /// Agenium's in `.agenium/base`, so the user goes on working in their folder as before. A folder that is no
    /// repository yet starts on `main` either way.
    pub fn create_project_on(
        &self,
        req: &CreateProject,
        branch: Option<&str>,
    ) -> Result<projects::Project> {
        let branch = branch.map(str::trim).filter(|b| !b.is_empty());
        if let Some(b) = branch {
            let ok = agenium_vcs::git::Git::default()
                .run(&self.layout().root, ["check-ref-format", "--branch", b])
                .map(|o| o.ok())
                .unwrap_or(false);
            // `agenium/<KEY>` are the tickets' branches: git cannot have a branch `agenium` beside them.
            if !ok || b == "agenium" || b.starts_with("agenium/") {
                return Err(OrchError::Invalid(format!(
                    "{b:?} cannot be the team's branch: give a git branch name other than agenium or agenium/… (the \
                     tickets' own), e.g. agenium-work"
                )));
            }
        }
        let name = req.name.trim();
        if name.is_empty() || name.contains(['/', '\\', ':']) {
            return Err(OrchError::Invalid(format!(
                "project name {name:?} must be non-empty and contain no path separators"
            )));
        }
        // Two projects of one name cannot be told apart in the project picker.
        if self
            .projects()?
            .iter()
            .any(|p| p.name.trim().eq_ignore_ascii_case(name))
        {
            return Err(OrchError::Conflict(format!(
                "a project named {name:?} already exists; pick another name"
            )));
        }
        let picked = req.path.is_some();
        let path = match &req.path {
            Some(p) => {
                self.check_picked(p)?;
                p.clone()
            }
            None => {
                let folder = req
                    .folder
                    .as_deref()
                    .map(str::trim)
                    .filter(|f| !f.is_empty())
                    .map(str::to_owned)
                    .or_else(|| {
                        req.repo
                            .as_deref()
                            .map(repo_folder)
                            .filter(|f| !f.is_empty())
                    })
                    .unwrap_or_else(|| name.to_owned());
                if folder.contains(['/', '\\', ':']) || folder == "." || folder == ".." {
                    return Err(OrchError::Invalid(format!(
                        "folder {folder:?}: give a folder name, not a path (projects live in the app's projects folder)"
                    )));
                }
                self.projects_dir().join(folder)
            }
        };
        let mut cloned_branch = None;
        if let Some(url) = req.repo.as_deref().map(str::trim).filter(|u| !u.is_empty()) {
            if path.exists() && !is_empty_dir(&path) {
                return Err(OrchError::Conflict(format!(
                    "the folder {} already exists; pick another folder name",
                    path.file_name().unwrap_or_default().to_string_lossy()
                )));
            }
            std::fs::create_dir_all(self.projects_dir())
                .map_err(|e| OrchError::Invalid(e.to_string()))?;
            let git = agenium_vcs::git::Git::default();
            let out = git.run(
                &self.projects_dir(),
                [
                    std::ffi::OsStr::new("clone"),
                    "--".as_ref(),
                    url.as_ref(),
                    path.as_os_str(),
                ],
            )?;
            if !out.ok() {
                let _ = std::fs::remove_dir_all(&path);
                return Err(OrchError::Invalid(format!(
                    "git clone failed: {}",
                    out.stderr.trim().lines().last().unwrap_or("unknown error")
                )));
            }
            cloned_branch = current_branch(&path);
            if let Some(b) = branch {
                git.check(&path, ["checkout", "-q", "-b", b])?;
                cloned_branch = Some(b.to_owned());
            }
        }
        std::fs::create_dir_all(&path)
            .map_err(|e| OrchError::Invalid(format!("{}: {e}", path.display())))?;
        let path = canonical(&path);
        let probe = Repo::open(&path, "main");
        // A repository the user picked is theirs: Agenium works on its current branch and commits only its own files.
        let adopted = picked && probe.is_repo();
        if picked && !adopted {
            if let Some(top) = probe.toplevel() {
                return Err(OrchError::Invalid(format!(
                    "{} is a folder inside the git repository {}: pick that repository itself, or a folder outside it",
                    path.display(),
                    top.display()
                )));
            }
        }
        let fresh_files = picked && !adopted && !is_empty_dir(&path);
        let current = adopted.then(|| current_branch(&path)).flatten();
        let manifest = match ProjectManifest::load(&path)? {
            Some(m) => m,
            None => {
                let branch = if adopted {
                    Some(branch.map(str::to_owned).or(current.clone()).ok_or_else(|| {
                        OrchError::Invalid(format!(
                            "{} has no branch checked out: check out the branch the team should work on, then create the project",
                            path.display()
                        ))
                    })?)
                } else {
                    cloned_branch.clone()
                };
                let m = ProjectManifest {
                    project: ProjectSection {
                        id: ProjectId::new().to_string(),
                        name: name.to_owned(),
                        key_prefix: req.key_prefix.clone().unwrap_or_else(|| "AGN".into()),
                        main_branch: branch.unwrap_or_else(|| "main".into()),
                        template: req.template.clone().unwrap_or_else(|| "coding@1".into()),
                    },
                    worktree: Default::default(),
                    checks: Default::default(),
                    network: Default::default(),
                };
                m.save(&path)?;
                m
            }
        };
        let id: ProjectId = manifest
            .project
            .id
            .parse()
            .map_err(|e| OrchError::Invalid(format!("agenium.toml: {e}")))?;
        // Agenium's own branch beside the user's: its files live in `.agenium/base`; `agenium.toml` stays local.
        let side = adopted && current.as_deref() != Some(manifest.project.main_branch.as_str());
        let layout = if side {
            side_checkout(&path, &manifest.project.main_branch)?
        } else {
            path.clone()
        };
        if fresh_files && !path.join(".gitignore").exists() {
            std::fs::write(path.join(".gitignore"), GITIGNORE)
                .map_err(|e| OrchError::Invalid(format!("{}: {e}", path.display())))?;
        }
        let mut ours = if side {
            vec![]
        } else {
            vec![crate::manifest::FILE.to_owned()]
        };
        for d in PROJECT_DIRS {
            let dir = layout.join(d);
            std::fs::create_dir_all(&dir)
                .map_err(|e| OrchError::Invalid(format!("{}: {e}", dir.display())))?;
            let keep = dir.join(".gitkeep");
            if is_empty_dir(&dir) {
                std::fs::write(&keep, b"")
                    .map_err(|e| OrchError::Invalid(format!("{}: {e}", keep.display())))?;
            }
            if keep.exists() {
                ours.push(format!("{d}/.gitkeep"));
            }
        }
        let had_git = path.join(".git").exists();
        let repo = Repo::open(&path, &manifest.project.main_branch);
        repo.init_local()?;
        let who = (
            agenium_vcs::git::COMMITTER_NAME,
            agenium_vcs::git::COMMITTER_EMAIL,
        );
        if side {
            exclude(&path, crate::manifest::FILE)?;
            let git = repo.git_for(&layout)?;
            let mut add = vec!["add", "--"];
            add.extend(ours.iter().map(String::as_str));
            git.check(&layout, add)?;
            if !git.run(&layout, ["diff", "--cached", "--quiet"])?.ok() {
                git.check_env(
                    &layout,
                    [
                        "commit",
                        "-q",
                        "--no-verify",
                        "-m",
                        "Agenium project layout",
                    ],
                    &[("GIT_AUTHOR_NAME", who.0), ("GIT_AUTHOR_EMAIL", who.1)],
                )?;
            }
        } else if adopted {
            let ours: Vec<&str> = ours.iter().map(String::as_str).collect();
            repo.commit_paths(&ours, "Agenium project layout", who)?;
        } else if let Err(e) = repo.commit_all(&path, "Agenium project layout", who, &[]) {
            // A folder of the user's files that cannot be committed (a secret in it) is left as it was found, but for
            // `agenium.toml`: the repository made a moment ago goes, so trying again starts the same way.
            if fresh_files && !had_git {
                let _ = std::fs::remove_dir_all(path.join(".git"));
            }
            return Err(e.into());
        }

        let path_str = path.to_string_lossy().into_owned();
        if let Some(existing) = self.store().read(|c| projects::by_path(c, &path_str))? {
            return Ok(existing);
        }
        let np = projects::NewProject {
            id,
            name: manifest.project.name.clone(),
            path: path_str,
            key_prefix: manifest.project.key_prefix.clone(),
            main_branch: manifest.project.main_branch.clone(),
        };
        let p = self
            .store()
            .commit(None, "user", |w| -> std::result::Result<_, StoreError> {
                let p = projects::insert(w.tx, &np)?;
                w.emit(DomainEvent::ProjectCreated {
                    project: p.id,
                    name: p.name.clone(),
                    path: p.path.clone(),
                })?;
                Ok(p)
            })?
            .value;
        Ok(p)
    }

    /// A folder the user picked for a project: a full path, holding none of Agenium's own data.
    fn check_picked(&self, p: &Path) -> Result<()> {
        if !p.is_absolute() {
            return Err(OrchError::Invalid(format!(
                "{} is not a full path: pick the folder",
                p.display()
            )));
        }
        let p = canonical(p);
        let root = canonical(&self.layout().root);
        let projects = canonical(&self.projects_dir());
        if root.starts_with(&p) || (p.starts_with(&root) && !p.starts_with(&projects)) {
            return Err(OrchError::Invalid(format!(
                "{} holds Agenium's own data: pick another folder",
                p.display()
            )));
        }
        Ok(())
    }

    /// What creating a project in `path` would do (the form shows it before the user creates it).
    pub fn inspect_folder(&self, path: &Path) -> Result<FolderInfo> {
        let mut info = FolderInfo {
            path: path.display().to_string(),
            exists: path.is_dir(),
            empty: true,
            repo: None,
            inside: None,
            files: 0,
            bytes: 0,
            project: path.join(crate::manifest::FILE).is_file(),
        };
        if !info.exists {
            return Ok(info);
        }
        info.empty = is_empty_dir(path);
        let probe = Repo::open(path, "main");
        if probe.is_repo() {
            let changes = probe
                .git()
                .check(path, ["status", "--porcelain"])
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false);
            info.repo = Some(RepoInfo {
                branch: current_branch(path),
                changes,
            });
        } else if let Some(top) = probe.toplevel() {
            info.inside = Some(top.display().to_string());
        } else {
            (info.files, info.bytes) = count_files(path);
        }
        Ok(info)
    }

    /// Replaces the project's reference folders: full paths of folders that exist, none holding the project or
    /// Agenium's data (their contents become readable to every run of the project).
    pub fn set_references(
        &self,
        project: ProjectId,
        refs: Vec<Reference>,
    ) -> Result<Vec<Reference>> {
        let p = self.project(project)?;
        let home = canonical(Path::new(&p.path));
        let root = canonical(&self.layout().root);
        let mut out: Vec<Reference> = Vec::new();
        for r in refs {
            let raw = PathBuf::from(r.path.trim());
            if !raw.is_absolute() || !raw.is_dir() {
                return Err(OrchError::Invalid(format!(
                    "{}: not a folder on this machine",
                    raw.display()
                )));
            }
            let dir = canonical(&raw);
            if dir.starts_with(&home) || home.starts_with(&dir) {
                return Err(OrchError::Invalid(format!(
                    "{}: the project's own folder (or one holding it) cannot be a reference: its agents read it already",
                    dir.display()
                )));
            }
            if root.starts_with(&dir) || dir.starts_with(&root) {
                return Err(OrchError::Invalid(format!(
                    "{}: Agenium's own data cannot be a reference",
                    dir.display()
                )));
            }
            let path = dir.display().to_string();
            if !out.iter().any(|o| o.path == path) {
                out.push(Reference {
                    path,
                    note: r.note.trim().to_owned(),
                });
            }
        }
        self.set_project_setting(project, projects::REFERENCES, serde_json::json!(out))?;
        Ok(out)
    }
}

/// The reference folders' lines for an agent's manifest ("" when none).
pub fn references_text(refs: &[Reference]) -> Option<String> {
    if refs.is_empty() {
        return None;
    }
    let list: Vec<String> = refs
        .iter()
        .map(|r| match r.note.is_empty() {
            true => r.path.clone(),
            false => format!("{} ({})", r.path, r.note),
        })
        .collect();
    Some(format!(
        "Reference folders (read-only: read and search them with your tools, never change them; your work goes in \
         your worktree): {}",
        list.join("; ")
    ))
}

/// Whether git starts here (a clean Windows has none, and every project is a git repository).
pub fn git_available(dir: &Path) -> bool {
    agenium_vcs::git::Git::default()
        .run(dir, ["--version"])
        .is_ok_and(|o| o.ok())
}

/// Where a project's files are checked out on its own branch: `.agenium/base` when Agenium works on a branch beside
/// the one the user has checked out in the folder (ADR-159), else the folder itself.
pub fn checkout_dir(project_path: &Path) -> PathBuf {
    let base = project_path.join(".agenium").join("base");
    if base.join(".git").exists() {
        base
    } else {
        project_path.to_path_buf()
    }
}

/// Makes Agenium's branch (from the user's current commit, unless it exists) and checks it out in `.agenium/base`.
fn side_checkout(path: &Path, branch: &str) -> Result<PathBuf> {
    let base = path.join(".agenium").join("base");
    if base.join(".git").exists() {
        return Ok(base);
    }
    let git = agenium_vcs::git::Git::default();
    let exists = git
        .run(
            path,
            [
                "rev-parse",
                "--verify",
                "-q",
                &format!("refs/heads/{branch}"),
            ],
        )?
        .ok();
    if !exists {
        let made = git.run(path, ["branch", branch, "HEAD"])?;
        if !made.ok() {
            return Err(OrchError::Invalid(format!(
                "the branch {branch} could not be made from the current commit (a repository with no commit yet \
                 cannot have one): {}",
                made.stderr.trim()
            )));
        }
    }
    std::fs::create_dir_all(path.join(".agenium"))
        .map_err(|e| OrchError::Invalid(format!("{}: {e}", path.display())))?;
    git.check(
        path,
        [
            std::ffi::OsStr::new("worktree"),
            "add".as_ref(),
            "-q".as_ref(),
            base.as_os_str(),
            branch.as_ref(),
        ],
    )?;
    Ok(base)
}

/// Adds `entry` to the repository's local excludes (`.git/info/exclude`), never to its `.gitignore`.
fn exclude(path: &Path, entry: &str) -> Result<()> {
    let file = path.join(".git").join("info").join("exclude");
    let cur = std::fs::read_to_string(&file).unwrap_or_default();
    if cur.lines().any(|l| l.trim() == entry) {
        return Ok(());
    }
    if let Some(d) = file.parent() {
        std::fs::create_dir_all(d)
            .map_err(|e| OrchError::Invalid(format!("{}: {e}", d.display())))?;
    }
    let sep = if cur.is_empty() || cur.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    std::fs::write(&file, format!("{cur}{sep}{entry}\n"))
        .map_err(|e| OrchError::Invalid(format!("{}: {e}", file.display())))
}

fn is_empty_dir(p: &Path) -> bool {
    std::fs::read_dir(p)
        .map(|mut d| d.next().is_none())
        .unwrap_or(true)
}

fn current_branch(path: &Path) -> Option<String> {
    agenium_vcs::git::Git::default()
        .check(path, ["symbolic-ref", "--short", "HEAD"])
        .ok()
        .map(|b| b.trim().to_owned())
        .filter(|b| !b.is_empty())
}

/// Files and bytes under `dir`, the folders [`GITIGNORE`] leaves out skipped (an estimate for the form).
fn count_files(dir: &Path) -> (u64, u64) {
    const SKIP: &[&str] = &[
        "node_modules",
        "target",
        "dist",
        "build",
        "out",
        ".venv",
        "venv",
        "__pycache__",
        ".gradle",
        ".idea",
        ".vs",
    ];
    let (mut files, mut bytes) = (0, 0);
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let Ok(t) = e.file_type() else { continue };
            if t.is_dir() {
                if !SKIP.contains(&e.file_name().to_string_lossy().as_ref()) {
                    stack.push(e.path());
                }
            } else if t.is_file() {
                files += 1;
                bytes += e.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
    }
    (files, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_lines_name_each_folder_and_its_note() {
        assert_eq!(references_text(&[]), None);
        let t = references_text(&[
            Reference {
                path: "/sdk".into(),
                note: "the device SDK".into(),
            },
            Reference {
                path: "/docs".into(),
                note: String::new(),
            },
        ])
        .unwrap();
        assert!(t.contains("/sdk (the device SDK); /docs"), "{t}");
        assert!(t.contains("read-only"));
    }
}

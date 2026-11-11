//! Projects in a folder the user picked (ADR-159): empty, holding their files, or a repository; reference folders.

use super::*;
use agenium_orchestrator::app::CreateProject;
use agenium_store::core::projects::Reference;
use agenium_vcs::git::Git;

fn picked(
    f: &Fixture,
    name: &str,
    path: &std::path::Path,
) -> Result<agenium_store::core::projects::Project> {
    f.app.create_project(&CreateProject {
        name: name.into(),
        path: Some(path.to_path_buf()),
        key_prefix: Some(name.to_uppercase()),
        template: None,
        folder: None,
        repo: None,
    })
}

fn git(dir: &std::path::Path, args: &[&str]) -> String {
    Git::default().check(dir, args).unwrap()
}

#[test]
fn a_folder_of_files_becomes_a_repository_with_them_as_its_first_commit() {
    let f = fixture();
    let dir = f._tmp.path().join("mine");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::create_dir_all(dir.join("node_modules/x")).unwrap();
    std::fs::write(dir.join("src/main.c"), "int main(){}\n").unwrap();
    std::fs::write(dir.join("node_modules/x/big.js"), "x").unwrap();
    let info = f.app.inspect_folder(&dir).unwrap();
    assert!(info.exists && !info.empty && info.repo.is_none() && info.inside.is_none());
    assert_eq!(info.files, 1, "node_modules is not counted");

    let p = picked(&f, "mine", &dir).unwrap();
    assert_eq!(p.main_branch, "main");
    let tracked = git(&dir, &["ls-files"]);
    assert!(
        tracked.contains("src/main.c")
            && tracked.contains("agenium.toml")
            && tracked.contains(".gitignore")
    );
    assert!(!tracked.contains("node_modules"), "{tracked}");
    assert!(git(&dir, &["status", "--porcelain"]).trim().is_empty());
}

#[test]
fn a_repository_is_taken_on_its_branch_and_the_users_changes_stay_theirs() {
    let f = fixture();
    let dir = f._tmp.path().join("theirs");
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "master"]);
    std::fs::write(dir.join("a.txt"), "one\n").unwrap();
    git(&dir, &["add", "a.txt"]);
    git(
        &dir,
        &[
            "-c",
            "user.name=u",
            "-c",
            "user.email=u@x",
            "commit",
            "-q",
            "-m",
            "first",
        ],
    );
    std::fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
    std::fs::write(dir.join("draft.txt"), "wip\n").unwrap();
    git(&dir, &["add", "draft.txt"]);
    let info = f.app.inspect_folder(&dir).unwrap();
    let repo = info.repo.unwrap();
    assert_eq!(repo.branch.as_deref(), Some("master"));
    assert!(repo.changes);

    let p = picked(&f, "theirs", &dir).unwrap();
    assert_eq!(p.main_branch, "master");
    let last = git(&dir, &["show", "--name-only", "--format=%s", "HEAD"]);
    assert!(last.starts_with("Agenium project layout"), "{last}");
    assert!(
        last.contains("agenium.toml") && !last.contains("a.txt") && !last.contains("draft.txt"),
        "{last}"
    );
    let status = git(&dir, &["status", "--porcelain"]);
    assert!(
        status.contains(" M a.txt") && status.contains("A  draft.txt"),
        "{status}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("a.txt")).unwrap(),
        "one\ntwo\n"
    );
}

#[test]
fn an_empty_folder_starts_a_project_and_a_folder_inside_a_repository_is_refused() {
    let f = fixture();
    let empty = f._tmp.path().join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    assert!(f.app.inspect_folder(&empty).unwrap().empty);
    picked(&f, "empty", &empty).unwrap();
    assert!(empty.join("agenium.toml").is_file());

    let sub = empty.join("deliverables").join("part");
    std::fs::create_dir_all(&sub).unwrap();
    assert!(f.app.inspect_folder(&sub).unwrap().inside.is_some());
    let e = picked(&f, "part", &sub).unwrap_err().to_string();
    assert!(e.contains("inside the git repository"), "{e}");
    assert!(!sub.join("agenium.toml").exists());

    let e = picked(&f, "rel", std::path::Path::new("relative"))
        .unwrap_err()
        .to_string();
    assert!(e.contains("not a full path"), "{e}");
    let e = picked(&f, "data", &f.root.join("db"))
        .unwrap_err()
        .to_string();
    assert!(e.contains("Agenium's own data"), "{e}");
}

#[test]
fn reference_folders_are_checked_and_kept() {
    let f = fixture();
    let sdk = f._tmp.path().join("sdk");
    std::fs::create_dir_all(&sdk).unwrap();
    let r = |p: &std::path::Path| Reference {
        path: p.display().to_string(),
        note: String::new(),
    };
    for bad in [
        f._tmp.path().join("missing"),
        std::path::PathBuf::from(&f.project.path),
        f.root.clone(),
    ] {
        assert!(
            f.app.set_references(f.project.id, vec![r(&bad)]).is_err(),
            "{}",
            bad.display()
        );
    }
    let kept = f
        .app
        .set_references(f.project.id, vec![r(&sdk), r(&sdk)])
        .unwrap();
    assert_eq!(kept.len(), 1);
    assert_eq!(f.app.project(f.project.id).unwrap().references(), kept);
    f.app.set_references(f.project.id, vec![]).unwrap();
    assert!(f.app.project(f.project.id).unwrap().references().is_empty());
}

#[test]
fn a_picked_repository_on_a_new_branch_keeps_the_users_folder_as_it_was() {
    let f = fixture();
    let dir = f._tmp.path().join("ongoing");
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "master"]);
    std::fs::write(dir.join("a.txt"), "one\n").unwrap();
    git(&dir, &["add", "a.txt"]);
    git(
        &dir,
        &[
            "-c",
            "user.name=u",
            "-c",
            "user.email=u@x",
            "commit",
            "-q",
            "-m",
            "first",
        ],
    );
    std::fs::write(dir.join("a.txt"), "one\nwip\n").unwrap();

    let p = f
        .app
        .create_project_on(
            &CreateProject {
                name: "ongoing".into(),
                path: Some(dir.clone()),
                key_prefix: Some("ONG".into()),
                template: None,
                folder: None,
                repo: None,
            },
            Some("agenium-work"),
        )
        .unwrap();
    assert_eq!(p.main_branch, "agenium-work");
    assert_eq!(
        git(&dir, &["symbolic-ref", "--short", "HEAD"]).trim(),
        "master"
    );
    // Their folder: the same branch, the same change in progress, nothing of Agenium's to commit.
    assert_eq!(git(&dir, &["status", "--porcelain"]).trim(), "M a.txt");
    assert_eq!(git(&dir, &["log", "--format=%s", "master"]).trim(), "first");
    let base = dir.join(".agenium").join("base");
    assert_eq!(
        agenium_orchestrator::project_create::checkout_dir(&dir),
        base
    );
    let files = git(&base, &["ls-files"]);
    assert!(
        files.contains("a.txt") && files.contains("deliverables/.gitkeep"),
        "{files}"
    );
    assert_eq!(
        std::fs::read_to_string(base.join("a.txt")).unwrap(),
        "one\n"
    );
    assert!(dir.join("agenium.toml").is_file() && !dir.join("deliverables").exists());

    let e = f
        .app
        .create_project_on(
            &CreateProject {
                name: "bad".into(),
                path: Some(f._tmp.path().join("other")),
                key_prefix: Some("BAD".into()),
                template: None,
                folder: None,
                repo: None,
            },
            Some("agenium"),
        )
        .unwrap_err()
        .to_string();
    assert!(e.contains("cannot be the team's branch"), "{e}");
}

#[test]
fn a_clone_on_a_new_branch_checks_it_out() {
    let f = fixture();
    let origin = f._tmp.path().join("origin");
    std::fs::create_dir_all(&origin).unwrap();
    git(&origin, &["init", "-q", "-b", "trunk"]);
    std::fs::write(origin.join("x.txt"), "x\n").unwrap();
    git(&origin, &["add", "x.txt"]);
    git(
        &origin,
        &[
            "-c",
            "user.name=u",
            "-c",
            "user.email=u@x",
            "commit",
            "-q",
            "-m",
            "x",
        ],
    );
    let req = |name: &str, folder: &str| CreateProject {
        name: name.into(),
        path: None,
        key_prefix: Some(name.to_uppercase()),
        template: None,
        folder: Some(folder.into()),
        repo: Some(origin.display().to_string()),
    };
    let own = f.app.create_project_on(&req("own", "own"), None).unwrap();
    assert_eq!(own.main_branch, "trunk");
    let new = f
        .app
        .create_project_on(&req("new", "new"), Some("work"))
        .unwrap();
    assert_eq!(new.main_branch, "work");
    let at = std::path::Path::new(&new.path);
    assert_eq!(git(at, &["symbolic-ref", "--short", "HEAD"]).trim(), "work");
    assert!(git(at, &["branch", "--list", "trunk"]).contains("trunk"));
}

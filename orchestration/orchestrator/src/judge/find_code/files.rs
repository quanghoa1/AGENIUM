//! Which files a search may read, and reading them: the tree under the search root as `.gitignore` leaves it, without
//! hidden folders, build output, lock files, binaries, oversized files or credential files by name (`.env`, keys,
//! certificates) — those are never read. A secret inside an ordinary file is masked on its way out: everything sent to the
//! judge passes the redactor (`client::ask`). (A first version left out every file holding something secret-looking,
//! which hid the secret scanner's own source — its tests hold sample keys — from a search about masking secrets.)

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// Files larger than this are not read (generated data, bundles).
const MAX_FILE_BYTES: u64 = 512 * 1024;
/// At most this many files are listed in one search.
const MAX_FILES: usize = 20_000;
/// Folders never searched (build output, dependencies, caches).
const SKIP_DIRS: &[&str] = &[
    "target",
    "node_modules",
    "dist",
    "build",
    "out",
    "coverage",
    "__pycache__",
    "venv",
    ".venv",
    "vendor",
];
/// Files never searched by extension (binaries, media, archives, maps, minified bundles).
const SKIP_EXT: &[&str] = &[
    "png",
    "jpg",
    "jpeg",
    "gif",
    "ico",
    "webp",
    "svg",
    "pdf",
    "zip",
    "gz",
    "zst",
    "tar",
    "7z",
    "exe",
    "dll",
    "so",
    "dylib",
    "a",
    "o",
    "obj",
    "lib",
    "wasm",
    "class",
    "jar",
    "bin",
    "db",
    "sqlite",
    "lock",
    "map",
    "woff",
    "woff2",
    "ttf",
    "otf",
    "mp4",
    "mp3",
    "wav",
    "onnx",
    "safetensors",
    "pt",
    "ckpt",
    "parquet",
];
/// Lock files and other generated manifests.
const SKIP_NAMES: &[&str] = &[
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "bun.lock",
    "bun.lockb",
    "Cargo.lock",
    "poetry.lock",
    "go.sum",
    "composer.lock",
    "Gemfile.lock",
];

/// A file name that usually holds credentials: never read.
pub(super) fn credential_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n == ".env"
        || n.starts_with(".env.")
        || n.ends_with(".env")
        || n.ends_with(".pem")
        || n.ends_with(".key")
        || n.ends_with(".p12")
        || n.ends_with(".pfx")
        || n.starts_with("id_rsa")
        || n.starts_with("id_ed25519")
        || n.starts_with("id_ecdsa")
        || matches!(
            n.as_str(),
            ".npmrc" | ".pypirc" | ".netrc" | "credentials" | "credentials.json"
        )
}

/// The searchable files under a root.
pub(super) struct Repo {
    pub root: PathBuf,
    /// Root-relative paths with `/`, sorted.
    pub files: Vec<String>,
    /// Reading stopped at `MAX_FILES`.
    pub truncated: bool,
    texts: std::sync::Mutex<HashMap<String, Option<String>>>,
}

impl Repo {
    /// Lists the searchable files under `root`.
    pub fn scan(root: &Path) -> Repo {
        let mut files = Vec::new();
        let mut truncated = false;
        let walk = ignore::WalkBuilder::new(root)
            .hidden(true)
            .git_ignore(true)
            .git_exclude(true)
            .parents(true)
            .require_git(false)
            .filter_entry(|e| {
                let n = e.file_name().to_string_lossy();
                !(e.file_type().is_some_and(|t| t.is_dir()) && SKIP_DIRS.contains(&n.as_ref()))
            })
            .build();
        for e in walk.flatten() {
            if !e.file_type().is_some_and(|t| t.is_file()) {
                continue;
            }
            let name = e.file_name().to_string_lossy().to_string();
            let ext = Path::new(&name)
                .extension()
                .map(|x| x.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            if credential_name(&name)
                || SKIP_NAMES.contains(&name.as_str())
                || SKIP_EXT.contains(&ext.as_str())
                || name.ends_with(".min.js")
                || e.metadata()
                    .map_or(true, |m| m.len() > MAX_FILE_BYTES || m.len() == 0)
            {
                continue;
            }
            let Ok(rel) = e.path().strip_prefix(root) else {
                continue;
            };
            if files.len() >= MAX_FILES {
                truncated = true;
                break;
            }
            files.push(rel.to_string_lossy().replace('\\', "/"));
        }
        files.sort();
        Repo {
            root: root.to_path_buf(),
            files,
            truncated,
            texts: Default::default(),
        }
    }

    /// The text of `rel`, or `None` when it is binary or not UTF-8.
    pub fn text(&self, rel: &str) -> Option<String> {
        let mut texts = self.texts.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(t) = texts.get(rel) {
            return t.clone();
        }
        let t = std::fs::read(self.root.join(rel))
            .ok()
            .filter(|b| !b.iter().take(8192).any(|x| *x == 0))
            .and_then(|b| String::from_utf8(b).ok());
        texts.insert(rel.to_owned(), t.clone());
        t
    }

    /// Folders (root-relative, "" for the root) with their direct subfolders and files.
    pub fn tree(&self) -> BTreeMap<String, (Vec<String>, Vec<String>)> {
        let mut t: BTreeMap<String, (Vec<String>, Vec<String>)> = BTreeMap::new();
        t.entry(String::new()).or_default();
        for f in &self.files {
            let (dir, _) = f.rsplit_once('/').unwrap_or(("", f));
            t.entry(dir.to_owned()).or_default().1.push(f.clone());
            let mut d = dir;
            while !d.is_empty() {
                let parent = d.rsplit_once('/').map_or("", |(p, _)| p);
                let kids = &mut t.entry(parent.to_owned()).or_default().0;
                if !kids.iter().any(|k| k == d) {
                    kids.push(d.to_owned());
                }
                d = parent;
            }
        }
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_and_build_output_are_never_read() {
        let tmp = tempfile::tempdir().unwrap();
        let r = tmp.path();
        let w = |p: &str, s: &str| {
            let p = r.join(p);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, s).unwrap();
        };
        w("src/auth.rs", "fn check() {}\n");
        w(
            "src/keys.rs",
            "const K: &str = \"sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\";\n",
        );
        w(".env", "TOKEN=x\n");
        w("config/prod.env", "TOKEN=x\n");
        w("deploy/server.pem", "-----BEGIN-----\n");
        w("target/debug/out.rs", "fn x() {}\n");
        w("node_modules/a/index.js", "x\n");
        w("Cargo.lock", "x\n");
        w(".hidden/notes.md", "x\n");
        let repo = Repo::scan(r);
        assert_eq!(repo.files, ["src/auth.rs", "src/keys.rs"]);
        assert!(repo.text("src/auth.rs").is_some());
        assert!(
            repo.text("src/keys.rs").is_some(),
            "an ordinary file is read; its secret is masked when it is sent"
        );
        let tree = repo.tree();
        assert_eq!(tree[""].0, ["src"]);
        assert_eq!(tree["src"].1, ["src/auth.rs", "src/keys.rs"]);
    }
}

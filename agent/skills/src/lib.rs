//! Skills (decision 34): a skill is a folder with `SKILL.md` (frontmatter `name`, `description` + a Markdown body)
//! and optional `scripts/`, `references/`, `assets/`. Progressive disclosure: the work order carries only name +
//! description (tier 1), `load_skill` returns the body (tier 2), referenced files are read with `read_file` (tier 3).
//!
//! Storage: an agent's own skills (and its copy-on-write overlays) live in `<root>/agents/<id>/skills/<name>/`, role
//! skills in `<root>/roles/<role>/skills/<name>/`. An agent's own copy wins over the role's skill of the same name.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;

mod files;
pub use files::file_path;

pub const SKILL_FILE: &str = "SKILL.md";
pub const MAX_NAME: usize = 64;
pub const MAX_TAGS: usize = 12;
pub const MAX_DESCRIPTION: usize = 1024;
pub const MAX_SKILL_MD: u64 = 256 * 1024;
pub const MAX_FILES: usize = 1000;
pub const MAX_TOTAL_BYTES: u64 = 50 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum SkillError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid skill: {0}")]
    Invalid(String),
    #[error("skill not found: {0}")]
    NotFound(String),
    #[error("skill {0:?} already exists (use replace)")]
    Exists(String),
    #[error("zip: {0}")]
    Zip(String),
}

pub type Result<T> = std::result::Result<T, SkillError>;

fn invalid(m: impl Into<String>) -> SkillError {
    SkillError::Invalid(m.into())
}

/// Where a skill comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// The system's skill library (`<root>/skills`), used by the agents that enable it (ADR-61).
    Library,
    /// Shared by every agent of the role.
    Role,
    /// The agent's own skill.
    Agent,
    /// The agent's own copy of a role or library skill (it replaces that version for this agent only).
    Overlay,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillMeta {
    pub name: String,
    pub description: String,
    pub version: Option<String>,
    /// Domains the skill belongs to (`pentest`, `coding`, `research`, `finance`, …), from the `tags` frontmatter
    /// field (comma- or space-separated). Lets the user see which fields an agent's skills cover.
    pub tags: Vec<String>,
    pub origin: Origin,
    pub dir: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct Skill {
    pub meta: SkillMeta,
    pub body: String,
    /// Files of the folder relative to it (forward slashes), `SKILL.md` excluded.
    pub files: Vec<String>,
}

impl Skill {
    /// Tier-2 text returned by `load_skill`.
    pub fn instructions(&self) -> String {
        let mut s = format!("# Skill: {}", self.meta.name);
        if let Some(v) = &self.meta.version {
            s.push_str(&format!(" (version {v})"));
        }
        s.push_str(&format!(
            "\nFolder (read-only): {}\n",
            self.meta.dir.display()
        ));
        if self.files.is_empty() {
            s.push_str("Files: none\n");
        } else {
            s.push_str(&format!(
                "Files (read them with read_file using the folder path; scripts run only through shell under the \
                 same approval rules as any command): {}\n",
                self.files.join(", ")
            ));
        }
        s.push('\n');
        s.push_str(self.body.trim());
        s
    }
}

/// Parsed frontmatter of SKILL.md.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Frontmatter {
    pub fields: BTreeMap<String, String>,
}

impl Frontmatter {
    pub fn get(&self, k: &str) -> Option<&str> {
        self.fields.get(k).map(String::as_str)
    }
}

fn unquote(v: &str) -> String {
    let v = v.trim();
    let b = v.as_bytes();
    if b.len() >= 2
        && ((b[0] == b'"' && b[b.len() - 1] == b'"') || (b[0] == b'\'' && b[b.len() - 1] == b'\''))
    {
        let inner = &v[1..v.len() - 1];
        if b[0] == b'"' {
            return inner.replace("\\\"", "\"").replace("\\n", "\n");
        }
        return inner.replace("''", "'");
    }
    v.to_owned()
}

/// Splits SKILL.md into frontmatter and body. Supports the subset of YAML skills use: top-level `key: value`
/// (plain or quoted), block scalars (`|`, `>`), and nested keys (ignored).
pub fn parse(text: &str) -> Result<(Frontmatter, String)> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return Err(invalid(
            "SKILL.md must start with a '---' frontmatter block",
        ));
    }
    let mut fm = Frontmatter::default();
    let mut block: Option<(String, bool, Vec<String>)> = None; // key, folded, lines
    let mut closed = false;
    let mut consumed = 1usize;
    for line in lines.by_ref() {
        consumed += 1;
        if line.trim_end() == "---" {
            closed = true;
            break;
        }
        let indented = line.starts_with(' ') || line.starts_with('\t');
        if let Some((_, _, buf)) = block.as_mut() {
            if indented || line.trim().is_empty() {
                buf.push(line.trim().to_owned());
                continue;
            }
            let (k, folded, buf) = block.take().unwrap();
            let joined = if folded {
                buf.join(" ")
            } else {
                buf.join("\n")
            };
            fm.fields.insert(k, joined.trim().to_owned());
        }
        if indented || line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue; // nested mapping or comment
        }
        let Some((k, v)) = line.split_once(':') else {
            return Err(invalid(format!(
                "frontmatter line is not 'key: value': {line:?}"
            )));
        };
        let k = k.trim().to_owned();
        let v = v.trim();
        match v {
            "|" | "|-" | "|+" => block = Some((k, false, vec![])),
            ">" | ">-" | ">+" => block = Some((k, true, vec![])),
            "" => {} // a nested mapping follows
            _ => {
                fm.fields.insert(k, unquote(v));
            }
        }
    }
    if let Some((k, folded, buf)) = block.take() {
        let joined = if folded {
            buf.join(" ")
        } else {
            buf.join("\n")
        };
        fm.fields.insert(k, joined.trim().to_owned());
    }
    if !closed {
        return Err(invalid("frontmatter is not closed with '---'"));
    }
    let body: Vec<&str> = text.lines().skip(consumed).collect();
    Ok((fm, body.join("\n")))
}

/// Tags of a skill, from a comma- or space-separated `tags:` field; lowercased, deduplicated, order kept.
pub fn parse_tags(raw: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in raw.split([',', ' ', '\t', '\n']) {
        let t = t.trim().to_ascii_lowercase();
        if !t.is_empty() && !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

/// Skill names: 1-64 characters of `a-z`, `0-9` and single hyphens, not at the ends.
pub fn valid_name(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= MAX_NAME
        && n.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !n.starts_with('-')
        && !n.ends_with('-')
        && !n.contains("--")
}

/// Relative paths the body refers to: Markdown link targets and backticked paths under scripts/, references/,
/// assets/. URLs and anchors are ignored.
pub fn references(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(i) = rest.find("](") {
        let after = &rest[i + 2..];
        let Some(end) = after.find(')') else { break };
        let target = after[..end].split_whitespace().next().unwrap_or("");
        rest = &after[end..];
        let target = target.trim_matches(|c| c == '<' || c == '>');
        if target.is_empty()
            || target.starts_with('#')
            || target.contains("://")
            || target.starts_with("mailto:")
        {
            continue;
        }
        let t = target.split(['#', '?']).next().unwrap_or("");
        if !t.is_empty() {
            out.push(t.to_owned());
        }
    }
    for (i, part) in body.split('`').enumerate() {
        if i % 2 == 1 && !part.contains(char::is_whitespace) {
            let p = part.trim_start_matches("./");
            if ["scripts/", "references/", "assets/"]
                .iter()
                .any(|d| p.starts_with(d))
                && p.len() > p.find('/').unwrap() + 1
            {
                out.push(p.to_owned());
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// A referenced path must be relative and stay inside the folder.
fn inside(rel: &str) -> Option<PathBuf> {
    let p = Path::new(rel);
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::Normal(s) => out.push(s),
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

fn walk(dir: &Path, base: &Path, out: &mut Vec<String>, total: &mut u64) -> Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.collect::<std::io::Result<_>>()?;
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let ft = e.file_type()?;
        let path = e.path();
        if ft.is_symlink() {
            return Err(invalid(format!(
                "symbolic links are not allowed in a skill: {}",
                path.display()
            )));
        }
        if ft.is_dir() {
            walk(&path, base, out, total)?;
        } else if ft.is_file() {
            *total += e.metadata()?.len();
            let rel = path.strip_prefix(base).unwrap_or(&path);
            out.push(rel.to_string_lossy().replace('\\', "/"));
            if out.len() > MAX_FILES {
                return Err(invalid(format!(
                    "a skill may hold at most {MAX_FILES} files"
                )));
            }
            if *total > MAX_TOTAL_BYTES {
                return Err(invalid(format!(
                    "a skill may hold at most {} MiB",
                    MAX_TOTAL_BYTES >> 20
                )));
            }
        }
    }
    Ok(())
}

/// Validates a skill folder: SKILL.md with a valid frontmatter whose `name` equals the folder name (unless
/// importing), every referenced path exists inside the folder, no symlinks, size limits.
pub fn validate(dir: &Path, origin: Origin) -> Result<Skill> {
    validate_named(dir, origin, true)
}

fn validate_named(dir: &Path, origin: Origin, check_dir_name: bool) -> Result<Skill> {
    let md = dir.join(SKILL_FILE);
    let meta = std::fs::symlink_metadata(&md)
        .map_err(|_| invalid(format!("{} has no {SKILL_FILE}", dir.display())))?;
    if !meta.is_file() {
        return Err(invalid(format!("{SKILL_FILE} must be a regular file")));
    }
    if meta.len() > MAX_SKILL_MD {
        return Err(invalid(format!(
            "{SKILL_FILE} is larger than {} KiB",
            MAX_SKILL_MD >> 10
        )));
    }
    let text = std::fs::read_to_string(&md)
        .map_err(|e| invalid(format!("{SKILL_FILE} is not UTF-8 text: {e}")))?;
    let (fm, body) = parse(&text)?;
    let name = fm.get("name").unwrap_or("").to_owned();
    if !valid_name(&name) {
        return Err(invalid(format!(
            "name {name:?} must be 1-{MAX_NAME} characters of a-z, 0-9 and single hyphens"
        )));
    }
    if check_dir_name {
        let dir_name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if dir_name != name {
            return Err(invalid(format!(
                "name {name:?} must equal the folder name {dir_name:?}"
            )));
        }
    }
    let description = fm.get("description").unwrap_or("").trim().to_owned();
    if description.is_empty() {
        return Err(invalid("description is required"));
    }
    if description.chars().count() > MAX_DESCRIPTION {
        return Err(invalid(format!(
            "description is longer than {MAX_DESCRIPTION} characters"
        )));
    }
    if body.trim().is_empty() {
        return Err(invalid("the SKILL.md body (instructions) is empty"));
    }
    let mut files = Vec::new();
    let mut total = 0;
    walk(dir, dir, &mut files, &mut total)?;
    files.retain(|f| f != SKILL_FILE);
    for r in references(&body) {
        let Some(rel) = inside(&r) else {
            return Err(invalid(format!(
                "referenced path {r:?} leaves the skill folder"
            )));
        };
        let target = dir.join(&rel);
        if !target.exists() {
            return Err(invalid(format!(
                "referenced path {r:?} does not exist in the skill folder"
            )));
        }
    }
    let tags = parse_tags(fm.get("tags").unwrap_or(""));
    for t in &tags {
        if !valid_name(t) {
            return Err(invalid(format!(
                "tag {t:?} must be 1-{MAX_NAME} characters of a-z, 0-9 and single hyphens"
            )));
        }
    }
    if tags.len() > MAX_TAGS {
        return Err(invalid(format!("at most {MAX_TAGS} tags")));
    }
    Ok(Skill {
        meta: SkillMeta {
            name,
            description,
            version: fm.get("version").map(str::to_owned),
            tags,
            origin,
            dir: dir.to_path_buf(),
        },
        body,
        files,
    })
}

/// A skill folder that failed validation (shown to the user, never to the agent).
#[derive(Debug, Clone, Serialize)]
pub struct Problem {
    pub dir: PathBuf,
    pub error: String,
}

/// The skills one agent can use in a run.
#[derive(Debug, Clone, Default)]
pub struct SkillSet {
    pub skills: Vec<Skill>,
    pub problems: Vec<Problem>,
}

impl SkillSet {
    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }

    pub fn get(&self, name: &str) -> Option<&Skill> {
        self.skills.iter().find(|s| s.meta.name == name)
    }

    pub fn names(&self) -> Vec<&str> {
        self.skills.iter().map(|s| s.meta.name.as_str()).collect()
    }

    /// Folders the agent may read (tier 3).
    pub fn dirs(&self) -> Vec<PathBuf> {
        self.skills.iter().map(|s| s.meta.dir.clone()).collect()
    }

    /// Tier-1 skills index for the work order (stable text: sorted by name, no timestamps).
    pub fn index(&self) -> String {
        if self.skills.is_empty() {
            return "(no skills are installed for you)".into();
        }
        let mut s = String::from(
            "Call load_skill with a skill's name to read its full instructions before doing work it covers.\n",
        );
        for k in &self.skills {
            let tag = if k.meta.origin == Origin::Overlay {
                " [updated]"
            } else {
                ""
            };
            s.push_str(&format!(
                "- {}{tag}: {}\n",
                k.meta.name,
                k.meta.description.replace('\n', " ")
            ));
        }
        s.trim_end().to_owned()
    }
}

fn scan(
    dir: &Path,
    origin: Origin,
    set: &mut BTreeMap<String, Skill>,
    problems: &mut Vec<Problem>,
) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || !e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        match validate(&p, origin) {
            Ok(mut s) => {
                if origin == Origin::Agent
                    && set
                        .get(&s.meta.name)
                        .is_some_and(|r| matches!(r.meta.origin, Origin::Role | Origin::Library))
                {
                    s.meta.origin = Origin::Overlay;
                }
                set.insert(s.meta.name.clone(), s);
            }
            Err(e) => problems.push(Problem {
                dir: p,
                error: e.to_string(),
            }),
        }
    }
}

/// Finds skills on disk.
#[derive(Debug, Clone)]
pub struct Registry {
    agents_dir: PathBuf,
    roles_dir: PathBuf,
}

/// Owner of a skill folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Owner {
    Agent(String),
    Role(String),
    /// The system's library, next to the agents folder (`<root>/skills`).
    Library,
}

impl Registry {
    pub fn new(agents_dir: impl Into<PathBuf>, roles_dir: impl Into<PathBuf>) -> Self {
        Self {
            agents_dir: agents_dir.into(),
            roles_dir: roles_dir.into(),
        }
    }

    pub fn dir_of(&self, owner: &Owner) -> PathBuf {
        match owner {
            Owner::Agent(a) => self.agents_dir.join(a).join("skills"),
            Owner::Role(r) => self.roles_dir.join(r).join("skills"),
            Owner::Library => self.agents_dir.with_file_name("skills"),
        }
    }

    /// Every skill of the system's library (what agents can enable).
    pub fn library(&self) -> SkillSet {
        let mut map = BTreeMap::new();
        let mut problems = Vec::new();
        scan(
            &self.dir_of(&Owner::Library),
            Origin::Library,
            &mut map,
            &mut problems,
        );
        SkillSet {
            skills: map.into_values().collect(),
            problems,
        }
    }

    /// Skills of an agent: the library skills it enables (`enabled`, the agent's skill binding), the role's
    /// skills, then the agent's own (an own skill with the name of a role or library skill is the agent's overlay
    /// of it; a role skill wins over a library skill of the same name).
    pub fn for_agent(&self, agent: &str, role: &str, enabled: &[String]) -> SkillSet {
        let mut map = BTreeMap::new();
        let mut problems = Vec::new();
        // A skill's folder is named after it, so only the enabled ones are read (not the whole library).
        let lib = self.dir_of(&Owner::Library);
        for name in enabled.iter().filter(|n| valid_name(n)) {
            let dir = lib.join(name);
            if !dir.is_dir() {
                continue;
            }
            match validate(&dir, Origin::Library) {
                Ok(s) => {
                    map.insert(s.meta.name.clone(), s);
                }
                Err(e) => problems.push(Problem {
                    dir,
                    error: e.to_string(),
                }),
            }
        }
        scan(
            &self.dir_of(&Owner::Role(role.into())),
            Origin::Role,
            &mut map,
            &mut problems,
        );
        scan(
            &self.dir_of(&Owner::Agent(agent.into())),
            Origin::Agent,
            &mut map,
            &mut problems,
        );
        SkillSet {
            skills: map.into_values().collect(),
            problems,
        }
    }

    /// Imports a skill from a folder or a `.zip` (the zip holds SKILL.md at its root or inside one top folder).
    /// The folder structure is kept. With `replace` an existing skill of that name is swapped atomically.
    pub fn import(&self, src: &Path, owner: &Owner, replace: bool) -> Result<SkillMeta> {
        let target_root = self.dir_of(owner);
        std::fs::create_dir_all(&target_root)?;
        let staging = target_root.join(format!(".import-{}", std::process::id()));
        if staging.exists() {
            std::fs::remove_dir_all(&staging)?;
        }
        std::fs::create_dir_all(&staging)?;
        let result = (|| {
            let is_zip = src.is_file()
                && src
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case("zip"));
            if is_zip {
                extract_zip(src, &staging)?;
            } else if src.is_dir() {
                copy_dir(src, &staging)?;
            } else {
                return Err(invalid(format!(
                    "{} is neither a folder nor a .zip file",
                    src.display()
                )));
            }
            let root = skill_root(&staging)?;
            let skill = validate_named(&root, Origin::Agent, false)?;
            let dest = target_root.join(&skill.meta.name);
            if dest.exists() {
                if !replace {
                    return Err(SkillError::Exists(skill.meta.name.clone()));
                }
                let old =
                    target_root.join(format!(".old-{}-{}", skill.meta.name, std::process::id()));
                std::fs::rename(&dest, &old)?;
                if let Err(e) = std::fs::rename(&root, &dest) {
                    let _ = std::fs::rename(&old, &dest);
                    return Err(e.into());
                }
                let _ = std::fs::remove_dir_all(&old);
            } else {
                std::fs::rename(&root, &dest)?;
            }
            let origin = match owner {
                Owner::Agent(_) => Origin::Agent,
                Owner::Role(_) => Origin::Role,
                Owner::Library => Origin::Library,
            };
            Ok(validate(&dest, origin)?.meta)
        })();
        let _ = std::fs::remove_dir_all(&staging);
        result
    }

    /// Writes a skill's SKILL.md from text (create or edit in place); its other files are kept. See
    /// [`Registry::write_with_files`].
    pub fn write(
        &self,
        owner: &Owner,
        name: &str,
        content: &str,
        create: bool,
    ) -> Result<SkillMeta> {
        self.write_with_files(owner, name, content, create, &[], &[])
    }

    /// Deletes a skill folder of an owner.
    pub fn remove(&self, owner: &Owner, name: &str) -> Result<()> {
        if !valid_name(name) {
            return Err(invalid(format!("invalid skill name {name:?}")));
        }
        let dir = self.dir_of(owner).join(name);
        if !dir.join(SKILL_FILE).exists() {
            return Err(SkillError::NotFound(name.into()));
        }
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }
}

/// SKILL.md at the staging root, or inside its single top folder.
fn skill_root(staging: &Path) -> Result<PathBuf> {
    if staging.join(SKILL_FILE).is_file() {
        // Move the content one level down so the folder can be renamed to the skill name.
        let inner = staging.join(".skill");
        std::fs::create_dir(&inner)?;
        for e in std::fs::read_dir(staging)? {
            let e = e?;
            if e.file_name() != ".skill" {
                std::fs::rename(e.path(), inner.join(e.file_name()))?;
            }
        }
        return Ok(inner);
    }
    let dirs: Vec<_> = std::fs::read_dir(staging)?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .collect();
    match dirs.as_slice() {
        [one] if one.path().join(SKILL_FILE).is_file() => Ok(one.path()),
        _ => Err(invalid(format!(
            "no {SKILL_FILE} at the root or in a single top folder"
        ))),
    }
}

fn copy_dir(src: &Path, dst: &Path) -> Result<()> {
    let mut files = Vec::new();
    let mut total = 0;
    walk(src, src, &mut files, &mut total)?; // rejects symlinks and enforces limits before copying
    for f in files {
        let from = src.join(&f);
        let to = dst.join(&f);
        if let Some(p) = to.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::copy(from, to)?;
    }
    Ok(())
}

/// An entry's name for messages (an undecodable name is shown as `?`).
fn entry_name<R: std::io::Read>(e: &zip::read::ZipFile<'_, R>) -> String {
    e.name()
        .map(|n| n.into_owned())
        .unwrap_or_else(|_| "?".into())
}

fn extract_zip(src: &Path, dst: &Path) -> Result<()> {
    let f = std::fs::File::open(src)?;
    let mut z = zip::ZipArchive::new(f).map_err(|e| SkillError::Zip(e.to_string()))?;
    if z.len() > MAX_FILES * 2 {
        return Err(invalid(format!(
            "the archive holds more than {} entries",
            MAX_FILES * 2
        )));
    }
    let mut total = 0u64;
    for i in 0..z.len() {
        let mut e = z.by_index(i).map_err(|e| SkillError::Zip(e.to_string()))?;
        if e.is_symlink() {
            return Err(invalid(format!(
                "symbolic links are not allowed: {}",
                entry_name(&e)
            )));
        }
        let Some(rel) = e.enclosed_name() else {
            return Err(invalid(format!(
                "archive entry {:?} leaves the folder",
                entry_name(&e)
            )));
        };
        let out = dst.join(rel);
        if e.is_dir() {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(p) = out.parent() {
            std::fs::create_dir_all(p)?;
        }
        let mut w = std::fs::File::create(&out)?;
        // Limit what is actually written, not what the header claims (zip bombs).
        let left = MAX_TOTAL_BYTES - total;
        let n = std::io::copy(&mut (&mut e).take(left + 1), &mut w)?;
        total += n;
        if total > MAX_TOTAL_BYTES {
            return Err(invalid(format!(
                "the archive unpacks to more than {} MiB",
                MAX_TOTAL_BYTES >> 20
            )));
        }
    }
    Ok(())
}

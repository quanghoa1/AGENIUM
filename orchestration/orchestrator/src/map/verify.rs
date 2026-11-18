//! How Agenium checks what an agent puts on the map (ADR-166), before it is kept and again after later changes:
//! a file must exist; a symbol must be declared in its file (the outline `find_code` reads code with); a link in code
//! must cite lines of the source's own declaration that name its target; anything else must be named by the page it
//! cites (fetched by Agenium itself) or by the code it cites.

use std::path::Path;

use serde_json::{json, Value};

use super::refs::Ref;

/// Lines one piece of code evidence may span.
pub const MAX_LINES: u32 = 30;

#[derive(Debug, Clone, PartialEq)]
pub enum Evidence {
    Code { file: String, start: u32, end: u32 },
    Web { url: String },
}

impl Evidence {
    /// `{"file": "src/a.py", "line": 12}` (or `"lines": [12, 18]`), or `{"url": "https://…"}`.
    pub fn parse(v: &Value) -> Result<Evidence, String> {
        if let Some(u) = v["url"].as_str() {
            let u = u.trim();
            if !(u.starts_with("https://") || u.starts_with("http://")) {
                return Err(format!("{u:?} is not an http(s) address"));
            }
            return Ok(Evidence::Web { url: u.to_owned() });
        }
        let file = v["file"]
            .as_str()
            .map(|f| {
                f.trim()
                    .replace('\\', "/")
                    .trim_start_matches("./")
                    .to_owned()
            })
            .filter(|f| !f.is_empty())
            .ok_or("evidence is {\"file\": …, \"line\": …} for code or {\"url\": …} for a page")?;
        let (start, end) = match (v["line"].as_u64(), v["lines"].as_array()) {
            (Some(l), _) => (l, l),
            (None, Some(a)) if a.len() == 2 => {
                (a[0].as_u64().unwrap_or(0), a[1].as_u64().unwrap_or(0))
            }
            _ => {
                return Err(format!(
                    "the evidence in {file} needs \"line\": n or \"lines\": [from, to]"
                ))
            }
        };
        if start == 0 || end < start || end - start >= MAX_LINES as u64 {
            return Err(format!(
                "lines {start}-{end} of {file}: cite the lines that show it (1-based, at most {MAX_LINES})"
            ));
        }
        Ok(Evidence::Code {
            file,
            start: start as u32,
            end: end as u32,
        })
    }

    pub fn json(&self) -> Value {
        match self {
            Evidence::Code { file, start, end } if start == end => {
                json!({ "file": file, "line": start })
            }
            Evidence::Code { file, start, end } => json!({ "file": file, "lines": [start, end] }),
            Evidence::Web { url } => json!({ "url": url }),
        }
    }

    pub fn text(&self) -> String {
        match self {
            Evidence::Code { file, start, end } if start == end => format!("{file}:{start}"),
            Evidence::Code { file, start, end } => format!("{file}:{start}-{end}"),
            Evidence::Web { url } => url.clone(),
        }
    }
}

fn read(root: &Path, file: &str) -> Result<String, String> {
    let p = root.join(file);
    if !agenium_tools::paths::confined(&p, root) {
        return Err(format!("{file} is outside the project"));
    }
    std::fs::read_to_string(&p).map_err(|_| format!("{file} is not a text file of the project"))
}

/// Where the code node `r` is: a file's whole length, a symbol's declaration (by the outline of its file).
pub fn locate(root: &Path, r: &Ref) -> Result<(u32, u32), String> {
    let file = r.file.as_deref().unwrap_or_default();
    let text = read(root, file)?;
    let n = text.lines().count().max(1) as u32;
    let Some(sym) = &r.symbol else {
        return Ok((1, n));
    };
    let ext = file.rsplit('.').next().unwrap_or_default();
    let units = crate::judge::find_code::outline::units(ext, &text);
    let short = r.term();
    // A container (a class, an impl) is replaced by its members, `Container::member`: it spans them.
    let hits: Vec<_> = units
        .iter()
        .filter(|u| !u.name.starts_with("lines "))
        .filter(|u| {
            let n = u.name.as_str();
            n == sym
                || n.rsplit("::").next() == Some(sym.as_str())
                || n.rsplit("::").next() == Some(short.as_str()) && sym.ends_with(&short)
                || n.starts_with(&format!("{sym}::"))
        })
        .collect();
    let (Some(lo), Some(hi)) = (
        hits.iter().map(|u| u.start).min(),
        hits.iter().map(|u| u.end).max(),
    ) else {
        let some: Vec<_> = units
            .iter()
            .filter(|u| !u.name.starts_with("lines "))
            .take(12)
            .map(|u| u.name.as_str())
            .collect();
        return Err(format!(
            "{file} declares no {sym:?}{}",
            if some.is_empty() {
                String::new()
            } else {
                format!(" (it declares {})", some.join(", "))
            }
        ));
    };
    Ok((lo as u32, hi as u32))
}

/// Whether `text` names `term`: in code as a whole identifier (not part of a longer one), elsewhere anywhere, case
/// and spacing aside.
pub fn names(text: &str, term: &str, code_ident: bool) -> bool {
    let norm = |s: &str| {
        s.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    };
    let (t, w) = (norm(text), norm(term));
    if w.is_empty() {
        return false;
    }
    if !code_ident {
        return t.contains(&w);
    }
    let ident = |c: char| c.is_alphanumeric() || c == '_';
    t.match_indices(&w).any(|(i, m)| {
        !t[..i].chars().next_back().is_some_and(ident)
            && !t[i + m.len()..].chars().next().is_some_and(ident)
    })
}

/// The cited lines of code evidence.
pub fn lines(root: &Path, file: &str, start: u32, end: u32) -> Result<String, String> {
    let text = read(root, file)?;
    let all: Vec<&str> = text.lines().collect();
    if end as usize > all.len() {
        return Err(format!("{file} has {} lines, not {end}", all.len()));
    }
    Ok(all[start as usize - 1..end as usize].join("\n"))
}

/// Checks code evidence of the link `src → dst`: the lines are in the source's own file (inside a symbol's
/// declaration) and name the target. Where the source is not code (a product that calls a domain), the lines only
/// have to name the target.
pub fn code_link(root: &Path, ev: &Evidence, src: &Ref, dst: &Ref) -> Result<(), String> {
    let Evidence::Code { file, start, end } = ev else {
        return Err("not code evidence".into());
    };
    if src.is_code() {
        let own = src.file.as_deref().unwrap_or_default();
        if own != file {
            return Err(format!(
                "the evidence is in {file}, but {} is in {own}: cite the lines of {own} that show it",
                src.key()
            ));
        }
        let (lo, hi) = locate(root, src)?;
        if *start < lo || *end > hi {
            return Err(format!(
                "lines {start}-{end} are outside {} (lines {lo}-{hi})",
                src.key()
            ));
        }
    }
    let text = lines(root, file, *start, *end)?;
    let term = dst.term();
    if !names(&text, &term, dst.is_code()) {
        return Err(format!(
            "{} does not name {term:?}: cite the line where it is used",
            ev.text()
        ));
    }
    Ok(())
}

/// Where code evidence holds again after the code moved: the same lines, else the first place in the source's
/// declaration (or file) whose line names the target. `None`: it does not hold any more.
pub fn moved(root: &Path, ev: &Evidence, src: &Ref, dst: &Ref) -> Option<Evidence> {
    if code_link(root, ev, src, dst).is_ok() {
        return Some(ev.clone());
    }
    let Evidence::Code { file, .. } = ev else {
        return None;
    };
    let (lo, hi) = if src.is_code() {
        locate(root, src).ok()?
    } else {
        (1, read(root, file).ok()?.lines().count() as u32)
    };
    let text = read(root, file).ok()?;
    let term = dst.term();
    text.lines()
        .enumerate()
        .skip(lo as usize - 1)
        .take((hi - lo + 1) as usize)
        .find(|(_, l)| names(l, &term, dst.is_code()))
        .map(|(i, _)| Evidence::Code {
            file: file.clone(),
            start: i as u32 + 1,
            end: i as u32 + 1,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_evidence_must_be_in_the_source_and_name_the_target() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("src")).unwrap();
        std::fs::write(
            d.path().join("src/app.py"),
            "import config\n\ndef main():\n    cfg = config.load_config()\n    run(cfg)\n\ndef run(c):\n    print(c)\n",
        )
        .unwrap();
        std::fs::write(
            d.path().join("src/config.py"),
            "def load_config():\n    return {}\n",
        )
        .unwrap();
        let r = |s: &str| Ref::parse(s).unwrap();
        let main = r("symbol:src/app.py#main");
        assert_eq!(locate(d.path(), &main).unwrap(), (3, 5));
        assert!(locate(d.path(), &r("symbol:src/app.py#nope"))
            .unwrap_err()
            .contains("declares main, run"));
        let load = r("symbol:src/config.py#load_config");
        let at = |l: u32| Evidence::Code {
            file: "src/app.py".into(),
            start: l,
            end: l,
        };
        assert!(code_link(d.path(), &at(4), &main, &load).is_ok());
        assert!(code_link(d.path(), &at(5), &main, &load)
            .unwrap_err()
            .contains("does not name"));
        assert!(code_link(d.path(), &at(8), &main, &load)
            .unwrap_err()
            .contains("outside"));
        let other = Evidence::Code {
            file: "src/config.py".into(),
            start: 1,
            end: 1,
        };
        assert!(code_link(d.path(), &other, &main, &load)
            .unwrap_err()
            .contains("is in src/app.py"));
        // A whole identifier only: `run` is not named by `runner`.
        assert!(!names("runner(x)", "run", true));
        assert!(names("x = run(cfg)", "run", true));
        assert!(names(
            "Calls  API.Anthropic.com/v1",
            "api.anthropic.com",
            false
        ));
        // Code moved down: the link follows inside its declaration.
        std::fs::write(
            d.path().join("src/app.py"),
            "import config\n\n\n\ndef main():\n    # read it\n    cfg = config.load_config()\n    run(cfg)\n",
        )
        .unwrap();
        assert_eq!(moved(d.path(), &at(4), &main, &load), Some(at(7)));
        std::fs::write(d.path().join("src/app.py"), "def main():\n    pass\n").unwrap();
        assert_eq!(moved(d.path(), &at(4), &main, &load), None);
        assert!(Evidence::parse(&json!({"file": "a.py", "lines": [1, 40]})).is_err());
        assert!(Evidence::parse(&json!({"url": "ftp://x"})).is_err());
    }
}

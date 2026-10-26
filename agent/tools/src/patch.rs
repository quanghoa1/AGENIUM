//! `apply_patch`: multi-file edits in the envelope format popularised by OpenAI Codex (independent
//! implementation from the format description). All files are computed in memory first; nothing is written unless
//! every hunk applies (atomic, decision 16).
//!
//! ```text
//! *** Begin Patch
//! *** Add File: path/new.txt
//! +content line
//! *** Update File: path/old.rs
//! *** Move to: path/renamed.rs
//! @@ fn context_anchor
//!  unchanged line
//! -removed line
//! +added line
//! *** Delete File: path/gone.txt
//! *** End Patch
//! ```

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileOp {
    Add {
        path: String,
        content: String,
    },
    Delete {
        path: String,
    },
    Update {
        path: String,
        move_to: Option<String>,
        hunks: Vec<Hunk>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Hunk {
    pub anchor: Option<String>,
    /// (' ' | '-' | '+', text)
    pub lines: Vec<(char, String)>,
    pub at_eof: bool,
}

pub fn parse(patch: &str) -> Result<Vec<FileOp>, String> {
    let lines: Vec<&str> = patch
        .lines()
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect();
    let mut i = lines
        .iter()
        .position(|l| l.trim() == "*** Begin Patch")
        .ok_or("patch must start with '*** Begin Patch'")?
        + 1;
    let mut ops = Vec::new();
    loop {
        let l = *lines.get(i).ok_or("missing '*** End Patch'")?;
        if l.trim() == "*** End Patch" {
            break;
        }
        if let Some(p) = l.strip_prefix("*** Add File: ") {
            i += 1;
            let mut content = String::new();
            while let Some(line) = lines.get(i).filter(|x| !x.starts_with("*** ")) {
                let body = line
                    .strip_prefix('+')
                    .ok_or_else(|| format!("line {}: Add File lines must start with '+'", i + 1))?;
                content.push_str(body);
                content.push('\n');
                i += 1;
            }
            ops.push(FileOp::Add {
                path: p.trim().to_owned(),
                content,
            });
        } else if let Some(p) = l.strip_prefix("*** Delete File: ") {
            ops.push(FileOp::Delete {
                path: p.trim().to_owned(),
            });
            i += 1;
        } else if let Some(p) = l.strip_prefix("*** Update File: ") {
            i += 1;
            let mut move_to = None;
            if let Some(m) = lines.get(i).and_then(|x| x.strip_prefix("*** Move to: ")) {
                move_to = Some(m.trim().to_owned());
                i += 1;
            }
            let mut hunks: Vec<Hunk> = Vec::new();
            let mut cur: Option<Hunk> = None;
            while let Some(line) = lines.get(i) {
                if line.trim() == "*** End of File" {
                    if let Some(h) = cur.as_mut() {
                        h.at_eof = true;
                    }
                    i += 1;
                    continue;
                }
                if line.starts_with("*** ") {
                    break;
                }
                if let Some(a) = line.strip_prefix("@@") {
                    if let Some(h) = cur.take() {
                        hunks.push(h);
                    }
                    let a = a.trim();
                    cur = Some(Hunk {
                        anchor: (!a.is_empty()).then(|| a.to_owned()),
                        ..Default::default()
                    });
                } else {
                    let (k, rest) = match line.chars().next() {
                        Some(c @ (' ' | '-' | '+')) => (c, &line[1..]),
                        // An empty line inside a hunk is an empty context line.
                        None => (' ', ""),
                        Some(_) => {
                            return Err(format!(
                                "line {}: hunk lines must start with ' ', '-' or '+': {line:?}",
                                i + 1
                            ))
                        }
                    };
                    cur.get_or_insert_with(Hunk::default)
                        .lines
                        .push((k, rest.to_owned()));
                }
                i += 1;
            }
            if let Some(h) = cur.take() {
                hunks.push(h);
            }
            if hunks.is_empty() && move_to.is_none() {
                return Err(format!("Update File {p} has no hunks"));
            }
            ops.push(FileOp::Update {
                path: p.trim().to_owned(),
                move_to,
                hunks,
            });
        } else if l.trim().is_empty() {
            i += 1;
        } else {
            return Err(format!(
                "line {}: expected a '*** Add/Update/Delete File:' header, got {l:?}",
                i + 1
            ));
        }
    }
    if ops.is_empty() {
        return Err("patch contains no file operations".into());
    }
    Ok(ops)
}

fn find(hay: &[String], needle: &[&str], from: usize, norm: fn(&str) -> &str) -> Option<usize> {
    if needle.is_empty() {
        return Some(from.min(hay.len()));
    }
    // More lines to match than the file has (an empty file has none): no match, not an index past the end.
    if needle.len() > hay.len() {
        return None;
    }
    (from..=hay.len() - needle.len()).find(|&s| {
        needle
            .iter()
            .enumerate()
            .all(|(k, n)| norm(&hay[s + k]) == norm(n))
    })
}

/// Applies the hunks of one file to its text. Matching is exact first, then ignoring trailing whitespace, then
/// ignoring surrounding whitespace.
pub fn apply_hunks(text: &str, hunks: &[Hunk], path: &str) -> Result<String, String> {
    let crlf = text.contains("\r\n");
    let trailing_nl = text.ends_with('\n') || text.is_empty();
    let body = text.strip_suffix('\n').unwrap_or(text);
    let mut lines: Vec<String> = if text.is_empty() {
        vec![]
    } else {
        body.split('\n')
            .map(|l| l.strip_suffix('\r').unwrap_or(l).to_owned())
            .collect()
    };
    let mut cursor = 0usize;
    for (n, h) in hunks.iter().enumerate() {
        if let Some(a) = &h.anchor {
            if let Some(pos) = lines
                .iter()
                .skip(cursor)
                .position(|l| l.contains(a.as_str()) || l.trim() == a.trim())
            {
                cursor += pos;
            }
        }
        let old: Vec<&str> = h
            .lines
            .iter()
            .filter(|(k, _)| *k != '+')
            .map(|(_, t)| t.as_str())
            .collect();
        let new: Vec<String> = h
            .lines
            .iter()
            .filter(|(k, _)| *k != '-')
            .map(|(_, t)| t.clone())
            .collect();
        let at = if h.at_eof && !old.is_empty() {
            let start = lines.len().saturating_sub(old.len());
            find(&lines, &old, start, |s| s).or_else(|| find(&lines, &old, start, str::trim_end))
        } else if old.is_empty() {
            Some(if h.at_eof || h.anchor.is_none() {
                lines.len()
            } else {
                (cursor + 1).min(lines.len())
            })
        } else {
            find(&lines, &old, cursor, |s| s)
                .or_else(|| find(&lines, &old, cursor, str::trim_end))
                .or_else(|| find(&lines, &old, cursor, str::trim))
                // A hunk may legitimately precede the anchor position of a previous hunk only if written out of
                // order; search from the start as a last resort.
                .or_else(|| find(&lines, &old, 0, str::trim_end))
        };
        let at = at.ok_or_else(|| {
            let preview: Vec<&str> = old.iter().take(3).copied().collect();
            format!("hunk {} of {path}: the context/removed lines were not found (first lines: {preview:?}); read the file and retry", n + 1)
        })?;
        lines.splice(at..at + old.len(), new.iter().cloned());
        cursor = at + new.len();
    }
    let sep = if crlf { "\r\n" } else { "\n" };
    let mut out = lines.join(sep);
    if trailing_nl && !lines.is_empty() {
        out.push_str(sep);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const P: &str = "*** Begin Patch
*** Add File: docs/new.md
+# Title
+text
*** Update File: src/lib.rs
@@ fn add
 fn add(a: i32, b: i32) -> i32 {
-    a - b
+    a + b
 }
*** Delete File: old.txt
*** End Patch";

    #[test]
    fn parses_all_operations() {
        let ops = parse(P).unwrap();
        assert_eq!(ops.len(), 3);
        assert_eq!(
            ops[0],
            FileOp::Add {
                path: "docs/new.md".into(),
                content: "# Title\ntext\n".into()
            }
        );
        match &ops[1] {
            FileOp::Update {
                path,
                move_to,
                hunks,
            } => {
                assert_eq!(
                    (path.as_str(), move_to, hunks.len()),
                    ("src/lib.rs", &None, 1)
                );
                assert_eq!(hunks[0].anchor.as_deref(), Some("fn add"));
            }
            o => panic!("{o:?}"),
        }
        assert!(parse("no header").is_err());
        assert!(parse("*** Begin Patch\n*** Update File: x\n*** End Patch").is_err());
        assert!(parse("*** Begin Patch\n*** Add File: x\nmissing plus\n*** End Patch").is_err());
    }

    #[test]
    fn applies_hunks_with_whitespace_tolerance_and_crlf() {
        let src = "fn sub(a: i32, b: i32) -> i32 {\r\n    a - b\r\n}\r\n\r\nfn add(a: i32, b: i32) -> i32 {\r\n    a - b   \r\n}\r\n";
        let FileOp::Update { hunks, .. } = &parse(P).unwrap()[1] else {
            unreachable!()
        };
        let out = apply_hunks(src, hunks, "src/lib.rs").unwrap();
        assert_eq!(out, "fn sub(a: i32, b: i32) -> i32 {\r\n    a - b\r\n}\r\n\r\nfn add(a: i32, b: i32) -> i32 {\r\n    a + b\r\n}\r\n", "anchor picks the second fn, CRLF kept");
        let bad = vec![Hunk {
            anchor: None,
            lines: vec![('-', "nope".into())],
            at_eof: false,
        }];
        assert!(apply_hunks(src, &bad, "x")
            .unwrap_err()
            .contains("not found"));
        let append = vec![Hunk {
            anchor: None,
            lines: vec![('+', "tail".into())],
            at_eof: true,
        }];
        assert_eq!(apply_hunks("a\n", &append, "x").unwrap(), "a\ntail\n");
        assert_eq!(apply_hunks("", &append, "x").unwrap(), "tail\n");
    }

    /// Review of 2026-10-27: a hunk with more old lines than the file indexed past its end, and an empty file
    /// panicked; both are a hunk that does not apply.
    #[test]
    fn a_hunk_longer_than_the_file_does_not_apply() {
        let hunk = |lines: &[(char, &str)], at_eof: bool| {
            vec![Hunk {
                anchor: None,
                lines: lines.iter().map(|(k, t)| (*k, (*t).to_owned())).collect(),
                at_eof,
            }]
        };
        let long = hunk(&[(' ', "a"), ('-', "b"), ('-', "c"), ('+', "d")], false);
        assert!(apply_hunks("a\n", &long, "x")
            .unwrap_err()
            .contains("not found"));
        assert!(apply_hunks("", &long, "x")
            .unwrap_err()
            .contains("not found"));
        let eof = hunk(&[('-', "a"), ('-', "b")], true);
        assert!(apply_hunks("", &eof, "x").is_err());
        assert!(apply_hunks("b\n", &eof, "x").is_err());
    }
}

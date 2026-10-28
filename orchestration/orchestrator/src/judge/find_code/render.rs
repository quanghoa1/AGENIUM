//! What `find_code` returns: the files in order of relevance first, then the selected units verbatim with line numbers
//! (as `read_file` shows them), then what the search could not cover. Evidence for the agent, not an answer.

/// Characters of excerpts returned at most.
const MAX_EXCERPTS: usize = 14_000;

/// A file the judge rated relevant.
pub(super) struct FileHit {
    pub path: String,
    pub p: f64,
}

/// A unit selected verbatim.
pub(super) struct UnitHit {
    pub path: String,
    pub name: String,
    pub start: usize,
    pub end: usize,
    pub p: f64,
    pub source: String,
}

pub(super) struct Found<'a> {
    pub question: &'a str,
    pub root: &'a str,
    pub files: Vec<FileHit>,
    pub units: Vec<UnitHit>,
    pub searched_files: usize,
    pub asked_files: usize,
    pub notes: Vec<String>,
}

fn pct(p: f64) -> String {
    format!("{:.0}%", p * 100.0)
}

pub(super) fn render(f: &Found<'_>) -> String {
    let mut s = format!("find_code: {:?} (in {})\n", f.question, f.root);
    if f.files.is_empty() {
        s.push_str(
            "No file was rated relevant. Ask again with other words, search a narrower folder, or use grep for an \
             exact name.\n",
        );
    } else {
        s.push_str("Relevant files, most relevant first:\n");
        for (i, h) in f.files.iter().enumerate() {
            let shown = f.units.iter().filter(|u| u.path == h.path).count();
            let excerpt = if shown > 0 {
                format!(", {shown} excerpt(s) below")
            } else {
                String::new()
            };
            s.push_str(&format!(
                "{:>3}. {} ({}{excerpt})\n",
                i + 1,
                h.path,
                pct(h.p)
            ));
        }
    }
    let mut used = 0;
    let mut left = 0;
    for u in &f.units {
        let block = format!(
            "\n--- {}:{}-{} {} ({})\n{}",
            u.path,
            u.start,
            u.end,
            u.name,
            pct(u.p),
            u.source
                .lines()
                .enumerate()
                .map(|(i, l)| format!("{:>6}\t{l}\n", u.start + i))
                .collect::<String>()
        );
        if used + block.len() > MAX_EXCERPTS && used > 0 {
            left += 1;
            continue;
        }
        used += block.len();
        s.push_str(&block);
    }
    if left > 0 {
        s.push_str(&format!(
            "\n[{left} more relevant unit(s) not shown: read them with read_file from the file list]\n"
        ));
    }
    s.push_str(&format!(
        "\n[searched {} files, {} read by the judge",
        f.searched_files, f.asked_files
    ));
    for n in &f.notes {
        s.push_str("; ");
        s.push_str(n);
    }
    s.push_str(
        ". Relevance is an estimate, not a guarantee that nothing else matters: read more with read_file; for an \
         exact name use grep.]\n",
    );
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_first_then_numbered_excerpts_within_budget() {
        let f = Found {
            question: "where is auth checked?",
            root: ".",
            files: vec![
                FileHit {
                    path: "src/auth.rs".into(),
                    p: 0.93,
                },
                FileHit {
                    path: "src/main.rs".into(),
                    p: 0.61,
                },
            ],
            units: vec![UnitHit {
                path: "src/auth.rs".into(),
                name: "check".into(),
                start: 10,
                end: 11,
                p: 0.9,
                source: "fn check() {\n}\n".into(),
            }],
            searched_files: 40,
            asked_files: 12,
            notes: vec![],
        };
        let s = render(&f);
        assert!(s.contains("  1. src/auth.rs (93%, 1 excerpt(s) below)\n  2. src/main.rs (61%)"));
        assert!(s.contains("--- src/auth.rs:10-11 check (90%)\n    10\tfn check() {\n    11\t}\n"));
        assert!(s.contains("[searched 40 files, 12 read by the judge."));
    }
}

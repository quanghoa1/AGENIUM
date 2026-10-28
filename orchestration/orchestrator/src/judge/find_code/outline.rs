//! The declarations of a source file — functions, methods, types — with their line ranges, so the judge reads code in
//! meaningful units and the agent gets whole units back. Line patterns per language (no parser): a declaration runs
//! to the line before the next one at the same or a lower indentation. A container (an `impl`, a class) that holds
//! declarations is replaced by them, named `Container::member`. Text in another language, or without declarations,
//! falls back to fixed windows.

use std::sync::OnceLock;

use regex::Regex;

/// Lines per fallback window.
const WINDOW: usize = 80;
/// A unit longer than this is cut into windows of it.
const MAX_UNIT: usize = 150;

/// One unit of source: 1-based inclusive lines.
#[derive(Debug, Clone, PartialEq)]
pub struct Unit {
    pub name: String,
    pub start: usize,
    pub end: usize,
}

fn patterns(ext: &str) -> Option<&'static Regex> {
    static RUST: OnceLock<Regex> = OnceLock::new();
    static PY: OnceLock<Regex> = OnceLock::new();
    static JS: OnceLock<Regex> = OnceLock::new();
    static GO: OnceLock<Regex> = OnceLock::new();
    static C_LIKE: OnceLock<Regex> = OnceLock::new();
    let re = |s: &str| Regex::new(s).expect("valid pattern");
    Some(match ext {
        "rs" => RUST.get_or_init(|| {
            re(r"^(\s*)(?:pub(?:\([^)]*\))?\s+)?(?:(?:async|const|unsafe|extern\s+\S+)\s+)*(?:fn|struct|enum|trait|union|type|mod|macro_rules!|impl(?:<[^>]*>)?)\s+([A-Za-z_][A-Za-z0-9_]*(?:<[^{]*?>)?(?:\s+for\s+[A-Za-z_][A-Za-z0-9_:<>]*)?)")
        }),
        "py" => PY.get_or_init(|| re(r"^(\s*)(?:async\s+)?(?:def|class)\s+([A-Za-z_][A-Za-z0-9_]*)")),
        "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs" => JS.get_or_init(|| {
            re(r"^(\s*)(?:export\s+)?(?:default\s+)?(?:declare\s+)?(?:abstract\s+)?(?:async\s+)?(?:function\*?\s+([A-Za-z_$][\w$]*)|class\s+([A-Za-z_$][\w$]*)|interface\s+([A-Za-z_$][\w$]*)|type\s+([A-Za-z_$][\w$]*)\s*=|(?:const|let)\s+([A-Za-z_$][\w$]*)\s*=\s*(?:async\s*)?(?:\(|function|[A-Za-z_$][\w$]*\s*=>)|(?:public\s+|private\s+|protected\s+|static\s+|readonly\s+)*(?:async\s+)?([A-Za-z_$][\w$]*)\s*\([^)]*\)\s*(?::\s*[^={]+)?\{)")
        }),
        "go" => GO.get_or_init(|| re(r"^()(?:func\s+(?:\([^)]*\)\s*)?([A-Za-z_]\w*)|type\s+([A-Za-z_]\w*))")),
        "java" | "kt" | "cs" | "swift" | "scala" | "c" | "cc" | "cpp" | "h" | "hpp" => C_LIKE.get_or_init(|| {
            re(r"^(\s*)(?:(?:public|private|protected|internal|static|final|abstract|override|virtual|inline|async|open|fun|func|def|class|struct|interface|enum|void|[A-Za-z_][\w<>,\[\]\s*&:]*)\s+)+([A-Za-z_]\w*)\s*\([^;]*$|^(\s*)(?:public\s+|private\s+)?(?:class|struct|interface|enum)\s+([A-Za-z_]\w*)")
        }),
        _ => return None,
    })
}

/// Words that a matched name may not be in JavaScript and the C family (control flow caught by the method pattern).
const NOT_NAMES: &[&str] = &[
    "if", "for", "while", "switch", "catch", "return", "match", "else", "new", "function",
];

/// A line of documentation or an attribute/decorator directly above a declaration.
fn doc_line(l: &str) -> bool {
    let t = l.trim_start();
    !t.is_empty()
        && ["///", "//", "/**", "* ", "*/", "#[", "@", "# ", "\"\"\""]
            .iter()
            .any(|p| t.starts_with(p))
        || t == "*"
}

fn windows(lines: usize) -> Vec<Unit> {
    (0..lines.div_ceil(WINDOW))
        .map(|i| Unit {
            name: format!("lines {}-{}", i * WINDOW + 1, ((i + 1) * WINDOW).min(lines)),
            start: i * WINDOW + 1,
            end: ((i + 1) * WINDOW).min(lines),
        })
        .collect()
}

/// The units of `text` (a file named with extension `ext`).
pub fn units(ext: &str, text: &str) -> Vec<Unit> {
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return vec![];
    }
    let Some(re) = patterns(ext) else {
        return windows(lines.len());
    };
    // (indent, name, start line index)
    let mut found: Vec<(usize, String, usize)> = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        let Some(c) = re.captures(l) else { continue };
        let indent = c
            .get(1)
            .or_else(|| c.get(3))
            .map_or(0, |m| m.as_str().replace('\t', "    ").len());
        let Some(name) = (2..c.len())
            .filter_map(|k| c.get(k))
            .map(|m| m.as_str().trim().to_owned())
            .find(|n| !n.is_empty() && n.chars().any(|ch| ch.is_alphanumeric()))
        else {
            continue;
        };
        let c_like = !matches!(ext, "rs" | "py" | "go");
        if c_like && NOT_NAMES.contains(&name.as_str()) {
            continue;
        }
        found.push((indent, name, i));
    }
    if found.is_empty() {
        return windows(lines.len());
    }
    // Each declaration ends before the next one at the same or a lower indentation (trailing blanks dropped).
    let mut spans: Vec<(usize, String, usize, usize)> = Vec::new();
    for (k, (indent, name, start)) in found.iter().enumerate() {
        // It ends before the next declaration at the same or a lower indentation, or before the first line that is
        // indented less than it (the closing brace of its container).
        let next = found[k + 1..]
            .iter()
            .find(|(i2, _, _)| i2 <= indent)
            .map_or(lines.len(), |(_, _, s)| *s);
        let dedent = (start + 1..next)
            .find(|j| {
                let l = lines[*j];
                let lead = &l[..l.len() - l.trim_start().len()];
                !l.trim().is_empty() && lead.replace('\t', "    ").len() < *indent
            })
            .unwrap_or(next);
        let mut end = next.min(dedent);
        while end > start + 1 && lines[end - 1].trim().is_empty() {
            end -= 1;
        }
        spans.push((*indent, name.clone(), *start, end));
    }
    // Containers (an impl, a class, a module) holding declarations give way to them, which take the container's name as
    // a prefix. A function holding functions stays whole: its inner helpers are part of it (a first version returned
    // `assemble::flush` instead of `assemble`).
    let callable = |start: usize| {
        let l = lines[start];
        let kind = |w: &str| {
            l.split(|c: char| !c.is_alphanumeric() && c != '_')
                .any(|t| t == w)
        };
        (kind("fn") || kind("def") || kind("function") || kind("func"))
            && !(kind("impl") || kind("class") || kind("trait") || kind("mod") || kind("interface"))
    };
    let inside =
        |s: usize, e: usize, (_, _, s2, e2): &(usize, String, usize, usize)| *s2 < s && *e2 >= e;
    let mut out = Vec::new();
    for (k, (_, name, start, end)) in spans.iter().enumerate() {
        let inner = spans
            .iter()
            .enumerate()
            .any(|(j, (_, _, s, e))| j != k && s > start && e <= end);
        if (inner && !callable(*start))
            || spans
                .iter()
                .any(|o| inside(*start, *end, o) && callable(o.2))
        {
            continue;
        }
        let parent = spans
            .iter()
            .filter(|o| inside(*start, *end, o))
            .max_by_key(|(_, _, s, _)| *s)
            .map(|(_, n, _, _)| n.split_whitespace().last().unwrap_or(n).to_owned());
        let name = match parent {
            Some(p) => format!("{p}::{name}"),
            None => name.clone(),
        };
        // The doc comment and attributes right above belong to the unit: they say what it is for.
        let mut first = *start;
        while first > 0 && doc_line(lines[first - 1]) {
            first -= 1;
        }
        let (s, e) = (first + 1, *end);
        if e - s + 1 > MAX_UNIT {
            let mut a = s;
            while a <= e {
                let b = (a + MAX_UNIT - 1).min(e);
                out.push(Unit {
                    name: format!("{name} (lines {a}-{b})"),
                    start: a,
                    end: b,
                });
                a = b + 1;
            }
        } else {
            out.push(Unit {
                name,
                start: s,
                end: e,
            });
        }
    }
    // Lines before the first declaration (imports, module docs) are not a unit; text between units is left out.
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(u: &[Unit]) -> Vec<(String, usize, usize)> {
        u.iter().map(|x| (x.name.clone(), x.start, x.end)).collect()
    }

    #[test]
    fn rust_methods_take_their_impl_as_a_prefix() {
        let src = "use x;\n\npub struct App {\n    a: u8,\n}\n\nimpl App {\n    pub fn new() -> Self {\n        App { a: 0 }\n    }\n\n    pub(crate) async fn run(&self) {\n        todo!()\n    }\n}\n\nfn helper() {}\n";
        assert_eq!(
            names(&units("rs", src)),
            [
                ("App".to_owned(), 3, 5),
                ("App::new".to_owned(), 8, 10),
                ("App::run".to_owned(), 12, 14),
                ("helper".to_owned(), 17, 17),
            ]
        );
    }

    #[test]
    fn a_function_with_an_inner_function_stays_whole() {
        let src = "fn assemble() -> String {
    fn flush(x: &mut u8) {
        *x = 0;
    }
    let mut a = 1;
    flush(&mut a);
    String::new()
}
";
        assert_eq!(names(&units("rs", src)), [("assemble".to_owned(), 1, 8)]);
    }

    #[test]
    fn a_unit_keeps_the_doc_comment_above_it() {
        let src = "use x;

/// Cuts long tool output
/// to head and tail.
#[inline]
pub fn truncate(s: &str) -> String {
    s.into()
}
";
        assert_eq!(names(&units("rs", src)), [("truncate".to_owned(), 3, 8)]);
    }

    #[test]
    fn python_and_typescript() {
        let py = "import os\n\nclass Store:\n    def get(self, k):\n        return 1\n\n    def put(self, k, v):\n        pass\n\ndef main():\n    pass\n";
        assert_eq!(
            names(&units("py", py)),
            [
                ("Store::get".to_owned(), 4, 5),
                ("Store::put".to_owned(), 7, 8),
                ("main".to_owned(), 10, 11),
            ]
        );
        let ts = "export function a(x: number): number {\n  if (x) {\n    return 1;\n  }\n  return 2;\n}\nexport const b = async (y) => {\n  return y;\n};\nexport class C {\n  run(): void {\n    a(1);\n  }\n}\n";
        assert_eq!(
            names(&units("ts", ts)),
            [
                ("a".to_owned(), 1, 6),
                ("b".to_owned(), 7, 9),
                ("C::run".to_owned(), 11, 13),
            ]
        );
    }

    #[test]
    fn other_text_and_long_units_use_windows() {
        let md: String = (0..170).map(|i| format!("line {i}\n")).collect();
        let u = units("md", &md);
        assert_eq!(u.len(), 3);
        assert_eq!((u[2].start, u[2].end), (161, 170));
        let long = format!("fn big() {{\n{}}}\n", "    x();\n".repeat(200));
        let u = units("rs", &long);
        assert_eq!(u.len(), 2);
        assert_eq!((u[0].start, u[0].end, u[1].end), (1, 150, 202));
    }
}

//! HTML (and the EPUB books made of it), CSV/TSV, and Jupyter notebooks as Markdown.

use std::sync::LazyLock;

use regex::Regex;

use super::{md, ooxml::Package, DocError};

/// Rows of a delimited file shown at most.
const CSV_ROWS: usize = 500;

static DROP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)<(script|style|noscript|svg|head|template)\b[^>]*>.*?</(script|style|noscript|svg|head|template)>|<!--.*?-->")
        .expect("regex")
});
static HEADING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<h([1-6])\b[^>]*>(.*?)</h[1-6]>").expect("regex"));
static TABLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<table\b[^>]*>(.*?)</table>").expect("regex"));
static ROW: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<tr\b[^>]*>(.*?)</tr>").expect("regex"));
static CELL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<t[hd]\b[^>]*>(.*?)</t[hd]>").expect("regex"));
static ITEM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<li\b[^>]*>").expect("regex"));
static PRE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<pre\b[^>]*>(.*?)</pre>").expect("regex"));
static BLOCK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)</?(p|div|br|ul|ol|section|article|header|footer|blockquote|dl|dt|dd|hr)\b[^>]*>",
    )
    .expect("regex")
});
static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<[^>]*>").expect("regex"));
static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[ \t\u{a0}]+").expect("regex"));
static NUM_ENTITY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"&#(x[0-9a-fA-F]+|[0-9]+);").expect("regex"));

fn entities(s: &str) -> String {
    let s = NUM_ENTITY.replace_all(s, |c: &regex::Captures| {
        let n = &c[1];
        let code = match n.strip_prefix('x') {
            Some(h) => u32::from_str_radix(h, 16).ok(),
            None => n.parse().ok(),
        };
        code.and_then(char::from_u32)
            .map(String::from)
            .unwrap_or_default()
    });
    s.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Inline text: tags dropped, entities read, spaces folded.
fn inline(s: &str) -> String {
    SPACES
        .replace_all(&entities(&TAG.replace_all(s, " ")), " ")
        .trim()
        .to_owned()
}

/// HTML as Markdown: headings, list items, tables and preformatted blocks kept; scripts, styles and comments dropped.
pub(crate) fn html(src: &str) -> String {
    let s = DROP.replace_all(src, " ");
    let s = PRE.replace_all(&s, |c: &regex::Captures| {
        format!(
            "\n\n```\n{}\n```\n\n",
            entities(&TAG.replace_all(&c[1], ""))
        )
    });
    let s = TABLE.replace_all(&s, |c: &regex::Captures| {
        let rows: Vec<Vec<String>> = ROW
            .captures_iter(&c[1])
            .map(|r| CELL.captures_iter(&r[1]).map(|x| inline(&x[1])).collect())
            .collect();
        format!("\n\n{}\n\n", md::table(&rows))
    });
    let s = HEADING.replace_all(&s, |c: &regex::Captures| {
        let n: usize = c[1].parse().unwrap_or(2);
        format!("\n\n{} {}\n\n", "#".repeat(n), inline(&c[2]))
    });
    let s = ITEM.replace_all(&s, "\n- ");
    let s = BLOCK.replace_all(&s, "\n");
    let s = TAG.replace_all(&s, "");
    let mut out = Vec::new();
    let mut fenced = false;
    for line in entities(&s).lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            out.push(line.trim().to_owned());
        } else if fenced {
            out.push(line.to_owned());
        } else {
            out.push(SPACES.replace_all(line, " ").trim().to_owned());
        }
    }
    out.join("\n")
}

/// CSV or TSV as a Markdown table (quoted fields with the delimiter, quotes or line breaks read right).
pub(crate) fn delimited(text: &str, sep: char, notes: &mut Vec<String>) -> String {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let (mut row, mut field, mut quoted) = (Vec::new(), String::new(), false);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            '"' if quoted => quoted = false,
            '"' if field.is_empty() => quoted = true,
            c if c == sep && !quoted => row.push(std::mem::take(&mut field)),
            '\n' if !quoted => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            '\r' if !quoted => {}
            c => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    if rows.len() > CSV_ROWS + 1 {
        notes.push(format!(
            "only the first {CSV_ROWS} of {} rows",
            rows.len() - 1
        ));
        rows.truncate(CSV_ROWS + 1);
    }
    md::table(&rows)
}

/// An EPUB book: its chapters in reading order (the package's spine), each read as HTML.
pub(crate) fn epub(bytes: &[u8], notes: &mut Vec<String>) -> Result<String, DocError> {
    let mut p = Package::open(bytes)?;
    let container = p
        .part("META-INF/container.xml")?
        .ok_or_else(|| DocError::Broken("no META-INF/container.xml: not an EPUB".into()))?;
    let opf_path = roxmltree::Document::parse(&container)
        .map_err(|e| DocError::Broken(e.to_string()))?
        .descendants()
        .find(|n| n.tag_name().name() == "rootfile")
        .and_then(|n| n.attribute("full-path").map(str::to_owned))
        .ok_or_else(|| DocError::Broken("the EPUB names no package file".into()))?;
    let opf = p
        .part(&opf_path)?
        .ok_or_else(|| DocError::Broken(format!("{opf_path} is missing")))?;
    let doc = roxmltree::Document::parse(&opf).map_err(|e| DocError::Broken(e.to_string()))?;
    let base = opf_path
        .rsplit_once('/')
        .map(|(d, _)| format!("{d}/"))
        .unwrap_or_default();
    let items: Vec<(String, String)> = doc
        .descendants()
        .filter(|n| n.tag_name().name() == "item")
        .filter_map(|n| {
            Some((
                n.attribute("id")?.to_owned(),
                n.attribute("href")?.to_owned(),
            ))
        })
        .collect();
    let mut out = Vec::new();
    for idref in doc
        .descendants()
        .filter(|n| n.tag_name().name() == "itemref")
        .filter_map(|n| n.attribute("idref"))
    {
        let Some((_, href)) = items.iter().find(|(id, _)| id == idref) else {
            continue;
        };
        if let Some(x) = p.part(&format!("{base}{href}"))? {
            let t = html(&x);
            if !t.trim().is_empty() {
                out.push(t);
            }
        }
    }
    if out.is_empty() {
        notes.push("the book has no readable chapter".into());
    }
    Ok(out.join("\n\n"))
}

/// A Jupyter notebook: markdown cells as they are, code cells fenced in the kernel's language, short text outputs.
pub(crate) fn notebook(text: &str, notes: &mut Vec<String>) -> Result<String, DocError> {
    let v: serde_json::Value =
        serde_json::from_str(text).map_err(|e| DocError::Broken(format!("not a notebook: {e}")))?;
    let lang = v["metadata"]["kernelspec"]["language"]
        .as_str()
        .or(v["metadata"]["language_info"]["name"].as_str())
        .unwrap_or("");
    let joined = |x: &serde_json::Value| match x {
        serde_json::Value::Array(a) => a.iter().filter_map(|l| l.as_str()).collect::<String>(),
        serde_json::Value::String(s) => s.clone(),
        _ => String::new(),
    };
    let mut out = Vec::new();
    let mut outputs_cut = false;
    for cell in v["cells"].as_array().into_iter().flatten() {
        let src = joined(&cell["source"]);
        match cell["cell_type"].as_str() {
            Some("markdown") => out.push(src),
            Some("code") => {
                out.push(format!("```{lang}\n{}\n```", src.trim_end()));
                for o in cell["outputs"].as_array().into_iter().flatten() {
                    let t = joined(&o["text"]) + &joined(&o["data"]["text/plain"]);
                    if t.trim().is_empty() {
                        continue;
                    }
                    let short: String = t.chars().take(2000).collect();
                    outputs_cut |= short.len() < t.len();
                    out.push(format!("Output:\n```\n{}\n```", short.trim_end()));
                }
            }
            _ => {}
        }
    }
    if outputs_cut {
        notes.push("long cell outputs were cut to 2000 characters".into());
    }
    Ok(out.join("\n\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_keeps_its_structure() {
        let page = "<html><head><title>x</title><style>p{}</style></head><body><h1>Guide &amp; rules</h1><p>One&nbsp;line.<br>Two</p>\
            <ul><li>first</li><li><b>second</b></li></ul><table><tr><th>A</th><th>B</th></tr><tr><td>1</td><td>2</td></tr></table>\
            <pre>let x = 1;\n  y</pre><script>alert(1)</script><!-- hidden --></body></html>";
        let md = crate::md::tidy(&html(page));
        assert_eq!(
            md,
            "# Guide & rules\n\nOne line.\nTwo\n\n- first\n- second\n\n| A | B |\n| --- | --- |\n| 1 | 2 |\n\n```\nlet x = 1;\n  y\n```"
        );
    }

    #[test]
    fn csv_with_quotes() {
        let mut notes = Vec::new();
        assert_eq!(
            delimited(
                "name,note\r\n\"Doe, J\",\"said \"\"hi\"\"\"\nx,\"two\nlines\"\n",
                ',',
                &mut notes
            ),
            "| name | note |\n| --- | --- |\n| Doe, J | said \"hi\" |\n| x | two lines |"
        );
    }
}

//! Word, PowerPoint and Excel files (Office Open XML: a zip of XML parts), read the way markitdown lays them out:
//! Word headings and lists as Markdown with its tables; a section per slide with its title, text, tables and notes; a
//! table per sheet. Parts are read with a size cap (a zip bomb stops at the cap).

use std::io::Read;

use roxmltree::{Document, Node};

use super::{md, DocError};

/// One part of the zip, uncompressed, at most.
const PART_MAX: u64 = 64 * 1024 * 1024;
/// Rows and columns of a sheet shown at most.
const SHEET_ROWS: usize = 500;
const SHEET_COLS: usize = 50;

pub(crate) struct Package(zip::ZipArchive<std::io::Cursor<Vec<u8>>>);

impl Package {
    pub(crate) fn open(bytes: &[u8]) -> Result<Self, DocError> {
        zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec()))
            .map(Package)
            .map_err(|e| DocError::Broken(format!("not a zip package: {e}")))
    }

    /// A part as text; `None` when it is not there.
    pub(crate) fn part(&mut self, name: &str) -> Result<Option<String>, DocError> {
        let name = name.trim_start_matches('/');
        let mut f = match self.0.by_name(name) {
            Ok(f) => f,
            Err(zip::result::ZipError::FileNotFound) => return Ok(None),
            Err(e) => return Err(DocError::Broken(e.to_string())),
        };
        if f.size() > PART_MAX {
            return Err(DocError::Broken(format!("{name} is too large")));
        }
        let mut s = String::new();
        f.by_ref()
            .take(PART_MAX)
            .read_to_string(&mut s)
            .map_err(|e| DocError::Broken(format!("{name}: {e}")))?;
        Ok(Some(s))
    }
}

fn parse(text: &str) -> Result<Document<'_>, DocError> {
    Document::parse(text).map_err(|e| DocError::Broken(format!("bad XML: {e}")))
}

fn named<'a, 'i>(n: Node<'a, 'i>, local: &'static str) -> impl Iterator<Item = Node<'a, 'i>> {
    n.children().filter(move |c| c.tag_name().name() == local)
}

fn attr<'a>(n: Node<'a, '_>, local: &str) -> Option<&'a str> {
    n.attributes()
        .find(|a| a.name() == local)
        .map(|a| a.value())
}

/// `target` of a relationship of the part `owner`, as a part name (`word/media/x.png`).
fn resolve(owner: &str, target: &str) -> String {
    if let Some(abs) = target.strip_prefix('/') {
        return abs.to_owned();
    }
    let mut parts: Vec<&str> = owner.split('/').collect();
    parts.pop();
    for seg in target.split('/') {
        match seg {
            ".." => {
                parts.pop();
            }
            "." | "" => {}
            s => parts.push(s),
        }
    }
    parts.join("/")
}

/// The relationships of `owner` (`ppt/presentation.xml` -> `ppt/_rels/presentation.xml.rels`): id -> (type, part).
fn rels(p: &mut Package, owner: &str) -> Result<Vec<(String, String, String)>, DocError> {
    let (dir, file) = owner.rsplit_once('/').unwrap_or(("", owner));
    let name = if dir.is_empty() {
        format!("_rels/{file}.rels")
    } else {
        format!("{dir}/_rels/{file}.rels")
    };
    let Some(text) = p.part(&name)? else {
        return Ok(Vec::new());
    };
    let doc = parse(&text)?;
    Ok(doc
        .descendants()
        .filter(|n| n.tag_name().name() == "Relationship")
        .filter_map(|n| {
            Some((
                attr(n, "Id")?.to_owned(),
                attr(n, "Type").unwrap_or("").to_owned(),
                resolve(owner, attr(n, "Target")?),
            ))
        })
        .collect())
}

/// The text of a run container: `w:t`/`a:t`, tabs and breaks; deleted text and field codes left out.
fn text_of(n: Node) -> String {
    let mut s = String::new();
    for d in n.descendants() {
        match d.tag_name().name() {
            "t" => s.push_str(d.text().unwrap_or("")),
            "tab" => s.push('\t'),
            "br" | "cr" => s.push('\n'),
            _ => {}
        }
    }
    s
}

/// A Word document: headings by their style, list items, tables.
pub(crate) fn docx(bytes: &[u8], notes: &mut Vec<String>) -> Result<String, DocError> {
    let mut p = Package::open(bytes)?;
    let text = p
        .part("word/document.xml")?
        .ok_or_else(|| DocError::Broken("no word/document.xml: not a Word document".into()))?;
    let doc = parse(&text)?;
    let Some(body) = doc.descendants().find(|n| n.tag_name().name() == "body") else {
        return Ok(String::new());
    };
    let mut out = Vec::new();
    for n in body.children().filter(|n| n.is_element()) {
        match n.tag_name().name() {
            "p" => {
                let t = text_of(n).trim().to_owned();
                if t.is_empty() {
                    continue;
                }
                let ppr = named(n, "pPr").next();
                let style = ppr
                    .and_then(|p| named(p, "pStyle").next())
                    .and_then(|s| attr(s, "val"))
                    .unwrap_or("");
                let level = ppr
                    .and_then(|p| named(p, "outlineLvl").next())
                    .and_then(|o| attr(o, "val"))
                    .and_then(|v| v.parse::<usize>().ok())
                    .map(|l| l + 1)
                    .or_else(|| {
                        let s = style.to_ascii_lowercase();
                        if s == "title" {
                            Some(1)
                        } else {
                            s.strip_prefix("heading")
                                .and_then(|d| d.parse::<usize>().ok())
                        }
                    });
                let listed = ppr.and_then(|p| named(p, "numPr").next()).is_some();
                out.push(match (level, listed) {
                    (Some(l), _) if (1..=6).contains(&l) => format!("{} {t}", "#".repeat(l)),
                    (_, true) => format!("- {t}"),
                    _ => t,
                });
            }
            "tbl" => {
                let rows: Vec<Vec<String>> = named(n, "tr")
                    .map(|tr| {
                        named(tr, "tc")
                            .map(|tc| text_of(tc).trim().to_owned())
                            .collect()
                    })
                    .collect();
                out.push(md::table(&rows));
            }
            _ => {}
        }
    }
    if out.is_empty() {
        notes.push("the document has no text".into());
    }
    Ok(out.join("\n\n"))
}

/// A slide's or a notes page's shapes: (title, text paragraphs, tables).
fn shapes(doc: &Document) -> (Option<String>, Vec<String>, Vec<String>) {
    let mut title = None;
    let mut lines = Vec::new();
    let mut tables = Vec::new();
    for sp in doc.descendants().filter(|n| n.tag_name().name() == "sp") {
        let ph = sp
            .descendants()
            .find(|n| n.tag_name().name() == "ph")
            .and_then(|p| attr(p, "type"));
        if matches!(ph, Some("sldNum" | "sldImg" | "dt" | "ftr" | "hdr")) {
            continue;
        }
        let paras: Vec<String> = sp
            .descendants()
            .filter(|n| {
                n.tag_name().name() == "p"
                    && n.tag_name()
                        .namespace()
                        .is_some_and(|ns| ns.contains("drawingml"))
            })
            .map(|p| text_of(p).trim().to_owned())
            .filter(|t| !t.is_empty())
            .collect();
        if matches!(ph, Some("title" | "ctrTitle")) && title.is_none() {
            title = Some(paras.join(" "));
        } else {
            lines.extend(paras);
        }
    }
    for tbl in doc.descendants().filter(|n| n.tag_name().name() == "tbl") {
        let rows: Vec<Vec<String>> = named(tbl, "tr")
            .map(|tr| {
                named(tr, "tc")
                    .map(|tc| text_of(tc).trim().to_owned())
                    .collect()
            })
            .collect();
        tables.push(md::table(&rows));
    }
    (title, lines, tables)
}

/// A presentation: a section per slide in the deck's order, its notes after it.
pub(crate) fn pptx(bytes: &[u8], notes: &mut Vec<String>) -> Result<String, DocError> {
    let mut p = Package::open(bytes)?;
    let pres = "ppt/presentation.xml";
    let text = p
        .part(pres)?
        .ok_or_else(|| DocError::Broken("no ppt/presentation.xml: not a presentation".into()))?;
    let order: Vec<String> = parse(&text)?
        .descendants()
        .filter(|n| n.tag_name().name() == "sldId")
        .filter_map(|n| {
            n.attributes()
                .find(|a| a.name() == "id" && a.namespace().is_some())
                .map(|a| a.value().to_owned())
        })
        .collect();
    let links = rels(&mut p, pres)?;
    let mut out = Vec::new();
    for (i, rid) in order.iter().enumerate() {
        let Some((_, _, part)) = links.iter().find(|(id, _, _)| id == rid) else {
            continue;
        };
        let Some(xml) = p.part(part)? else {
            continue;
        };
        let (title, lines, tables) = shapes(&parse(&xml)?);
        let mut s = match title.filter(|t| !t.is_empty()) {
            Some(t) => format!("## Slide {}: {t}", i + 1),
            None => format!("## Slide {}", i + 1),
        };
        for l in lines {
            s.push_str(&format!("\n\n{l}"));
        }
        for t in tables {
            s.push_str(&format!("\n\n{t}"));
        }
        let note = rels(&mut p, part)?
            .into_iter()
            .find(|(_, ty, _)| ty.ends_with("/notesSlide"));
        if let Some((_, _, np)) = note {
            if let Some(nx) = p.part(&np)? {
                let (_, nl, _) = shapes(&parse(&nx)?);
                if !nl.is_empty() {
                    s.push_str(&format!("\n\n### Notes\n\n{}", nl.join("\n")));
                }
            }
        }
        out.push(s);
    }
    if out.is_empty() {
        notes.push("the presentation has no slides".into());
    }
    Ok(out.join("\n\n"))
}

/// `B12` -> column 1 (zero-based).
fn column(r: &str) -> Option<usize> {
    let letters: String = r.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    if letters.is_empty() {
        return None;
    }
    Some(
        letters
            .to_ascii_uppercase()
            .bytes()
            .fold(0usize, |n, b| n * 26 + (b - b'A' + 1) as usize)
            - 1,
    )
}

/// A workbook: a table per sheet, at most [`SHEET_ROWS`] rows and [`SHEET_COLS`] columns each.
pub(crate) fn xlsx(bytes: &[u8], notes: &mut Vec<String>) -> Result<String, DocError> {
    let mut p = Package::open(bytes)?;
    let book = "xl/workbook.xml";
    let text = p
        .part(book)?
        .ok_or_else(|| DocError::Broken("no xl/workbook.xml: not a workbook".into()))?;
    let sheets: Vec<(String, String)> = parse(&text)?
        .descendants()
        .filter(|n| n.tag_name().name() == "sheet")
        .filter_map(|n| {
            let rid = n
                .attributes()
                .find(|a| a.name() == "id" && a.namespace().is_some())?
                .value()
                .to_owned();
            Some((attr(n, "name").unwrap_or("Sheet").to_owned(), rid))
        })
        .collect();
    let links = rels(&mut p, book)?;
    let shared: Vec<String> = match p.part("xl/sharedStrings.xml")? {
        Some(s) => parse(&s)?
            .descendants()
            .filter(|n| n.tag_name().name() == "si")
            .map(|si| {
                si.descendants()
                    .filter(|n| n.tag_name().name() == "t")
                    .filter_map(|t| t.text())
                    .collect()
            })
            .collect(),
        None => Vec::new(),
    };
    let mut out = Vec::new();
    for (name, rid) in sheets {
        let Some((_, _, part)) = links.iter().find(|(id, _, _)| *id == rid) else {
            continue;
        };
        let Some(xml) = p.part(part)? else {
            continue;
        };
        let doc = parse(&xml)?;
        let mut grid: Vec<Vec<String>> = Vec::new();
        let mut cut = false;
        for row in doc.descendants().filter(|n| n.tag_name().name() == "row") {
            if grid.len() >= SHEET_ROWS {
                cut = true;
                break;
            }
            let mut cells: Vec<String> = Vec::new();
            for (j, c) in named(row, "c").enumerate() {
                let col = attr(c, "r").and_then(column).unwrap_or(j);
                if col >= SHEET_COLS {
                    cut = true;
                    continue;
                }
                let v = named(c, "v").next().and_then(|v| v.text()).unwrap_or("");
                let value = match attr(c, "t") {
                    Some("s") => v
                        .parse::<usize>()
                        .ok()
                        .and_then(|k| shared.get(k).cloned())
                        .unwrap_or_default(),
                    Some("inlineStr") => text_of(c),
                    Some("b") => (if v == "1" { "TRUE" } else { "FALSE" }).to_owned(),
                    _ => v.to_owned(),
                };
                if cells.len() <= col {
                    cells.resize(col + 1, String::new());
                }
                cells[col] = value;
            }
            grid.push(cells);
        }
        if cut {
            notes.push(format!(
                "sheet {name}: only the first {SHEET_ROWS} rows and {SHEET_COLS} columns"
            ));
        }
        let t = md::table(&grid);
        out.push(if t.is_empty() {
            format!("## Sheet: {name}\n\n(empty)")
        } else {
            format!("## Sheet: {name}\n\n{t}")
        });
    }
    Ok(out.join("\n\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_and_columns() {
        assert_eq!(
            resolve("ppt/presentation.xml", "slides/slide1.xml"),
            "ppt/slides/slide1.xml"
        );
        assert_eq!(
            resolve("ppt/slides/slide1.xml", "../notesSlides/notesSlide1.xml"),
            "ppt/notesSlides/notesSlide1.xml"
        );
        assert_eq!(
            resolve("xl/workbook.xml", "/xl/worksheets/sheet1.xml"),
            "xl/worksheets/sheet1.xml"
        );
        assert_eq!(column("A1"), Some(0));
        assert_eq!(column("AB7"), Some(27));
        assert_eq!(column("12"), None);
    }
}

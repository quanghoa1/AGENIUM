//! Documents read as Markdown (ADR-152), the way Microsoft's markitdown does it, in our own code: one converter per
//! format chosen by the name (and the content's signature), Markdown out because models read it well and it keeps the
//! structure (headings, lists, tables) for few tokens, and no format fails the whole read when a plainer one works.
//! Built in: PDF (its text layer, page by page), Word, PowerPoint, Excel, CSV/TSV, HTML, EPUB, Jupyter notebooks and
//! text. When the user has markitdown installed, [`markitdown`] reads the rest (and the office formats better), in the
//! sandbox.

pub mod markitdown;
mod md;
mod ooxml;
mod pdf;
pub mod samples;
mod web;

use serde::Serialize;

/// A document read at most (bytes).
pub const MAX_BYTES: usize = 30 * 1024 * 1024;

/// A document read as Markdown.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Converted {
    pub text: String,
    /// What it was read as (`pdf`, `docx`, `markitdown`…).
    pub format: String,
    /// What the user should know (pages without text, rows left out, a format read by markitdown…).
    pub notes: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum DocError {
    #[error("{0} is not a format Agenium reads")]
    Unsupported(String),
    #[error("the document is larger than {} MB", MAX_BYTES / 1024 / 1024)]
    TooLarge,
    #[error("the document could not be read: {0}")]
    Broken(String),
}

/// The format of a document by its name, then by its first bytes.
fn kind(name: &str, bytes: &[u8]) -> String {
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    if !ext.is_empty() {
        return ext;
    }
    if bytes.starts_with(b"%PDF") {
        "pdf".into()
    } else if bytes.starts_with(b"PK\x03\x04") {
        "zip".into()
    } else {
        "txt".into()
    }
}

/// Formats [`convert`] reads (by extension), for the file picker.
pub const BUILT_IN: &[&str] = &[
    "pdf", "docx", "pptx", "xlsx", "csv", "tsv", "html", "htm", "xhtml", "epub", "ipynb", "md",
    "markdown", "txt", "rst", "adoc", "json", "yaml", "yml", "toml", "xml", "py", "js", "ts",
    "tsx", "jsx", "rs", "go", "java", "kt", "cs", "c", "h", "cpp", "hpp", "rb", "php", "sh", "ps1",
    "sql", "css", "ini", "cfg", "log",
];

/// Whether markitdown reads `name` better than the built-in converters (the office formats, a PDF's tables) or reads
/// it at all (old `.doc`/`.xls`/`.ppt`, Outlook `.msg`, images, audio, archives).
pub fn better_with_markitdown(name: &str) -> bool {
    let k = kind(name, &[]);
    !matches!(
        k.as_str(),
        "csv" | "tsv" | "html" | "htm" | "xhtml" | "ipynb"
    ) && !is_text(&k)
}

fn is_text(k: &str) -> bool {
    BUILT_IN.contains(&k)
        && !matches!(
            k,
            "pdf"
                | "docx"
                | "pptx"
                | "xlsx"
                | "epub"
                | "csv"
                | "tsv"
                | "html"
                | "htm"
                | "xhtml"
                | "ipynb"
        )
}

/// Reads `bytes` (a document named `name`) as Markdown with the built-in converters.
pub fn convert(name: &str, bytes: &[u8]) -> Result<Converted, DocError> {
    if bytes.len() > MAX_BYTES {
        return Err(DocError::TooLarge);
    }
    let k = kind(name, bytes);
    let mut notes = Vec::new();
    let text = match k.as_str() {
        "pdf" => pdf::read(bytes, &mut notes)?,
        "docx" => ooxml::docx(bytes, &mut notes)?,
        "pptx" => ooxml::pptx(bytes, &mut notes)?,
        "xlsx" => ooxml::xlsx(bytes, &mut notes)?,
        "epub" => web::epub(bytes, &mut notes)?,
        "csv" => web::delimited(&utf8(bytes, &mut notes), ',', &mut notes),
        "tsv" => web::delimited(&utf8(bytes, &mut notes), '\t', &mut notes),
        "html" | "htm" | "xhtml" => web::html(&utf8(bytes, &mut notes)),
        "ipynb" => web::notebook(&utf8(bytes, &mut notes), &mut notes)?,
        k if is_text(k) => utf8(bytes, &mut notes),
        other => return Err(DocError::Unsupported(format!(".{other}"))),
    };
    Ok(Converted {
        text: md::tidy(&text),
        format: k,
        notes,
    })
}

/// The bytes as UTF-8 (a BOM dropped); invalid bytes are replaced, and the note says so.
fn utf8(bytes: &[u8], notes: &mut Vec<String>) -> String {
    let b = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    match std::str::from_utf8(b) {
        Ok(s) => s.to_owned(),
        Err(_) => {
            notes.push("not UTF-8: unreadable bytes were replaced".into());
            String::from_utf8_lossy(b).into_owned()
        }
    }
}

//! A PDF's text layer, page by page (`pdf-extract`). A scanned PDF has none: it says so instead of returning nothing
//! (markitdown, when installed, is tried for it). The parser runs behind `catch_unwind`: a malformed file must not take
//! the app down.

use super::DocError;

pub(crate) fn read(bytes: &[u8], notes: &mut Vec<String>) -> Result<String, DocError> {
    let pages = std::panic::catch_unwind(|| pdf_extract::extract_text_from_mem_by_pages(bytes))
        .map_err(|_| DocError::Broken("the PDF reader failed on this file".into()))?
        .map_err(|e| DocError::Broken(e.to_string()))?;
    let total = pages.len();
    let empty = pages.iter().filter(|p| p.trim().is_empty()).count();
    if total > 0 && empty == total {
        return Err(DocError::Broken(
            "the PDF has no text layer (a scanned document needs OCR, e.g. markitdown with its OCR plugin)".into(),
        ));
    }
    if empty > 0 {
        notes.push(format!(
            "{empty} of {total} pages have no text (scanned or images)"
        ));
    }
    Ok(pages
        .iter()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n"))
}

//! ADR-152: `read_file` reads a document of the worktree as Markdown (paged like text, marked as a converted copy that
//! the edit tools do not apply to); a text file reads as before.

mod common;

use std::io::Write;

use common::*;
use serde_json::json;

/// A one-page text PDF (Helvetica; the cross-reference offsets computed).
fn pdf(text: &str) -> Vec<u8> {
    let content = format!("BT /F1 12 Tf 72 720 Td ({text}) Tj ET");
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [4 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents 5 0 R >>"
            .to_owned(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offsets {
        out.extend(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objs.len() + 1
        )
        .as_bytes(),
    );
    out
}

fn docx(paragraphs: &[&str]) -> Vec<u8> {
    let body: String = paragraphs
        .iter()
        .map(|p| format!("<w:p><w:r><w:t>{p}</w:t></w:r></w:p>"))
        .collect();
    let xml = format!(
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#
    );
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    w.start_file(
        "word/document.xml",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    w.write_all(xml.as_bytes()).unwrap();
    w.finish().unwrap().into_inner()
}

#[tokio::test]
async fn documents_are_read_as_markdown_and_stay_read_only() {
    let (wt, c) = setup();
    std::fs::create_dir_all(wt.root.join("docs")).unwrap();
    std::fs::write(
        wt.root.join("docs/spec.pdf"),
        pdf("Checkout must keep the cart"),
    )
    .unwrap();
    std::fs::write(
        wt.root.join("docs/notes.docx"),
        docx(&["First rule", "Second rule", "Third rule"]),
    )
    .unwrap();

    let out = call(&c, "read_file", json!({"path": "docs/spec.pdf"}))
        .await
        .unwrap();
    assert!(
        out.text.starts_with("[docs/spec.pdf read as Markdown by "),
        "{}",
        out.text
    );
    assert!(out.text.contains("a converted copy, read-only"));
    assert!(
        out.text.contains("Checkout must keep the cart"),
        "{}",
        out.text
    );

    // Paged like text: offset and limit apply to the Markdown lines.
    let out = call(
        &c,
        "read_file",
        json!({"path": "docs/notes.docx", "offset": 3, "limit": 1}),
    )
    .await
    .unwrap();
    assert!(
        out.text.contains("Second rule") && !out.text.contains("First rule"),
        "{}",
        out.text
    );
    assert!(out.text.contains("use offset to read more"));

    // Not recorded as read: the edit tools refuse a converted copy (they would write Markdown over a binary file).
    let e = call(
        &c,
        "edit_file",
        json!({"path": "docs/notes.docx", "old_string": "First rule", "new_string": "x"}),
    )
    .await
    .unwrap_err();
    assert!(!e.message.is_empty());

    // A broken document says why; a text file reads as before.
    std::fs::write(wt.root.join("docs/broken.pdf"), b"%PDF-1.4 nothing").unwrap();
    let e = call(&c, "read_file", json!({"path": "docs/broken.pdf"}))
        .await
        .unwrap_err();
    assert_eq!(e.code, "unreadable_document");
    let out = call(&c, "read_file", json!({"path": "src/lib.rs"}))
        .await
        .unwrap();
    assert!(out.text.contains("pub fn add") && !out.text.contains("read as Markdown"));
}

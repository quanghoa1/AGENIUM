//! Small documents made here, no binary file in the repository: the self-test of a markitdown install (ADR-152) reads
//! them, and so do the converters' tests. Each is a complete package (content types, relationships, explicit shape
//! positions) so any reader opens it, not only Agenium's.

use std::io::Write;

/// A sample document and what a good reading of it holds.
pub struct Sample {
    pub name: &'static str,
    pub bytes: Vec<u8>,
    /// Phrases the Markdown holds when the document was read well.
    pub expect: &'static [&'static str],
}

/// One sample per format markitdown is used for: Word, Excel, PowerPoint and PDF.
pub fn samples() -> Vec<Sample> {
    vec![
        Sample {
            name: "sample.docx",
            bytes: docx(),
            expect: &["Checkout keeps the cart", "Cart kept"],
        },
        Sample {
            name: "sample.xlsx",
            bytes: xlsx(),
            expect: &["Blue mug", "12.5"],
        },
        Sample {
            name: "sample.pptx",
            bytes: pptx(),
            expect: &["Release plan", "Ship on Friday"],
        },
        Sample {
            name: "sample.pdf",
            bytes: pdf("Checkout must keep the cart"),
            expect: &["Checkout must keep the cart"],
        },
    ]
}

fn zip_of(parts: &[(&str, &str)]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, body) in parts {
        // Writing to memory cannot fail.
        let _ = w.start_file(*name, zip::write::SimpleFileOptions::default());
        let _ = w.write_all(body.as_bytes());
    }
    w.finish().map(|c| c.into_inner()).unwrap_or_default()
}

const XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#;
const RELS_NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const OFFICE_DOC: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";

fn content_types(overrides: &[(&str, &str)]) -> String {
    let o: String = overrides
        .iter()
        .map(|(part, ty)| format!(r#"<Override PartName="{part}" ContentType="{ty}"/>"#))
        .collect();
    format!(
        r#"{XML}<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/>{o}</Types>"#
    )
}

/// A relationships part: (id, type, target).
fn rels(list: &[(&str, &str, &str)]) -> String {
    let r: String = list
        .iter()
        .map(|(id, ty, target)| {
            format!(r#"<Relationship Id="{id}" Type="{ty}" Target="{target}"/>"#)
        })
        .collect();
    format!(r#"{XML}<Relationships xmlns="{RELS_NS}">{r}</Relationships>"#)
}

fn docx() -> Vec<u8> {
    const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
    let p = |t: &str| format!("<w:p><w:r><w:t>{t}</w:t></w:r></w:p>");
    let cell = |t: &str| format!("<w:tc>{}</w:tc>", p(t));
    let body = format!(
        "{}{}<w:tbl><w:tr>{}{}</w:tr><w:tr>{}{}</w:tr></w:tbl>{}",
        p("Agenium self-test"),
        p("Checkout keeps the cart."),
        cell("Case"),
        cell("Expect"),
        cell("Decline"),
        cell("Cart kept"),
        p("End.")
    );
    let doc = format!(r#"{XML}<w:document xmlns:w="{W}"><w:body>{body}</w:body></w:document>"#);
    zip_of(&[
        (
            "[Content_Types].xml",
            &content_types(&[(
                "/word/document.xml",
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml",
            )]),
        ),
        (
            "_rels/.rels",
            &rels(&[("rId1", OFFICE_DOC, "word/document.xml")]),
        ),
        ("word/document.xml", &doc),
    ])
}

fn xlsx() -> Vec<u8> {
    const S: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
    const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    let text = |r: &str, t: &str| format!(r#"<c r="{r}" t="inlineStr"><is><t>{t}</t></is></c>"#);
    let sheet = format!(
        r#"{XML}<worksheet xmlns="{S}"><sheetData><row r="1">{}{}</row><row r="2">{}<c r="B2"><v>12.5</v></c></row></sheetData></worksheet>"#,
        text("A1", "Item"),
        text("B1", "Price"),
        text("A2", "Blue mug"),
    );
    let book = format!(
        r#"{XML}<workbook xmlns="{S}" xmlns:r="{R}"><sheets><sheet name="Orders" sheetId="1" r:id="rId1"/></sheets></workbook>"#
    );
    zip_of(&[
        (
            "[Content_Types].xml",
            &content_types(&[
                (
                    "/xl/workbook.xml",
                    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml",
                ),
                (
                    "/xl/worksheets/sheet1.xml",
                    "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml",
                ),
            ]),
        ),
        (
            "_rels/.rels",
            &rels(&[("rId1", OFFICE_DOC, "xl/workbook.xml")]),
        ),
        ("xl/workbook.xml", &book),
        (
            "xl/_rels/workbook.xml.rels",
            &rels(&[(
                "rId1",
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet",
                "worksheets/sheet1.xml",
            )]),
        ),
        ("xl/worksheets/sheet1.xml", &sheet),
    ])
}

fn pptx() -> Vec<u8> {
    const NS: &str = r#"xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
    // Text boxes with their own position: a reader needs no slide layout to place them.
    let shape = |id: u32, y: u64, t: &str| {
        format!(
            r#"<p:sp><p:nvSpPr><p:cNvPr id="{id}" name="Text {id}"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="457200" y="{y}"/><a:ext cx="8229600" cy="914400"/></a:xfrm></p:spPr><p:txBody><a:bodyPr/><a:p><a:r><a:t>{t}</a:t></a:r></a:p></p:txBody></p:sp>"#
        )
    };
    let slide = format!(
        r#"{XML}<p:sld {NS}><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/>{}{}</p:spTree></p:cSld></p:sld>"#,
        shape(2, 457200, "Release plan"),
        shape(3, 1828800, "Ship on Friday")
    );
    let pres = format!(
        r#"{XML}<p:presentation {NS}><p:sldIdLst><p:sldId id="256" r:id="rId2"/></p:sldIdLst><p:sldSz cx="9144000" cy="6858000"/><p:notesSz cx="6858000" cy="9144000"/></p:presentation>"#
    );
    zip_of(&[
        (
            "[Content_Types].xml",
            &content_types(&[
                (
                    "/ppt/presentation.xml",
                    "application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml",
                ),
                (
                    "/ppt/slides/slide1.xml",
                    "application/vnd.openxmlformats-officedocument.presentationml.slide+xml",
                ),
            ]),
        ),
        (
            "_rels/.rels",
            &rels(&[("rId1", OFFICE_DOC, "ppt/presentation.xml")]),
        ),
        ("ppt/presentation.xml", &pres),
        (
            "ppt/_rels/presentation.xml.rels",
            &rels(&[(
                "rId2",
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide",
                "slides/slide1.xml",
            )]),
        ),
        ("ppt/slides/slide1.xml", &slide),
    ])
}

/// A one-page text PDF (Helvetica; the cross-reference offsets computed).
pub fn pdf(text: &str) -> Vec<u8> {
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

/// Whether `markdown` holds every phrase `sample` expects; `Err` names the first missing.
pub fn check(sample: &Sample, markdown: &str) -> Result<(), String> {
    match sample.expect.iter().find(|e| !markdown.contains(*e)) {
        None => Ok(()),
        Some(missing) => Err(format!("\"{missing}\" not in the Markdown")),
    }
}

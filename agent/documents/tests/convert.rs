//! Each built-in converter on a small document made here (no binary fixture in the repository).

use std::io::Write;

use agenium_documents::{convert, DocError};

fn zip_of(parts: &[(&str, &str)]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, body) in parts {
        w.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        w.write_all(body.as_bytes()).unwrap();
    }
    w.finish().unwrap().into_inner()
}

const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;
const P: &str = r#"xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
const R: &str = r#"xmlns="http://schemas.openxmlformats.org/package/2006/relationships""#;

#[test]
fn a_word_document_keeps_headings_lists_and_tables() {
    let doc = format!(
        r#"<w:document {W}><w:body>
<w:p><w:pPr><w:pStyle w:val="Title"/></w:pPr><w:r><w:t>Checkout guide</w:t></w:r></w:p>
<w:p><w:pPr><w:pStyle w:val="Heading2"/></w:pPr><w:r><w:t>Rules</w:t></w:r></w:p>
<w:p><w:r><w:t xml:space="preserve">Totals match </w:t></w:r><w:r><w:t>line items.</w:t></w:r><w:r><w:delText>gone</w:delText></w:r></w:p>
<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t>One order per click</w:t></w:r></w:p>
<w:tbl><w:tr><w:tc><w:p><w:r><w:t>Case</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>Expect</w:t></w:r></w:p></w:tc></w:tr>
<w:tr><w:tc><w:p><w:r><w:t>Decline</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>Cart kept</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
</w:body></w:document>"#
    );
    let c = convert("guide.docx", &zip_of(&[("word/document.xml", &doc)])).unwrap();
    assert_eq!(c.format, "docx");
    assert_eq!(
        c.text,
        "# Checkout guide\n\n## Rules\n\nTotals match line items.\n\n- One order per click\n\n| Case | Expect |\n| --- | --- |\n| Decline | Cart kept |"
    );
}

#[test]
fn a_presentation_has_a_section_per_slide_with_its_notes() {
    let pres = format!(
        r#"<p:presentation {P}><p:sldIdLst><p:sldId id="257" r:id="rId3"/><p:sldId id="256" r:id="rId2"/></p:sldIdLst></p:presentation>"#
    );
    let rels = format!(
        r#"<Relationships {R}><Relationship Id="rId2" Type="http://x/slide" Target="slides/slide1.xml"/><Relationship Id="rId3" Type="http://x/slide" Target="slides/slide2.xml"/></Relationships>"#
    );
    let slide = |title: &str, body: &str| {
        format!(
            r#"<p:sld {P}><p:cSld><p:spTree>
<p:sp><p:nvSpPr><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:r><a:t>{title}</a:t></a:r></a:p></p:txBody></p:sp>
<p:sp><p:nvSpPr><p:nvPr><p:ph idx="1"/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:r><a:t>{body}</a:t></a:r></a:p></p:txBody></p:sp>
<p:sp><p:nvSpPr><p:nvPr><p:ph type="sldNum"/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:r><a:t>7</a:t></a:r></a:p></p:txBody></p:sp>
</p:spTree></p:cSld></p:sld>"#
        )
    };
    let s1 = slide("Intro", "Why tests");
    let s2 = slide("Plan", "Three steps");
    let s1rels = format!(
        r#"<Relationships {R}><Relationship Id="rId9" Type="http://x/notesSlide" Target="../notesSlides/notesSlide1.xml"/></Relationships>"#
    );
    let notes = format!(
        r#"<p:notes {P}><p:cSld><p:spTree><p:sp><p:nvSpPr><p:nvPr><p:ph type="body"/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:r><a:t>Say hello first.</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:notes>"#
    );
    let bytes = zip_of(&[
        ("ppt/presentation.xml", &pres),
        ("ppt/_rels/presentation.xml.rels", &rels),
        ("ppt/slides/slide1.xml", &s1),
        ("ppt/slides/slide2.xml", &s2),
        ("ppt/slides/_rels/slide1.xml.rels", &s1rels),
        ("ppt/notesSlides/notesSlide1.xml", &notes),
    ]);
    let c = convert("deck.pptx", &bytes).unwrap();
    // The deck's order (rId3 first), the slide number placeholder left out, the notes after their slide.
    assert_eq!(
        c.text,
        "## Slide 1: Plan\n\nThree steps\n\n## Slide 2: Intro\n\nWhy tests\n\n### Notes\n\nSay hello first."
    );
}

#[test]
fn a_workbook_has_a_table_per_sheet() {
    let book = r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Prices" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
    let rels = format!(
        r#"<Relationships {R}><Relationship Id="rId1" Type="http://x/worksheet" Target="/xl/worksheets/sheet1.xml"/></Relationships>"#
    );
    let shared = r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><si><t>Item</t></si><si><t>Price</t></si><si><r><t>Te</t></r><r><t>a</t></r></si></sst>"#;
    let sheet = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>
<row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c></row>
<row r="2"><c r="A2" t="s"><v>2</v></c><c r="C2" t="b"><v>1</v></c><c r="B2"><v>2.5</v></c></row>
<row r="3"><c r="A3" t="inlineStr"><is><t>Cake</t></is></c></row></sheetData></worksheet>"#;
    let bytes = zip_of(&[
        ("xl/workbook.xml", book),
        ("xl/_rels/workbook.xml.rels", &rels),
        ("xl/sharedStrings.xml", shared),
        ("xl/worksheets/sheet1.xml", sheet),
    ]);
    let c = convert("prices.xlsx", &bytes).unwrap();
    assert_eq!(
        c.text,
        "## Sheet: Prices\n\n| Item | Price |  |\n| --- | --- | --- |\n| Tea | 2.5 | TRUE |\n| Cake |  |  |"
    );
}

#[test]
fn a_book_reads_its_chapters_in_order() {
    let container = r#"<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>"#;
    let opf = r#"<package xmlns="http://www.idpf.org/2007/opf"><manifest><item id="b" href="b.xhtml"/><item id="a" href="a.xhtml"/></manifest><spine><itemref idref="a"/><itemref idref="b"/></spine></package>"#;
    let bytes = zip_of(&[
        ("META-INF/container.xml", container),
        ("OEBPS/content.opf", opf),
        (
            "OEBPS/a.xhtml",
            "<html><body><h1>One</h1><p>First.</p></body></html>",
        ),
        (
            "OEBPS/b.xhtml",
            "<html><body><h1>Two</h1><p>Second.</p></body></html>",
        ),
    ]);
    assert_eq!(
        convert("book.epub", &bytes).unwrap().text,
        "# One\n\nFirst.\n\n# Two\n\nSecond."
    );
}

/// A PDF with one text page per string (Helvetica, the offsets of the cross-reference table computed).
fn pdf(pages: &[&str]) -> Vec<u8> {
    let n = pages.len();
    let mut objs: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        format!(
            "<< /Type /Pages /Kids [{}] /Count {n} >>",
            (0..n)
                .map(|i| format!("{} 0 R", 4 + 2 * i))
                .collect::<Vec<_>>()
                .join(" ")
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".into(),
    ];
    for (i, text) in pages.iter().enumerate() {
        let content = format!("BT /F1 12 Tf 72 720 Td ({text}) Tj ET");
        objs.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>",
            5 + 2 * i
        ));
        objs.push(format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ));
    }
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

#[test]
fn a_pdf_is_read_page_by_page_and_a_scan_says_so() {
    let c = convert("guide.pdf", &pdf(&["Checkout rules", "Second page"])).unwrap();
    assert_eq!(c.format, "pdf");
    assert!(c.text.contains("Checkout rules"), "{:?}", c.text);
    assert!(c.text.contains("Second page"), "{:?}", c.text);
    let i = c.text.find("Checkout").unwrap();
    assert!(i < c.text.find("Second").unwrap());
    // A page with no text (a scan): the read fails with why, rather than an empty document.
    let e = convert("scan.pdf", &pdf(&[""])).unwrap_err();
    assert!(e.to_string().contains("no text layer"), "{e}");
    // Not a PDF at all.
    assert!(matches!(
        convert("x.pdf", b"%PDF-1.4 garbage"),
        Err(DocError::Broken(_))
    ));
}

#[test]
fn notebooks_text_and_what_is_not_read() {
    let nb = r##"{"metadata":{"kernelspec":{"language":"python"}},"cells":[
        {"cell_type":"markdown","source":["# Title\n","Some text"]},
        {"cell_type":"code","source":"print(1)","outputs":[{"text":["1\n"]}]}]}"##;
    assert_eq!(
        convert("n.ipynb", nb.as_bytes()).unwrap().text,
        "# Title\nSome text\n\n```python\nprint(1)\n```\n\nOutput:\n```\n1\n```"
    );
    let t = convert("notes.md", "\u{feff}# Hi\r\n\r\n\r\ntext".as_bytes()).unwrap();
    assert_eq!(t.text, "# Hi\n\ntext");
    let latin = convert("old.txt", &[b'c', b'a', 0xe9]).unwrap();
    assert!(latin.notes[0].contains("not UTF-8"));
    assert!(matches!(
        convert("photo.png", b"\x89PNG"),
        Err(DocError::Unsupported(_))
    ));
    assert!(matches!(
        convert("old.doc", b"\xd0\xcf"),
        Err(DocError::Unsupported(_))
    ));
    assert!(agenium_documents::better_with_markitdown("old.doc"));
    assert!(agenium_documents::better_with_markitdown("a.pdf"));
    assert!(!agenium_documents::better_with_markitdown("a.csv"));
    assert!(!agenium_documents::better_with_markitdown("a.md"));
}

#[test]
fn the_self_test_samples_read_with_the_built_in_converters() {
    for s in agenium_documents::samples::samples() {
        let c = convert(s.name, &s.bytes).unwrap_or_else(|e| panic!("{}: {e}", s.name));
        agenium_documents::samples::check(&s, &c.text)
            .unwrap_or_else(|e| panic!("{}: {e}\n{}", s.name, c.text));
    }
}

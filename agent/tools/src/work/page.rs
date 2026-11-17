//! A fetched body as text for the model (ADR-163): HTML as Markdown without the page's chrome (navigation, header,
//! footer, scripts, forms), links made absolute so the model can follow them; the body decoded in its own charset;
//! a document (PDF, Word, ...) read as Markdown like `read_file` reads one.

/// Tags whose content is page chrome or code, not the page's text.
const SKIP: &[&str] = &[
    "script", "style", "noscript", "svg", "template", "iframe", "nav", "header", "footer", "aside",
    "form", "button", "select", "dialog",
];

/// The page's main part when it marks one (`<main>`, else a single `<article>`), else the whole body.
fn main_part(html: &str) -> &str {
    use std::sync::LazyLock;
    static MAIN: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?is)<main[\s>].*</main>").expect("regex"));
    static ARTICLE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?is)<article[\s>].*?</article>").expect("regex"));
    if let Some(m) = MAIN.find(html) {
        return m.as_str();
    }
    let mut articles = ARTICLE.find_iter(html);
    match (articles.next(), articles.next()) {
        (Some(a), None) => a.as_str(),
        _ => html,
    }
}

/// HTML as Markdown: headings, lists, tables, code and links kept; relative links resolved against `base`.
pub fn html_markdown(html: &str, base: &reqwest::Url) -> String {
    use std::sync::LazyLock;
    static LINK: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"\]\(([^)\s]+)").expect("regex"));
    let converter = htmd::HtmlToMarkdown::builder()
        .skip_tags(SKIP.to_vec())
        .build();
    let md = converter
        .convert(main_part(html))
        .unwrap_or_else(|_| main_part(html).to_owned());
    let md = LINK.replace_all(&md, |c: &regex::Captures| {
        let href = &c[1];
        match base.join(href) {
            Ok(u) if !href.starts_with('#') => format!("]({u}"),
            _ => c[0].to_owned(),
        }
    });
    // At most one blank line in a row.
    let mut out = String::new();
    let mut blank = 0;
    for line in md.lines().map(str::trim_end) {
        blank = if line.is_empty() { blank + 1 } else { 0 };
        if blank < 2 {
            out.push_str(line);
            out.push('\n');
        }
    }
    out.trim().to_owned()
}

/// `bytes` decoded in the charset the response names (`content-type`, else an HTML page's `<meta charset>`), UTF-8
/// when it names none.
pub fn decode(bytes: &[u8], content_type: &str) -> String {
    let label = charset(content_type).or_else(|| {
        let head = String::from_utf8_lossy(&bytes[..bytes.len().min(2048)]).to_ascii_lowercase();
        let at = head.find("charset=")? + "charset=".len();
        let rest = head[at..].trim_start_matches(['"', '\'']);
        let end = rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))?;
        Some(rest[..end].to_owned())
    });
    let enc = label
        .and_then(|l| encoding_rs::Encoding::for_label(l.as_bytes()))
        .unwrap_or(encoding_rs::UTF_8);
    enc.decode(bytes).0.into_owned()
}

fn charset(content_type: &str) -> Option<String> {
    content_type.split(';').find_map(|p| {
        let (k, v) = p.split_once('=')?;
        (k.trim().eq_ignore_ascii_case("charset")).then(|| v.trim().trim_matches('"').to_owned())
    })
}

/// The document kind of a response (by its type, else the URL's extension): read as a document, not as text.
pub fn document_name(url: &reqwest::Url, content_type: &str) -> Option<String> {
    let t = content_type.to_ascii_lowercase();
    let by_type = [
        ("application/pdf", "pdf"),
        ("wordprocessingml", "docx"),
        ("presentationml", "pptx"),
        ("spreadsheetml", "xlsx"),
        ("application/epub", "epub"),
        ("application/msword", "doc"),
        ("application/vnd.ms-excel", "xls"),
        ("application/vnd.ms-powerpoint", "ppt"),
    ]
    .iter()
    .find(|(k, _)| t.contains(k))
    .map(|(_, e)| *e);
    let ext = url
        .path()
        .rsplit('.')
        .next()
        .map(str::to_ascii_lowercase)
        .filter(|e| super::read_file::DOCUMENTS.contains(&e.as_str()));
    let ext = by_type.map(str::to_owned).or(ext)?;
    // Only when the server does not say it is a web page or text.
    (!t.contains("html") && !t.starts_with("text/")).then(|| format!("page.{ext}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_reads_as_its_main_part_in_markdown_with_absolute_links() {
        let base = reqwest::Url::parse("https://docs.example.com/guide/start.html").unwrap();
        let html = r##"<html><head><title>T</title><style>p{}</style></head><body>
            <nav><a href="/">Home</a> <a href="/blog">Blog</a></nav>
            <main><h1>Install</h1><p>Run <code>tool init</code> &#x2014; then read
            <a href="../ref/config.html">the config reference</a> or <a href="#flags">flags</a>.</p>
            <table><tr><th>Flag</th><th>Meaning</th></tr><tr><td>-v</td><td>verbose</td></tr></table>
            <script>track()</script></main><footer>(c) Example</footer></body></html>"##;
        let md = html_markdown(html, &base);
        assert!(md.contains("# Install"), "{md}");
        assert!(md.contains("`tool init` \u{2014} then"), "{md}");
        assert!(
            md.contains("](https://docs.example.com/ref/config.html)"),
            "{md}"
        );
        assert!(md.contains("](#flags)"), "an anchor stays: {md}");
        assert!(md.contains("| -v"), "the table stays a table: {md}");
        for gone in ["Home", "Blog", "track()", "(c) Example", "p{}"] {
            assert!(!md.contains(gone), "{gone} is chrome: {md}");
        }
    }

    #[test]
    fn a_body_is_decoded_in_its_charset() {
        let latin1 = [b'c', b'a', b'f', 0xe9];
        assert_eq!(decode(&latin1, "text/plain; charset=ISO-8859-1"), "café");
        let page =
            b"<html><head><meta charset=\"windows-1252\"></head><body>\x93hi\x94</body></html>";
        assert!(decode(page, "text/html").contains("\u{201c}hi\u{201d}"));
        assert_eq!(decode("phở".as_bytes(), "text/html"), "phở");
    }

    #[test]
    fn a_document_is_told_by_its_type_or_its_extension() {
        let u = |s: &str| reqwest::Url::parse(s).unwrap();
        assert_eq!(
            document_name(&u("https://x.org/a"), "application/pdf").as_deref(),
            Some("page.pdf")
        );
        assert_eq!(
            document_name(&u("https://x.org/r.docx"), "application/octet-stream").as_deref(),
            Some("page.docx")
        );
        assert_eq!(document_name(&u("https://x.org/r.pdf"), "text/html"), None);
        assert_eq!(document_name(&u("https://x.org/r.txt"), ""), None);
    }
}

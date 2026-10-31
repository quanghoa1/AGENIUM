//! Markdown helpers: a table from rows, and the clean-up every converter's output gets.

/// A Markdown table; the first row is the header. Pipes and line breaks in cells are escaped, short rows padded.
pub(crate) fn table(rows: &[Vec<String>]) -> String {
    let rows: Vec<&Vec<String>> = rows
        .iter()
        .filter(|r| r.iter().any(|c| !c.trim().is_empty()))
        .collect();
    let Some(width) = rows.iter().map(|r| r.len()).max() else {
        return String::new();
    };
    let cell = |c: &str| c.trim().replace('|', "\\|").replace(['\r', '\n'], " ");
    let line = |r: &Vec<String>| {
        let cells: Vec<String> = (0..width)
            .map(|i| cell(r.get(i).map(String::as_str).unwrap_or("")))
            .collect();
        format!("| {} |", cells.join(" | "))
    };
    let mut out = vec![line(rows[0]), format!("|{}", " --- |".repeat(width))];
    out.extend(rows[1..].iter().map(|r| line(r)));
    out.join("\n")
}

/// Lines trimmed at the end, runs of blank lines folded to one, a numbering left alone on its line (".1", "2.") joined
/// to the text after it (PDF extraction splits them; markitdown does the same fix).
pub(crate) fn tidy(text: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut blank = false;
    let mut pending: Option<String> = None;
    for raw in text.replace("\r\n", "\n").replace('\r', "\n").lines() {
        let line = raw.trim_end().replace('\u{0}', "");
        if line.trim().is_empty() {
            if pending.is_none() && !blank && !out.is_empty() {
                out.push(String::new());
            }
            blank = true;
            continue;
        }
        blank = false;
        let t = line.trim();
        let numbering = t.len() <= 4
            && (t.starts_with('.') || t.ends_with('.'))
            && t.trim_matches('.').chars().all(|c| c.is_ascii_digit())
            && !t.trim_matches('.').is_empty();
        if numbering {
            pending = Some(t.to_owned());
            continue;
        }
        match pending.take() {
            Some(n) => out.push(format!("{n} {t}")),
            None => out.push(line),
        }
    }
    if let Some(n) = pending {
        out.push(n);
    }
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_and_tidying() {
        let rows = vec![
            vec!["Name".into(), "Note".into()],
            vec!["a|b".into(), "two\nlines".into()],
            vec!["".into(), "".into()],
            vec!["short".into()],
        ];
        assert_eq!(
            table(&rows),
            "| Name | Note |\n| --- | --- |\n| a\\|b | two lines |\n| short |  |"
        );
        assert_eq!(
            tidy("Title  \r\n\r\n\r\n.1\nThe intent of this.\n2.\n\nNext item\n\n"),
            "Title\n\n.1 The intent of this.\n2. Next item"
        );
    }
}

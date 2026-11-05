//! Output truncation (decision 16): what goes back to the model is cut to the limits keeping the head and the tail,
//! with a marker saying how much was dropped and where the full text is.

/// Cuts `s` to at most `max_lines` lines and `max_chars` characters (head + tail). Returns the text and whether
/// anything was cut.
pub fn truncate(s: &str, max_chars: usize, max_lines: usize) -> (String, bool) {
    let mut cut = false;
    let lines: Vec<&str> = s.split_inclusive('\n').collect();
    let mut text = if lines.len() > max_lines {
        cut = true;
        let head = max_lines / 2;
        let tail = max_lines - head;
        let omitted = lines.len() - head - tail;
        format!(
            "{}[... {omitted} lines omitted ...]\n{}",
            lines[..head].concat(),
            lines[lines.len() - tail..].concat()
        )
    } else {
        s.to_owned()
    };
    let n = text.chars().count();
    if n > max_chars {
        cut = true;
        let head = max_chars / 2;
        let tail = max_chars - head;
        // Byte offsets of the cut points (no copy of the text as chars).
        let at = |k: usize| text.char_indices().nth(k).map_or(text.len(), |(i, _)| i);
        let (a, b) = (at(head), at(n - tail));
        text = format!(
            "{}\n[... {} characters omitted ...]\n{}",
            &text[..a],
            n - head - tail,
            &text[b..]
        );
    }
    (text, cut)
}

/// SHA-256 of bytes (read-before-edit ledger).
pub fn digest(b: &[u8]) -> [u8; 32] {
    use sha2::Digest;
    sha2::Sha256::digest(b).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_head_and_tail() {
        let s: String = (0..1000).map(|i| format!("line {i}\n")).collect();
        let (t, cut) = truncate(&s, 30_000, 256);
        assert!(cut);
        assert!(t.starts_with("line 0\n") && t.ends_with("line 999\n"));
        assert!(t.contains("[... 744 lines omitted ...]"));
        let (t, cut) = truncate(&"x".repeat(100), 40, 256);
        assert!(cut && t.contains("[... 60 characters omitted ...]"));
        assert_eq!(truncate("short", 10, 10), ("short".into(), false));
        let (t, _) = truncate(&"é".repeat(50), 20, 10);
        assert!(t.starts_with("éééééééééé"), "cuts on characters, not bytes");
    }
}

/// Whether `text` is `read_file`'s numbered view copied back (each line `  12\t...`, the numbers counting up) rather
/// than a file's own text: a weaker model wrote a whole spec so, every line prefixed twice.
pub fn numbered_copy(text: &str) -> bool {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.len() < 3 {
        return false;
    }
    let mut prev: Option<u64> = None;
    let mut counted = 0;
    for l in &lines {
        let Some((n, _)) = l.split_once('\t') else {
            continue;
        };
        let Ok(n) = n.trim_start().parse::<u64>() else {
            continue;
        };
        if prev.is_none_or(|p| n > p) {
            counted += 1;
        }
        prev = Some(n);
    }
    counted * 10 >= lines.len() * 8
}

/// The refusal for [`numbered_copy`] content.
pub fn numbered_refusal(rel: &str) -> crate::ToolError {
    crate::ToolError::new(
        "numbered_lines",
        format!(
            "the content for {rel} still has read_file's line numbers (each line starts with its number and a tab): \
             write the file's own text without them"
        ),
    )
}

#[cfg(test)]
mod numbered_tests {
    use super::*;

    #[test]
    fn read_files_numbers_are_told_from_a_files_own_text() {
        assert!(numbered_copy(
            "     1\t# Spec\n     2\t\n     3\tLine\n     4\tMore\n"
        ));
        assert!(
            numbered_copy("     1\t     1\t# Spec\n     2\t     2\tA\n     3\t     3\tB\n"),
            "twice"
        );
        assert!(!numbered_copy("# Spec\n\nSome text\nmore\n"));
        assert!(!numbered_copy("a\tb\nc\td\ne\tf\n"), "a tab-separated file");
        assert!(
            !numbered_copy("3\tx\n2\ty\n1\tz\n"),
            "numbers not counting up"
        );
        assert!(!numbered_copy("1\tonly two\n2\tlines\n"));
    }
}

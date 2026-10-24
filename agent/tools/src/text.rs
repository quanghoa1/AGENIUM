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
        let chars: Vec<char> = text.chars().collect();
        text = format!(
            "{}\n[... {} characters omitted ...]\n{}",
            chars[..head].iter().collect::<String>(),
            n - head - tail,
            chars[n - tail..].iter().collect::<String>()
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

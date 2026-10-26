//! How `edit_file` finds the text to replace (ADR-102). Models write `\n`, straight quotes and ASCII dashes; files
//! may have CRLF line ends, a byte-order mark, typographic quotes or trailing spaces. The file is matched in a form
//! without those differences, and the edit is spliced into the file as it is: its BOM and line ends are written
//! back, and the lines the edit did not touch keep every byte.

use std::ops::Range;

/// A file's text as edits see it.
pub struct Doc {
    bom: bool,
    crlf: bool,
    /// The text without the BOM; with LF line ends when the file uses CRLF throughout.
    pub text: String,
}

impl Doc {
    pub fn open(raw: &str) -> Doc {
        let (bom, body) = match raw.strip_prefix('\u{feff}') {
            Some(rest) => (true, rest),
            None => (false, raw),
        };
        // Only a file whose every line end is CRLF is normalized; a mixed file is matched as it is.
        let crlf = body.contains("\r\n") && !body.replace("\r\n", "").contains('\n');
        Doc {
            bom,
            crlf,
            text: if crlf {
                body.replace("\r\n", "\n")
            } else {
                body.to_owned()
            },
        }
    }

    /// The text a model sent, in the form of this document's `text`.
    pub fn input(&self, s: &str) -> String {
        if self.text.contains("\r\n") {
            s.to_owned()
        } else {
            s.replace("\r\n", "\n")
        }
    }

    /// The file content to write.
    pub fn save(&self) -> String {
        let body = if self.crlf {
            self.text.replace('\n', "\r\n")
        } else {
            self.text.clone()
        };
        if self.bom {
            format!("\u{feff}{body}")
        } else {
            body
        }
    }
}

/// The ASCII stand-in of a typographic character, if it has one.
fn plain(c: char) -> Option<char> {
    match c {
        '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}' | '\u{2032}' => Some('\''),
        '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{201F}' | '\u{2033}' => Some('"'),
        '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2014}' | '\u{2015}'
        | '\u{2212}' => Some('-'),
        '\u{00A0}' | '\u{2007}' | '\u{202F}' => Some(' '),
        _ => None,
    }
}

/// The loose form of `s` (typographic characters made ASCII, trailing spaces and tabs of each line dropped), with the
/// source byte range of every byte of it.
fn loose(s: &str) -> (String, Vec<Range<usize>>) {
    let mut view = String::with_capacity(s.len());
    let mut src = Vec::with_capacity(s.len());
    let mut line_start = 0;
    for line in s.split_inclusive('\n') {
        let body = line.strip_suffix('\n').unwrap_or(line);
        let body = body.strip_suffix('\r').unwrap_or(body);
        let kept = body.trim_end_matches([' ', '\t', '\u{00A0}']);
        for (i, c) in kept.char_indices() {
            let c2 = plain(c).unwrap_or(c);
            let at = line_start + i..line_start + i + c.len_utf8();
            let mut buf = [0u8; 4];
            for _ in c2.encode_utf8(&mut buf).bytes() {
                src.push(at.clone());
            }
            view.push(c2);
        }
        // The line end itself (\r\n or \n) stays.
        let end_start = line_start + body.len();
        for (i, c) in line[body.len()..].char_indices() {
            view.push(c);
            src.push(end_start + i..end_start + i + 1);
        }
        line_start += line.len();
    }
    (view, src)
}

/// Where `old` occurs in `text`: exactly if it does, else in the loose form. The ranges are byte ranges of `text`,
/// non-overlapping, in order; the flag says the loose form was needed.
pub fn find(text: &str, old: &str) -> (Vec<Range<usize>>, bool) {
    let exact: Vec<Range<usize>> = text
        .match_indices(old)
        .map(|(i, m)| i..i + m.len())
        .collect();
    if !exact.is_empty() {
        return (exact, false);
    }
    let (view, src) = loose(text);
    let (needle, _) = loose(old);
    if needle.trim().is_empty() {
        return (vec![], false);
    }
    let found: Vec<Range<usize>> = view
        .match_indices(needle.as_str())
        .map(|(i, m)| src[i].start..src[i + m.len() - 1].end)
        .collect();
    let loose_used = !found.is_empty();
    (found, loose_used)
}

/// `text` with every range replaced by `new` (ranges in order, non-overlapping).
pub fn splice(text: &str, ranges: &[Range<usize>], new: &str) -> String {
    let mut out = String::with_capacity(text.len() + new.len());
    let mut at = 0;
    for r in ranges {
        out.push_str(&text[at..r.start]);
        out.push_str(new);
        at = r.end;
    }
    out.push_str(&text[at..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(raw: &str, old: &str, new: &str) -> (String, bool) {
        let mut d = Doc::open(raw);
        let (r, loose) = find(&d.text, &d.input(old));
        assert_eq!(r.len(), 1, "{raw:?} / {old:?}");
        d.text = splice(&d.text, &r, &d.input(new));
        (d.save(), loose)
    }

    #[test]
    fn crlf_and_bom_are_kept() {
        let raw = "\u{feff}fn a() {\r\n    1\r\n}\r\n";
        let (out, loose) = edit(raw, "    1\n}", "    2\n}");
        assert_eq!(out, "\u{feff}fn a() {\r\n    2\r\n}\r\n");
        assert!(!loose);
        // The model sent CRLF itself.
        let (out, _) = edit(raw, "    1\r\n", "    3\r\n");
        assert_eq!(out, "\u{feff}fn a() {\r\n    3\r\n}\r\n");
    }

    #[test]
    fn a_mixed_file_is_matched_as_it_is() {
        let raw = "a\r\nb\nc\r\n";
        let (out, _) = edit(raw, "b\nc", "B\nC");
        assert_eq!(out, "a\r\nB\nC\r\n", "untouched line ends stay");
    }

    #[test]
    fn typographic_quotes_dashes_and_trailing_spaces_match_loosely() {
        let raw = "say(\u{201C}it\u{2019}s ok\u{201D})   \nrange 1\u{2013}9\nend\n";
        let (out, loose) = edit(
            raw,
            "say(\"it's ok\")\nrange 1-9",
            "say(\"fine\")\nrange 1-10",
        );
        assert!(loose);
        assert_eq!(out, "say(\"fine\")\nrange 1-10\nend\n");
        // Exact text is preferred, and an untouched typographic line keeps its bytes.
        let raw = "a \u{2014} b\nx\n";
        let (out, loose) = edit(raw, "x", "y");
        assert!(!loose);
        assert_eq!(out, "a \u{2014} b\ny\n");
    }

    #[test]
    fn loose_matches_count_for_uniqueness() {
        let d = Doc::open("it\u{2019}s\nit's\n");
        let (r, loose) = find(&d.text, "it's");
        assert_eq!(r.len(), 1, "the exact one wins");
        assert!(!loose);
        let d = Doc::open("it\u{2019}s\nit\u{2018}s\n");
        let (r, _) = find(&d.text, "it's");
        assert_eq!(r.len(), 2, "two loose matches are ambiguous");
        let (r, _) = find(&d.text, "   ");
        assert!(r.is_empty(), "whitespace alone never matches loosely");
    }
}

//! What makes a skill's text someone else's instructions: phrases of prompt injection, a pipe into a shell, and
//! characters a reader does not see (bidi controls that reorder what is shown, zero-width characters, tag characters
//! that carry hidden text). Every gate on a skill's text and the scorecard's "safe" read it here (NVIDIA
//! SkillEvaluator's review, 2026-11-18: the hidden characters were not looked for).

/// Phrases that turn a skill into someone else's instructions. Specific on purpose: a skill about prompts may name
/// "the system prompt" (that alone was refused before).
const PHRASES: &[&str] = &[
    "ignore previous instructions",
    "ignore all previous",
    "disregard the above",
    "disregard your instructions",
    "you are now",
    "ignore the system prompt",
    "ignore your system prompt",
    "reveal your system prompt",
    "print your system prompt",
    "new system prompt:",
    "| sh",
    "| bash",
    "iex(",
    "invoke-expression",
];

/// What in `text` reads as injected: each phrase it holds, and each kind of hidden character (`U+202E`, …).
pub(crate) fn findings(text: &str) -> Vec<String> {
    let lower = text.to_lowercase();
    let mut out: Vec<String> = PHRASES
        .iter()
        .filter(|p| lower.contains(*p))
        .map(|p| format!("{p:?}"))
        .collect();
    let chars: Vec<char> = text.chars().collect();
    for (i, &ch) in chars.iter().enumerate() {
        let cp = ch as u32;
        // A joiner's neighbours in a script that uses it (Persian, Hindi with its virama, emoji): not ASCII, not space.
        let letter = |j: Option<&char>| j.is_some_and(|c| !c.is_ascii() && !c.is_whitespace());
        let hidden = matches!(cp, 0x202A..=0x202E | 0x2066..=0x2069 | 0x200B | 0x2060 | 0xE0000..=0xE007F)
            // A byte-order mark is fine where a file starts.
            || (cp == 0xFEFF && i > 0)
            // Joiners belong between the characters of some scripts and in emoji sequences.
            || (matches!(cp, 0x200C | 0x200D)
                && !(letter(i.checked_sub(1).and_then(|k| chars.get(k))) && letter(chars.get(i + 1))));
        if hidden {
            let f = format!("a hidden character U+{cp:04X}");
            if !out.contains(&f) {
                out.push(f);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phrases_and_hidden_characters_are_found_and_prompts_may_be_named() {
        assert_eq!(
            findings("Then ignore previous instructions."),
            ["\"ignore previous instructions\""]
        );
        assert!(findings("Write the system prompt in plain words.").is_empty());
        assert_eq!(
            findings("Run the tests\u{202E}gnirts"),
            ["a hidden character U+202E"]
        );
        assert_eq!(findings("a\u{200B}b"), ["a hidden character U+200B"]);
        assert_eq!(findings("x\u{E0041}y"), ["a hidden character U+E0041"]);
        // A joiner between letters (Persian, Hindi) is text; a byte-order mark at the start is a file's.
        assert!(findings("می\u{200C}خواهم क्\u{200D}ष").is_empty());
        assert!(findings("\u{FEFF}# Skill").is_empty());
        assert!(!findings("a \u{200D} b").is_empty());
    }
}

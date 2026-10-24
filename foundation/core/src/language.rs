//! Language policy (ADR-40): the user may talk to the Lead/Solo in any language; everything from the Lead downward
//! (tickets, plans, comments, handoffs, memory, skills, tool-call text) is English. Single source for prompts.

/// Rule injected into every agent's system rules.
pub const ENGLISH_BELOW_LEAD: &str =
    "Language: write every ticket, plan, comment, report, handoff, memory and \
tool-call argument in English, whatever language the user writes in.";

/// Extra rule for agents that talk to the user directly (Lead and Solo). The user's language is not configured:
/// the agent answers in the language the user writes in.
pub const USER_FACING: &str = "Talk to the user in the language they write in. Translate their requests into \
English before writing tickets, plans or anything other agents read, and reply to them in their language.";

/// Very rough check used by evals and diagnostics: share of ASCII letters among letters. Vietnamese text with
/// diacritics scores well below 1.0; English scores ~1.0.
pub fn ascii_letter_ratio(s: &str) -> f64 {
    let (mut ascii, mut all) = (0usize, 0usize);
    for c in s.chars().filter(|c| c.is_alphabetic()) {
        all += 1;
        if c.is_ascii_alphabetic() {
            ascii += 1;
        }
    }
    if all == 0 {
        1.0
    } else {
        ascii as f64 / all as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules() {
        assert!(USER_FACING.contains("language they write in"));
        assert!(ascii_letter_ratio("Thêm đăng nhập bằng Google") < 0.9);
        assert!(ascii_letter_ratio("Add Google login") > 0.99);
    }
}

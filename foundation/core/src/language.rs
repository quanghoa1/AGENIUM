//! Language policy (ADR-40): the user may talk to the Lead/Solo in any language; everything from the Lead downward
//! (tickets, plans, comments, handoffs, memory, skills, tool-call text) is English. Single source for prompts.

/// Rule injected into every agent's system rules.
pub const ENGLISH_BELOW_LEAD: &str =
    "Language: write every ticket, plan, comment, report, handoff, memory and \
tool-call argument in English, whatever language the user writes in.";

/// Extra rule for agents that talk to the user directly (Lead and Solo).
pub fn user_facing_rule(user_language: &str) -> String {
    let lang = user_language.trim();
    if lang.is_empty() || lang.eq_ignore_ascii_case("en") {
        "Talk to the user in English.".to_owned()
    } else {
        format!(
            "Talk to the user in their language ({lang}); translate their requests into English before writing \
             tickets or plans, and translate results back when you reply to them."
        )
    }
}

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
        assert!(user_facing_rule("vi").contains("(vi)"));
        assert_eq!(user_facing_rule(""), "Talk to the user in English.");
        assert!(ascii_letter_ratio("Thêm đăng nhập bằng Google") < 0.9);
        assert!(ascii_letter_ratio("Add Google login") > 0.99);
    }
}

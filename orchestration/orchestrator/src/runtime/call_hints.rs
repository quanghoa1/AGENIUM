//! How to make a refused protocol call right, said with the refusal beyond each field's own message (2026-11-19: a
//! worker cut its report's summary from 997 to 828 to 814 characters for a limit of 800 and was stopped; told to put the
//! long part in a file, it would have been done at once).

use agenium_core::validate::FormErrors;

/// A report's summary over its limit: what goes in a file instead.
const REPORT_SUMMARY: &str = "keep summary to a few sentences, well under its limit: write the full report into a file \
                              inside the paths your ticket may write (its scope_paths), name that file in the summary \
                              and attach it in artifacts as {kind: \"file\", ref: <its path in your worktree>}; each \
                              criterion's evidence can point to the lines of it";

/// Any other text over its limit.
const TOO_LONG: &str = "shorten the text well under its limit (models count characters badly: aim for about two thirds \
                        of it); when it has to be long, write it into a file in your worktree and name the file instead";

pub(super) fn hint(tool: &str, errors: &FormErrors) -> Option<&'static str> {
    let too_long =
        |e: &&agenium_core::validate::FieldError| e.message.starts_with("must have at most");
    let long: Vec<&str> = errors
        .0
        .iter()
        .filter(too_long)
        .map(|e| e.field.as_str())
        .collect();
    if tool == "submit_report" && long.contains(&"summary") {
        Some(REPORT_SUMMARY)
    } else if !long.is_empty() {
        Some(TOO_LONG)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agenium_core::validate::FieldError;

    fn errs(field: &str, message: &str) -> FormErrors {
        FormErrors(vec![FieldError {
            field: field.into(),
            message: message.into(),
        }])
    }

    #[test]
    fn a_long_report_summary_is_told_to_go_in_a_file() {
        let long = "must have at most 800 characters (got 814, 14 too many: shorten it well under the limit)";
        assert_eq!(
            hint("submit_report", &errs("summary", long)),
            Some(REPORT_SUMMARY)
        );
        assert_eq!(
            hint("ask_question", &errs("question", long)),
            Some(TOO_LONG)
        );
        assert_eq!(
            hint("submit_report", &errs("summary", "must not be empty")),
            None
        );
    }
}

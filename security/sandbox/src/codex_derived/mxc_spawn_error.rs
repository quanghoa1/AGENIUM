//! DERIVED FROM CODEX — keep in step with the vendored upstream file:
//!   vendor/codex-sandbox/upstream/codex-rs/mxc-sandbox/src/native.rs (openai/codex @ e5839cbc591df02408d5009e287887c7d57a9167)
//! Licensed under the Apache License 2.0 (OpenAI Codex), see NOTICE.
//! Changes from upstream: the same parsing, as a function Agenium's launcher (`mxc.rs`) calls; the message it builds
//! is Agenium's.

/// The Windows error number of a failed `CreateProcessW` in a process container, read from the SDK's extended error.
/// That field also carries the working directory and other diagnostics, so only the number is ever surfaced.
pub fn create_process_error(detail: &str) -> Option<u32> {
    let number = detail
        .strip_prefix(
            "CreateProcessW(PROC_THREAD_ATTRIBUTE_SECURITY_ENVIRONMENT) failed: WIN32_ERROR(",
        )?
        .split_once(')')?
        .0;
    number.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_number_is_taken_from_the_extended_error() {
        assert_eq!(
            create_process_error(
                "CreateProcessW(PROC_THREAD_ATTRIBUTE_SECURITY_ENVIRONMENT) failed: WIN32_ERROR(5) (working \
                 directory: C:\\Users\\someone\\secret-project)"
            ),
            Some(5)
        );
        assert_eq!(
            create_process_error("BaseContainer job-object setup failed: x"),
            None
        );
        assert_eq!(
            create_process_error(
                "CreateProcessW(PROC_THREAD_ATTRIBUTE_SECURITY_ENVIRONMENT) failed: WIN32_ERROR(abc)"
            ),
            None
        );
    }
}

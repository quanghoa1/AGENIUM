#[test]
fn waiting_on_a_background_command_is_no_loop() {
    assert!(super::polls("shell_output"));
    assert!(super::polls("mcp__agenium__shell_output"));
    assert!(!super::polls("shell"));
    assert!(!super::polls("read_file"));
}

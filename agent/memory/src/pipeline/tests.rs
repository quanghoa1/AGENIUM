/// A memory id the model writes as text still names the memory; anything else names none.
#[test]
fn a_memory_id_written_as_text_is_read() {
    use serde_json::json;
    let ids: Vec<Option<i64>> = [
        json!(2),
        json!("2"),
        json!(" id=7 "),
        json!("two"),
        json!(2.5),
        json!(null),
    ]
    .iter()
    .map(super::memory_id)
    .collect();
    assert_eq!(ids, vec![Some(2), Some(2), Some(7), None, None, None]);
}

/// An extraction answer cut off mid-fact keeps the facts it finished (braces and escapes inside strings do not
/// confuse it); an answer without a single finished fact keeps nothing.
#[test]
fn a_cut_off_extraction_keeps_its_complete_facts() {
    use super::complete_items;
    let cut = r#"{"facts": [{"text": "Owner line {first} is \"Owner: docs-team\".", "kind": "lesson", "entities": [{"name": "docs"}]}, {"text": "Second fact", "kind": "fact"}, {"text": "Third fact is cut off at "#;
    let kept = complete_items(cut, "facts").unwrap();
    assert_eq!(kept.len(), 2);
    assert_eq!(
        kept[0]["text"],
        "Owner line {first} is \"Owner: docs-team\"."
    );
    assert_eq!(kept[1]["text"], "Second fact");
    assert!(complete_items(r#"{"facts": [{"text": "cut"#, "facts").is_none());
    assert!(complete_items("no json at all", "facts").is_none());
}

/// The noisy facts the 2026-10-25 memory-reuse run extracted are refused; the useful ones from the same run pass.
#[test]
fn episode_details_are_not_memories() {
    use super::ephemeral;
    for (text, what) in [
        ("Agent agent_01M5ST0WXECQ69JP9PZX76J1X4 created a personal skill named deliverable-premerge-check.", "an id"),
        ("In the docs project, deliverables/bye.txt was added on main by commit 85471b8 for ticket AGN-2.", "a commit hash"),
        ("deliverables/bye.txt contains two lines; it is 26 bytes, UTF-8 with no BOM.", "an exact file size"),
    ] {
        assert_eq!(ephemeral(text), Some(what), "{text}");
    }
    for text in [
        "In the docs project, deliverables must also satisfy the repo's pre-merge checks (the 'Owner:' first line).",
        "write_file refuses to overwrite an existing file until read_file has been called on it (error code read_first).",
        "Uploads over 50 MB fail on the staging server.",
        "Commit subjects in this project start with the ticket key, for example AGN-12: Add greeting.",
    ] {
        assert_eq!(ephemeral(text), None, "{text}");
    }
}

use super::*;

#[test]
fn json_in_prose() {
    let v =
        json_object("Sure!\n```json\n{\"facts\": [{\"text\": \"a {b} \\\"c\\\"\"}]}\n```").unwrap();
    assert_eq!(v["facts"][0]["text"], "a {b} \"c\"");
    assert!(json_object("no json").is_none());
}

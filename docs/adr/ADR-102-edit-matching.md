# ADR-102: `edit_file` matches what the model meant and writes the file back as it was

Status: Accepted (2026-10-26). Evidence: `agent/tools/src/edit_match.rs` tests and
`agent/tools/tests/tools.rs::edits_keep_line_ends_and_bom_and_match_typographic_text`. Source: the pi coding agent's
edit-tool bug history (`docs/reference/PI-COMPARISON.md`): CRLF files, a byte-order mark and typographic characters
made "text not found" one of its recurring failures, and a fuzzy match once rewrote a whole file in normalized form.

## Decision

1. **Line ends and BOM.** A file whose line ends are all CRLF is matched with LF, as models write it; a leading BOM is
   set aside. Both are written back. A file with mixed line ends is matched as it is, so no line it did not touch
   changes.
2. **Exact first, then loose.** When the text is not found exactly, it is looked for with typographic quotes, dashes
   and non-breaking spaces read as their ASCII form and trailing spaces of each line ignored. The match is mapped back
   to the file's own bytes and only that span is replaced; the rest of the file keeps every byte. A loose match must
   be unique like an exact one (two loose matches are ambiguous), and the tool's answer says the loose form was used.
3. Whitespace alone never matches loosely.

Not done: Unicode normalization (NFC/NFKC) beyond those characters; `apply_patch` keeps exact matching.

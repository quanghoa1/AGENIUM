# ADR-105: Harmless argument mistakes are repaired; read-only calls of one turn run together

Status: Accepted (2026-10-26). Evidence: `agent/tools/src/coerce.rs` tests,
`agent/loop/tests/loop.rs::read_only_calls_run_together_and_arguments_are_repaired`. Source: pi's tool-argument bug
class and its parallel tool execution (`docs/reference/PI-COMPARISON.md`).

## Decision

1. **Repair against the tool's schema, before the call** (work, protocol and MCP tools alike): a number or a boolean
   sent as text (`"40"`, `"true"`), a number or boolean where text is expected, a list or an object sent as a JSON
   string, the whole argument object sent as a JSON string, one item (or one text) where a list is expected. Local
   `$ref`s are followed. Anything doubtful — an unknown field, `"yes"` for a boolean, `"forty"` for a number — is
   left for the tool to refuse as before.
2. **Read-only work tools of one turn run together** (read, list, glob, grep, fetch, read-only MCP tools); results are
   returned in call order and the loop checks (repeated call, errors in a row) apply unchanged. A turn with any
   protocol or writing tool runs one call at a time as before.

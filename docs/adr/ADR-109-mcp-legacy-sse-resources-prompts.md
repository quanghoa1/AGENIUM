# ADR-109: MCP — the legacy HTTP+SSE transport, resources and prompts

Status: Accepted (2026-10-26). Evidence: `agent/mcp/tests/legacy_sse.rs`, `agent/mcp/tests/mcp.rs`
(`resources_and_prompts_are_read_only_tools`), and the official reference server `@modelcontextprotocol/server-everything`
probed in both its `sse` and `streamableHttp` modes with `cargo run -p agenium-mcp --example mcp_probe -- <url>`.

## Decision

1. **Legacy HTTP+SSE (revision 2024-11-05).** A server URL is tried with Streamable HTTP first. When the `initialize`
   POST is refused with 400, 404 or 405, the URL is opened as an event stream: its `endpoint` event names where
   requests are POSTed (it must stay on the server's origin), and the answers come back on the stream, matched by id;
   server requests (ping) are answered. An OAuth token, when the server has one, goes on the stream and on every POST.
2. **Resources and prompts.** A server that declares `resources` gets two read-only tools, `list_resources` and
   `read_resource {uri}` (text verbatim, binary contents as a placeholder); one that declares `prompts` gets
   `list_prompts` and `get_prompt {name, arguments}` (the messages as `[role] text`). They are named like the server's
   own tools (`mcp__<server>__…`), obey the agent's tool filter and are marked untrusted like every MCP output.
3. **Server-sent text is decoded across chunk boundaries** (a character split between two network chunks used to
   become U+FFFD), in both the Streamable HTTP and the legacy stream readers.

Reference server results: over legacy SSE it negotiated 2025-06-18; 16 tools, 7 resources (one read), 4 prompts (one
fetched); over Streamable HTTP the same. Its SDK's newest revision is 2025-11-25, which Agenium already knows; the
"2026-07-28" revision it mentions is not in that SDK, so there is nothing to follow yet.

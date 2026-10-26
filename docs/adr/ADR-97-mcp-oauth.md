# ADR-97: OAuth for remote MCP servers — one sign-in per server, tokens in the vault

Status: Accepted (2026-10-26). Evidence: `agent/mcp/tests/oauth.rs` against `agenium-mcp-oauth-test-server`, and the
CLI run against the same server (below). Amends ADR-62 (MCP keys) for servers that ask for a signed-in user.

## Context

Remote MCP servers (Streamable HTTP) increasingly refuse static keys and ask for OAuth: GitHub's, Linear's, Notion's,
Atlassian's. Until now Agenium sent only the `headers` of the server config, so such a server answered 401 and could
not be used. The MCP specification (revisions 2025-06-18 and 2025-11-25) fixes the flow:

1. the server answers 401 with `WWW-Authenticate: Bearer resource_metadata="…"` (and maybe `scope="…"`);
2. its protected-resource metadata (RFC 9728) names the authorization server, at the challenge's URL or at
   `/.well-known/oauth-protected-resource{path}`;
3. the authorization server's metadata (RFC 8414, or OpenID discovery) gives the endpoints;
4. the client registers itself (RFC 7591) or uses a client registered for it;
5. authorization code flow with PKCE S256 (a server whose metadata does not list S256 is refused), a loopback redirect
   for a native app (RFC 8252), and `resource` = the MCP server (RFC 8707), so the token is valid there only;
6. the access token goes in `Authorization: Bearer`, to that server only; a refresh token renews it.

## Decision

1. **One sign-in per server, by the user.** `agenium mcp login <id>` (CLI) or **Sign in** on the server (app) runs
   the flow above in the user's browser and keeps the tokens in the vault as `mcp-oauth/<server>`. Every agent bound to
   the server uses that sign-in; per-agent keys (ADR-62) stay for servers with static keys. An agent can never sign
   in: a tool call that needs it fails with `mcp_needs_sign_in`, which tells the agent to ask the user.
2. **A token goes only where it was issued for.** The stored tokens name the server URL; a changed URL needs a new
   sign-in. Endpoints must be HTTPS (HTTP only on this machine). A config `Authorization` header wins over OAuth.
   Token values are registered with the redactor when read or written, and `Tokens`' Debug output omits them.
3. **Renewal without the user.** One token holder per server is shared by every connection. An access token within
   30 s of expiry is refreshed before it is sent. After a 401 the holder first takes a newer stored token (a sign-in
   since, or another process's refresh), then refreshes once (rotated refresh tokens are saved), then asks for
   sign-in. A 403 with `insufficient_scope` asks the user to sign in again with the wider scope.
4. **Registration.** Dynamic registration as a public client (`token_endpoint_auth_method: none`) at each sign-in,
   for the loopback redirect of that sign-in. For an authorization server that does not register clients (GitHub),
   the server config takes a pre-registered client: `oauth.client_id`, optional `client_secret` (`${secret:…}`),
   `scopes`, and `callback_port` for clients registered with an exact redirect URI.
5. **Servers of revision 2025-03-26** without resource metadata: the server's origin is the authorization server,
   and without its metadata the default endpoints (`/authorize`, `/token`, `/register`) are used.

6. **The answer must come from the server Agenium asked** (RFC 9207, learned from pi): an `iss` in the sign-in answer
   must name the discovered authorization server, and must be there when its metadata promises it
   (`authorization_response_iss_parameter_supported`). An empty `scope` is treated as none.

Not built: Client ID Metadata Documents (2025-11-25; dynamic registration or a configured client instead), step-up
authorization in place (the user signs in again instead), per-agent sign-ins.

## Test server

`agenium-mcp-oauth-test-server` (not shipped) is the authorization server and the MCP server in one process:
resource and authorization-server metadata, dynamic registration (loopback redirect URIs only), the code flow with
PKCE S256 only, `resource` checked at authorize, token and use, expiring access tokens, rotating refresh tokens, a
pre-registered client `agenium-test-client`, and an Approve/Deny page (or `--auto-approve`). Its tools: `whoami`
(which client and which token answered) and `echo`.

The tests cover: the 401 before sign-in naming the command; discovery, registration and sign-in; tools listed and
called; a server that forgot its access tokens (401 → one refresh → the call goes through, the rotated refresh token
saved); an access token about to expire refreshed before use; every token revoked (the tool says only the user can
sign in); a new sign-in picked up by existing connections; tokens not used for another URL; the user denying on the
page; a pre-registered client used without registration; an unreachable or plain-HTTP remote server refused before a
browser opens.

**CLI run (2026-10-26, Windows, a throwaway root):** `mcp add` → `mcp test` refused with the sign-in hint →
`mcp login --no-browser` printed the authorization URL (response_type, client_id, redirect_uri, code_challenge,
code_challenge_method, state, resource, scope); the approval page was answered, the redirect reached
`http://127.0.0.1:<port>/callback` and the page said Agenium is signed in → `mcp list` shows `(signed in)` →
`mcp test` lists `whoami` and `echo` → no token in plain text under the root → `mcp logout` → `mcp test` asks for
sign-in again → `mcp remove`. Workspace: 306 passed, 0 failed; clippy clean; the desktop app builds and its UI
type-checks (Sign in / Sign out on an HTTP server, a "signed in" chip).

## Consequences

- Remote servers that need OAuth work once the user has signed in, and keep working across restarts.
- The vault holds one more kind of secret; removing a server removes its tokens.
- Testing against a real provider (GitHub, Linear, …) needs the user's account and browser; the test server covers
  the protocol.

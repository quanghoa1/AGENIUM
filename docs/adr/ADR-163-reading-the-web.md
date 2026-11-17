# ADR-163: Reading the web: search, an open fetch, pages as Markdown, sources cited

Status: Accepted (2026-11-18). Owner: "decide and do what is reasonable" after a review of how agents find information
on the internet.

## Context

Agents work on things that change after a model's training: a library's current version and API, a tool's flags, an
error message's fix. What Agenium gave them for it:

1. **No search.** `fetch` read a URL the agent already knew or guessed; the vendor CLIs' own web tools were withheld
   (ADR-56, ADR-108). An agent that could not find a page recalled the answer, often an old one.
2. **An empty allowlist.** `fetch` refused every host not on the project's allowlist, so each site was a
   `request_permission` and a wait. Meanwhile the run's shell commands reach the network by default (`[sandbox]
   network = true`, ADR-95): the allowlist held back the tool that is logged and fenced, not the commands.
3. **Crude pages.** A regex reduced HTML to text: navigation, footers and cookie banners kept, links dropped (the agent
   could not follow one), tables flattened, numeric entities and other charsets garbled. A PDF came back as bytes,
   although `read_file` reads documents (ADR-152). `Agenium-fetch/0.1` was refused by many sites.

A keyless search was tried: DuckDuckGo's HTML endpoint answered with a bot challenge, and getting past one is not
Agenium's to do. Exa's hosted MCP server (`mcp.exa.ai/mcp`, the search OpenCode uses) answers without a key, rate
limited, with good results (ripgrep's release page first, with its date and text).

## Decision

1. **The CLIs that search on their vendor's servers do.** Claude Code's `WebSearch` is allowed (its `WebFetch`, which
   reads pages from this machine, stays off); Codex runs with `web_search = "live"`. Both run on the vendor's side and
   touch nothing on the machine. `Driver::own_web_search` says which CLIs do; Grok's, OpenCode's and Antigravity's
   stay off (not verified here).
2. **`web_search` for the other agents**, free through Exa's hosted MCP server (`initialize`, then `tools/call` of
   `web_search_exa`), no key needed. The user may add one key per API in Settings → Web search, the API told by
   the key: Tavily (`tvly-`), Brave Search (`BSA`), Exa (any other key, sent as `x-api-key`). Searches go to the user's keys first, the strongest first (owner, 2026-11-18): Exa (the richest results,
   a page's text), Tavily (made for agents), Brave Search (its own index, short extracts); Exa's free search always comes
   last, the fallback when the keys are out of their limits or there are none (its limit is counted apart from an
   Exa key's). With all three keys: Exa (key), Tavily, Brave Search, Exa (free). An API that answers 401, 402, 403, 429, 432 or 433
   (or Exa saying its rate limit is reached) hands the search to the next and rests for 15 minutes, for the whole app;
   the answer says which API was passed over. Another failure (a timeout) also goes to the next, without the rest. Kept
   in the secret store (`web-search/exa|tavily|brave-search`), registered with the redactor, never shown again. A result is its title, URL, date when the
   engine knows it and an extract, fenced as untrusted (ADR-142); `recent` limits it to the last day, week, month or
   year. Offered only in runs that have `fetch` (not the read-only runs that withhold it) and not to an agent whose CLI
   searches by itself.
3. **`fetch` follows the commands.** When the run's commands may reach the network (a sandbox with `[sandbox] network`
   on and a network it can give, or a run without a sandbox by the user's choice), `fetch` reaches any public site. The
   private network (loopback, private ranges, link-local and the cloud metadata address, CGNAT; every redirect checked)
   is refused as before unless a host is allowlisted by its exact name. With the network off, the allowlist rules as
   before. The capability manifest says which.
4. **Pages as Markdown.** HTML goes through `htmd` (html5ever): the page's `<main>` (or its only `<article>`) when it
   marks one; navigation, header, footer, aside, forms, scripts and styles dropped; headings, lists, tables, code and
   links kept, links made absolute. The body is decoded in the charset it names (`content-type` or `<meta charset>`).
   A PDF or Office document (by type, or by extension when the server does not call it a page) is read like
   `read_file` reads one. The header line names the final URL. A long page is cut and saved whole, read further with
   `read_file` (as every tool output, ADR-142). The user agent is browser-like and names Agenium.
5. **Look up, cite, check.** A rule for every agent: what may have changed since training is looked up rather than
   recalled; the official documentation or source over a blog; the URL and the version or date it applies to cited;
   a fact the work depends on checked in a second source; a search extract is not the page.

## Consequences

- Every agent searches the web with no setup: Claude Code and Codex with their own search (the vendor's usage),
  the others through the user's search keys first and Exa's free search last (alone when there is no key).
  Queries go to those APIs.
- A page an agent reads can try to steer it (prompt injection) or ask it to send data to a URL. Pages are fenced as
  data (ADR-142) and only read with GET, without cookies or credentials; the commands of the same run could reach the
  same hosts. A project that wants neither sets `[sandbox] network = false`: commands get no network and `fetch` only
  the allowlist.
- The browser tool (ADR-158) keeps its allowlist: it is for checking the app the agent builds.
- Not done: the API providers' own search (Anthropic's and OpenAI's server tools, Gemini's grounding), which needs
  each wire's server-tool support; a judge extracting the part of a page a question needs; a JavaScript-only page
  falling back to the browser.

## Verification

- `agent/tools`: a page reads as its main part in Markdown with absolute links and without its chrome; bodies decode
  in their charset; documents are told by type or extension; an open fetch still refuses loopback and `localhost`;
  `web_search` sends a Tavily key as a bearer token with `time_range`, reads Tavily's and Brave's results the same way
  and refuses a bad `recent`; without a key it goes to Exa (`initialize`, a session, `tools/call` without a key, the
  event-stream answer read as results).
- `agent/cli-backend`: Claude Code's `WebSearch` is offered and allowed, `WebFetch` refused; Codex's config has live
  search.
- `agent/tools`: an API out of its limit (Tavily's 432) hands the search to the next (Brave) and the answer says so; the
  next search passes over the resting API; the order is the keys (Exa, Tavily, Brave) and
  the free Exa last, an Exa key's too.
- `orchestrator`: no key means Exa alone; keys of several APIs are kept one per API, in order, and removed one by one;
  the tool is not offered to a CLI that searches.
- `quality/eval/examples/web_research_real.rs`: real pages (a documentation page, an RFC as PDF, the metadata address
  refused), real searches with `recent` (Exa without a key; Tavily and Brave each with a key from the environment) and a real Solo agent on Claude Code and on Codex finding ripgrep's latest release (checked against GitHub's
  API) with its own search, reading it with `fetch` without asking for network access, and citing the URL.
- The UI smoke test: Exa alone and free; a Brave and a Tavily key added in Settings join the order behind it; one is
  removed alone.

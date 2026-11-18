# ADR-166: The project map, built by the project's agents and checked by Agenium

Status: Accepted (2026-11-18). Owner:
- a map of how a project's parts connect, as a code graph or system map does, made for a team of agents;
- agents add to it while they work, and Agenium verifies what they add;
- one map per project, kept outside the project's repository;
- the user only looks at it and searches it, in a layout like BloodHound's;
- the memory graph page goes, because users could not read it.

## Context

When an agent fixes or extends code, it first has to find how the parts connect: what calls this function, where a
request goes. Research and system work need the same thing for things that are not code. For example, a vendor has a
web app, a desktop app, a CLI and an API, and they live on and talk to certain domains. Agents found all of this again
on every ticket, and what one learned was lost to the next.

Tools that build a code graph by static analysis were tried on this repository and dropped:
- GitNexus: heavy, license-encumbered.
- Graphify: on Rust it linked 9 of the 20 files that use `WorkReport`, and it missed callers.

A wrong map misleads more than no map does. Static analysis also cannot map anything that is not code.

## Decision

1. **One map per project, made by its agents, checked by Agenium.** It is stored in `core.db` (`map_node`,
   `map_link`, migration 9), not in the repository, so only the map tools change it.
   - **A node** is `kind:name`:
     - `file:<path>`;
     - `symbol:<path>#<name>`;
     - any other lowercase kind for what is not code: `product:Claude Desktop`, `domain:api.anthropic.com`,
       `service:billing`.
   - **A link** joins two nodes with a type (`calls`, `imports`, `requests`, `hosted_at`…) and carries its evidence.

2. **Agenium checks everything before keeping it** (`map/verify.rs`, `map/tools.rs`):
   - **A file** must exist in the run's tree.
   - **A symbol** must be declared in its file, by the outline `find_code` reads code with. A container (a class, an
     impl) spans its members.
   - **A link in code** must cite one to 30 lines of the source's own declaration, or of its file, and those lines must
     name the target as a whole identifier. Evidence in another file, outside the declaration, or not naming the target
     is refused, and so is a target that does not exist.
   - **Anything else** must be named by its evidence:
     - a page, which Agenium fetches itself under `fetch`'s rules; the page must name both ends of the link;
     - or cited code, for example a domain in a config file.

     A link of code cannot rest on a page.
   - **What is refused comes back with the reason**, so the agent can fix it and add it again.
   - **A run adds at most 80 nodes and 160 links,** so agents map what matters rather than everything.

3. **Lifecycle.**
   - **While a ticket is open**, what it added is *tentative*.
   - **When the ticket closes**, its entries are confirmed, and the map's code evidence is checked again on the main
     branch (`map_settle` at each runtime tick).
     - Code that moved: the link follows it to the first line in the source's declaration that names the target.
     - Code that no longer names the target: the link becomes *stale*. It is kept, drawn dashed, and agents are told to
       check it again.
   - **A cancelled ticket's** tentative entries are dropped.
   - **A run without a ticket** (a chat) adds confirmed entries at once.
   - **`map_retract`** takes back what the agent's own ticket added. Anything else goes only once its evidence no
     longer holds; while it still holds, the retraction is refused with `still_holds`.

4. **Agents use it** with three tools:
   - `map_query`:
     - find nodes (`kind:` keeps one kind);
     - list a node's links by direction and type, up to depth 3;
     - what depends on a node (`impact`);
     - the chain from one node to another (`path_to`).

     Every link comes with its evidence, to read before relying on it.
   - `map_add` and `map_retract`, as described above.
   - **In the work order:** a ticket's first message carries the links of the nodes its text or scope names (at most
     24).
   - **A rule in the prompt:** ask the map before tracing from scratch, and add what was traced once it holds.

5. **The user looks: the Map page** (Monitor → Map), laid out as BloodHound's Explore page.
   - **The canvas fills the page.**
   - **Search and Path float at the top left.** Search suggests nodes as you type, each with its kind; `kind:` keeps
     one kind. Path takes a start and an end, with a swap button.
   - **The node panel is on the right**, with sections:
     - Information;
     - Outbound and Inbound, grouped by link type with counts; each group can be put on the canvas;
     - Evidence: the cited lines as the main branch has them, the cited line marked, or a page opened in the app's
       Browser.
   - **Selection and menu:** clicking a node dims what does not touch it. A right-click starts or ends a path, shows what
     is around the node, or copies its name.
   - **Layouts:** Sequential (the default; columns along the links' direction, as BloodHound's) or Free (force).
   - **Line style:** solid for verified, dashed for tentative or stale.
   - **Size:** a view draws at most 200 nodes (the most connected, a node's surroundings, or a path), and the rest are
     counted.
   - **Read-only:** nothing on the page changes the map.

6. **The memory graph page is removed:**
   - the Memory page's Graph tab;
   - `memory_graph` and its strings.

   Graph memory itself (ADR-51) works as before.

## Not done

- No static-analysis indexer: no whole-repository graph built from parsing.
- No editing of the map by the user.
- No map shared between projects.

## Consequences

- Only what agents traced is on the map. It grows with the work instead of being complete at once, and every piece on
  it was checked.
- Re-checking on each close reads the cited files once per link, which is cheap for maps of a few thousand links. Web
  evidence is not re-fetched on close.
- Tests:
  - `orchestration/orchestrator/tests/runtime/map.rs`:
    - a scripted agent's adds through a real run: two kept, two refused with their reasons;
    - `map_query`'s impact;
    - confirmation on close;
    - code that moved, and code that went stale;
    - a cancelled ticket's entries dropped;
    - a page-backed link kept and a link the page does not name refused;
    - `still_holds` on a retraction.
  - `map/refs.rs` and `map/verify.rs`: unit tests.
  - The UI smoke test (step 8c): the page draws a map, its search, the node panel with the cited code, and the path
    menu.
  - `quality/eval/examples/map_real.rs`: three kinds of work with a real CLI agent: a new project mapped and then a fix
    that asks the map first, a review of existing code mapped from the API down to the database, and research mapped
    from pages. On 2026-11-18 it passed with Claude Code (Haiku) and with Codex (gpt-6-luna).
    - Both mapped the new tool's modules, functions, calls and imports (6 to 8 nodes, 4 to 6 code links).
    - The review's chain `api.handle → orders.order_total → db.find_order` was found as a path on the map.
    - The research mapped ripgrep's repository, releases and guide from GitHub pages Agenium fetched (4 to 6 links).
    - All were confirmed when the tickets closed; nothing was stale.
    - The fix ticket started with the map's links in its work order. Codex also asked `map_query` twice.
  - The store and map tests also pass on Ubuntu 25.10.

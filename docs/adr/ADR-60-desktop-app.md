# ADR-60: Desktop app — Tauri 2 shell, own workspace, UI from the AGENIUM-UX mockup

Status: Accepted (2026-10-24, owner's request: build the UI from `D:\Work\AGENIUM-UX`, a mockup, adapted to Agenium)

## Context

The architecture fixes the UI stack: Tauri 2 + React + TypeScript (Vite), realtime through Tauri events, i18n
en + vi. The owner's mockup (`AGENIUM-UX`, Next.js + Tailwind, demo data only, 19 views, no routing) shows the wanted
look and screens, but also concepts Agenium does not have (workspaces, monthly budgets, verified vs claimed
progress, an inter-agent mailbox, config versioning, capability probes per effort level, a watchdog incident model).

## Decision

1. **Location and build.** `apps/desktop` is its own Cargo workspace (excluded from the root one): WebView
   dependencies (WebView2, webkit2gtk) stay out of the core build and the Linux/WSL CI. `apps/desktop/ui` is the Vite
   app (npm; built into `ui/dist`, embedded with the `tauri/custom-protocol` feature). Tauri 2 builds with the
   `x86_64-pc-windows-gnu` toolchain (verified).
2. **Thin Rust side.** `src/api.rs` commands call the same App/Runtime methods as the CLI (rules stay in the
   orchestrator); read models come from `views` (board, plan, audit, `needs_you`), `inbox` and store queries.
   `src/state.rs` holds the opened root, the runtime, a `UiApprover` (shell commands wait for the user in the app,
   denied after 30 min), and the autopilot (scheduler rounds for every project while on). Committed events go to
   the UI as `agenium://event`, live run output as `agenium://run`, command approvals as `agenium://approval`.
   Capabilities: `core:default` for the main window (events need it). Pre-merge checks are not run from the app yet
   (`NoCheckFactory`). The kill switch cancels live runs and re-arms after 5 s (`Cancel::reset`).
3. **Mockup → Agenium mapping.** Kept: the shell (collapsible sidebar with grouped navigation and badges, top bar
   with project selector, search/command palette, autopilot, stop-all, language and theme), the visual language
   (tokens light/dark, mono micro-labels, semantic colours), the metric cards, the attention queue, the agent
   matrix, the consultation console. Mapped: Overview + Watchdog → dashboard and "Needs you" (derived, actionable:
   spec/plan approval, permissions outside the allowlist, user questions, merge review, blocked agents,
   escalations, command approvals); Task Ledger → tickets (board by `TicketStatus`, plan by milestone, Lead inbox)
   and a ticket view (AC, timeline by comment kind with inline answers, runs, audit, user overrides); Executions →
   runs (steps, JSON, live stream); Governance → audit trail (Markdown export) and usage (display only); Teams →
   create from a template (Lead / planner / workers, per-role models) or a Solo agent. Dropped: workspaces, budgets,
   verified-vs-claimed progress, mailbox, config history, per-effort capability probes.
4. **Browser mode.** Outside Tauri the UI answers from `ui/src/lib/demo.ts` (read-only demo data) — for layout work
   and headless screenshots (`msedge --headless --screenshot`).
5. **Testing.** The real app is driven through WebView2's DevTools port (`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=
   --remote-debugging-port=9222`): IPC calls, keyboard input, screenshots, exception capture (scripts kept out of the
   repo for now). Found that way: Chromium's `scrollIntoView` now returns a Promise (became a React effect cleanup and
   blanked the chat view), and `listen()` needs the `core:default` capability.

## Consequences

- Verified end to end on a real root: every read command, a Vietnamese request typed into the Lead console with
  the reply from DeepSeek, the spec approval appearing in "Needs you" through live events.
- Open: Monaco diff and xterm terminal, drag-and-drop on the board, markdown rendering in chat, the agent memory
  explorer, skills/MCP forms, notifications, installer/updater, a scripted UI test suite in CI, full i18n of
  secondary strings.

## Motion (2026-10-24, owner's request: "animate-ui, smooth and alive")

- Learned the rules from the animate-ui skill (`skills/animate-ui/SKILL.md`): subtle level with an "alive" feel, one
  token set for every animation, transforms and opacity only, `motion` 14 for the animations, the whole app inside
  `MotionConfig reducedMotion="user"` (fades and blurs stay, movement goes). Owner's decision: learn the logic, do not
  reuse their code; the first pass imported `@animate-ui/react` (tokens, sliding/counting numbers) and was replaced
  the same day by our own tokens (`spring`, `tx` in `components/motion.tsx`), `RollingNumber` and `CountUp`.
- Applied: route entrance (`tx.reveal` + `spring.entrance`, out of a 2px blur); one gliding selection for the
  sidebar, every segmented control and tab bar (`layoutId`, `spring.selection`, labels above it); tab panels taking
  turns (`tx.panelOut/panelIn`, 5px blur, 12px slide by direction); lists that add, remove and close gaps (needs,
  board cards, ticket timeline, chat: `layout="position"`, `spring.reflow`, `tx.fade`); counting numbers on the
  dashboard (`CountUp`) and rolling, bumping badges in the sidebar (`RollingNumber`, `spring.bump`); progress bars
  on `spring.progress` (scaleX); the command palette (grows from 0.9 on `tx.menu`, the selected result's background
  gliding on `spring.highlight`); the confirm dialog (tipped from `perspective(500px) rotateX(-20deg) scale(0.8)` on
  `spring.dialog`, dismissed on `tx.dismiss`); toasts; press feedback in CSS (0.97 controls, 0.985 cards and rows,
  ease-out 150 ms, none under reduced motion), hover = colour/border/shadow only.
- HTML5 drag-and-drop stays on plain elements inside the animated wrappers (Motion's own drag props would take the
  native drag events).
- Verified on the real app through the DevTools port, frames mid-animation: sidebar highlight mid-glide with labels
  above it, tab surface sliding while the panel comes out of its blur, palette fading and growing; under emulated
  reduced motion the route appears in place and the highlight jumps. No runtime errors on any view.
- Cost: the UI bundle grows from ~114 kB to ~168 kB gzipped (one chunk; a desktop app loads it from disk).

## Live tab and the pixel office (2026-10-24, owner's requests: "a tab to see the teams and what each agent is
doing"; "the office like pixel-agents"; the owner's layout sketch: rooms, office left, inspector and chat right)

- The window opens maximized (the owner designs on a 2560x1440 screen); F11 toggles real fullscreen
  (`core:window:allow-set-fullscreen`).
- `Trực tiếp` (Live): telemetry (agents running, tools being called, MCP servers enabled in the room, tokens today,
  Pause all), room tabs (the shared Solo office with the pool, one office per team, all rooms at once as "CCTV").
  Left (62 %): the room's pixel office (member chips, zoom fit/2x/3x/4x), and below it the room log (domain events
  plus the tool / skill / MCP calls streamed from runs) or the room's plan. Right (38 %): the selected agent: role,
  team and Lead, status (THINKING / CALLING TOOL / WRITING / WAITING / IDLE / OFFLINE), task; 1 live thinking (or a
  note that the model does not stream its reasoning); 2 the calls of its current or last run, classified as MCP
  (`mcp__<server>__<tool>`), Skill (`load_skill`), Agenium (protocol tools) or Tool, with arguments, output and
  duration (`ms`, now recorded in the run log); 3 the conversation its role allows: chat with a Solo agent or a team's
  Lead (the Lead's delegations inline, each with "watch"), a worker read-only (its message bus: the assignment from the
  Lead, questions, answers, reports, reviews) with "message the Team Lead about @worker", which opens the Lead's chat
  with `@worker (TICKET): ` typed and focused. IPC: `apps/desktop/src/live.rs` (`live`, `agent_bus`, `agent_trace`).
- The office (`components/PixelOffice.tsx`): the rules learned from pixel-agents (top-down tile room, back wall with
  shelves, clock and pictures, wooden floor, desks with CRT screens, characters that sit at their desk, walk on free
  tiles and rest in the lounge, bubbles for their state); the sprites and furniture are our own (hand-made 12x18
  character maps with three hair styles and seeded colours, furniture from rectangles), BFS paths on the tile grid,
  integer zoom. Team rooms: the Lead's wider desk and crown, a whiteboard with the plan's progress, dashed delegation
  lines to the workers holding tickets, a sign over each desk with the MCP / Skill / Tool being called; envelopes fly
  for comments between the Lead and a worker. Under reduced motion nobody walks and nothing animates.
- Found on the way: `scrollIntoView` in a nested list scrolls the page around it (the CCTV view jumped to the log):
  lists now scroll their own container; CLI detection ran on the UI thread at every visit of the providers page (now
  async, cached).

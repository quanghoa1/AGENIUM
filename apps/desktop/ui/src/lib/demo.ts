// Demo answers for a plain browser (no Tauri): a small project shaped like real Agenium data, for layout work and
// screenshots. Read-only: actions resolve without changing anything.
import type {
  PriceView,
  VendorUsage,
  AgentEntry,
  Capabilities,
  ChatLine,
  Column,
  EventRecord,
  InboxItem,
  Overview,
  PlanView,
  ProjectSummary,
  RunDetail,
  RunRow,
  Status,
  TicketDetail,
  TicketStatus,
  Usage,
} from "./types";

const P = "proj_01DEMO";
const now = Date.now();
const iso = (minAgo: number) => new Date(now - minAgo * 60_000).toISOString();
const usage = (i: number, o: number, c = 0): Usage => ({
  tokens_in: i,
  tokens_out: o,
  cache_read: Math.round(i * 0.6),
  cache_write: 0,
  estimated_cost_usd: c,
  elapsed_ms: 0,
});

const ids = {
  lead: "agent_01LEAD",
  planner: "agent_01PLAN",
  backend: "agent_01BACK",
  frontend: "agent_01FRONT",
  solo: "agent_01SOLO",
};

const agents: AgentEntry[] = [
  ["shop-lead", ids.lead, "coding-lead", "lead", "Idle", ["planning", "review"], { provider: "opencode-go", model: "deepseek-v4.1-flash", effort: "high" }, "Api"],
  ["shop-planner", ids.planner, "coding-planner", "worker", "Idle", ["planning"], { provider: "opencode-go", model: "deepseek-v4.1-flash", effort: "high" }, "Api"],
  ["shop-backend", ids.backend, "coding-backend", "worker", "Working", ["backend", "api", "db"], { provider: "codex", model: "gpt-6-luna", effort: "medium" }, "Cli"],
  ["shop-frontend", ids.frontend, "coding-frontend", "worker", "Waiting", ["frontend", "web", "ui"], { provider: "claude", model: "haiku", effort: "medium" }, "Cli"],
].map(([name, id, role, kind, status, tags, model, backend]) => ({
  agent: { id, name, team: "team_01", role, role_kind: kind, tags, status, backend, model, skills: [], created_at: iso(900) } as any,
  project: P,
  project_name: "shop",
  team: "Coding team",
  solo: false,
  is_lead: id === ids.lead,
  is_planner: id === ids.planner,
}));

const runs: RunRow[] = [
  { id: "run_05", ticket: "t2", agent: ids.backend, agent_name: "shop-backend", provider: "codex", model: "codex:gpt-6-luna", started_at: iso(3), ended_at: null, outcome: null, steps: 7, usage: usage(48_000, 2_100) },
  { id: "run_04", ticket: "t3", agent: ids.frontend, agent_name: "shop-frontend", provider: "claude", model: "claude:haiku", started_at: iso(9), ended_at: iso(6), outcome: "Parked", steps: 4, usage: usage(31_000, 900) },
  { id: "run_03", ticket: "t1", agent: ids.lead, agent_name: "shop-lead", provider: "opencode-go", model: "deepseek-v4.1-flash", started_at: iso(22), ended_at: iso(21), outcome: "Completed", steps: 3, usage: usage(22_000, 600, 0.004) },
  { id: "run_02", ticket: null, agent: ids.lead, agent_name: "shop-lead", provider: "opencode-go", model: "deepseek-v4.1-flash", started_at: iso(40), ended_at: iso(39), outcome: "Completed", steps: 5, usage: usage(19_000, 1_400, 0.003) },
  { id: "run_01", ticket: null, agent: ids.planner, agent_name: "shop-planner", provider: "opencode-go", model: "deepseek-v4.1-flash", started_at: iso(46), ended_at: iso(44), outcome: "Completed", steps: 6, usage: usage(26_000, 2_800, 0.005) },
];

const card = (key: string, title: string, status: TicketStatus, extra: Partial<Column["cards"][number]> = {}) => ({
  status,
  card: {
    key,
    title,
    kind: "Task" as const,
    priority: 2,
    plan_item: "PI-1",
    assignee: null,
    blocked_by: [],
    open_questions: 0,
    approved: false,
    merged: false,
    escalated: false,
    reopen_count: 0,
    ...extra,
  },
});

const cards = [
  card("SHOP-1", "Product list API: GET /products with paging", "Closed", { plan_item: "PI-1", assignee: "shop-backend", approved: true, merged: true, priority: 1 }),
  card("SHOP-2", "Cart API: add, remove, total", "InProgress", { plan_item: "PI-2", assignee: "shop-backend", priority: 1 }),
  card("SHOP-3", "Product list page reading /products", "Pending", { plan_item: "PI-3", assignee: "shop-frontend", open_questions: 1 }),
  card("SHOP-4", "Cart page with live total", "Assigned", { plan_item: "PI-4", assignee: "shop-frontend", blocked_by: ["SHOP-2"] }),
  card("SHOP-5", "Checkout summary e-mail template", "New", { plan_item: "PI-5", priority: 3 }),
];

const statuses: TicketStatus[] = ["New", "Assigned", "InProgress", "Pending", "Resolved", "Reopened", "Closed", "Cancelled"];

const board: Column[] = statuses.map((s) => ({ status: s, cards: cards.filter((c) => c.status === s).map((c) => c.card) }));

const demoUsage: VendorUsage[] = [
  { vendor: "claude", windows: [{ window: "5h", used_percent: 9, resets_at: iso(-133) }, { window: "week", used_percent: 18, resets_at: iso(-4000) }], updated_at: iso(12) },
  { vendor: "codex", plan: "plus", windows: [{ window: "5h", used_percent: 64, resets_at: iso(-80) }, { window: "week", used_percent: 31, resets_at: iso(-7000) }], note: "2 reset credit(s)", updated_at: iso(3) },
  { vendor: "agy", windows: [], note: "Antigravity does not report its quota to other apps", updated_at: "" },
  { vendor: "opencode-go:opencode-go", plan: "Go", windows: [{ window: "5h", used_percent: 86, resets_at: iso(-45) }, { window: "week", used_percent: 40, resets_at: iso(-5000) }], updated_at: iso(3) },
];

const demoPrices: PriceView = {
  source: "https://models.dev/api.json",
  fetched_at: iso(60),
  providers: {
    "opencode-go": {
      "deepseek-v4.1-flash": { input: 0.15, output: 0.6, cache_read: 0.003, cache_write: 0.1875 },
      "beam-free": { input: 0, output: 0, cache_read: 0, cache_write: 0 },
    },
  },
};

const projects: ProjectSummary[] = [
  {
    id: P,
    name: "shop",
    path: "C:\\Users\\you\\.agenium\\projects\\shop",
    key_prefix: "SHOP",
    status: "Active",
    main_branch: "main",
    team: { id: "team_01", name: "Coding team", solo: false, template: "coding@1", lead: ids.lead, members: 4 },
    tickets: 5,
    by_status: { Closed: 1, InProgress: 1, Pending: 1, Assigned: 1, New: 1 },
    plan_items: 5,
    plan_done: 1,
    plan_version: 1,
    spec_version: 1,
    needs_you: 2,
    inbox: 1,
  },
];

const overview: Overview = {
  needs: [
    { project_name: "shop", item: { kind: "Permission", project: P, ticket: "SHOP-2", comment: "c1", text: "SHOP-2: Network permission requested: npm install express (registry.npmjs.org)", at: iso(4) } },
    { project_name: "shop", item: { kind: "Question", project: P, ticket: "SHOP-3", comment: "c2", text: "SHOP-3: Should the product list show prices with VAT?", at: iso(6) } },
  ],
  agents: agents.map((a) => ({
    id: a.agent.id,
    name: a.agent.name,
    role: a.agent.role,
    role_kind: a.agent.role_kind,
    status: a.agent.status,
    backend: a.agent.backend,
    model: a.agent.model,
    current:
      a.agent.id === ids.backend
        ? { project: "shop", ticket: "SHOP-2", title: "Cart API: add, remove, total" }
        : a.agent.id === ids.frontend
          ? { project: "shop", ticket: "SHOP-3", title: "Product list page reading /products" }
          : null,
  })),
  live_runs: runs.filter((r) => !r.ended_at),
  recent_runs: runs,
  usage_today: usage(146_000, 7_800, 0.012),
  runs_today: runs.length,
  command_approvals: [
    { id: 1, project: P, ticket: "SHOP-2", agent: "shop-backend", command: "npm test -- cart", cwd: "…/worktrees/SHOP-2", at: iso(1) },
  ],
};

const plan: PlanView = {
  spec: { version: 1, body: { goals: ["A small shop: product list, cart, checkout e-mail"], requirements: [{ id: "REQ-1", text: "Customers browse products" }, { id: "REQ-2", text: "Customers manage a cart" }] }, approved_by: "user", locked_at: iso(60) },
  plan: { version: 1, spec_version: 1, reason: null, approved_by: "user", proposed_by: "agent:" + ids.lead, body: { milestones: [{ id: "M1", title: "Browse and cart" }, { id: "M2", title: "Checkout" }] } },
  current_plan_version: 1,
  items: [
    { id: "PI-1", milestone: "M1", title: "Product list API", done: true, tickets: [["SHOP-1", "Closed"]] },
    { id: "PI-2", milestone: "M1", title: "Cart API", done: false, tickets: [["SHOP-2", "InProgress"]] },
    { id: "PI-3", milestone: "M1", title: "Product list page", done: false, tickets: [["SHOP-3", "Pending"]] },
    { id: "PI-4", milestone: "M1", title: "Cart page", done: false, tickets: [["SHOP-4", "Assigned"]] },
    { id: "PI-5", milestone: "M2", title: "Checkout e-mail", done: false, tickets: [["SHOP-5", "New"]] },
  ],
};

const inbox: InboxItem[] = [
  { kind: "Assign", ticket: "SHOP-5", created_at: iso(30), text: "SHOP-5 (Checkout summary e-mail template) has no worker: assign_ticket to a matching team member" },
];

function ticket(key: string): TicketDetail {
  const c = cards.find((x) => x.card.key === key) ?? cards[0];
  return {
    ticket: {
      id: "t-" + key,
      key,
      project: P,
      kind: "Task",
      title: c.card.title,
      description: "Implement it inside the ticket scope with tests that cover the acceptance criteria.",
      acceptance: [
        { id: "AC-1", text: "The endpoint answers with the documented JSON shape" },
        { id: "AC-2", text: "Tests cover the happy path and one error" },
      ],
      status: c.status,
      assignee: null,
      plan_item: c.card.plan_item,
      priority: c.card.priority,
      depends_on: c.card.blocked_by,
      reopen_count: 0,
      escalated: false,
      required_role: null,
      required_tags: [],
      scope_paths: ["src/cart/**", "tests/cart/**"],
      report_template: "coding@1",
      approved: c.card.approved,
      merged: c.card.merged,
      has_code: true,
      close_reason: null,
      usage: usage(48_000, 2_100),
      created_at: iso(38),
      updated_at: iso(3),
    },
    assignee_name: c.card.assignee,
    comments: [
      { id: "c0", kind: "System", author: "system", body: { type: "System", data: { text: "Run started on worktree agenium/" + key } }, resolved: false, created_at: iso(10) },
      { id: "c2", kind: "Question", author: "shop-frontend", body: { type: "Question", data: { question: "Should the product list show prices with VAT?", category: "Requirement", blocking: true, options: [{ label: "With VAT" }, { label: "Without VAT" }] } }, resolved: false, created_at: iso(6) },
    ],
    runs: runs.slice(0, 2),
    workspace: { worktree: "…/worktrees/" + key, branch: "agenium/" + key, base_sha: "4f2a9c1", head_sha: null, removed: false },
    audit: [
      { at: iso(38), actor: "shop-lead", ticket: key, action: "TicketCreated", intent_id: "run_02:call_1", accepted: true, detail: {} },
      { at: iso(37), actor: "shop-lead", ticket: key, action: "TicketAssigned", intent_id: "run_02:call_3", accepted: true, detail: {} },
      { at: iso(8), actor: "shop-backend", ticket: key, action: "submit_report", intent_id: "run_05:4", accepted: false, detail: { error: "invalid: AC-2 missing" } },
    ],
  };
}

const runDetail = (id: string): RunDetail => ({
  run: runs.find((r) => r.id === id) ?? runs[0],
  ticket_key: "SHOP-2",
  log: [
    { n: 1, kind: "tool", at: iso(3), data: { tool: "read_file", args: { path: "src/cart/mod.rs" }, ok: true, result: "pub fn total(…)" } },
    { n: 2, kind: "tool", at: iso(2), data: { tool: "edit", args: { path: "src/cart/mod.rs" }, ok: true, result: "1 replacement" } },
    { n: 3, kind: "tool", at: iso(2), data: { tool: "shell", args: { command: "npm test -- cart" }, ok: false, result: "command_denied: waiting for approval" } },
  ],
});

const events: EventRecord[] = [
  { seq: 41, kind: "TicketStatusChanged", ticket: "t2", actor: "system", payload: { key: "SHOP-2", from: "Assigned", to: "InProgress" }, at: iso(3) },
  { seq: 40, kind: "CommentAdded", ticket: "t3", actor: "agent:" + ids.frontend, payload: { key: "SHOP-3", kind: "Question" }, at: iso(6) },
  { seq: 39, kind: "TicketMerged", ticket: "t1", actor: "system", payload: { key: "SHOP-1", sha: "9b1e0d4" }, at: iso(21) },
  { seq: 38, kind: "PlanApproved", ticket: null, actor: "user", payload: { project: P, version: 1, by: "user" }, at: iso(42) },
];

const capabilities: Capabilities = {
  providers: [{ id: "opencode-go", kind: "openai_compat", base_url: "https://opencode.ai/zen/go/v1", has_key: true, models: [], max_concurrency: 4 }],
  clis: [
    { kind: "Codex", path: "C:\\…\\codex.exe", version: "0.163.0", login: "LoggedIn", compatible: true },
    { kind: "Claude", path: "C:\\…\\claude.exe", version: "2.1.310", login: "LoggedIn", compatible: true },
    { kind: "Agy", path: "C:\\…\\agy.exe", version: "1.4.4", login: "LoggedIn", compatible: true },
  ],
  templates: [],
  cli_fallback: { provider: "opencode-go", model: "deepseek-v4.1-flash" },
  cli_models: { opencode: ["deepseek-v4.1-flash-free", "glm-5.3-flash-free"], codex: ["gpt-6.1-sol", "gpt-6-luna"], claude: ["haiku", "sonnet", "opus"], agy: ["gemini-3.8-flash-low"] },
};

const chat: ChatLine[] = [
  { role: "user", text: "Tôi cần một cửa hàng nhỏ: danh sách sản phẩm, giỏ hàng và e-mail xác nhận." },
  { role: "assistant", text: "Tôi đã đề xuất spec v1 (3 yêu cầu). Bạn duyệt spec, planner sẽ phác plan để tôi xem lại và tạo plan." },
];

const status: Status = { root: "C:\\Users\\you\\.agenium", ready: true, version: "0.1.0", ui_language: "vi" };

export async function demo(cmd: string, args?: Record<string, unknown>): Promise<unknown> {
  await new Promise((r) => setTimeout(r, 60));
  switch (cmd) {
    case "vendor_usage":
    case "refresh_vendor_usage":
    case "read_claude_usage":
      return demoUsage;
    case "memory":
      return {
        hide_below: 0.2,
        overlays: [],
        facts: [
          { id: 12, text: "Breaking changes go first in release notes.", kind: "lesson", scope: { project: "shop" }, cue: "writing release notes", evidence: "Reopened by the review", source: "ticket:SHOP-4", untrusted: false, confidence: 0.7, weight: 0.95, created_at: iso(3000), used_at: iso(60), validated: ["p1/ticket:SHOP-6", "p1/ticket:SHOP-9"] },
          { id: 7, text: "The shop API runs on port 8080 in development.", kind: "fact", scope: {}, source: "ticket:SHOP-2", untrusted: false, confidence: 0.55, weight: 0.8, created_at: iso(9000) },
        ],
      };
    case "memory_action":
      return { ok: true };
    case "decide_role_proposal":
    case "save_template":
    case "remove_template":
    case "template_toml":
      return null;
    case "template_from_team":
      return null;
    case "prices":
      return demoPrices;
    case "refresh_prices":
      return { priced: 0, view: demoPrices };
    case "command_policy":
    case "set_command_policy":
      return { approval: args?.approval ?? "sandboxed", backend: "WindowsMxc", reason: "MXC process container: writes only in the worktree, reads limited to toolchains, no network" };
    case "status":
      return status;
    case "projects":
      return projects;
    case "overview":
      return overview;
    case "needs_you":
      return { needs: overview.needs.map((n) => n.item), command_approvals: overview.command_approvals };
    case "inbox":
      return inbox;
    case "board":
      return board;
    case "plan":
      return plan;
    case "ticket":
      return ticket(String(args?.key ?? "SHOP-2"));
    case "agents":
      return agents;
    case "runs":
      return runs;
    case "run_detail":
      return runDetail(String(args?.run ?? "run_05"));
    case "events":
      return events;
    case "capabilities":
      return capabilities;
    case "templates":
      return [];
    case "chat_history":
      return chat;
    case "audit":
      return ticket("SHOP-2").audit;
    case "ticket_diff":
      return {
        branch: "agenium/" + String(args?.key ?? "SHOP-2"),
        diff: [
          "diff --git a/src/cart/mod.rs b/src/cart/mod.rs",
          "index 4f2a9c1..9b1e0d4 100644",
          "--- a/src/cart/mod.rs",
          "+++ b/src/cart/mod.rs",
          "@@ -10,7 +10,12 @@ pub struct Cart {",
          "     items: Vec<Item>,",
          " }",
          "-pub fn total(c: &Cart) -> u64 { 0 }",
          "+/// Sum of item prices times quantities, in cents.",
          "+pub fn total(c: &Cart) -> u64 {",
          "+    c.items.iter().map(|i| i.price * i.qty as u64).sum()",
          "+}",
          "diff --git a/tests/cart/total.rs b/tests/cart/total.rs",
          "new file mode 100644",
          "--- /dev/null",
          "+++ b/tests/cart/total.rs",
          "@@ -0,0 +1,4 @@",
          "+#[test]",
          "+fn total_sums_lines() {",
          "+    assert_eq!(total(&cart(&[(250, 2), (100, 1)])), 600);",
          "+}",
        ].join("\n"),
      };
    case "agent_detail": {
      const a = agents.find((x) => x.agent.id === args?.agent) ?? agents[2];
      return {
        agent: { ...a.agent, mcp: [] },
        planner: a.is_planner,
        runs: runs.filter((r) => r.agent === a.agent.id),
        facts: [
          { id: 3, text: "In this repository, cart totals are integers in cents; never use floats for money.", kind: "lesson", project: "shop", scope: { project: "shop" }, source: "ticket:SHOP-1", untrusted: false, confidence: 0.8, weight: 1, created_at: iso(300), used_at: iso(3) },
          { id: 2, text: "The test command is `npm test -- <area>`; the user approves it per run.", kind: "fact", project: "shop", scope: { project: "shop" }, source: "ticket:SHOP-1", untrusted: false, confidence: 0.7, weight: 1, created_at: iso(320), used_at: null },
        ],
        facts_total: 2,
        skills: [{ name: "rust-testing", description: "How this team writes and runs Rust tests", version: "1.0.0", origin: "Role" }],
        mcp: [],
      };
    }
    // Registry and Live tab: empty in the browser demo (the real app has the data).
    case "live":
      return { agents: [], teams: [], totals: { live_runs: 0, tokens_today: 0, working: 0 } };
    case "teams":
      return { teams: [], pool: [] };
    case "skill_library":
      return { skills: [], problems: [] };
    case "agent_trace":
      return { run: null, steps: [], terminal: [] };
    case "roles":
    case "mcp_servers":
    case "local_servers":
    case "provider_models":
    case "agent_bus":
      return [];
    default:
      return null;
  }
}

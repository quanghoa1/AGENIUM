// JSON shapes of the desktop IPC commands (apps/desktop/src/api.rs) and of the orchestrator types they return.
// Hand-written (ADR-48); keep in step with the Rust side.

export type TicketStatus =
  | "New"
  | "Assigned"
  | "InProgress"
  | "Pending"
  | "Resolved"
  | "Reopened"
  | "Closed"
  | "Cancelled";
export type AgentStatus = "Idle" | "Working" | "Waiting" | "QuotaExhausted" | "Offline";
export type RoleKind = "lead" | "worker";
export type BackendKind = "Api" | "Cli";
export type Effort = "low" | "medium" | "high" | "xhigh" | "max";
export type TicketKind = "Epic" | "Story" | "Task" | "Bug" | "Requirement";

export interface Usage {
  tokens_in: number;
  tokens_out: number;
  cache_read: number;
  cache_write: number;
  estimated_cost_usd: number;
  elapsed_ms: number;
}

export interface ModelBinding {
  provider: string;
  model: string;
  effort: Effort;
  max_tokens?: number | null;
}

export interface Status {
  root: string;
  ready: boolean;
  version: string;
  ui_language?: string | null;
}

export interface ProjectSummary {
  id: string;
  name: string;
  path: string;
  key_prefix: string;
  status: string;
  main_branch: string;
  team: { id: string; name: string; solo: boolean; template: string; lead?: string | null; members: number } | null;
  tickets: number;
  by_status: Record<string, number>;
  plan_items: number;
  plan_done: number;
  plan_version?: number | null;
  spec_version?: number | null;
  needs_you: number;
  inbox: number;
  /** Agreeing: Lead and user settle the requirements; Running: the team works on its own; Paused by the user; Done. */
  run_state?: "Agreeing" | "Running" | "Paused" | "Done";
}

export type NeedKind =
  | "AgentBlocked"
  | "Escalation"
  | "SpecApproval"
  | "PlanApproval"
  | "Permission"
  | "Question"
  | "MergeReview"
  | "RoleProposal";

export interface NeedItem {
  kind: NeedKind;
  project: string;
  ticket?: string | null;
  comment?: string | null;
  version?: number | null;
  agent?: string | null;
  /** The role proposal to decide (`RoleProposal`). */
  proposal?: string | null;
  text: string;
  at: string;
}

/** sandboxed: commands run inside a working sandbox without asking (otherwise you approve them); on-request: you
 *  approve every command not on the project allowlist; never: no asking, refused without a sandbox (ADR-64). */
export type ApprovalPolicy = "sandboxed" | "on-request" | "never";

export interface CommandPolicy {
  approval: ApprovalPolicy;
  backend: string | null;
  reason: string;
}

export interface CommandApproval {
  id: number;
  project: string;
  ticket?: string | null;
  agent: string;
  command: string;
  cwd: string;
  at: string;
}

export interface RunRow {
  id: string;
  ticket?: string | null;
  agent: string;
  agent_name?: string | null;
  provider: string;
  model: string;
  started_at: string;
  ended_at?: string | null;
  outcome?: string | null;
  steps: number;
  usage: Usage;
}

export interface AgentRow {
  id: string;
  name: string;
  role: string;
  role_kind: RoleKind;
  status: AgentStatus;
  backend: BackendKind;
  model: ModelBinding;
  current?: { project: string; ticket: string; title: string } | null;
}

export interface Overview {
  needs: { project_name: string; item: NeedItem }[];
  agents: AgentRow[];
  live_runs: RunRow[];
  recent_runs: RunRow[];
  usage_today: Usage;
  runs_today: number;
  command_approvals: CommandApproval[];
}

export interface Card {
  key: string;
  title: string;
  kind: TicketKind;
  priority: number;
  plan_item: string;
  assignee?: string | null;
  blocked_by: string[];
  open_questions: number;
  approved: boolean;
  merged: boolean;
  escalated: boolean;
  reopen_count: number;
}

export interface Column {
  status: TicketStatus;
  cards: Card[];
}

export interface Criterion {
  id: string;
  text: string;
}

export interface Ticket {
  id: string;
  key: string;
  project: string;
  parent?: string | null;
  kind: TicketKind;
  title: string;
  description: string;
  acceptance: Criterion[];
  status: TicketStatus;
  assignee?: string | null;
  plan_item: string;
  priority: number;
  depends_on: string[];
  reopen_count: number;
  escalated: boolean;
  required_role?: string | null;
  required_tags: string[];
  scope_paths: string[];
  report_template: string;
  approved: boolean;
  merged: boolean;
  has_code: boolean;
  close_reason?: string | null;
  usage: Usage;
  created_at: string;
  updated_at: string;
}

export type CommentKind =
  | "Question"
  | "Answer"
  | "Progress"
  | "Report"
  | "Review"
  | "Permission"
  | "Handoff"
  | "System"
  | "User";

export interface CommentRow {
  id: string;
  kind: CommentKind;
  author: string;
  reply_to?: string | null;
  body: { type: string; data: any };
  resolved: boolean;
  created_at: string;
}

export interface AuditEntry {
  at: string;
  actor: string;
  ticket?: string | null;
  action: string;
  intent_id?: string | null;
  accepted: boolean;
  detail: any;
}

export interface TicketDetail {
  ticket: Ticket;
  assignee_name?: string | null;
  comments: CommentRow[];
  runs: RunRow[];
  workspace?: { worktree: string; branch: string; base_sha: string; head_sha?: string | null; removed: boolean } | null;
  audit: AuditEntry[];
}

export interface PlanItem {
  id: string;
  milestone: string;
  title: string;
  done: boolean;
  tickets: [string, TicketStatus][];
}

export interface PlanView {
  spec?: { version: number; body: { goals: string[]; requirements: { id: string; text: string }[] }; approved_by?: string | null; locked_at?: string | null } | null;
  plan?: { version: number; spec_version: number; reason?: string | null; approved_by?: string | null; proposed_by: string; body: { milestones: { id: string; title: string }[] } } | null;
  current_plan_version?: number | null;
  items: PlanItem[];
}

export interface InboxItem {
  kind: "BlockingQuestion" | "Permission" | "Planning" | "Plan" | "Assign" | "Question";
  ticket?: string | null;
  comment?: string | null;
  created_at: string;
  text: string;
}

export interface AgentEntry {
  agent: {
    id: string;
    name: string;
    /** `null`: in the pool (no team yet). */
    team: string | null;
    role: string;
    role_kind: RoleKind;
    tags: string[];
    status: AgentStatus;
    backend: BackendKind;
    model: ModelBinding;
    skills: string[];
    created_at: string;
  };
  project: string | null;
  project_name: string | null;
  team: string | null;
  team_id?: string | null;
  solo: boolean;
  is_lead: boolean;
  is_planner: boolean;
  /** No team yet. */
  pool?: boolean;
}

export interface LogEntry {
  n: number;
  kind: string;
  data: any;
  at: string;
}

export interface RunDetail {
  run: RunRow;
  ticket_key?: string | null;
  log: LogEntry[];
}

export interface EventRecord {
  seq: number;
  intent_id?: string | null;
  kind: string;
  ticket?: string | null;
  actor: string;
  payload: any;
  at: string;
}

export interface RoleSpec {
  id: string;
  kind: RoleKind;
  tags: string[];
  prompt: string;
  skills: string[];
  allowed_paths: string[];
  default_model?: string | null;
  count: number;
  planner: boolean;
}

export interface TeamTemplate {
  id: string;
  version: string;
  name: string;
  description: string;
  report_template: string;
  /** Command run in the worktree by the merge queue before merge (empty = none). */
  pre_merge?: string;
  roles: RoleSpec[];
  /** Built in (or overriding a built-in when `custom` too). */
  builtin?: boolean;
  /** Saved by the user under the root. */
  custom?: boolean;
}

export interface CliInfo {
  kind: string;
  path?: string | null;
  version?: string | null;
  login: any;
  compatible: boolean;
  note?: string | null;
}

export interface Capabilities {
  providers: { id: string; kind: string; base_url?: string | null; has_key: boolean; models: string[]; max_concurrency: number }[];
  clis: CliInfo[];
  templates: TeamTemplate[];
  cli_fallback: { provider: string; model: string };
  /** Models a CLI backend can run that Agenium knows of (opencode: OpenCode's free models). */
  cli_models?: Record<string, string[]>;
}

export interface Fact {
  id: number;
  text: string;
  kind: "fact" | "preference" | "lesson";
  project?: string | null;
  scope: any;
  cue?: string | null;
  evidence?: string | null;
  source: string;
  untrusted: boolean;
  confidence: number;
  weight: number;
  created_at: string;
  used_at?: string | null;
  /** Contexts a lesson held in (decision 35). */
  validated?: string[];
}

/** An agent's own version of a skill (decision 35). */
export interface OverlayView {
  skill: string;
  base_version: string;
  diff: string;
  changelog: { at?: string; reason?: string; lesson?: number; lesson_text?: string; rebased_onto?: string }[];
  /** `role` or `library`. */
  base: string;
  /** The base SKILL.md changed since this overlay was built; it can be rebased onto the new base. */
  stale: boolean;
}

/** A skill the agent created itself from a validated lesson (ADR-91). */
export interface CreatedSkill {
  lesson: number;
  lesson_text: string;
  reason: string;
  at: string;
  /** Set when the lesson stopped holding: why, and when. */
  flagged?: string | null;
}

export interface MemoryData {
  facts: Fact[];
  overlays: OverlayView[];
  /** Skill name -> what the agent created and why. */
  created: Record<string, CreatedSkill>;
  hide_below: number;
}

export interface AgentDetail {
  agent: AgentEntry["agent"] & { mcp: any };
  planner: boolean;
  runs: RunRow[];
  facts: Fact[];
  facts_total: number;
  skills: { name: string; description: string; version?: string | null; origin: string }[];
  mcp: any;
}

export interface ChatLine {
  role: "user" | "assistant" | "system";
  text: string;
}

// ---------------------------------------------------------------- registry (ADR-61)

export interface RoleInfo {
  id: string;
  prompt: string;
  tags: string[];
  allowed_paths: string[];
  planner: boolean;
  lead: boolean;
  /** `<template>@<version>`; empty for the user's own role. */
  template: string;
  exists: boolean;
  agents: number;
  /** Advice about the ROLE.md text: missing or empty sections, too long, nearly the same as another role. */
  lint: string[];
}

export type AgentRecord = AgentEntry["agent"] & { mcp: any };

export interface TeamRecord {
  id: string;
  name: string;
  template: string;
  project?: string | null;
  lead?: string | null;
  solo: boolean;
  created_at: string;
}

export interface TeamsData {
  teams: { team: TeamRecord; project_name?: string | null; members: AgentRecord[]; planner?: string | null }[];
  pool: AgentRecord[];
}

export interface LeadSuggestion {
  agent: string;
  reasons: string[];
}

export interface LibrarySkill {
  name: string;
  description: string;
  version?: string | null;
  files: number;
  users: string[];
  dir: string;
}

export interface McpServerConfig {
  id: string;
  command?: string | null;
  args?: string[];
  env?: Record<string, string>;
  url?: string | null;
  headers?: Record<string, string>;
  timeout_secs: number;
}

export interface McpServerRow {
  config: McpServerConfig;
  users: string[];
}

export interface UsageWindow {
  window: "5h" | "day" | "week" | "month";
  used_percent: number;
  resets_at?: string | null;
}

/** A vendor's subscription usage (`claude`, `codex`, `agy`, `opencode-go:<provider>`). */
export interface VendorUsage {
  vendor: string;
  plan?: string | null;
  windows: UsageWindow[];
  note?: string | null;
  error?: string | null;
  updated_at: string;
}

/** USD per million tokens. */
export interface Price {
  input: number;
  output: number;
  cache_read: number;
  cache_write: number;
}

export interface PriceView {
  source?: string | null;
  fetched_at?: string | null;
  /** provider id -> model -> price */
  providers: Record<string, Record<string, Price>>;
}

export interface ModelInfo {
  id: string;
  display_name?: string | null;
  context_window?: number | null;
  supports_tools?: boolean | null;
  supports_vision?: boolean | null;
}

export interface LocalServer {
  kind: string;
  base_url: string;
  models: string[];
}

// ---------------------------------------------------------------- live tab

export interface LiveRun {
  run: RunRow;
  /** `work` (its own ticket), `review` (a ticket it reviews as Lead), `lead` (chat, inbox, plan). */
  kind: "work" | "review" | "lead";
  ticket_key?: string | null;
  last_tool?: { tool: string; args: any; ok?: boolean | null; at: string; ms?: number | null } | null;
  tools: number;
}

/** One key of an MCP server an agent uses: the server's (system) value and the agent's own key. */
export interface AgentMcpKey {
  server: string;
  name: string;
  system?: string | null;
  own?: string | null;
}

export interface LiveAgent {
  agent: AgentRecord;
  team?: string | null;
  team_name?: string | null;
  solo: boolean;
  is_lead: boolean;
  is_planner: boolean;
  project?: string | null;
  project_name?: string | null;
  run?: LiveRun | null;
  open_tickets: { key: string; title: string; status: TicketStatus; project: string }[];
}

export interface LiveTeam {
  id: string;
  name: string;
  solo: boolean;
  lead?: string | null;
  project?: string | null;
  project_name?: string | null;
  members: string[];
}

export interface LiveData {
  agents: LiveAgent[];
  teams: LiveTeam[];
  totals: { live_runs: number; tokens_today: number; working: number };
}

export type BusItem =
  | { type: "assignment"; ticket: string; project: string; title: string; description: string; acceptance: Criterion[]; status: TicketStatus; scope_paths: string[]; at: string }
  | { type: "comment"; ticket: string; project: string; id: string; kind: CommentKind; author: string; mine: boolean; body: { type: string; data: any }; resolved: boolean; at: string };

export interface AgentTrace {
  run: RunRow | null;
  kind?: LiveRun["kind"];
  ticket_key?: string | null;
  live?: boolean;
  steps: LogEntry[];
  terminal: { n: number; command: string; ok?: boolean | null; output?: string | null; ms?: number | null; at: string }[];
}

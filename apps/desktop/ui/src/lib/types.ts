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
  user_language?: string | null;
  autopilot: boolean;
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
}

export type NeedKind =
  | "AgentBlocked"
  | "Escalation"
  | "SpecApproval"
  | "PlanApproval"
  | "Permission"
  | "Question"
  | "MergeReview";

export interface NeedItem {
  kind: NeedKind;
  project: string;
  ticket?: string | null;
  comment?: string | null;
  version?: number | null;
  agent?: string | null;
  text: string;
  at: string;
}

export interface CommandApproval {
  id: number;
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
    team: string;
    role: string;
    role_kind: RoleKind;
    tags: string[];
    status: AgentStatus;
    backend: BackendKind;
    model: ModelBinding;
    skills: string[];
    created_at: string;
  };
  project: string;
  project_name: string;
  team: string;
  solo: boolean;
  is_lead: boolean;
  is_planner: boolean;
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
  roles: RoleSpec[];
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
}

export interface ChatLine {
  role: "user" | "assistant" | "system";
  text: string;
}

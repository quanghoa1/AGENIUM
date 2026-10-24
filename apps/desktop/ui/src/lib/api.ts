// Typed wrappers over the Tauri IPC commands. Outside Tauri (a plain browser: `npm run dev` for layout work and
// screenshots) the demo backend in demo.ts answers instead, read-only.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { demo } from "./demo";
import type {
  AgentDetail,
  AgentEntry,
  AgentRecord,
  AgentTrace,
  BusItem,
  LeadSuggestion,
  LibrarySkill,
  LiveData,
  LocalServer,
  McpServerConfig,
  McpServerRow,
  ModelInfo,
  RoleInfo,
  TeamRecord,
  TeamsData,
  AuditEntry,
  Capabilities,
  ChatLine,
  Column,
  CommandApproval,
  EventRecord,
  InboxItem,
  NeedItem,
  Overview,
  PlanView,
  ProjectSummary,
  RunDetail,
  RunRow,
  Status,
  TeamTemplate,
  TicketDetail,
} from "./types";

export const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!inTauri) return demo(cmd, args) as Promise<T>;
  return invoke<T>(cmd, args);
}

export interface ModelArg {
  provider: string;
  model: string;
  effort: string;
  backend?: "api" | "cli";
}

export const api = {
  status: () => call<Status>("status"),
  setup: (uiLanguage: string) => call<Status>("setup", { uiLanguage }),
  projects: () => call<ProjectSummary[]>("projects"),
  overview: () => call<Overview>("overview"),
  needsYou: (project: string) =>
    call<{ needs: NeedItem[]; command_approvals: CommandApproval[] }>("needs_you", { project }),
  inbox: (project: string) => call<InboxItem[]>("inbox", { project }),
  board: (project: string) => call<Column[]>("board", { project }),
  plan: (project: string) => call<PlanView>("plan", { project }),
  ticket: (project: string, key: string) => call<TicketDetail>("ticket", { project, key }),
  ticketDiff: (project: string, key: string) =>
    call<{ diff: string; truncated?: boolean; branch?: string; merged?: string | null; note?: string }>("ticket_diff", { project, key }),
  agentDetail: (agent: string) => call<AgentDetail>("agent_detail", { agent }),
  setTeamLead: (project: string, agent: string) => call<void>("set_team_lead", { project, agent }),
  audit: (project: string, key?: string, format?: "md") =>
    call<AuditEntry[] | string>("audit", { project, key, format }),
  agents: () => call<AgentEntry[]>("agents"),
  runs: (limit?: number) => call<RunRow[]>("runs", { limit }),
  runDetail: (run: string) => call<RunDetail>("run_detail", { run }),
  events: (since?: number, limit?: number) => call<EventRecord[]>("events", { since, limit }),
  capabilities: (refresh = false) => call<Capabilities>("capabilities", { refresh }),
  templates: () => call<TeamTemplate[]>("templates"),
  chat: (project: string, message: string) =>
    call<{ run: string; outcome: string; text: string }>("chat", { project, message }),
  chatHistory: (project: string) => call<ChatLine[]>("chat_history", { project }),
  approve: (project: string, what: "spec" | "plan" | "merge", target: string) =>
    call<void>("approve", { project, what, target }),
  answer: (project: string, ticket: string, question: string, answer: string, option?: number) =>
    call<unknown>("answer", { project, ticket, question, answer, option }),
  decidePermission: (project: string, ticket: string, request: string, approve: boolean, always = false, note?: string) =>
    call<unknown>("decide_permission", { project, ticket, request, approve, always, note }),
  decideCommand: (id: number, decision: "allow" | "always" | "deny", reason?: string) =>
    call<boolean>("decide_command", { id, decision, reason }),
  comment: (project: string, ticket: string, text: string) =>
    call<void>("comment", { project, ticket, text }),
  ticketAction: (
    project: string,
    ticket: string,
    action: "priority" | "assign" | "approve" | "reopen" | "close" | "cancel",
    extra: { reason?: string; agent?: string; priority?: number; feedback?: { ac_id: string; feedback: string }[] } = {},
  ) => call<unknown>("ticket_action", { project, ticket, action, ...extra }),
  createProject: (name: string, path?: string) => call<{ id: string }>("create_project", { name, path }),
  createTeam: (project: string, template: string, model: ModelArg, leadModel?: ModelArg, plannerModel?: ModelArg, prefix?: string) =>
    call<{ agents: [string, string][]; missing_skills: string[] }>("create_team", {
      project,
      template,
      model,
      leadModel,
      plannerModel,
      prefix,
    }),
  createSolo: (project: string, name: string, model: ModelArg) =>
    call<unknown>("create_solo", { project, name, model }),
  setAgentModel: (agent: string, model: ModelArg) => call<void>("set_agent_model", { agent, model }),
  setAgentIdle: (agent: string) => call<void>("set_agent_status", { agent, status: "Idle" }),
  runRound: (project: string) => call<unknown[]>("run_round", { project }),
  setAutopilot: (on: boolean) => call<boolean>("set_autopilot", { on }),
  stopAll: () => call<void>("stop_all"),

  // Registry (ADR-61)
  roles: () => call<RoleInfo[]>("roles"),
  putRole: (role: { id: string; prompt: string; tags: string[]; allowed_paths: string[]; planner: boolean; lead: boolean }) =>
    call<RoleInfo>("put_role", { role }),
  removeRole: (id: string) => call<void>("remove_role", { id }),
  createAgent: (form: { name: string; role: string; tags?: string[] | null; model: ModelArg; skills: string[]; mcp: string[] }) =>
    call<AgentRecord>("create_agent", { form }),
  setAgentRole: (agent: string, role: string, tags?: string[] | null) => call<void>("set_agent_role", { agent, role, tags }),
  toggleAgentSkill: (agent: string, skill: string, on: boolean) => call<string[]>("toggle_agent_skill", { agent, skill, on }),
  toggleAgentMcp: (agent: string, server: string, on: boolean) => call<void>("toggle_agent_mcp", { agent, server, on }),
  deleteAgent: (agent: string, confirm: string) => call<{ name: string; pending: string[] }>("delete_agent", { agent, confirm }),
  teams: () => call<TeamsData>("teams"),
  composeTeam: (form: { name: string; project?: string | null; lead: string; members: string[]; solo: boolean }) =>
    call<TeamRecord>("compose_team", { form }),
  suggestLead: (agents: string[]) => call<LeadSuggestion | null>("suggest_lead", { agents }),
  teamMember: (team: string, agent: string, add: boolean) => call<void>("team_member", { team, agent, add }),
  setLeadOfTeam: (team: string, agent: string) => call<void>("set_lead_of_team", { team, agent }),
  setTeamProject: (team: string, project: string | null) => call<void>("set_team_project", { team, project }),
  dissolveTeam: (team: string) => call<void>("dissolve_team", { team }),
  skillLibrary: () => call<{ skills: LibrarySkill[]; problems: { dir: string; error: string }[] }>("skill_library"),
  importSkill: (path: string, replace = false) => call<{ name: string }>("import_skill", { path, replace }),
  removeSkill: (name: string, force = false) => call<void>("remove_skill", { name, force }),
  skillText: (name: string) => call<{ name: string; body: string; files: string[] }>("skill_text", { name }),
  mcpServers: () => call<McpServerRow[]>("mcp_servers"),
  putMcp: (server: McpServerConfig) => call<void>("put_mcp", { server }),
  importMcpJson: (text: string) => call<{ servers: string[]; secrets: string[] }>("import_mcp_json", { text }),
  removeMcp: (id: string, force = false) => call<void>("remove_mcp", { id, force }),
  testMcp: (id: string) =>
    call<{ server: any; protocol: string; tools: { name: string; description?: string | null; read_only: boolean }[] }>("test_mcp", { id }),
  putProvider: (form: { id: string; kind: string; base_url?: string | null; api_key?: string | null; clear_key?: boolean; max_concurrency?: number; models?: string[] }) =>
    call<void>("put_provider", { form }),
  removeProvider: (id: string) => call<void>("remove_provider", { id }),
  providerModels: (id: string) => call<ModelInfo[]>("provider_models", { id }),
  localServers: () => call<LocalServer[]>("local_servers"),

  // Live tab
  live: () => call<LiveData>("live"),
  agentBus: (agent: string, limit?: number) => call<BusItem[]>("agent_bus", { agent, limit }),
  agentTrace: (agent: string, run?: string) => call<AgentTrace>("agent_trace", { agent, run }),
};

/** Committed domain events (`agenium://event`), live run output (`agenium://run`), command approvals. */
export async function onEvent(f: (e: EventRecord) => void): Promise<UnlistenFn> {
  if (!inTauri) return () => {};
  return listen<EventRecord>("agenium://event", (e) => f(e.payload));
}

export async function onRun(f: (run: string, ev: any) => void): Promise<UnlistenFn> {
  if (!inTauri) return () => {};
  return listen<{ run: string; event: any }>("agenium://run", (e) => f(e.payload.run, e.payload.event));
}

export async function onApproval(f: (a: CommandApproval) => void): Promise<UnlistenFn> {
  if (!inTauri) return () => {};
  return listen<CommandApproval>("agenium://approval", (e) => f(e.payload));
}

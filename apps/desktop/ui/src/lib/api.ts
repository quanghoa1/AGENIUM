// Typed wrappers over the Tauri IPC commands. Outside Tauri (a plain browser: `npm run dev` for layout work and
// screenshots) the demo backend in demo.ts answers instead, read-only.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { demo } from "./demo";
import type {
  AgentEntry,
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
  setup: (uiLanguage: string, userLanguage: string) =>
    call<Status>("setup", { uiLanguage, userLanguage }),
  projects: () => call<ProjectSummary[]>("projects"),
  overview: () => call<Overview>("overview"),
  needsYou: (project: string) =>
    call<{ needs: NeedItem[]; command_approvals: CommandApproval[] }>("needs_you", { project }),
  inbox: (project: string) => call<InboxItem[]>("inbox", { project }),
  board: (project: string) => call<Column[]>("board", { project }),
  plan: (project: string) => call<PlanView>("plan", { project }),
  ticket: (project: string, key: string) => call<TicketDetail>("ticket", { project, key }),
  audit: (project: string, key?: string, format?: "md") =>
    call<AuditEntry[] | string>("audit", { project, key, format }),
  agents: () => call<AgentEntry[]>("agents"),
  runs: (limit?: number) => call<RunRow[]>("runs", { limit }),
  runDetail: (run: string) => call<RunDetail>("run_detail", { run }),
  events: (since?: number, limit?: number) => call<EventRecord[]>("events", { since, limit }),
  capabilities: () => call<Capabilities>("capabilities"),
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

// Typed wrappers over the Tauri IPC commands. Outside Tauri (a plain browser: `npm run dev` for layout work and
// screenshots) the demo backend in demo.ts answers instead, read-only.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { demo } from "./demo";
import type { AgentDetail, AgentEntry, AgentMcpKey, AgentRecord, AgentTrace, ApprovalPolicy, AuditEntry, BusItem, Capabilities, ChatLine, Column, CommandApproval, CommandPolicy, EventRecord, FolderInfo, InboxItem, JudgeActivity, JudgeConfig, JudgeForm, LeadSuggestion, LibrarySkill, LiveData, LocalServer, McpApproval, McpApprovalSettings, McpServerConfig, McpServerRow, MemoryData, MemoryGraphData, ModelCapsRow, ModelInfo, NeedItem, Overview, PlanView, Plugin, PluginPreview, PriceView, ProjectSummary, QueuedMessage, Reference, RoleInfo, RunDetail, RunRow, Status, TeamRecord, TeamsData, TeamTemplate, TicketDetail, ToolSearchSettings, VendorUsage } from "./types";

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
  commandPolicy: () => call<CommandPolicy>("command_policy"),
  prices: () => call<PriceView>("prices"),
  vendorUsage: () => call<VendorUsage[]>("vendor_usage"),
  refreshVendorUsage: () => call<VendorUsage[]>("refresh_vendor_usage"),
  readClaudeUsage: () => call<VendorUsage[]>("read_claude_usage"),
  decideRoleProposal: (project: string, proposal: string, approve: boolean, note?: string) =>
    call<unknown>("decide_role_proposal", { project, proposal, approve, note: note ?? null }),
  refreshPrices: () => call<{ priced: number | null; view: PriceView }>("refresh_prices"),
  embeddingModel: () => call<{ id: string; mb: number; installed: boolean; active: string }>("embedding_model"),
  installEmbeddingModel: () => call<void>("install_embedding_model"),
  retryEmbeddingModel: () => call<void>("retry_embedding_model"),
  setCommandPolicy: (approval: ApprovalPolicy) => call<CommandPolicy>("set_command_policy", { approval }),
  setUnsandboxed: (on: boolean) => call<CommandPolicy>("set_unsandboxed", { on }),
  // Preview browser (ADR-121): one native view per tab, placed over the page; logical pixels.
  browserOpen: (tab: string, url: string, x: number, y: number, w: number, h: number) => call<void>("browser_open", { tab, url, x, y, w, h }),
  browserBounds: (tab: string, x: number, y: number, w: number, h: number) => call<void>("browser_bounds", { tab, x, y, w, h }),
  browserVisible: (tab: string, show: boolean) => call<void>("browser_visible", { tab, show }),
  browserHideAll: () => call<void>("browser_hide_all"),
  browserClose: (tab: string) => call<void>("browser_close", { tab }),
  browserCloseAll: () => call<void>("browser_close_all"),
  browserAction: (tab: string, action: "back" | "forward" | "reload" | "pick-on" | "pick-off") => call<void>("browser_action", { tab, action }),
  browserUrl: (tab: string) => call<string | null>("browser_url", { tab }),
  /** PNG (base64) of a box of the tab's page, in CSS pixels; `dpr` = the page's device pixel ratio. */
  browserCapture: (tab: string, x: number, y: number, w: number, h: number, dpr: number) => call<string>("browser_capture", { tab, x, y, w, h, dpr }),
  /** Saves a pick's picture into the project (where its agents can view it); returns the file path. */
  browserSavePick: (project: string, png: string) => call<string>("browser_save_pick", { project, png }),
  /** Linux and macOS first start: a folder from the system's dialog (an "Agenium" folder inside it unless it is one). */
  pickWorkFolder: () => call<string | null>("pick_work_folder"),
  chooseWorkFolder: (folder: string) => call<Status>("choose_work_folder", { folder }),
  projects: () => call<ProjectSummary[]>("projects"),
  /** The latest handover report of a project (ADR-147); null before the project first completes. */
  handover: (project: string) =>
    call<{ path: string; version: number; markdown: string } | null>("handover", { project }),
  overview: () => call<Overview>("overview"),
  needsYou: (project: string) =>
    call<{ needs: NeedItem[]; command_approvals: CommandApproval[] }>("needs_you", { project }),
  inbox: (project: string) => call<InboxItem[]>("inbox", { project }),
  board: (project: string) => call<Column[]>("board", { project }),
  plan: (project: string) => call<PlanView>("plan", { project }),
  ticket: (project: string, key: string) => call<TicketDetail>("ticket", { project, key }),
  ticketDiff: (project: string, key: string) =>
    call<{ diff: string; truncated?: boolean; branch?: string; merged?: string | null; note?: string }>("ticket_diff", { project, key }),
  evidenceImage: (project: string, key: string, image: string) =>
    call<{ media_type: string; base64: string }>("evidence_image", { project, key, image }),
  ticketFile: (project: string, key: string, path: string) =>
    call<{ original: string | null; modified: string | null; binary: boolean; too_big: boolean }>("ticket_file", { project, key, path }),
  agentDetail: (agent: string) => call<AgentDetail>("agent_detail", { agent }),
  audit: (project: string, key?: string, format?: "md") =>
    call<AuditEntry[] | string>("audit", { project, key, format }),
  agents: () => call<AgentEntry[]>("agents"),
  runs: (limit?: number) => call<RunRow[]>("runs", { limit }),
  /** The helpers' model calls (team designer, skill writer) as runs with no ticket (ADR-152). */
  helperCalls: (limit?: number) => call<RunRow[]>("helper_calls", { limit }),
  runDetail: (run: string) => call<RunDetail>("run_detail", { run }),
  revertMemoryRun: (run: string, force: boolean) =>
    call<{ done: boolean; undone?: number; warnings: string[]; skills_flagged?: string[] }>("revert_memory_run", { run, force }),
  events: (since?: number, limit?: number) => call<EventRecord[]>("events", { since, limit }),
  /** `clis: false`: without the agent CLIs (finding them takes seconds the first time). */
  capabilities: (refresh = false, clis = true) => call<Capabilities>("capabilities", { refresh, clis }),
  templates: () => call<TeamTemplate[]>("templates"),
  memory: (agent: string) => call<MemoryData>("memory", { agent }),
  memoryGraph: (agent: string) => call<MemoryGraphData>("memory_graph", { agent }),
  resolveMemoryConflict: (agent: string, a: number, b: number, choice: "a" | "b" | "both") =>
    call<void>("resolve_memory_conflict", { agent, a, b, choice }),
  memoryAction: (agent: string, action: string, fact?: number, skill?: string) =>
    call<unknown>("memory_action", { agent, action, fact: fact ?? null, skill: skill ?? null }),
  saveTemplate: (template: TeamTemplate) => call<TeamTemplate>("save_template", { template }),
  removeTemplate: (id: string) => call<void>("remove_template", { id }),
  templateFromTeam: (team: string, id: string, name: string) => call<TeamTemplate>("template_from_team", { team, id, name }),
  /** Export (`id`: the TOML text) or import (`text`: saved, returns the template). */
  templateToml: (id?: string, text?: string) => call<unknown>("template_toml", { id: id ?? null, text: text ?? null }),
  /** `kind` "queued": the Lead is busy and the message waits in its queue (ADR-149). */
  chat: (project: string, message: string) =>
    call<{ run: string; kind: string; outcome: string; text: string }>("chat", { project, message }),
  leadQueue: (project: string) => call<QueuedMessage[]>("lead_queue", { project }),
  editLeadMessage: (project: string, id: number, text: string) => call<void>("edit_lead_message", { project, id, text }),
  dropLeadMessage: (project: string, id: number) => call<void>("drop_lead_message", { project, id }),
  /** The waiting messages answered now (after turns that failed, they wait for this or the next message). */
  deliverLeadQueue: (project: string) =>
    call<{ run: string; kind: string; outcome: string; text: string }>("deliver_lead_queue", { project }),
  chatHistory: (project: string) => call<ChatLine[]>("chat_history", { project }),
  approve: (project: string, what: "spec" | "plan" | "merge", target: string) =>
    call<void>("approve", { project, what, target }),
  answer: (project: string, ticket: string, question: string, answer: string, option?: number) =>
    // In a team the answer goes to the Lead, who answers on the ticket (`routed_to: "lead"`, ADR-122).
    call<{ routed_to?: string }>("answer", { project, ticket, question, answer, option }),
  decidePermission: (project: string, ticket: string, request: string, approve: boolean, always = false, note?: string) =>
    call<unknown>("decide_permission", { project, ticket, request, approve, always, note }),
  decideCommand: (id: number, decision: "allow" | "always" | "deny", reason?: string) =>
    call<boolean>("decide_command", { id, decision, reason }),
  ticketAction: (
    project: string,
    ticket: string,
    action: "priority" | "assign" | "approve" | "reopen" | "close" | "cancel" | "waive",
    extra: { reason?: string; agent?: string; priority?: number; feedback?: { ac_id: string; feedback: string }[] } = {},
  ) => call<unknown>("ticket_action", { project, ticket, action, ...extra }),
  /** `branch`: a new branch for the team (ADR-159); unset, the repository's own. */
  createProject: (name: string, source?: string, keyPrefix?: string, branch?: string) =>
    call<{ id: string; name: string }>("create_project", { name, source, keyPrefix, branch }),
  /** A folder picked in the system's dialog, as it is (null: closed). */
  pickFolder: () => call<string | null>("pick_folder"),
  /** Whether git starts on this machine (every project is a git repository). */
  gitAvailable: () => call<boolean>("git_available"),
  inspectFolder: (path: string) => call<FolderInfo>("inspect_folder", { path }),
  /** Replaces the project's reference folders; returns them as kept. */
  setReferences: (project: string, references: Reference[]) => call<Reference[]>("set_references", { project, references }),
  /** A free ticket prefix made from a project name. */
  suggestKeyPrefix: (name: string) => call<string>("suggest_key_prefix", { name }),
  /** `roleModels`: each role on its template's own model where this machine runs it (ADR-152). */
  createTeam: (project: string, template: string, model: ModelArg, leadModel?: ModelArg, plannerModel?: ModelArg, prefix?: string, roleModels?: boolean) =>
    call<{ agents: [string, string][]; missing_skills: string[]; model_notes: string[] }>("create_team", {
      project,
      template,
      model,
      leadModel,
      plannerModel,
      prefix,
      roleModels,
    }),
  createSolo: (project: string, name: string, model: ModelArg) =>
    call<unknown>("create_solo", { project, name, model }),
  setAgentModel: (agent: string, model: ModelArg) => call<void>("set_agent_model", { agent, model }),
  /** The agent's context against the window of the model it has now (ADR-155). */
  agentContext: (agent: string) => call<{ used: number; was: number; window: number; over: boolean }>("agent_context", { agent }),
  /** The agent's context now and the window of `model` (ADR-155): `over` when the next run will fold to fit. */
  contextCheck: (agent: string, model: ModelArg) =>
    call<{ used: number; was: number; window: number; over: boolean }>("context_check", { agent, model }),
  renameAgent: (agent: string, name: string) => call<void>("rename_agent", { agent, name }),
  setAgentIdle: (agent: string) => call<void>("set_agent_status", { agent, status: "Idle" }),
  /** The user closes a done project (ADR-130): the freed team, and the project waiting for it that took it. */
  closeProject: (project: string) => call<{ team: string | null; taken_by: string | null }>("close_project", { project }),
  /** The user reopens a closed project: a free team on it now, a busy one it waits for (`wait`), or none yet. */
  reopenProject: (project: string, team: string | null, wait: boolean) => call<void>("reopen_project", { project, team, wait }),
  pauseProject: (project: string) => call<void>("pause_project", { project }),
  resumeProject: (project: string) => call<void>("resume_project", { project }),
  stopAll: () => call<void>("stop_all"),
  supportBundle: (out: string) => call<string>("support_bundle", { out }),
  judgeStatus: () => call<{ configured: boolean; disabled: boolean; reason?: string | null }>("judge_status"),
  enableJudge: (name?: string) => call<void>("enable_judge", { name: name ?? null }),
  judgeAddKey: (name: string, key: string) => call<void>("judge_add_key", { name, key }),
  judgeRemoveKey: (name: string) => call<void>("judge_remove_key", { name }),
  judgeConfig: () => call<JudgeConfig>("judge_config"),
  judgeActivity: () => call<JudgeActivity[]>("judge_activity"),
  setUiLanguage: (language: string) => call<void>("set_ui_language", { language }),
  /** `api_key` empty or null keeps the stored key; `clear_key` removes it. */
  setJudge: (form: JudgeForm) => call<void>("set_judge", { form }),
  exportAgent: (agent: string, out: string) => call<string>("export_agent", { agent, out }),
  importAgent: (file: string, project: string, name?: string) => call<{ id: string; name: string }>("import_agent", { file, project, name: name ?? null }),
  projectReport: (project: string) => call<string>("project_report", { project }),

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
  agentMcpKeys: (agent: string) => call<AgentMcpKey[]>("agent_mcp_keys", { agent }),
  setAgentMcpKey: (agent: string, server: string, name: string, value: string | null) => call<void>("set_agent_mcp_key", { agent, server, name, value }),
  deleteAgent: (agent: string, confirm: string) => call<{ name: string; pending: string[] }>("delete_agent", { agent, confirm }),
  teams: () => call<TeamsData>("teams"),
  composeTeam: (form: { name: string; project?: string | null; lead: string; members: string[]; solo: boolean }) =>
    call<TeamRecord>("compose_team", { form }),
  suggestLead: (agents: string[]) => call<LeadSuggestion | null>("suggest_lead", { agents }),
  teamMember: (team: string, agent: string, add: boolean) => call<void>("team_member", { team, agent, add }),
  setLeadOfTeam: (team: string, agent: string) => call<void>("set_lead_of_team", { team, agent }),
  setTeamProject: (team: string, project: string | null) => call<void>("set_team_project", { team, project }),
  dissolveTeam: (team: string) => call<void>("dissolve_team", { team }),
  plugins: () => call<{ plugin: Plugin; users: string[] }[]>("plugins"),
  previewPlugin: (source: string, entry?: string) => call<PluginPreview>("preview_plugin", { source, entry: entry || null }),
  previewPluginUpdate: (name: string) => call<PluginPreview>("preview_plugin_update", { name }),
  installPlugin: (p: Plugin) =>
    call<Plugin>("install_plugin", { source: p.source, reference: p.reference, entry: p.entry ?? null, commit: p.commit }),
  removePlugin: (name: string) => call<void>("remove_plugin", { name }),
  toggleAgentPlugin: (agent: string, name: string, on: boolean) => call<string[]>("toggle_agent_plugin", { agent, name, on }),
  skillLibrary: () => call<{ skills: LibrarySkill[]; problems: { dir: string; error: string }[] }>("skill_library"),
  importSkill: (path: string, replace = false) =>
    call<{ name: string; description: string; shape_notes: string[]; quality: string[] }>("import_skill", { path, replace }),
  /** The skills a folder holds (itself or its subfolders), to import many at once. */
  scanSkills: (path: string) => call<FoundSkill[]>("scan_skills", { path }),
  importSkills: (paths: string[], replace = false) => call<{ path: string; name?: string; error?: string }[]>("import_skills", { paths, replace }),
  removeSkill: (name: string, force = false) => call<void>("remove_skill", { name, force }),
  skillText: (name: string) => call<{ name: string; body: string; raw: string; files: string[] }>("skill_text", { name }),
  // Re-run the shape, conflict and (when the judge is on) quality checks on a library skill on demand (ADR-88/90/92).
  evaluateSkill: (name: string) => call<{ shape_notes: string[]; quality: string[] | null }>("evaluate_skill", { name }),
  skillSkeleton: () => call<{ body: string }>("skill_skeleton"),
  // Write a library skill's SKILL.md (create or edit); returns the shape and (when the judge is on) quality checks.
  saveSkill: (name: string, content: string, create: boolean, files: { path: string; text?: string; base64?: string }[] = [], removed: string[] = []) =>
    call<{ name: string; shape_notes: string[]; quality: string[] | null }>("save_skill", { name, content, create, files, removed }),
  // Evaluate unsaved SKILL.md text (the editor's "Evaluate" button): shape + (judge on) quality, no save, no conflicts.
  evaluateSkillText: (content: string) => call<{ shape_notes: string[]; quality: string[] | null }>("evaluate_skill_text", { content }),
  // Read / write a skill's tier-3 file (references/*, scripts/*).
  skillFile: (name: string, path: string) => call<{ path: string; content?: string; binary?: boolean; size?: number; base64?: string }>("skill_file", { name, path }),
  mcpServers: () => call<McpServerRow[]>("mcp_servers"),
  putMcp: (server: McpServerConfig) => call<void>("put_mcp", { server }),
  mcpApproval: () => call<McpApprovalSettings>("mcp_approval"),
  setMcpApproval: (server: string | null, approval: McpApproval | null) => call<McpApprovalSettings>("set_mcp_approval", { server, approval }),
  toolSearch: () => call<ToolSearchSettings>("tool_search_settings"),
  setToolSearch: (section: ToolSearchSettings) => call<ToolSearchSettings>("set_tool_search", { section }),
  importMcpJson: (text: string) => call<{ servers: string[] }>("import_mcp_json", { text }),
  removeMcp: (id: string, force = false) => call<void>("remove_mcp", { id, force }),
  loginMcp: (id: string) => call<{ client_id: string; scope?: string | null }>("login_mcp", { id }),
  logoutMcp: (id: string) => call<void>("logout_mcp", { id }),
  testMcp: (id: string) =>
    call<{ server: any; protocol: string; tools: { name: string; description?: string | null; read_only: boolean; destructive?: boolean | null }[] }>("test_mcp", { id }),
  putProvider: (form: {
    id: string;
    kind: string;
    base_url?: string | null;
    api_key?: string | null;
    clear_key?: boolean;
    max_concurrency?: number;
    models?: string[];
    headers?: { name: string; value: string | null; secret: boolean }[];
    model_caps?: ModelCapsRow[];
    body?: { path: string; value: string; model?: string | null }[];
    effort_field?: string;
  }) =>
    call<void>("put_provider", { form }),
  removeProvider: (id: string) => call<void>("remove_provider", { id }),
  /** A provider's models as kept (listed again with `refresh`, when new or a week old): with when and any error. */
  providerModels: (id: string, refresh = false) => call<CachedModels>("provider_models", { id, refresh }),
  /** One turn of the team designer (ADR-151) with the user's chosen model; the draft opens in the template editor. */
  designTeam: (model: ModelArg, turns: ChatTurn[], documents: SkillDoc[] = [], session?: string) =>
    call<TeamDesign>("design_team", { model, turns, documents, session }),
  /** One turn of the skill writer (ADR-151): the request, the user's documents, the designer's proposal if any;
   * `mergeInto` asks for the draft to improve that library skill instead (ADR-152). */
  writeSkill: (model: ModelArg, turns: ChatTurn[], documents: SkillDoc[], seed: NewSkill | null, mergeInto?: string | null, session?: string) =>
    call<SkillWrite>("write_skill", { model, turns, documents, seed, mergeInto: mergeInto ?? null, session }),
  /** A document read as Markdown by the backend (ADR-152): its bytes in base64, or its path on this machine. */
  /** With `project`, the file is also kept where that project's runs can open it again (`file`). */
  readDocument: (name: string, base64: string, project?: string) => call<ReadDoc>("read_document", { name, base64, project: project ?? null }),
  readDocumentPath: (path: string, project?: string) => call<ReadDoc>("read_document_path", { path, project: project ?? null }),
  /** A helper chat kept in the root (ADR-152): made on the first message, saved after each answer, reopened later. */
  newSession: (kind: "design" | "skill", source: unknown, model: ModelArg) => call<HelperSession>("new_session", { kind, source, model }),
  saveSession: (id: string, s: { model: ModelArg; turns: ChatTurn[]; transcript: unknown; attachments: SkillDoc[]; draft?: unknown; result?: string | null }) =>
    call<HelperSession>("save_session", { id, model: s.model, turns: s.turns, transcript: s.transcript, attachments: s.attachments, draft: s.draft ?? null, result: s.result ?? null }),
  sessions: (kind: "design" | "skill") => call<HelperSession[]>("sessions", { kind }),
  session: (id: string) => call<HelperSession>("session", { id }),
  removeSession: (id: string) => call<void>("remove_session", { id }),
  /** Names a session (kept through later turns); an empty name gives it back its own title (its draft's name). */
  renameSession: (id: string, title: string) => call<HelperSession>("rename_session", { id, title }),
  finishSession: (id: string, result: string) => call<HelperSession>("finish_session", { id, result }),
  /** An agent's skill proposal (ADR-152) and the user's decision on it. */
  skillProposal: (id: string) => call<SkillProposal>("skill_proposal", { id }),
  decideSkillProposal: (id: string, approve: boolean, savedAs?: string | null, note?: string | null) =>
    call<SkillProposal>("decide_skill_proposal", { id, approve, savedAs: savedAs ?? null, note: note ?? null }),
  /** Agent CLIs updated on request (ADR-153): versions, and the CLI's own updater with a check after. */
  cliUpdates: () => call<CliUpdateInfo[]>("cli_updates"),
  updateCli: (cli: string, version?: string | null) => call<CliUpdated>("update_cli", { cli, version: version ?? null }),
  /** markitdown in Settings (ADR-152): Agenium's own copy installed, self-tested, updated, removed. */
  markitdownStatus: () => call<MarkitdownStatus>("markitdown_status"),
  installMarkitdown: (method?: "uv" | "python" | "uv-standalone") => call<MarkitdownStatus>("install_markitdown", { method: method ?? null }),
  updateMarkitdown: () => call<MarkitdownStatus>("update_markitdown"),
  checkMarkitdownUpdate: () => call<MarkitdownStatus>("check_markitdown_update"),
  testMarkitdown: () => call<[string, string][]>("test_markitdown"),
  removeMarkitdown: () => call<MarkitdownStatus>("remove_markitdown"),
  /** A skill on the 12 criteria (ADR-152), checked locally; `skip` is the library skill being scored. */
  skillScorecard: (name: string, description: string, body: string, origin: string, skip?: string | null) =>
    call<Scorecard>("skill_scorecard", { name, description, body, origin, skip: skip ?? null }),
  skillEffects: () => call<Record<string, SkillEffect>>("skill_effects"),
  /** The user's last choice under `key` (ADR-152), kept in the root; null when none. */
  preference: (key: PrefKey) => call<unknown>("preference", { key }),
  setPreference: (key: PrefKey, value: unknown) => call<void>("set_preference", { key, value }),
  /** Lists one CLI's models again now. */
  reloadCliModels: (cli: string) => call<CachedModels>("reload_cli_models", { cli }),
  localServers: () => call<LocalServer[]>("local_servers"),
  providerPresets: () =>
    call<{ id: string; name: string; kind: "anthropic" | "gemini" | "openai_compat"; base_url?: string | null; key_env: string; note: string }[]>(
      "provider_presets",
    ),

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

export async function onRun(f: (run: string, ev: any, agent: string | null) => void): Promise<UnlistenFn> {
  if (!inTauri) return () => {};
  return listen<{ run: string; agent?: string | null; event: any }>("agenium://run", (e) =>
    f(e.payload.run, e.payload.event, e.payload.agent ?? null),
  );
}

/** The preview browser's events, per tab: its page changed, an element was picked, the page left picking (Esc). */
export async function onBrowser(f: {
  url: (tab: string, u: string) => void;
  picked: (tab: string, json: string) => void;
  pickOff: (tab: string) => void;
}): Promise<UnlistenFn> {
  if (!inTauri) return () => {};
  const offs = await Promise.all([
    listen<{ tab: string; url: string }>("agenium://browser-url", (e) => f.url(e.payload.tab, e.payload.url)),
    listen<{ tab: string; pick: string }>("agenium://browser-picked", (e) => f.picked(e.payload.tab, e.payload.pick)),
    listen<{ tab: string }>("agenium://browser-pick-off", (e) => f.pickOff(e.payload.tab)),
  ]);
  return () => offs.forEach((o) => o());
}

/** The embedding model's preparation moved on (`agenium://model`). */
export async function onModel(f: () => void): Promise<UnlistenFn> {
  if (!inTauri) return () => {};
  return listen("agenium://model", () => f());
}

/** A step of a markitdown install, or a newer version found (`agenium://markitdown`, ADR-152). */
export async function onMarkitdown(f: (e: { step?: string; update?: string }) => void): Promise<UnlistenFn> {
  if (!inTauri) return () => {};
  return listen<{ step?: string; update?: string }>("agenium://markitdown", (e) => f(e.payload));
}

/** The vendors' usage was read again (`agenium://usage`). */
export async function onUsage(f: () => void): Promise<UnlistenFn> {
  if (!inTauri) return () => {};
  return listen("agenium://usage", () => f());
}

export async function onApproval(f: (a: CommandApproval) => void): Promise<UnlistenFn> {
  if (!inTauri) return () => {};
  return listen<CommandApproval>("agenium://approval", (e) => f(e.payload));
}

/** A source's models as kept in the model cache. */
export interface CachedModels {
  value: ModelInfo[];
  at?: string | null;
  error?: string | null;
}

/** One turn of a helper chat: the user's text, or the helper's whole answer (sent back so a change edits its draft). */
export interface ChatTurn {
  role: "user" | "assistant";
  text: string;
}

/** A skill the team designer would give a role but the library lacks. */
export interface NewSkill {
  name: string;
  does: string;
  when: string;
  roles: string[];
}

export interface TeamDesign {
  reply: string;
  questions: string[];
  draft: { template: TeamTemplate; why: string; roles: { id: string; why: string }[]; new_skills: NewSkill[]; notes: string[] } | null;
  raw: string;
  model: string;
}

export interface SkillDoc {
  name: string;
  text: string;
  /** Where the original is kept for the project's runs (chat attachments). */
  file?: string | null;
}

export interface SimilarSkill {
  name: string;
  words: number;
  meaning?: number | null;
  level: "duplicate" | "near";
}

export interface SkillWrite {
  reply: string;
  questions: string[];
  draft: {
    name: string;
    description: string;
    tags: string[];
    body: string;
    files: { path: string; text: string }[];
    skill_md: string;
    exists: boolean;
    notes: string[];
    /** The library skill this draft improves (a merge): saving it edits that skill. */
    extends?: string | null;
    /** That skill's SKILL.md as it is now. */
    original?: string | null;
    similar: SimilarSkill[];
  } | null;
  raw: string;
  model: string;
}

/** A document read as Markdown (ADR-152). */
export interface ReadDoc {
  name: string;
  text: string;
  format: string;
  notes: string[];
  file?: string | null;
}

export interface HelperSession {
  id: string;
  kind: "design" | "skill";
  title: string;
  /** The user named it: the title no longer follows the draft. */
  named?: boolean;
  model: ModelArg | null;
  turns: ChatTurn[];
  transcript: { user: boolean; text: string; questions?: string[]; model?: string }[];
  attachments: SkillDoc[];
  draft?: unknown;
  source?: { proposal?: string; seed?: NewSkill } | null;
  status: "open" | "saved" | "closed";
  result?: string | null;
  created_at: string;
  updated_at: string;
}

export interface SkillProposal {
  id: string;
  agent: string;
  agent_name?: string;
  name: string;
  text: string;
  lesson_text: string;
  contexts: string[];
  similar: SimilarSkill[];
  notes: string[];
  reason: string;
  status: string;
  skill?: string | null;
}

export interface MarkitdownStatus {
  source: "agenium" | "path" | "none";
  program?: string | null;
  version?: string | null;
  method?: string | null;
  installed_at?: string | null;
  pinned: string;
  latest?: string | null;
  update: boolean;
  checked_at?: string | null;
  tested_at?: string | null;
  tests: [string, string][];
  busy: boolean;
  size_mb: number;
  blocked: boolean;
}

export interface CliUpdateInfo {
  kind: string;
  program: string;
  installed?: string | null;
  latest?: string | null;
  latest_error?: string | null;
  verified?: string | null;
  update: boolean;
  beyond_verified: boolean;
  can_pin: boolean;
  busy: number;
  /** The installed version answered Agenium's check after an update on this machine. */
  checked?: { version: string; at: string; updater_had_none: boolean } | null;
}

export interface CliUpdated {
  kind: string;
  before?: string | null;
  after?: string | null;
  output: string;
  check_ok: boolean;
  check: string;
  rollback?: string | null;
}

export interface Criterion {
  id: string;
  status: "ok" | "warn" | "info";
  note: string;
}

export interface Scorecard {
  criteria: Criterion[];
  ok: number;
  warn: number;
}

/** A skill folder found in a chosen folder: `exists`, the library has one of its name. */
export interface FoundSkill {
  path: string;
  name: string;
  description: string;
  error: string | null;
  exists: boolean;
}

export interface SkillEffect {
  tickets: number;
  reopened: number;
  others: number;
  others_reopened: number;
  enough: boolean;
  worse: boolean;
}

/** Where a model is picked and remembered (ADR-152). */
export type PrefKey = "model.agent" | "model.team" | "model.skill_writer" | "model.designer";

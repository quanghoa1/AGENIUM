// The shapes of api.ts calls that are not domain records (those are in types.ts).
import type { ModelArg } from "./api";
import type { ModelInfo, TeamTemplate } from "./types";

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
  /** `based_on`: the template this draft is a variant of (the user may save it over that one). */
  draft: { template: TeamTemplate; why: string; roles: { id: string; why: string }[]; new_skills: NewSkill[]; notes: string[]; based_on?: string | null } | null;
  raw: string;
  model: string;
}

export interface SkillDoc {
  name: string;
  text: string;
  /** Where the original is kept for the project's runs (chat attachments). */
  file?: string | null;
}

interface SimilarSkill {
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

/** The order web searches go in (ADR-163): the user's keys, the strongest first, then Exa's free search. */
export interface WebSearchStatus {
  order: { api: string; key: boolean }[];
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

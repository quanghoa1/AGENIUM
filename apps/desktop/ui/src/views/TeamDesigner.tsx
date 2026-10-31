// "Propose a team" (ADR-151): the user describes the team they want, with documents if they have them (ADR-152); a
// model of their choosing proposes the roles, how many of each, their library skills, write scopes and models, with
// why. The draft opens in the template editor to be changed and saved; a skill the library lacks has a button that
// writes it with the skill writer. The chat is a session, listed under History.
import { AlertTriangle, ArrowRight, Sparkles, Wand2 } from "lucide-react";
import { useState } from "react";
import { HelperChat } from "../components/HelperChat";
import { Markdown } from "../components/Markdown";
import { Mono, cn } from "../components/ui";
import { api, type NewSkill, type TeamDesign } from "../lib/api";
import { missionOf } from "../lib/roles";
import { useApp } from "../lib/store";
import type { TeamTemplate } from "../lib/types";
import { SkillWriter } from "./capabilities/SkillWriter";

export function TeamDesigner({ onClose, onDraft }: { onClose: () => void; onDraft: (t: TeamTemplate) => void }) {
  const app = useApp();
  const [design, setDesign] = useState<TeamDesign | null>(null);
  const [writing, setWriting] = useState<NewSkill | null>(null);
  // Skills written from this draft: they are in the library now, so the role can have them.
  const [written, setWritten] = useState<Record<string, string>>({});
  const [session, setSession] = useState<string | null>(null);
  const d = design?.draft;
  const template = (): TeamTemplate => {
    const t = structuredClone(d!.template);
    for (const s of d!.new_skills) {
      const name = written[s.name];
      if (!name) continue;
      for (const r of t.roles) if (s.roles.includes(r.id) && !r.skills.includes(name)) r.skills.push(name);
    }
    return t;
  };
  return (
    <>
      <HelperChat<TeamDesign>
        kind="design"
        prefKey="model.designer"
        title={app.t("design.title")}
        intro={app.t("design.intro")}
        placeholder={app.t("design.placeholder")}
        ask={(m, turns, docs, s) => api.designTeam(m, turns, docs, s)}
        onAnswer={(a) => a.draft && setDesign(a)}
        onSession={setSession}
        onRestore={(draft) => setDesign(draft ? { reply: "", questions: [], raw: "", model: "", draft: draft as TeamDesign["draft"] } : null)}
        onClose={onClose}
      >
        {!d ? (
          <div className="flex h-full flex-col items-center justify-center gap-2 text-center text-xs text-subtle">
            <Sparkles className="h-6 w-6" />
            <div className="max-w-md leading-relaxed">{app.t("design.empty")}</div>
          </div>
        ) : (
          <div className="space-y-4">
            <div className="flex items-start gap-3">
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  <span className="text-sm font-semibold">{d.template.name}</span>
                  <Mono className="text-subtle">{d.template.id}</Mono>
                  <span className="chip border-line-strong text-muted">{d.template.report_template}</span>
                </div>
                {d.template.description && <div className="mt-1 text-xs text-muted">{d.template.description}</div>}
              </div>
              <button
                className="btn-primary shrink-0"
                onClick={() => {
                  if (session) api.finishSession(session, d.template.id).catch(() => undefined);
                  onDraft(template());
                }}
              >
                {app.t("design.open")} <ArrowRight className="h-3.5 w-3.5" />
              </button>
            </div>
            {d.why && (
              <div className="rounded-lg border border-line bg-elevated/40 p-3 text-xs">
                <div className="label mb-1">{app.t("design.why")}</div>
                <Markdown text={d.why} />
              </div>
            )}
            {d.template.pre_merge && (
              <div className="text-[11px] text-subtle">
                {app.t("templates.preMerge")}: <Mono>{d.template.pre_merge}</Mono>
              </div>
            )}
            <div className="grid gap-2 xl:grid-cols-2">
              {d.template.roles.map((r) => {
                const why = d.roles.find((x) => x.id === r.id)?.why;
                const kind = r.planner ? "planner" : r.kind;
                const proposed = d.new_skills.filter((s) => s.roles.includes(r.id));
                return (
                  <div key={r.id} className="rounded-lg border border-line bg-elevated/40 p-3">
                    <div className="flex items-center gap-2">
                      <span className="text-xs font-semibold">{r.id}</span>
                      <span className={cn("chip", kind === "lead" ? "border-purple-500/30 text-purple-600 dark:text-purple-300" : kind === "planner" ? "border-cyan-500/30 text-cyan-600 dark:text-cyan-300" : "border-line-strong text-muted")}>
                        {kind}
                        {r.count > 1 ? ` ×${r.count}` : ""}
                      </span>
                      {r.default_model && <Mono className="ml-auto truncate text-subtle">{r.default_model}</Mono>}
                    </div>
                    <div className="mt-1.5 text-[11px] text-muted">{missionOf(r.prompt)}</div>
                    {why && <div className="mt-1.5 text-[11px] text-subtle">{why}</div>}
                    <div className="mt-2 flex flex-wrap items-center gap-1">
                      {r.skills.map((s) => (
                        <span key={s} className="chip border-cyan-500/30 text-cyan-600 dark:text-cyan-300">
                          {s}
                        </span>
                      ))}
                      {proposed.map((s) => (
                        <span key={s.name} className={cn("chip", written[s.name] ? "border-cyan-500/30 text-cyan-600 dark:text-cyan-300" : "border-dashed border-amber-500/50 text-amber-600 dark:text-amber-400")} title={written[s.name] ? "" : app.t("design.notInLibrary")}>
                          {written[s.name] ?? s.name}
                        </span>
                      ))}
                      {!r.skills.length && !proposed.length && <span className="text-[11px] text-subtle">{app.t("design.noSkills")}</span>}
                    </div>
                    {r.allowed_paths.length > 0 && <div className="mt-1.5 font-mono text-[11px] text-muted">scope: {r.allowed_paths.join(", ")}</div>}
                  </div>
                );
              })}
            </div>
            {d.new_skills.length > 0 && (
              <div className="rounded-lg border border-dashed border-amber-500/40 p-3">
                <div className="label mb-1">{app.t("design.newSkills")}</div>
                <div className="mb-2 text-[11px] text-subtle">{app.t("design.newSkillsNote")}</div>
                <div className="space-y-2">
                  {d.new_skills.map((s) => (
                    <div key={s.name} className="flex items-start gap-3 text-xs">
                      <div className="min-w-0 flex-1">
                        <Mono className="font-semibold">{s.name}</Mono>
                        <span className="text-subtle"> · {s.roles.join(", ")}</span>
                        {(s.does || s.when) && (
                          <div className="text-[11px] text-muted">
                            {s.does}
                            {s.when && ` — ${s.when}`}
                          </div>
                        )}
                      </div>
                      {written[s.name] ? (
                        <span className="chip border-emerald-500/40 text-emerald-600 dark:text-emerald-400">{app.t("design.written")}</span>
                      ) : (
                        <button className="btn shrink-0" onClick={() => setWriting(s)}>
                          <Wand2 className="h-3.5 w-3.5" /> {app.t("skillWriter.auto")}
                        </button>
                      )}
                    </div>
                  ))}
                </div>
              </div>
            )}
            {d.notes.length > 0 && (
              <div className="space-y-1">
                {d.notes.map((n, i) => (
                  <div key={i} className="flex items-start gap-1.5 text-[11px] text-amber-600 dark:text-amber-400">
                    <AlertTriangle className="mt-0.5 h-3 w-3 shrink-0" /> {n}
                  </div>
                ))}
              </div>
            )}
            <div className="text-[11px] text-subtle">{app.t("design.reviewNote")}</div>
          </div>
        )}
      </HelperChat>
      {writing && <SkillWriter seed={writing} onClose={() => setWriting(null)} onSaved={(name) => (setWritten((w) => ({ ...w, [writing.name]: name })), setWriting(null))} />}
    </>
  );
}

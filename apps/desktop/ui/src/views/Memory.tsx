// Memory (decision 35, ADR-72/73): what each agent learned. Lessons with their cue, scope, confidence, decayed weight
// and the contexts they held in; facts and preferences; the agent's skill overlays with their diff against the base
// (revert or promote). Memory stays the agent's own: the user reads it, retires a wrong entry, tidies it.
import { Brain, CheckCircle2, GitMerge, Lightbulb, RotateCcw, Search, Sparkles, Trash2, Wand2 } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { DiffView } from "../components/DiffView";
import { Segmented } from "../components/motion";
import { Avatar, Empty, Mono, PageHeader, Section, ago, cn } from "../components/ui";
import { api } from "../lib/api";
import { useApp } from "../lib/store";
import type { AgentEntry, Fact, MemoryData } from "../lib/types";

type Tab = "lesson" | "fact" | "preference" | "overlay";

function Bar({ value, tone }: { value: number; tone: string }) {
  return (
    <span className="relative inline-block h-1.5 w-16 overflow-hidden rounded-full bg-line">
      <span className={cn("absolute inset-y-0 left-0 rounded-full", tone)} style={{ width: `${Math.round(Math.max(0.02, Math.min(1, value)) * 100)}%` }} />
    </span>
  );
}

function FactRow({ f, hideBelow, onRetire }: { f: Fact; hideBelow: number; onRetire: () => void }) {
  const app = useApp();
  const faded = f.weight < hideBelow;
  const scope = Object.entries(f.scope ?? {});
  const held = f.validated?.length ?? 0;
  return (
    <div className={cn("rounded-lg border border-line bg-elevated/40 p-3", faded && "opacity-55")}>
      <div className="flex items-start gap-3">
        <div className="min-w-0 flex-1">
          <div className="text-xs">{f.text}</div>
          {f.cue && (
            <div className="mt-1 text-[11px] text-muted">
              <span className="label mr-1">{app.t("memory.when")}</span>
              {f.cue}
            </div>
          )}
          <div className="mt-2 flex flex-wrap items-center gap-1.5 text-[11px]">
            {scope.length === 0 ? (
              <span className="chip border-line-strong text-subtle">{app.t("memory.everywhere")}</span>
            ) : (
              scope.map(([k, v]) => (
                <span key={k} className="chip border-cyan-500/30 font-mono text-cyan-600 dark:text-cyan-300">
                  {k}={String(v)}
                </span>
              ))
            )}
            {f.kind === "lesson" && held > 0 && (
              <span className={cn("chip", held >= 2 ? "border-emerald-500/40 bg-emerald-500/10 text-emerald-600 dark:text-emerald-300" : "border-line-strong text-muted")} title={(f.validated ?? []).join("\n")}>
                <CheckCircle2 className="h-3 w-3" /> {app.t("memory.held").replace("{n}", String(held))}
                {held >= 2 ? ` · ${app.t("memory.skillReady")}` : ""}
              </span>
            )}
            {f.untrusted && <span className="chip border-amber-500/40 text-amber-600 dark:text-amber-300">{app.t("memory.untrusted")}</span>}
            {faded && <span className="chip border-line-strong text-subtle">{app.t("memory.faded")}</span>}
            <Mono className="text-subtle">
              #{f.id} · {f.source}
              {f.used_at ? ` · ${app.t("memory.used")} ${ago(f.used_at)}` : ""}
            </Mono>
          </div>
        </div>
        <div className="flex shrink-0 flex-col items-end gap-1 text-[11px] text-subtle">
          <span className="flex items-center gap-1.5" title={app.t("memory.confidenceHint")}>
            {app.t("memory.confidence")} <Bar value={f.confidence} tone="bg-blue-500" /> <Mono>{Math.round(f.confidence * 100)}%</Mono>
          </span>
          <span className="flex items-center gap-1.5" title={app.t("memory.weightHint")}>
            {app.t("memory.weight")} <Bar value={f.weight} tone={faded ? "bg-zinc-400" : "bg-emerald-500"} /> <Mono>{Math.round(f.weight * 100)}%</Mono>
          </span>
          <button className="mt-1 rounded p-1 text-subtle hover:bg-elevated hover:text-rose-500" title={app.t("memory.retire")} onClick={onRetire}>
            <Trash2 className="h-3.5 w-3.5" />
          </button>
        </div>
      </div>
      {f.evidence && <div className="mt-2 border-t border-line pt-1.5 text-[11px] text-subtle">{f.evidence}</div>}
    </div>
  );
}

/** Memory recalls by meaning only with the local embedding model; without it, by shared words (ADR-47). */
function EmbedderNote() {
  const app = useApp();
  const [m, setM] = useState<{ id: string; mb: number; installed: boolean; active: string } | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    api.embeddingModel().then(setM).catch(() => undefined);
  }, []);
  if (!m || m.active === m.id) return null;
  const install = () => {
    setBusy(true);
    api
      .installEmbeddingModel()
      .then(() => api.embeddingModel().then(setM))
      .catch((e) => app.notify(String(e)))
      .finally(() => setBusy(false));
  };
  return (
    <div className="flex items-center gap-3 rounded-lg border border-line bg-elevated/40 p-3 text-xs">
      <Brain className="h-4 w-4 shrink-0 text-accent" />
      <div className="min-w-0 flex-1">{m.installed ? app.t("memory.embedRestart") : app.t("memory.embedMissing").replace("{mb}", String(m.mb))}</div>
      {!m.installed && (
        <button className="btn" disabled={busy} onClick={install}>
          {busy ? app.t("memory.embedInstalling") : app.t("memory.embedInstall")}
        </button>
      )}
    </div>
  );
}

export function MemoryView() {
  const app = useApp();
  const [agents, setAgents] = useState<AgentEntry[]>([]);
  const [agent, setAgent] = useState<string | null>(null);
  const [data, setData] = useState<MemoryData | null>(null);
  const [tab, setTab] = useState<Tab>("lesson");
  const [q, setQ] = useState("");
  const [showFaded, setShowFaded] = useState(false);
  useEffect(() => {
    api.agents().then((xs) => {
      setAgents(xs);
      setAgent((cur) => cur ?? xs[0]?.agent.id ?? null);
    });
  }, [app.tick]);
  const load = () => {
    if (agent) api.memory(agent).then(setData).catch((e) => app.notify(String(e)));
  };
  useEffect(() => {
    setData(null);
    load();
  }, [agent, app.tick]);
  const act = (action: string, extra: { fact?: number; skill?: string }, msg?: string) =>
    agent &&
    api
      .memoryAction(agent, action, extra.fact, extra.skill)
      .then((r) => (app.notify(msg ?? app.t("common.done") + (r && (r as any).consolidated !== undefined ? ` (${(r as any).decayed} / ${(r as any).consolidated})` : "")), load()))
      .catch((e) => app.notify(String(e)));
  const counts = useMemo(() => {
    const c = { lesson: 0, fact: 0, preference: 0 };
    for (const f of data?.facts ?? []) c[f.kind]++;
    return c;
  }, [data]);
  const shown = (data?.facts ?? [])
    .filter((f) => f.kind === tab)
    .filter((f) => showFaded || f.weight >= (data?.hide_below ?? 0.2))
    .filter((f) => !q.trim() || `${f.text} ${f.cue ?? ""}`.toLowerCase().includes(q.trim().toLowerCase()))
    .sort((a, b) => (b.validated?.length ?? 0) - (a.validated?.length ?? 0) || b.confidence - a.confidence || b.id - a.id);
  const current = agents.find((a) => a.agent.id === agent);
  return (
    <div className="space-y-4">
      <PageHeader title={app.t("nav.memory")} sub={app.t("memory.sub")}>
        <button className="btn" disabled={!agent} onClick={() => act("tidy", {})} title={app.t("memory.tidyHint")}>
          <Wand2 className="h-3.5 w-3.5" /> {app.t("memory.tidy")}
        </button>
      </PageHeader>
      <EmbedderNote />
      <div className="grid gap-4 lg:grid-cols-[280px_minmax(0,1fr)]">
        <Section title={app.t("nav.agents")}>
          <div className="space-y-1">
            {agents.map((a) => (
              <button
                key={a.agent.id}
                onClick={() => setAgent(a.agent.id)}
                className={cn("flex w-full items-center gap-2.5 rounded-lg px-2.5 py-2 text-left text-xs", a.agent.id === agent ? "bg-blue-600/15 text-blue-600 dark:text-blue-400" : "hover:bg-elevated")}
              >
                <Avatar name={a.agent.name} className="h-7 w-7 text-[11px]" />
                <span className="min-w-0 flex-1">
                  <span className="block truncate font-medium">{a.agent.name}</span>
                  <span className="block truncate text-[11px] text-subtle">
                    {a.agent.role}
                    {a.team ? ` · ${a.team}` : ""}
                  </span>
                </span>
              </button>
            ))}
            {agents.length === 0 && <Empty>{app.t("common.none")}</Empty>}
          </div>
        </Section>
        <div className="min-w-0 space-y-3">
          <div className="flex flex-wrap items-center gap-2">
            {current && (
              <span className="flex items-center gap-1.5 text-sm font-semibold">
                <Brain className="h-4 w-4 text-purple-500" /> {current.agent.name}
              </span>
            )}
            <Segmented
              id="memory-tab"
              value={tab}
              onChange={setTab}
              items={[
                { value: "lesson" as const, label: `${app.t("memory.lessons")} (${counts.lesson})` },
                { value: "fact" as const, label: `${app.t("memory.facts")} (${counts.fact})` },
                { value: "preference" as const, label: `${app.t("memory.preferences")} (${counts.preference})` },
                { value: "overlay" as const, label: `${app.t("memory.overlays")} (${(data?.overlays.length ?? 0) + Object.keys(data?.created ?? {}).length})` },
              ]}
            />
            {tab !== "overlay" && (
              <>
                <div className="relative ml-auto">
                  <Search className="absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-subtle" />
                  <input className="input w-72 pl-8" placeholder={app.t("memory.search")} value={q} onChange={(e) => setQ(e.target.value)} />
                </div>
                <label className="flex items-center gap-1.5 text-xs text-muted">
                  <input type="checkbox" checked={showFaded} onChange={(e) => setShowFaded(e.target.checked)} /> {app.t("memory.showFaded")}
                </label>
              </>
            )}
          </div>
          {tab === "lesson" && <div className="flex items-start gap-2 rounded-lg border border-line bg-elevated/40 p-2.5 text-[11px] text-muted"><Lightbulb className="mt-0.5 h-3.5 w-3.5 shrink-0 text-amber-500" />{app.t("memory.lessonNote")}</div>}
          {!data ? (
            <div className="text-xs text-subtle">{app.t("common.loading")}</div>
          ) : tab === "overlay" ? (
            <div className="space-y-3">
              {/* ADR-91: skills the agent created itself — the lesson behind each, flagged when it stops holding. */}
              {Object.entries(data.created ?? {}).map(([name, c]) => (
                <Section
                  key={`created-${name}`}
                  title={
                    <span className="flex items-center gap-2 normal-case">
                      <Sparkles className="h-3.5 w-3.5 text-emerald-500" /> {name}
                      <span className="chip border-emerald-500/40 bg-emerald-500/10 text-emerald-600 dark:text-emerald-400">{app.t("memory.created")}</span>
                      {c.flagged && (
                        <span className="chip border-amber-500/40 bg-amber-500/10 text-amber-600 dark:text-amber-400" title={c.flagged}>
                          {app.t("memory.flagged")}
                        </span>
                      )}
                    </span>
                  }
                  right={
                    <button className="btn" onClick={() => act("remove_skill", { skill: name }, app.t("memory.skillRemoved"))}>
                      <Trash2 className="h-3.5 w-3.5" /> {app.t("memory.removeSkill")}
                    </button>
                  }
                >
                  <div className="space-y-1 text-[11px]">
                    <div>
                      <Mono className="text-subtle">{ago(c.at)}</Mono> <b>{c.reason}</b>
                    </div>
                    <div className="text-muted">
                      {app.t("memory.fromLesson")} #{c.lesson}: {c.lesson_text}
                    </div>
                    {c.flagged && <div className="text-amber-600 dark:text-amber-400">{c.flagged}</div>}
                  </div>
                </Section>
              ))}
              {data.overlays.map((o) => (
                <Section
                  key={o.skill}
                  title={
                    <span className="flex items-center gap-2 normal-case">
                      <Sparkles className="h-3.5 w-3.5 text-purple-500" /> {o.skill}
                      <span className="chip border-line-strong text-muted">
                        {app.t("memory.base")} {o.base} · {o.base_version}
                      </span>
                      {o.stale && (
                        <span className="chip border-amber-500/40 bg-amber-500/10 text-amber-600 dark:text-amber-400">
                          {app.t("memory.stale")}
                        </span>
                      )}
                    </span>
                  }
                  right={
                    <span className="flex gap-1.5">
                      {o.stale && (
                        <button className="btn" onClick={() => act("rebase", {}, app.t("memory.rebased"))} title={app.t("memory.rebaseHint")}>
                          <GitMerge className="h-3.5 w-3.5" /> {app.t("memory.rebase")}
                        </button>
                      )}
                      <button className="btn" onClick={() => act("revert", { skill: o.skill })}>
                        <RotateCcw className="h-3.5 w-3.5" /> {app.t("memory.revert")}
                      </button>
                      <button className="btn-primary" onClick={() => act("promote", { skill: o.skill }, app.t("memory.promoted"))} title={app.t("memory.promoteHint")}>
                        <GitMerge className="h-3.5 w-3.5" /> {app.t("memory.promote")}
                      </button>
                    </span>
                  }
                >
                  <div className="mb-3 space-y-1.5">
                    {o.changelog.map((c, i) => (
                      <div key={i} className="rounded-lg bg-elevated px-2.5 py-1.5 text-[11px]">
                        <Mono className="text-subtle">{c.at ? ago(c.at) : ""}</Mono> <b>{c.reason}</b>
                        {c.lesson_text && (
                          <div className="text-muted">
                            {app.t("memory.fromLesson")} #{c.lesson}: {c.lesson_text}
                          </div>
                        )}
                      </div>
                    ))}
                  </div>
                  <DiffView diff={o.diff} />
                </Section>
              ))}
              {data.overlays.length === 0 && Object.keys(data.created ?? {}).length === 0 && <Empty>{app.t("memory.noOverlays")}</Empty>}
            </div>
          ) : (
            <div className="space-y-2">
              {shown.map((f) => (
                <FactRow key={f.id} f={f} hideBelow={data.hide_below} onRetire={() => act("retire", { fact: f.id })} />
              ))}
              {shown.length === 0 && <Empty>{app.t("common.none")}</Empty>}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

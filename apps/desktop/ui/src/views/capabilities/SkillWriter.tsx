// "Write a skill with an agent" (ADR-151), beside writing one by hand: the user says what the skill should do and adds
// documents (any format, read as Markdown); a model of their choosing writes it to Agenium's skill contract. It sees
// the whole library (ADR-152): a draft close to a library skill says so, and "merge into" asks it to improve that skill
// instead, opened in the editor beside the skill as it is. An agent's proposal can be discussed here too: the chat
// starts from it and its evidence. The draft opens in the skill editor, where the user evaluates and saves it.
import { AlertTriangle, ArrowRight, GitMerge, Wand2 } from "lucide-react";
import { useState } from "react";
import { HelperChat } from "../../components/HelperChat";
import { Markdown } from "../../components/Markdown";
import { Mono, cn } from "../../components/ui";
import { api, type NewSkill, type SkillDoc, type SkillProposal, type SkillWrite } from "../../lib/api";
import { friendly } from "../../lib/errors";
import { useApp } from "../../lib/store";
import { ScorecardView, useScorecard } from "./Scorecard";
import { SkillEditor } from "./SkillEditor";

type Draft = NonNullable<SkillWrite["draft"]>;

export function SkillWriter({
  seed,
  proposal,
  onClose,
  onSaved,
}: {
  seed?: NewSkill;
  /** An agent's proposal to discuss (ADR-152): the chat starts from it and its evidence. */
  proposal?: SkillProposal;
  onClose: () => void;
  onSaved: (name: string) => void;
}) {
  const app = useApp();
  const [d, setDraft] = useState<Draft | null>(null);
  const [session, setSession] = useState<string | null>(null);
  const [withDocs, setWithDocs] = useState(false);
  const [mergeInto, setMergeInto] = useState<string | null>(null);
  const [queued, setQueued] = useState<{ text: string; n: number } | null>(null);
  const [editing, setEditing] = useState<{ files0: string[]; original?: string } | null>(null);
  const origin = withDocs ? "writer-docs" : "writer";
  const card = useScorecard(d ? { name: d.name, description: d.description, body: d.body, origin, skip: d.extends ?? null } : null);

  const first = proposal
    ? app
        .t("skillWriter.proposalMessage")
        .replace("{agent}", proposal.agent_name || "–")
        .replace("{name}", proposal.name)
        .replace("{n}", String(proposal.contexts.length))
        .replace("{lesson}", proposal.lesson_text.trim())
    : seed
      ? app
          .t("skillWriter.seedMessage")
          .replace("{name}", seed.name)
          .replace("{does}", seed.does.trim().replace(/\.+$/, "") || "–")
          // The designer often writes "Use when …" itself.
          .replace("{when}", seed.when.trim().replace(/^use (it )?when:?\s*/i, "").replace(/\.+$/, "") || "–")
      : undefined;
  // The proposal and its evidence are material for the writer, read like any document the user attaches.
  const docs0: SkillDoc[] | undefined = proposal
    ? [
        { name: `${proposal.name}.SKILL.md`, text: proposal.text },
        {
          name: "evidence.md",
          text: `Lesson: ${proposal.lesson_text}\n\nHeld in: ${proposal.contexts.join(", ")}\n\nWhy the agent proposes it: ${proposal.reason}${proposal.notes.length ? `\n\nNotes: ${proposal.notes.join("; ")}` : ""}`,
        },
      ]
    : undefined;

  const merge = (name: string) => {
    setMergeInto(name);
    setQueued((q) => ({ text: app.t("skillWriter.mergeMessage").replaceAll("{name}", name), n: (q?.n ?? 0) + 1 }));
  };
  const open = () => {
    if (!d) return;
    if (!d.extends) return setEditing({ files0: [] });
    api.skillText(d.extends).then(
      (t) => setEditing({ files0: t.files, original: d.original ?? t.raw }),
      (e) => app.notify(friendly(e, app.t)),
    );
  };
  const saved = (name: string) => {
    if (session) api.finishSession(session, name).catch(() => undefined);
    if (proposal)
      api
        .decideSkillProposal(proposal.id, true, name)
        .then(() => app.notify(app.t("proposal.saved").replace("{name}", name).replace("{agent}", proposal.agent_name || "–")))
        .catch((e) => app.notify(friendly(e, app.t)));
    onSaved(name);
  };

  return (
    <>
      <HelperChat<SkillWrite>
        kind="skill"
        prefKey="model.skill_writer"
        title={proposal ? `${app.t("skillWriter.title")} · ${proposal.name}` : app.t("skillWriter.title")}
        intro={app.t("skillWriter.intro")}
        placeholder={app.t("skillWriter.placeholder")}
        first={first}
        initialDocs={docs0}
        source={proposal ? { proposal: proposal.id } : seed ? { seed } : null}
        queued={queued}
        ask={(m, turns, docs, s) => {
          setWithDocs(docs.length > 0);
          return api.writeSkill(m, turns, docs, seed ?? null, mergeInto, s);
        }}
        onAnswer={(a) => {
          setMergeInto(null);
          if (a.draft) setDraft(a.draft);
        }}
        onSession={setSession}
        onRestore={(draft) => setDraft((draft as Draft | null) ?? null)}
        onClose={onClose}
      >
        {!d ? (
          <div className="flex h-full flex-col items-center justify-center gap-2 text-center text-xs text-subtle">
            <Wand2 className="h-6 w-6" />
            <div className="max-w-md leading-relaxed">{app.t("skillWriter.empty")}</div>
          </div>
        ) : (
          <div className="space-y-3">
            <div className="flex items-start gap-3">
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  <Mono className="text-sm font-semibold">{d.name}</Mono>
                  {d.tags.map((t) => (
                    <span key={t} className="chip border-indigo-500/30 text-indigo-600 dark:text-indigo-300">
                      {t}
                    </span>
                  ))}
                </div>
                <div className="mt-1 text-xs text-muted">{d.description}</div>
              </div>
              <button className="btn-primary shrink-0" onClick={open}>
                {app.t("skillWriter.open")} <ArrowRight className="h-3.5 w-3.5" />
              </button>
            </div>
            {d.extends && (
              <div className="flex items-start gap-1.5 rounded-lg border border-cyan-500/30 bg-cyan-500/5 p-2 text-[11px] text-cyan-700 dark:text-cyan-300">
                <GitMerge className="mt-0.5 h-3 w-3 shrink-0" /> {app.t("skillWriter.extends").replace("{name}", d.extends)}
              </div>
            )}
            {!d.extends && d.similar.length > 0 && (
              <div className="rounded-lg border border-amber-500/30 p-2">
                <div className="label mb-1.5">{app.t("skillWriter.similar")}</div>
                <div className="space-y-1.5">
                  {d.similar.map((x) => (
                    <div key={x.name} className="flex items-center gap-2 text-xs">
                      <Mono className="font-semibold">{x.name}</Mono>
                      <span className={cn("chip", x.level === "duplicate" ? "border-rose-500/40 text-rose-600 dark:text-rose-400" : "border-amber-500/40 text-amber-600 dark:text-amber-400")}>
                        {app.t(x.level === "duplicate" ? "skillWriter.level.duplicate" : "skillWriter.level.near")}
                      </span>
                      <span className="text-[11px] text-subtle">
                        {app.t("skillWriter.scores")
                          .replace("{w}", String(Math.round(x.words * 100)))
                          .replace("{m}", x.meaning == null ? "–" : String(Math.round(x.meaning * 100)))}
                      </span>
                      <button className="btn ml-auto h-7 px-2 text-[11px]" onClick={() => merge(x.name)}>
                        <GitMerge className="h-3.5 w-3.5" /> {app.t("skillWriter.merge").replace("{name}", x.name)}
                      </button>
                    </div>
                  ))}
                </div>
              </div>
            )}
            {d.exists && !d.extends && (
              <div className="flex items-start gap-1.5 text-[11px] text-amber-600 dark:text-amber-400">
                <AlertTriangle className="mt-0.5 h-3 w-3 shrink-0" /> {app.t("skillWriter.exists")}
              </div>
            )}
            {d.notes.map((n, i) => (
              <div key={i} className="flex items-start gap-1.5 text-[11px] text-amber-600 dark:text-amber-400">
                <AlertTriangle className="mt-0.5 h-3 w-3 shrink-0" /> {n}
              </div>
            ))}
            <div className="rounded-lg border border-line p-2.5">
              <ScorecardView card={card} />
            </div>
            {d.files.length > 0 && (
              <div className="font-mono text-[11px] text-muted">
                {app.t("skillWriter.files")}: {d.files.map((f) => f.path).join(" · ")}
              </div>
            )}
            <div className="rounded-lg border border-line bg-elevated/40 p-3 text-xs">
              <Markdown text={d.body} />
            </div>
          </div>
        )}
      </HelperChat>
      {editing && d && (
        <SkillEditor
          initial={d.skill_md}
          create={!d.extends}
          name0={d.extends ?? ""}
          files0={editing.files0}
          newFiles0={d.files}
          original={editing.original}
          origin={proposal ? `proposal:${proposal.contexts.length}` : origin}
          onClose={() => setEditing(null)}
          onSaved={(name) => name && saved(name)}
        />
      )}
    </>
  );
}

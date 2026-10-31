import { useEffect, useState } from "react";
import { ConfirmModal } from "../components/motion";
import { Mono, Section, ago, cn } from "../components/ui";
import { api } from "../lib/api";
import { useApp } from "../lib/store";
import type { Key } from "../lib/i18n";
import type { JudgeActivity, JudgeConfig, JudgeForm, JudgeKeyState, JudgeUses } from "../lib/types";

/** The optional TypeSafe Jev judge (ADR-75, ADR-81, ADR-126..128): endpoint, model, named keys (values in the vault;
 *  they take turns; a key out of quota is set aside until the user re-enables it, the user being told which; a failing
 *  key rests 30 minutes to an hour and comes back by itself),
 *  what it may do (every use on by default), and what it was asked lately. Without a usable key Agenium works as
 *  before, only without the judge's help. */
export function JudgeSection() {
  const app = useApp();
  const [c, setC] = useState<JudgeConfig | null>(null);
  const [form, setForm] = useState<JudgeForm | null>(null);
  const [busy, setBusy] = useState(false);
  const [activity, setActivity] = useState<JudgeActivity[]>([]);
  useEffect(() => {
    api.judgeActivity().then((a) => setActivity(a ?? [])).catch(() => undefined);
  }, [app.tick, busy]);
  const load = () =>
    api
      .judgeConfig()
      .then((j) => {
        setC(j);
        setForm({
          base_url: j.base_url || "https://api.typesafe.ai",
          model: j.model || "jev-1.14.0",
          review_assist: j.review_assist,
          skill_hint: j.skill_hint,
          memory_gate: j.memory_gate,
          memory_settle: j.memory_settle,
          skill_quality: j.skill_quality,
          done_check: j.done_check,
          trim_output: j.trim_output,
          find_code: j.find_code,
          shadow: j.shadow,
        });
      })
      .catch(() => undefined);
  useEffect(() => {
    load();
  }, []);
  if (!c || !form) return null;
  const run = (p: Promise<unknown>, done: string) => {
    setBusy(true);
    p.then(() => (app.notify(done), load(), app.refresh()))
      .catch((e) => (app.notify(String(e)), load()))
      .finally(() => setBusy(false));
  };
  const uses: [keyof JudgeUses, Key][] = [
    ["review_assist", "settings.judgeReview"],
    ["skill_hint", "settings.judgeSkill"],
    ["memory_gate", "settings.judgeMemory"],
    ["memory_settle", "settings.judgeSettle"],
    ["skill_quality", "settings.judgeQuality"],
    ["done_check", "settings.judgeDone"],
    ["trim_output", "settings.judgeTrim"],
    ["find_code", "settings.judgeFindCode"],
    ["shadow", "settings.judgeShadow"],
  ];
  return (
    <Section title={app.t("settings.judge")}>
      <div className="text-[11px] text-subtle">{app.t("settings.judgeNote")}</div>
      <div className="mt-2 text-xs">
        {!c.configured ? (
          <span className="text-muted">{app.t("settings.judgeOff")}</span>
        ) : c.disabled ? (
          <span className="text-rose-600 dark:text-rose-400">{app.t("settings.judgeAllOff")}</span>
        ) : (
          <span className="text-emerald-600 dark:text-emerald-400">{app.t("settings.judgeOn")}</span>
        )}
      </div>
      <div className="mt-3 grid gap-2 md:grid-cols-2">
        <label className="block text-xs">
          <span className="label">{app.t("settings.judgeEndpoint")}</span>
          <input className="input mt-1" value={form.base_url} onChange={(e) => setForm({ ...form, base_url: e.target.value })} />
        </label>
        <label className="block text-xs">
          <span className="label">{app.t("settings.judgeModel")}</span>
          <input className="input mt-1" value={form.model} onChange={(e) => setForm({ ...form, model: e.target.value })} />
        </label>
      </div>
      <JudgeKeys keys={c.keys} busy={busy} run={run} />
      <div className="mt-3 label">{app.t("settings.judgeUses")}</div>
      <div className="mt-1 space-y-1">
        {uses.map(([k, label]) => (
          <label key={k} className="flex items-center gap-2 text-xs">
            <input type="checkbox" checked={form[k]} onChange={(e) => setForm({ ...form, [k]: e.target.checked })} />
            <span title={k}>{app.t(label)}</span>
          </label>
        ))}
      </div>
      <div className="mt-3 flex flex-wrap gap-2">
        <button className="btn-primary" disabled={busy || !form.base_url.trim() || !form.model.trim()} onClick={() => run(api.setJudge(form), app.t("settings.judgeSaved"))}>
          {app.t("settings.judgeSave")}
        </button>
        {c.configured && (
          <button className="btn" disabled={busy} onClick={() => run(api.setJudge({ ...form, base_url: "" }), app.t("settings.judgeSaved"))}>
            {app.t("settings.judgeTurnOff")}
          </button>
        )}
      </div>
      <JudgeActivityList activity={activity} />
    </Section>
  );
}

/** The judge's keys in the order they are used, with their state and the actions on each, and a form to add one. */
function JudgeKeys({ keys, busy, run }: { keys: JudgeKeyState[]; busy: boolean; run: (p: Promise<unknown>, done: string) => void }) {
  const app = useApp();
  const [name, setName] = useState("");
  const [value, setValue] = useState("");
  // Removing a key deletes its stored value too: asked first (audit L12, 2026-10-31).
  const [removing, setRemoving] = useState<string | null>(null);
  const resting = (k: JudgeKeyState) => !!k.off && !k.off.quota && !!k.off.until && new Date(k.off.until) > new Date();
  const out = (k: JudgeKeyState) => !!k.off && (k.off.quota || resting(k));
  return (
    <div className="mt-3">
      <div className="label">{app.t("settings.judgeKeys")}</div>
      {keys.length === 0 ? (
        <div className="mt-1 text-[11px] text-subtle">{app.t("settings.judgeNoKeys")}</div>
      ) : (
        <div className="mt-1 space-y-1">
          {keys.map((k) => (
            <div key={k.name} className={cn("flex flex-wrap items-center gap-2 rounded-lg border px-2.5 py-1.5 text-xs", out(k) ? (k.off?.quota ? "border-rose-500/40 bg-rose-500/5" : "border-amber-500/40 bg-amber-500/5") : "border-line")}>
              <Mono className="font-medium">{k.name}</Mono>
              <span className={cn("text-[11px]", k.off?.quota ? "text-rose-600 dark:text-rose-300" : resting(k) ? "text-amber-600 dark:text-amber-300" : k.has_value ? "text-emerald-600 dark:text-emerald-400" : "text-subtle")}>
                {k.off?.quota
                  ? app.t("settings.judgeKeyQuota")
                  : resting(k)
                    ? `${app.t("settings.judgeKeyResting")} ${new Date(k.off!.until!).toLocaleTimeString()}`
                    : !k.has_value
                      ? app.t("settings.judgeKeyNoValue")
                      : app.t("settings.judgeKeyInUse")}
              </span>
              {out(k) && <span className="min-w-0 flex-1 truncate text-[11px] text-subtle" title={k.off!.reason}>{k.off!.reason}</span>}
              <span className="ml-auto flex gap-1.5">
                {out(k) && (
                  <button className="btn" disabled={busy} onClick={() => run(api.enableJudge(k.name), app.t("settings.judgeEnabled"))}>
                    {app.t("settings.judgeKeyEnable")}
                  </button>
                )}
                <button className="btn" disabled={busy} onClick={() => setRemoving(k.name)}>
                  {app.t("settings.judgeKeyRemove")}
                </button>
              </span>
            </div>
          ))}
        </div>
      )}
      <div className="mt-2 flex flex-wrap items-end gap-2">
        <label className="block text-xs">
          <span className="label">{app.t("settings.judgeKeyName")}</span>
          <input className="input mt-1 w-40" value={name} onChange={(e) => setName(e.target.value)} />
        </label>
        <label className="block min-w-[16rem] flex-1 text-xs">
          <span className="label">{app.t("settings.judgeKey")}</span>
          <input className="input mt-1" type="password" autoComplete="off" value={value} onChange={(e) => setValue(e.target.value)} />
        </label>
        <button
          className="btn"
          disabled={busy || !name.trim() || !value.trim()}
          onClick={() => {
            run(
              api.judgeAddKey(name.trim(), value.trim()).then(() => api.enableJudge(name.trim())),
              app.t("settings.judgeEnabled"),
            );
            setName("");
            setValue("");
          }}
        >
          {app.t("settings.judgeKeyAdd")}
        </button>
      </div>
      <ConfirmModal
        open={!!removing}
        title={`${app.t("settings.judgeKeyRemove")} ${removing ?? ""}`}
        note={app.t("settings.judgeKeyRemoveNote")}
        confirm={app.t("settings.judgeKeyRemove")}
        onClose={() => setRemoving(null)}
        onConfirm={() => {
          const n = removing!;
          setRemoving(null);
          run(api.judgeRemoveKey(n), app.t("settings.judgeSaved"));
        }}
      />
    </div>
  );
}

/** What the judge was asked lately, newest first. */
function JudgeActivityList({ activity }: { activity: JudgeActivity[] }) {
  const app = useApp();
  return (
    <>
      <div className="mt-4 label">{app.t("settings.judgeActivity")}</div>
      {activity.length === 0 ? (
        <div className="mt-1 text-[11px] text-subtle">{app.t("settings.judgeNoActivity")}</div>
      ) : (
        <div className="mt-1 max-h-72 space-y-1 overflow-y-auto">
          {activity.map((x, i) => (
            <div key={i} className={cn("flex items-start gap-2 rounded-lg border px-2.5 py-1.5 text-[11px]", x.ok ? "border-line" : "border-rose-500/40 bg-rose-500/5")}>
              <span className="chip shrink-0 border-fuchsia-500/40 bg-fuchsia-500/10 text-fuchsia-600 dark:text-fuchsia-300">Jev</span>
              <Mono className="shrink-0 text-subtle">{x.use}</Mono>
              {x.ticket && <Mono className="shrink-0 text-cyan-600 dark:text-cyan-400">{x.ticket}</Mono>}
              <span className={cn("min-w-0 flex-1", !x.ok && "text-rose-600 dark:text-rose-300")}>{x.outcome}</span>
              <Mono className="shrink-0 text-subtle">
                {x.key ? `${x.key} · ` : ""}
                {x.ms} ms · {ago(x.at)}
              </Mono>
            </div>
          ))}
        </div>
      )}
    </>
  );
}

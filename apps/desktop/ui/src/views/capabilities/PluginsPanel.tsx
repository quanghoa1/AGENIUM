import { Download, Package, RefreshCw, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import { ConfirmModal, Modal } from "../../components/motion";
import { Empty, Mono, Section } from "../../components/ui";
import { api } from "../../lib/api";
import { useApp } from "../../lib/store";
import type { Plugin, PluginPreview } from "../../lib/types";

const short = (c: string) => (c.length === 40 ? c.slice(0, 7) : c);

/** Plugins (ADR-157): a vendor's skills and MCP servers imported from its repository, shown before anything is
 *  written, updated whole from the vendor, read-only for the agents, switched on per agent in its Plugin tab. */
export function PluginsPanel() {
  const app = useApp();
  const [list, setList] = useState<{ plugin: Plugin; users: string[] }[] | null>(null);
  const [source, setSource] = useState("");
  const [entry, setEntry] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [preview, setPreview] = useState<PluginPreview | null>(null);
  const [del, setDel] = useState<Plugin | null>(null);
  const load = () => api.plugins().then(setList).catch((e) => app.notify(String(e)));
  useEffect(() => {
    load();
  }, [app.tick]);
  const run = (key: string, p: Promise<unknown>) => {
    setBusy(key);
    p.catch((e) => app.notify(String(e))).finally(() => setBusy(null));
  };
  const look = () => run("preview", api.previewPlugin(source.trim(), entry.trim()).then(setPreview));
  const checkUpdate = (name: string) => run(`update:${name}`, api.previewPluginUpdate(name).then(setPreview));
  const install = (p: PluginPreview) =>
    run(
      "install",
      api.installPlugin(p.plugin).then((x) => {
        app.notify(`${app.t("plugins.installed")} ${x.name} @ ${short(x.commit)}`);
        setPreview(null);
        setSource("");
        setEntry("");
        load();
        app.refresh();
      }),
    );
  const same = preview?.installed && preview.installed.commit === preview.plugin.commit;
  return (
    <div className="space-y-4">
      <Section title={app.t("plugins.add")}>
        <div className="mb-2 text-[11px] text-subtle">{app.t("plugins.addNote")}</div>
        <div className="flex flex-wrap gap-2">
          <input className="input min-w-[260px] flex-1" placeholder="obra/superpowers" value={source} onChange={(e) => setSource(e.target.value)} onKeyDown={(e) => e.key === "Enter" && source.trim() && look()} />
          <input className="input w-44" placeholder={app.t("plugins.entry")} title={app.t("plugins.entryHint")} value={entry} onChange={(e) => setEntry(e.target.value)} />
          <button className="btn-primary" disabled={!source.trim() || busy !== null} onClick={look}>
            <Download className="h-3.5 w-3.5" /> {busy === "preview" ? app.t("common.loading") : app.t("plugins.preview")}
          </button>
        </div>
      </Section>
      <Section title={app.t("plugins.installedTitle")}>
        <div className="space-y-2">
          {(list ?? []).map(({ plugin: p, users }) => (
            <div key={p.name} className="rounded-lg border border-line p-3">
              <div className="flex flex-wrap items-center gap-2">
                <Package className="h-3.5 w-3.5 text-muted" />
                <span className="text-xs font-semibold">{p.name}</span>
                {p.version && <Mono className="text-subtle">v{p.version}</Mono>}
                <Mono className="text-subtle">
                  {p.source}@{p.reference} · {short(p.commit)}
                </Mono>
                <span className="chip border-line-strong text-muted">{app.t("plugins.readOnly")}</span>
                <span className="ml-auto flex gap-1.5">
                  <button className="btn" disabled={busy !== null} onClick={() => checkUpdate(p.name)}>
                    <RefreshCw className="h-3.5 w-3.5" /> {busy === `update:${p.name}` ? app.t("common.loading") : app.t("plugins.update")}
                  </button>
                  <button className="btn" aria-label={`${app.t("plugins.remove")} ${p.name}`} onClick={() => setDel(p)}>
                    <Trash2 className="h-3.5 w-3.5" />
                  </button>
                </span>
              </div>
              {p.description && <div className="mt-1 text-[11px] text-subtle">{p.description}</div>}
              <div className="mt-1 text-[11px] text-muted">
                {app.t("plugins.counts").replace("{skills}", String(Object.keys(p.skills).length)).replace("{mcp}", String(p.mcp.length))}
                {p.hooks.length > 0 && ` · ${app.t("plugins.hookCount").replace("{n}", String(p.hooks.length))}`}
                {" · "}
                {users.length ? `${app.t("plugins.usedBy")} ${users.join(", ")}` : app.t("plugins.noUsers")}
              </div>
            </div>
          ))}
          {list && list.length === 0 && <Empty>{app.t("plugins.empty")}</Empty>}
        </div>
      </Section>
      <Modal open={!!preview} wide title={preview ? `${preview.plugin.name}${preview.plugin.version ? ` v${preview.plugin.version}` : ""} · ${short(preview.plugin.commit)}` : ""} onClose={() => setPreview(null)}>
        {preview && (
          <div className="space-y-3 text-xs">
            {preview.plugin.description && <div className="text-muted">{preview.plugin.description}</div>}
            {preview.installed && (
              <div className="rounded-lg border border-line p-2">
                {same ? app.t("plugins.upToDate") : `${app.t("plugins.replaces")} ${short(preview.installed.commit)} → ${short(preview.plugin.commit)}`}
                {preview.agents.length > 0 && ` · ${app.t("plugins.usedBy")} ${preview.agents.join(", ")}`}
              </div>
            )}
            <Pieces title={app.t("plugins.skills")} all={Object.keys(preview.plugin.skills)} added={preview.installed ? preview.skills_added : []} changed={preview.skills_changed} removed={preview.skills_removed} />
            <Pieces title="MCP" all={preview.plugin.mcp.map((m) => `${m.id}: ${m.url ?? [m.command, ...(m.args ?? [])].join(" ")}`)} added={preview.installed ? preview.mcp_added : []} changed={preview.mcp_changed} removed={preview.mcp_removed} />
            {preview.plugin.hooks.length > 0 && (
              <div>
                <div className="label mb-1">{app.t("plugins.hooks")}</div>
                <div className="mb-1 text-[11px] text-subtle">{app.t("plugins.hooksNote")}</div>
                <ul className="space-y-0.5 font-mono text-[11px]">
                  {preview.plugin.hooks.map((h) => (
                    <li key={h} className="break-all rounded border border-line px-1.5 py-0.5">
                      {h}
                    </li>
                  ))}
                </ul>
              </div>
            )}
            {preview.plugin.skipped.length > 0 && (
              <div>
                <div className="label mb-1">{app.t("plugins.skipped")}</div>
                <ul className="list-disc space-y-0.5 pl-4 text-[11px] text-subtle">
                  {preview.plugin.skipped.map((s) => (
                    <li key={s}>{s}</li>
                  ))}
                </ul>
              </div>
            )}
            <div className="text-[11px] text-subtle">{app.t("plugins.trust")}</div>
            <div className="flex justify-end gap-2">
              <button className="btn" onClick={() => setPreview(null)}>
                {app.t("common.cancel")}
              </button>
              <button className="btn-primary" disabled={busy !== null || !!same} onClick={() => install(preview)}>
                <Download className="h-3.5 w-3.5" /> {busy === "install" ? app.t("common.loading") : preview.installed ? app.t("plugins.applyUpdate") : app.t("plugins.install")}
              </button>
            </div>
          </div>
        )}
      </Modal>
      <ConfirmModal
        open={!!del}
        title={`${app.t("plugins.remove")} ${del?.name ?? ""}`}
        note={app.t("plugins.removeNote")}
        confirm={app.t("plugins.remove")}
        onClose={() => setDel(null)}
        onConfirm={() => {
          const p = del;
          setDel(null);
          if (p) api.removePlugin(p.name).then(() => (load(), app.refresh())).catch((e) => app.notify(String(e)));
        }}
      />
    </div>
  );
}

/** A plugin's skills or servers, the ones an update adds, changes or takes away marked. */
function Pieces({ title, all, added, changed, removed }: { title: string; all: string[]; added: string[]; changed: string[]; removed: string[] }) {
  const app = useApp();
  const mark = (x: string) => {
    const id = x.split(":")[0];
    if (added.includes(id)) return <span className="chip border-emerald-500/40 text-emerald-600 dark:text-emerald-300">{app.t("plugins.new")}</span>;
    if (changed.includes(id)) return <span className="chip border-amber-500/40 text-amber-600 dark:text-amber-300">{app.t("plugins.changed")}</span>;
    return null;
  };
  return (
    <div>
      <div className="label mb-1">
        {title} ({all.length})
      </div>
      {all.length === 0 && removed.length === 0 ? (
        <div className="text-[11px] text-subtle">–</div>
      ) : (
        <div className="flex flex-wrap gap-1">
          {all.map((x) => (
            <span key={x} className="inline-flex items-center gap-1 rounded border border-line px-1.5 py-0.5 font-mono text-[11px]">
              {x} {mark(x)}
            </span>
          ))}
          {removed.map((x) => (
            <span key={x} className="inline-flex items-center gap-1 rounded border border-red-500/40 px-1.5 py-0.5 font-mono text-[11px] text-red-600 line-through dark:text-red-300">
              {x}
            </span>
          ))}
        </div>
      )}
    </div>
  );
}

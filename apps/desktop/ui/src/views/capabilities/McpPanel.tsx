import { Plug, Plus, RefreshCw, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import { ConfirmModal, PanelSwap, Segmented } from "../../components/motion";
import { Empty, Mono, cn } from "../../components/ui";
import { api } from "../../lib/api";
import { useApp } from "../../lib/store";
import type { McpServerConfig, McpServerRow } from "../../lib/types";
import { ToolSearchCard } from "./ToolSearchCard";

const lines = (s: string) =>
  s
    .split("\n")
    .map((x) => x.trim())
    .filter(Boolean);

function pairs(s: string, sep: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const l of lines(s)) {
    const i = l.indexOf(sep);
    if (i > 0) out[l.slice(0, i).trim()] = l.slice(i + 1).trim();
  }
  return out;
}

export function McpPanel() {
  const app = useApp();
  const [rows, setRows] = useState<McpServerRow[] | null>(null);
  const [mode, setMode] = useState<"stdio" | "http" | "json">("stdio");
  const [form, setForm] = useState({ id: "", command: "", args: "", env: "", url: "", headers: "", timeout: 120, json: "" });
  const [tested, setTested] = useState<Record<string, { tools: { name: string; description?: string | null; read_only: boolean }[]; error?: string } | "busy">>({});
  const [del, setDel] = useState<McpServerRow | null>(null);
  const [signing, setSigning] = useState<string | null>(null);
  const load = () => api.mcpServers().then(setRows);
  const signIn = (id: string) => {
    setSigning(id);
    app.notify(app.t("library.signingIn"));
    api
      .loginMcp(id)
      .then(() => (app.notify(`${id}: ${app.t("library.signedIn")}`), setTested((t) => ({ ...t, [id]: undefined as never })), load()))
      .catch((e) => app.notify(String(e)))
      .finally(() => setSigning(null));
  };
  const signOut = (id: string) =>
    api
      .logoutMcp(id)
      .then(load)
      .catch((e) => app.notify(String(e)));
  useEffect(() => {
    load();
  }, [app.tick]);
  const set = (p: Partial<typeof form>) => setForm((f) => ({ ...f, ...p }));
  const save = () => {
    const done = () => (app.notify(app.t("common.done")), setForm({ id: "", command: "", args: "", env: "", url: "", headers: "", timeout: 120, json: "" }), load());
    if (mode === "json") {
      api
        .importMcpJson(form.json)
        .then((r) => (app.notify(r.servers.join(", ")), done()))
        .catch((e) => app.notify(String(e)));
      return;
    }
    const server: McpServerConfig =
      mode === "stdio"
        ? { id: form.id.trim(), command: form.command.trim(), args: lines(form.args), env: pairs(form.env, "="), timeout_secs: form.timeout }
        : { id: form.id.trim(), url: form.url.trim(), headers: pairs(form.headers, ":"), timeout_secs: form.timeout };
    api
      .putMcp(server)
      .then(done)
      .catch((e) => app.notify(String(e)));
  };
  const test = (id: string) => {
    setTested((t) => ({ ...t, [id]: "busy" }));
    api
      .testMcp(id)
      .then((r) => setTested((t) => ({ ...t, [id]: { tools: r.tools } })))
      .catch((e) => setTested((t) => ({ ...t, [id]: { tools: [], error: String(e) } })));
  };
  return (
    <div className="space-y-4">
      <div className="card space-y-3 p-4">
        <div className="flex flex-wrap items-center gap-3">
          <div className="flex items-center gap-2 text-xs font-semibold">
            <Plug className="h-4 w-4 text-accent" /> {app.t("library.addMcp")}
          </div>
          <Segmented
            id="mcp-mode"
            value={mode}
            onChange={setMode}
            items={[
              { value: "stdio", label: app.t("library.stdio") },
              { value: "http", label: "URL (HTTP)" },
              { value: "json", label: app.t("library.pasteJson") },
            ]}
          />
        </div>
        <PanelSwap id={mode}>
          {mode === "json" ? (
            <textarea
              className="input min-h-[140px] font-mono text-[11px]"
              value={form.json}
              placeholder={'{"mcpServers": {"github": {"command": "npx", "args": ["-y", "@modelcontextprotocol/server-github"], "env": {"GITHUB_PERSONAL_ACCESS_TOKEN": "…"}}}}'}
              onChange={(e) => set({ json: e.target.value })}
            />
          ) : (
            <div className="grid gap-3 md:grid-cols-2">
              <label>
                <span className="label">id</span>
                <input className="input mt-1" value={form.id} placeholder="github" onChange={(e) => set({ id: e.target.value.toLowerCase() })} />
              </label>
              {mode === "stdio" ? (
                <label>
                  <span className="label">{app.t("library.command")}</span>
                  <input className="input mt-1 font-mono" value={form.command} placeholder="npx" onChange={(e) => set({ command: e.target.value })} />
                </label>
              ) : (
                <label>
                  <span className="label">URL</span>
                  <input className="input mt-1 font-mono" value={form.url} placeholder="https://mcp.example.com/mcp" onChange={(e) => set({ url: e.target.value })} />
                </label>
              )}
              {mode === "stdio" ? (
                <>
                  <label>
                    <span className="label">{app.t("library.args")}</span>
                    <textarea className="input mt-1 min-h-[70px] font-mono text-[11px]" value={form.args} placeholder={"-y\n@modelcontextprotocol/server-github"} onChange={(e) => set({ args: e.target.value })} />
                  </label>
                  <label>
                    <span className="label">env (KEY=value)</span>
                    <textarea className="input mt-1 min-h-[70px] font-mono text-[11px]" value={form.env} placeholder={"GITHUB_PERSONAL_ACCESS_TOKEN=ghp_…"} onChange={(e) => set({ env: e.target.value })} />
                  </label>
                </>
              ) : (
                <label className="md:col-span-2">
                  <span className="label">headers (Name: value)</span>
                  <textarea className="input mt-1 min-h-[70px] font-mono text-[11px]" value={form.headers} placeholder={"Authorization: Bearer sk-…"} onChange={(e) => set({ headers: e.target.value })} />
                </label>
              )}
            </div>
          )}
        </PanelSwap>
        <div className="text-[11px] text-subtle">{app.t("library.secretNote")}</div>
        <button className="btn-primary" disabled={mode === "json" ? !form.json.trim() : !form.id.trim() || (mode === "stdio" ? !form.command.trim() : !form.url.trim())} onClick={save}>
          <Plus className="h-3.5 w-3.5" /> {app.t("common.save")}
        </button>
      </div>
      {!rows && <div className="text-xs text-subtle">{app.t("common.loading")}</div>}
      <div className="space-y-2">
        {rows?.map((r) => {
          const t = tested[r.config.id];
          return (
            <div key={r.config.id} className="card p-4">
              <div className="flex flex-wrap items-center gap-2">
                <Plug className="h-4 w-4 text-purple-500" />
                <span className="text-sm font-semibold">{r.config.id}</span>
                <Mono className="truncate text-subtle">{r.config.url ?? [r.config.command, ...(r.config.args ?? [])].join(" ")}</Mono>
                {r.signed_in && <span className="chip border-emerald-500/30 text-emerald-600">{app.t("library.signedIn")}</span>}
                <div className="ml-auto flex gap-1.5">
                  {r.config.url &&
                    (r.signed_in ? (
                      <button className="btn" onClick={() => signOut(r.config.id)}>
                        {app.t("library.signOut")}
                      </button>
                    ) : (
                      <button className="btn" disabled={signing === r.config.id} onClick={() => signIn(r.config.id)}>
                        <RefreshCw className={cn("h-3.5 w-3.5", signing === r.config.id && "animate-spin")} /> {app.t("library.signIn")}
                      </button>
                    ))}
                  <button className="btn" disabled={t === "busy"} onClick={() => test(r.config.id)}>
                    <RefreshCw className={cn("h-3.5 w-3.5", t === "busy" && "animate-spin")} /> {app.t("library.test")}
                  </button>
                  <button className="btn" onClick={() => setDel(r)}>
                    <Trash2 className="h-3.5 w-3.5" />
                  </button>
                </div>
              </div>
              {Object.keys({ ...r.config.env, ...r.config.headers }).length > 0 && (
                <div className="mt-2 space-y-0.5">
                  <span className="label">{app.t("agents.systemKey")}</span>
                  {Object.entries({ ...r.config.env, ...r.config.headers }).map(([k, v]) => (
                    <div key={k} className="truncate font-mono text-[11px]">
                      <span className="text-subtle">{k}</span> = {v}
                    </div>
                  ))}
                </div>
              )}
              <div className="mt-2 flex flex-wrap items-center gap-1">
                <span className="label mr-1">{app.t("library.enabledBy")}</span>
                {r.users.length ? (
                  r.users.map((u) => (
                    <span key={u} className="chip border-line-strong bg-elevated text-muted">
                      {u}
                    </span>
                  ))
                ) : (
                  <span className="text-[11px] text-subtle">–</span>
                )}
              </div>
              {t && t !== "busy" && (
                <div className="mt-3 rounded-lg border border-line bg-elevated/40 p-3">
                  {t.error ? (
                    <div className="text-[11px] text-rose-600 dark:text-rose-400">{t.error}</div>
                  ) : (
                    <div className="space-y-1">
                      <div className="text-[11px] text-emerald-600 dark:text-emerald-400">
                        {app.t("library.connected")} · {t.tools.length} tool{app.lang === "en" && t.tools.length !== 1 ? "s" : ""}
                      </div>
                      {t.tools.map((x) => (
                        <div key={x.name} className="text-[11px]">
                          <Mono className="font-semibold">{x.name}</Mono>
                          {x.read_only && <span className="chip ml-1 border-emerald-500/30 text-emerald-600">read-only</span>}
                          <span className="ml-2 text-subtle">{x.description}</span>
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              )}
            </div>
          );
        })}
      </div>
      {rows && rows.length === 0 && <Empty>{app.t("library.noMcp")}</Empty>}
      <ToolSearchCard />
      <ConfirmModal
        open={!!del}
        title={`${app.t("common.delete")} ${del?.config.id ?? ""}`}
        note={del?.users.length ? `${app.t("library.inUse")} ${del.users.join(", ")}.` : app.t("library.deleteMcpNote")}
        confirm={del?.users.length ? app.t("library.deleteForce") : app.t("common.delete")}
        onClose={() => setDel(null)}
        onConfirm={() =>
          api
            .removeMcp(del!.config.id, del!.users.length > 0)
            .then(() => (setDel(null), load()))
            .catch((e) => (setDel(null), app.notify(String(e))))
        }
      />
    </div>
  );
}

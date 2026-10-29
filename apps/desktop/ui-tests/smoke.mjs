// UI smoke test of the running desktop app (debug build, WebView2 DevTools port), on a fresh root:
// setup -> project -> team -> every view renders without console errors -> provider form (preset, placeholder refusal,
// save, delete through the confirm dialog) -> agent export and import. Run by devtools/scripts/ui-smoke.ps1; no model is called.
// Usage: node smoke.mjs <screenshot-dir> [devtools-port]
import { mkdirSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";

const out = process.argv[2] ?? "ui-smoke";
const port = process.argv[3] ?? "9222";
mkdirSync(out, { recursive: true });

let targets = [];
for (let i = 0; i < 120 && !targets.some((x) => x.type === "page"); i++) {
  try {
    targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
  } catch {
    /* the app is starting */
  }
  if (!targets.some((x) => x.type === "page")) await new Promise((r) => setTimeout(r, 1000));
}
const page = targets.find((x) => x.type === "page");
if (!page) {
  console.log("FAIL no app page on the DevTools port");
  process.exit(1);
}
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((r) => ws.addEventListener("open", r, { once: true }));
let id = 0;
const pending = new Map();
const errors = [];
ws.addEventListener("message", (m) => {
  const msg = JSON.parse(m.data);
  if (msg.id && pending.has(msg.id)) (pending.get(msg.id))(msg), pending.delete(msg.id);
  if (msg.method === "Runtime.exceptionThrown")
    errors.push("exception: " + msg.params.exceptionDetails.text + " " + (msg.params.exceptionDetails.exception?.description ?? ""));
  if (msg.method === "Runtime.consoleAPICalled" && msg.params.type === "error")
    errors.push("console.error: " + msg.params.args.map((a) => a.value ?? a.description).join(" "));
});
const send = (method, params = {}) =>
  new Promise((r) => {
    const i = ++id;
    pending.set(i, r);
    ws.send(JSON.stringify({ id: i, method, params }));
  });
const js = async (expr) => (await send("Runtime.evaluate", { expression: expr, awaitPromise: true, returnByValue: true })).result?.result?.value;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const until = async (expr, ms = 20000) => {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    if (await js(expr)) return true;
    await sleep(250);
  }
  return false;
};
const shot = async (name) => {
  const s = await send("Page.captureScreenshot", { format: "png" });
  if (s.result?.data) writeFileSync(`${out}/${name}.png`, Buffer.from(s.result.data, "base64"));
};
let fails = 0;
const check = (name, ok, seen = "") => {
  if (!ok) fails++;
  console.log(`${ok ? "PASS" : "FAIL"} ${name}${seen ? `: ${seen}` : ""}`);
};
const invoke = (cmd, args = {}) => js(`window.__TAURI_INTERNALS__.invoke(${JSON.stringify(cmd)}, ${JSON.stringify(args)}).then(r => JSON.stringify(r ?? null), e => 'ERR ' + String(e))`);
const clickText = (text, scope = "document") =>
  js(`(() => {
    const all = [...${scope}.querySelectorAll('button')];
    const b = all.find(b => b.textContent.trim() === ${JSON.stringify(text)}) ?? all.find(b => b.textContent.trim().startsWith(${JSON.stringify(text)}));
    if (!b || b.disabled) return false; b.click(); return true; })()`);
const typeInto = (selector, value) =>
  js(`(() => {
    const el = document.querySelector(${JSON.stringify(selector)}); if (!el) return false;
    const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(proto, 'value').set.call(el, ${JSON.stringify(value)});
    el.dispatchEvent(new Event('input', { bubbles: true })); return true; })()`);
const mainText = () => js("document.querySelector('main')?.innerText ?? ''");

await send("Runtime.enable");
await send("Page.enable");
await js(`localStorage.setItem('agenium-lang','en')`);
await send("Page.reload");
await until("document.body.innerText.length > 20", 30000);

// 1. First start: the embedding model is put in place (the script seeds the root with it, so this is quick), then the
// setup screen, then the app.
const modelReady = await until(
  "window.__TAURI_INTERNALS__.invoke('status').then((s) => s.model?.phase === 'ready')",
  180000,
);
check("the embedding model is in place before the app opens", modelReady);
const setupShown = await until("document.body.innerText.includes('Set up') || document.body.innerText.includes('Start')", 15000);
await shot("01-setup");
const started = setupShown && (await js(`(() => { const b = document.querySelector('.btn-primary'); if (!b) return false; b.click(); return true; })()`));
const shell = await until("!!document.querySelector('main')", 30000);
check("setup opens the app", started && shell);

// 2. A project, created through the form.
await js(`window.location.hash = '/projects'`);
await until("document.querySelector('main input') !== null");
await typeInto("main input", "smoke shop");
const created = await clickText("Create project", "document.querySelector('main')");
const hasProject = await until(`document.querySelector('header select')?.innerText.includes('smoke shop') || document.body.innerText.includes('smoke shop')`, 15000);
check("a project is created from the form", created && hasProject);

// 3. A team from the coding template (Claude CLI members; nothing runs).
const status = JSON.parse((await invoke("projects")) || "[]");
const project = Array.isArray(status) ? status.find((p) => p.name === "smoke shop") : null;
const team = project
  ? await invoke("create_team", { project: project.id, template: "coding", model: { provider: "claude", model: "haiku", effort: "low", backend: "cli" } })
  : "no project";
check("a team is created from the coding template", typeof team === "string" && !team.startsWith("ERR") && team !== "no project", String(team).slice(0, 120));
// A model picked from the agent CLIs makes every member run on that CLI.
const agents = JSON.parse((await invoke("agents")) || "[]");
const backends = Array.isArray(agents) ? [...new Set(agents.map((a) => String(a.agent?.backend).toLowerCase()))] : [];
check("the members run on the picked CLI", backends.length === 1 && backends[0] === "cli", JSON.stringify(backends));
await js("location.reload()");
await until("!!document.querySelector('main')", 30000);

// 4. Every view renders, without console errors.
const views = ["overview", "tickets", "runs", "needs", "projects", "agents", "teams", "providers", "templates", "memory", "audit", "usage", "settings", "live", "library", "chat", "browser"];
for (const v of views) {
  const before = errors.length;
  await js(`window.location.hash = '/${v}'`);
  const ok = await until("(document.querySelector('main')?.innerText ?? '').replace(/Loading…?/g, '').trim().length > 10", 15000);
  await sleep(400);
  await shot(`view-${v}`);
  check(`view ${v}`, ok && errors.length === before, errors.slice(before).join(" | ").slice(0, 200));
}
const team_view = await js(`(window.location.hash = '/teams', new Promise(r => setTimeout(() => r(document.querySelector('main').innerText), 1500)))`);
check("the team shows its Lead", /lead/i.test(team_view ?? ""), "");

// 5. Providers: a preset with a placeholder is refused; a local preset is saved, then deleted through the dialog.
await js(`window.location.hash = '/providers'`);
await until("[...document.querySelectorAll('main button')].some(b => b.textContent.trim() === 'Add provider')");
await clickText("Add provider", "document.querySelector('main')");
const pick = (value) =>
  js(`(() => {
    const s = [...document.querySelectorAll('main select')].find(s => [...s.options].some(o => o.value === ${JSON.stringify(value)}));
    if (!s) return false;
    Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, 'value').set.call(s, ${JSON.stringify(value)});
    s.dispatchEvent(new Event('change', { bubbles: true })); return true; })()`);
await until("[...document.querySelectorAll('main select')].some(s => [...s.options].some(o => o.value === 'azure-openai'))");
await pick("azure-openai");
const hint = await until("document.querySelector('main').innerText.includes('Replace the <')", 5000);
// Save waits for what is missing (here the API key), then the server refuses the unfilled URL.
const saveOff = await js(`[...document.querySelectorAll('main button')].find(b => b.textContent.trim() === 'Save')?.disabled === true`);
check("Save waits for the missing API key", saveOff);
await js(`(() => { const el = document.querySelector('main input[type=password]'); if (!el) return false;
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(el, 'test-key-not-real');
  el.dispatchEvent(new Event('input', { bubbles: true })); return true; })()`);
await clickText("Save", "document.querySelector('main')");
const refused = await until("document.body.innerText.includes('replace <resource>')", 8000);
await shot("providers-placeholder");
check("an unfilled preset URL is refused", hint && refused);
await pick("ollama");
await sleep(500);
await clickText("Save", "document.querySelector('main')");
const saved = await until("[...document.querySelectorAll('main td')].some(td => td.textContent.trim() === 'ollama')", 10000);
check("a local preset is saved", saved);
const opened = await js(`(() => {
  const row = [...document.querySelectorAll('main tr')].find(tr => tr.querySelector('td')?.textContent.trim() === 'ollama');
  const b = row && [...row.querySelectorAll('button')].find(b => /remove|delete/i.test(b.getAttribute('aria-label') ?? ''));
  if (!b) return false; b.click(); return true; })()`);
const dialog = await until("[...document.querySelectorAll('.card')].some(c => c.querySelector('.btn-danger') && c.innerText.includes('Cancel'))", 5000);
await shot("providers-confirm");
await js(`[...document.querySelectorAll('.card')].find(c => c.querySelector('.btn-danger') && c.innerText.includes('Cancel'))?.querySelector('.btn-danger')?.click()`);
const gone = await until("![...document.querySelectorAll('main td')].some(td => td.textContent.trim() === 'ollama')", 10000);
check("a provider is deleted through the confirm dialog", opened && dialog && gone);

// 6. Agent export and import: the team's Lead, exported, joins a new project as its Solo agent through the dialog.
const all = JSON.parse((await invoke("agents")) || "[]");
const lead = Array.isArray(all) ? all.find((a) => a.is_lead) : null;
const exported = lead ? JSON.parse((await invoke("export_agent", { agent: lead.agent.id, out: resolve(out) })) || "null") : null;
await invoke("create_project", { name: "import target" });
await js("location.reload()");
await until("!!document.querySelector('main')", 30000);
await js(`window.location.hash = '/agents'`);
await until("[...document.querySelectorAll('main button')].some(b => b.textContent.trim() === 'Import agent')");
await clickText("Import agent", "document.querySelector('main')");
const dlg = "[...document.querySelectorAll('.card')].find(c => /export file/i.test(c.innerText))";
await until(`!!${dlg}`, 5000);
const setInput = (n, v) =>
  js(`(() => { const el = ${dlg}?.querySelectorAll('input')[${n}]; if (!el) return false;
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(el, ${JSON.stringify(v)});
    el.dispatchEvent(new Event('input', { bubbles: true })); return true; })()`);
await setInput(0, exported ?? "");
await setInput(1, "lead-copy");
await js(`[...(${dlg}?.querySelectorAll('button') ?? [])].find(b => b.textContent.trim() === 'Import agent')?.click()`);
const imported = await until("location.hash.startsWith('#/agent/') && document.querySelector('main').innerText.includes('lead-copy')", 20000);
await shot("agent-imported");
check("an exported agent is imported through the dialog", !!exported && imported, String(exported));

// 7. Many MCP tools (ADR-139): on the library's MCP tab, the judge set to read every search and saved to config.toml.
await js(`window.location.hash = '/library'`);
await until("[...document.querySelectorAll('main button')].some(b => b.textContent.trim() === 'MCP')");
await clickText("MCP", "document.querySelector('main')");
const tsCard = "[...document.querySelectorAll('main .card')].find(c => c.innerText.includes('Many MCP tools'))";
await until(`!!${tsCard}`, 10000);
const tsPicked = await js(`(() => {
  const s = [...(${tsCard}?.querySelectorAll('select') ?? [])].find(s => [...s.options].some(o => o.value === 'unsure'));
  if (!s) return false;
  Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, 'value').set.call(s, 'always');
  s.dispatchEvent(new Event('change', { bubbles: true })); return true; })()`);
await sleep(200);
await js(`[...(${tsCard}?.querySelectorAll('button') ?? [])].find(b => b.textContent.trim() === 'Save')?.click()`);
const tsSaved = await until(`window.__TAURI_INTERNALS__.invoke('tool_search_settings').then(s => s.judge === 'always')`, 10000);
await shot("library-mcp-tool-search");
check("the tool search settings are saved from the MCP tab", tsPicked && tsSaved);

// 8. MCP tools that ask first (ADR-143): the default set to every tool not read-only, on the same tab.
const apCard = "[...document.querySelectorAll('main .card')].find(c => c.innerText.includes('MCP tools that ask first'))";
await until(`!!${apCard}`, 10000);
await js(`[...(${apCard}?.querySelectorAll('button[role=radio]') ?? [])].find(b => b.innerText.includes('Every tool that is not read-only'))?.click()`);
const apSaved = await until(`window.__TAURI_INTERNALS__.invoke('mcp_approval').then(s => s.default === 'writes')`, 10000);
await shot("library-mcp-approval");
check("the MCP approval default is saved from the MCP tab", apSaved);
// The price view carries the models the catalogue marks deprecated (shown on runtime badges and in the model picker).
const pv = JSON.parse((await invoke("prices")) || "{}");
check("the price view lists deprecated models", !!pv && typeof pv.deprecated === "object", Object.keys(pv ?? {}).join(","));

// 9. Settling memory (ADR-145): an agent imported with three memories of its dev server — two that say the same, one
// with another port. Tidy folds the two, puts the other port to the user; the user says the other port holds.
if (exported) {
  const { createHash } = await import("node:crypto");
  const { readFileSync } = await import("node:fs");
  const lines = readFileSync(exported, "utf8").split("\n").filter(Boolean).slice(0, -1);
  const at = lines.findIndex((l) => JSON.parse(l).table === "fact");
  const fact = (id, text) =>
    JSON.stringify({ kind: "row", v: [id, text, "fact", null, "{}", null, null, "ticket:AGN-1", 0, { r: 0.6 }, { r: 1.0 }, null, null, "2026-10-29T10:00:00.000Z", "2026-10-29T10:00:00.000Z", null, null, null, "own", null, 0, "[]"] });
  lines.splice(at + 1, 0,
    fact(1, "The dev server listens on port 8080."),
    fact(2, "The dev server listens on port 8080 locally."),
    fact(3, "The dev server listens on port 9090."),
    fact(4, "Release notes list breaking changes first."));
  const sha = createHash("sha256").update(lines.map((l) => l + "\n").join("")).digest("hex");
  lines.push(JSON.stringify({ kind: "end", lines: lines.length, sha256: sha }));
  const seeded = resolve(out, "memo.agenium.jsonl");
  writeFileSync(seeded, lines.join("\n") + "\n");
  const proj = JSON.parse((await invoke("create_project", { name: "memory target" })) || "null");
  const memo = JSON.parse((await invoke("import_agent", { file: seeded, project: proj?.id ?? "", name: "memo" })) || "null");
  await js("location.reload()");
  await until("!!document.querySelector('main')", 30000);
  await js(`window.location.hash = '/memory'`);
  const memoButton = "[...document.querySelectorAll('main button')].find(b => b.innerText.includes('memo (solo)'))";
  await until(`!!${memoButton}`, 15000);
  await js(`${memoButton}?.click()`);
  await sleep(800);
  await clickText("Tidy memory", "document.querySelector('main')");
  const card = "document.querySelector('main [data-conflict]')";
  const shownConflict = await until(`!!${card} && ${card}.innerText.includes('9090') && ${card}.innerText.includes('8080')`, 30000);
  await shot("memory-conflict");
  check("tidying folds the same memories and shows the contradicting ones", shownConflict, memo?.id);
  await js(`(() => { const side = [...${card}.querySelectorAll('div')].find(d => d.querySelectorAll('button').length === 1 && d.innerText.includes('9090')); side?.querySelector('button')?.click(); })()`);
  const decided = await until(`!${card}`, 15000);
  const m = JSON.parse((await invoke("memory", { agent: memo?.id ?? "" })) || "{}");
  const live = (m.facts ?? []).map((f) => f.text);
  check(
    "the user's decision keeps the port that holds",
    decided && (m.conflicts ?? []).length === 0 && live.some((t) => t.includes("9090")) && !live.some((t) => t.includes("8080")) && live.length === 2,
    JSON.stringify(live),
  );
}

check("no console errors overall", errors.length === 0, errors.slice(0, 5).join(" | ").slice(0, 400));
console.log(fails === 0 ? "ALL PASS" : `${fails} FAILED`);
ws.close();
process.exit(fails === 0 ? 0 : 1);

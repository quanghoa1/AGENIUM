// UI smoke test of the running desktop app (debug build, WebView2 DevTools port), on a fresh root:
// setup -> project -> team -> every view renders without console errors -> provider form (preset, placeholder refusal,
// save, delete through the confirm dialog) -> agent export and import. Run by devtools/scripts/ui-smoke.ps1; no model is called.
// Usage: node smoke.mjs <screenshot-dir> [devtools-port]
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const out = process.argv[2] ?? "ui-smoke";
const port = process.argv[3] ?? "9222";
mkdirSync(out, { recursive: true });

// The app's page once it shows the app: it starts with no address, then `about:blank`, before the app's first
// navigation, and the reload below sent then cancelled that navigation for good (an empty window: "setup opens the
// app" failed now and then, 2026-10-31).
const appPage = (ts) => ts.find((x) => x.type === "page" && /^(https?|tauri):/.test(x.url));
let targets = [];
for (let i = 0; i < 120 && !appPage(targets); i++) {
  try {
    targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
  } catch {
    /* the app is starting */
  }
  if (!appPage(targets)) await new Promise((r) => setTimeout(r, 1000));
}
const page = appPage(targets);
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
// The reload only once the app's page has loaded: one sent while its first navigation was still going cancelled it,
// and the window stayed blank (`about:blank`).
await until("document.readyState === 'complete' && !!window.__TAURI_INTERNALS__", 60000);
await js(`localStorage.setItem('agenium-lang','en')`);
await send("Page.reload");
await until("document.body.innerText.length > 20", 30000);

// 1. First start: the embedding model is put in place (the script seeds the root with it, so this is quick), then the
// app opens at once on a root set up on its own (no setup screen, ADR-154); the test then speaks English.
const modelReady = await until(
  "window.__TAURI_INTERNALS__.invoke('status').then((s) => s.model?.phase === 'ready')",
  180000,
);
check("the embedding model is in place before the app opens", modelReady);
const started = await until("window.__TAURI_INTERNALS__.invoke('status').then((s) => s.ready && !s.choose_root)", 60000);
await js(`window.__TAURI_INTERNALS__.invoke('set_ui_language', { language: 'en' }).then(() => localStorage.setItem('agenium-lang', 'en'))`);
await send("Page.reload");
await until("document.readyState === 'complete' && !!window.__TAURI_INTERNALS__", 30000);
await shot("01-opened");
const shell = await until("!!document.querySelector('main') && document.body.innerText.includes('Overview')", 30000);
const seen =
  started && shell
    ? ""
    : (await js(`JSON.stringify({ state: document.readyState, root: !!document.querySelector('#root')?.childElementCount, text: document.body.innerText.length, url: location.href })`)) +
      " targets " +
      JSON.stringify((await (await fetch(`http://127.0.0.1:${port}/json`)).json()).map((t) => [t.type, t.url, t.id.slice(0, 8)])) +
      " attached " +
      page.id.slice(0, 8);
check("the app opens on its own, without a setup screen", started && shell, seen);

// 2. A project, created through the form.
await js(`window.location.hash = '/projects'`);
await until("document.querySelector('main input') !== null");
// A machine without git (a clean Windows) is told on the page, before the form is used.
if ((await invoke("git_available")) === "false") {
  const told = await until("!!document.querySelector('main [role=alert]') && document.querySelector('main [role=alert]').innerText.includes('Git')", 10000);
  await shot("02-no-git");
  check("a machine without git is told so on the Projects page", told);
}
await typeInto("main input", "smoke shop");
const created = await clickText("Create project", "document.querySelector('main')");
const hasProject = await until(`document.querySelector('header select')?.innerText.includes('smoke shop') || document.body.innerText.includes('smoke shop')`, 15000);
check("a project is created from the form", created && hasProject);

// 2b. A project in a folder picked anywhere (ADR-159): the form says what it holds, the files become the first commit;
// a reference folder shows on the project's card.
// Outside the repository the screenshots go to: a folder inside another repository is refused.
const scratch = mkdtempSync(resolve(tmpdir(), "agenium-smoke-"));
const picked = resolve(scratch, "picked-folder");
mkdirSync(resolve(picked, "src"), { recursive: true });
writeFileSync(resolve(picked, "src", "main.c"), "int main(void) { return 0; }\n");
const sdk = resolve(scratch, "reference-sdk");
mkdirSync(sdk, { recursive: true });
// A repository to clone asks for the team's branch (nothing is cloned: the form is left before Create).
await typeInto('main input[placeholder*="github.com"]', "https://github.com/owner/repo.git");
const asked = await until("document.querySelector('main [role=radiogroup]') !== null", 10000);
check("a repository asks which branch the team works on", asked, (await mainText()).slice(0, 200));
await typeInto("main input", "picked app");
await typeInto('main input[placeholder*="github.com"]', picked);
const noted = await until("document.querySelector('main').innerText.includes('first commit takes')", 15000);
await shot("02-picked-folder");
check("a picked folder of files is described before the project is made", noted, (await mainText()).slice(0, 200));
await clickText("Create project", "document.querySelector('main')");
const pickedMade = await until("document.querySelector('main').innerText.includes('picked app')", 15000);
const pickedProject = JSON.parse((await invoke("projects")) || "[]").find?.((p) => p.name === "picked app");
check("the project is made in the picked folder", pickedMade && /[\\/]picked-folder$/i.test(pickedProject?.path ?? ""), pickedProject?.path);
const refs = pickedProject
  ? await invoke("set_references", { project: pickedProject.id, references: [{ path: sdk, note: "the SDK" }] })
  : "no project";
await js("location.reload()");
await until("!!document.querySelector('main')", 30000);
await js(`window.location.hash = '/projects'`);
const refShown = await until(`[...document.querySelectorAll('main input')].some((i) => i.value === 'the SDK')`, 15000);
await shot("02b-reference-folder");
check("a reference folder shows on the project's card", !String(refs).startsWith("ERR") && refShown, String(refs).slice(0, 160));

// 2c. Many skills at once: a folder of skill folders is read, its skills listed to tick (a broken one is shown, not
// importable), and the ticked ones imported together.
const collection = resolve(scratch, "skill-collection");
for (const [dir, name] of [["pdf-tools", "pdf-tools"], [join("design", "brand-colors"), "brand-colors"]]) {
  mkdirSync(resolve(collection, dir), { recursive: true });
  writeFileSync(resolve(collection, dir, "SKILL.md"), `---\nname: ${name}\ndescription: Use when the task needs ${name}.\n---\n## Steps\n1. Do it.\n`);
}
mkdirSync(resolve(collection, "broken"), { recursive: true });
writeFileSync(resolve(collection, "broken", "SKILL.md"), "no frontmatter\n");
await js(`window.location.hash = '/library'`);
await until(`!!document.querySelector('main input[placeholder*=".zip"]')`, 15000);
await typeInto('main input[placeholder*=".zip"]', collection);
await clickText("Import", "document.querySelector('main')");
const listed = await until("document.body.innerText.includes('Skills found: 3')", 15000);
await shot("02c-skill-picker");
check("a folder of skills lists them to tick", listed, (await js("document.body.innerText")).slice(0, 300));
await clickText("Import 2");
const libNames = async () => (JSON.parse((await invoke("skill_library")) || "{}").skills ?? []).map((x) => x.name);
const both = await until(
  "window.__TAURI_INTERNALS__.invoke('skill_library').then((l) => ['pdf-tools', 'brand-colors'].every((n) => l.skills.some((s) => s.name === n)))",
  20000,
);
check("the ticked skills are imported together", both, JSON.stringify(await libNames()));

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
    fact(2, "On port 8080 the dev server listens."),
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
  // The graph tab draws the two contradicting memories joined by a red line, and a click shows the other one.
  await clickText("Graph", "document.querySelector('main')");
  const graph = "document.querySelector('main svg[role=img]')";
  const drawn = await until(`!!${graph} && ${graph}.querySelectorAll('rect').length >= 2 && [...${graph}.querySelectorAll('line')].some(l => l.getAttribute('stroke') === '#e5484d')`, 15000);
  await js(`${graph}.querySelector('g.cursor-pointer')?.dispatchEvent(new MouseEvent('click', { bubbles: true }))`);
  const linked = await until(`(document.querySelector('main')?.innerText ?? '').includes('contradiction') && [...document.querySelectorAll('main li button')].some(b => /8080|9090/.test(b.innerText))`, 10000);
  await shot("memory-graph");
  check("the memory graph joins the contradicting memories and lists what a node touches", drawn && linked);
  await clickText("Lessons", "document.querySelector('main')");
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

// 10. The chat's model dialog is a dialog: the keyboard moves into it and Escape closes it (it was an Overlay with
// neither, so it stayed open while the chat behind could still take a message).
await js(`window.location.hash = '/chat'`);
const chatReady = await until(`[...document.querySelectorAll('main button')].some(b => b.textContent.trim() === 'Change model')`, 20000);
await clickText("Change model", "document.querySelector('main')");
const modelOpen = await until(`!!document.querySelector('[role=dialog]')`, 10000);
const focusIn = await js(`!!document.querySelector('[role=dialog]')?.contains(document.activeElement)`);
await js(`window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))`);
const modelClosed = await until(`!document.querySelector('[role=dialog]')`, 10000);
await shot("chat-model-dialog");
check("the chat's model dialog takes the keyboard and Escape closes it", chatReady && modelOpen && focusIn && modelClosed);

// 11. A CLI's MCP call reads by its server and tool (ADR-153): Codex wraps every one in `mcp_tool_call`, Antigravity in
// `call_mcp_tool`, with the server, tool and the tool's own arguments inside. A tool of Agenium's own server reads as an
// API agent's call of it does, whatever the brain; another server's by its name. The chat and the Live view name it
// through the dev server's own module (a production bundle is not loaded here).
// An installed app has only its bundle: this check runs against the dev server, and says so when skipped.
const devServer = await js("location.port === '5173'");
const named = !devServer ? "SKIP" : await js(`import('/src/views/live/calls.tsx').then(m => JSON.stringify([
  m.classify('mcp_tool_call', {server:'agenium', tool:'read_file', arguments:{path:'.agenium/attachments/seo-checklist.docx'}}),
  m.classify('call_mcp_tool', {ServerName:'figma', ToolName:'get_file', Arguments:{node:'1:2'}}),
  m.classify('mcp_tool_call', {server:'agenium', tool:'submit_report', arguments:{ticket_id:'AGN-1'}}),
  m.classify('read_file', {path:'.agenium/attachments/seo-checklist.docx'}),
  m.classify('cli:agenium__shell', {command:'git status --porcelain'}),
  m.classify('shell', {command:'git status --porcelain'}),
  m.classify('cli:command_execution', {command:'git status --porcelain'}),
])).catch(e => 'ERR ' + e)`);
if (named === "SKIP") console.log("SKIP a call reads the same whatever the brain (needs the dev server's modules)");
else {
const callReads = typeof named === "string" && named.startsWith("[") ? JSON.parse(named) : [];
check(
  "a call reads the same whatever the brain: Agenium's tools tagged Agenium, a CLI's own as Tool, another server as MCP",
  callReads.length === 7 &&
    callReads[0].kind === "agenium" &&
    callReads[0].label === "read_file: .agenium/attachments/seo-checklist.docx" &&
    callReads[3].kind === "agenium" &&
    callReads[3].label === callReads[0].label &&
    callReads[4].kind === "agenium" &&
    callReads[4].label === "bash: git status --porcelain" &&
    callReads[5].kind === "agenium" &&
    callReads[5].label === callReads[4].label &&
    callReads[6].kind === "tool" &&
    callReads[6].label === callReads[4].label &&
    callReads[1].kind === "mcp" &&
    callReads[1].label === "figma :: get_file: 1:2" &&
    callReads[2].kind === "agenium" &&
    callReads[2].label === "submit_report AGN-1",
  named,
);
}

check("no console errors overall", errors.length === 0, errors.slice(0, 5).join(" | ").slice(0, 400));
console.log(fails === 0 ? "ALL PASS" : `${fails} FAILED`);
ws.close();
try {
  rmSync(scratch, { recursive: true, force: true });
} catch {
  /* the app may still hold a file of it; the system TEMP clears it */
}
process.exit(fails === 0 ? 0 : 1);

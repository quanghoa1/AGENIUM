// Full UI check of the running desktop app (debug build, WebView2 DevTools port) on a root that already holds real
// work: a project whose Solo agent closed tickets (spec, plan, work runs and reviews, merged diffs, memory runs). Every
// screen is opened and its features used: the header (language, theme, the team's state), the board with its project
// picker and search and a ticket's popup (timeline, Monaco diff, runs, audit), the runs with their search, a work run's
// steps and terminal, the memories a run was reminded of and what a memory run learned, the live office and its dock,
// a chat turn with the Lead (a real model call), agents with their search, memory search, providers and CLIs, the skill
// library and its editor, the audit trail, the usage charts and the vendors' prices, settings (shell command modes,
// judge keys) and the command palette. Screenshots and the results go to <out>.
// Usage: node full.mjs <screenshot-dir> [devtools-port] [closed-ticket] [fresh-ticket]
//   closed-ticket: a closed ticket with a diff (default AGN-1); fresh-ticket: one closed after this build, so its runs
//   carry the memory lines (default AGN-3).
import { mkdirSync, writeFileSync } from "node:fs";

const out = process.argv[2] ?? "ui-full";
const port = process.argv[3] ?? "9223";
const done = process.argv[4] ?? "AGN-1";
const fresh = process.argv[5] ?? "AGN-3";
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
const results = [];
const check = (name, ok, seen = "") => {
  if (!ok) fails++;
  const line = `${ok ? "PASS" : "FAIL"} ${name}${seen ? `: ${seen}` : ""}`;
  results.push(line);
  console.log(line);
};
const main = () => js("document.querySelector('main')?.innerText ?? ''");
const has = (text, ms = 15000) => until(`(document.querySelector('main')?.innerText ?? '').includes(${JSON.stringify(text)})`, ms);
const go = async (view) => {
  await js(`window.location.hash = ${JSON.stringify("/" + view)}`);
  await until("(document.querySelector('main')?.innerText ?? '').replace(/Loading…?/g, '').trim().length > 10", 15000);
  await sleep(500);
};
const click = (text, scope = "document") =>
  js(`(() => {
    const all = [...${scope}.querySelectorAll('button, a, [role=tab]')];
    const b = all.find(b => b.textContent.trim() === ${JSON.stringify(text)}) ?? all.find(b => b.textContent.trim().startsWith(${JSON.stringify(text)}));
    if (!b || b.disabled) return false; b.click(); return true; })()`);
const clickExact = (text) =>
  js(`(() => { const b = [...document.querySelectorAll('button')].find((b) => b.textContent.trim() === ${JSON.stringify(text)}); if (!b) return false; b.click(); return true; })()`);
const typeInto = (selector, value) =>
  js(`(() => {
    const el = document.querySelector(${JSON.stringify(selector)}); if (!el) return false;
    const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(proto, 'value').set.call(el, ${JSON.stringify(value)});
    el.dispatchEvent(new Event('input', { bubbles: true })); return true; })()`);
const noNewErrors = (before) => errors.length === before;

await send("Runtime.enable");
await send("Page.enable");
await send("Emulation.setDeviceMetricsOverride", { width: 2560, height: 1369, deviceScaleFactor: 1, mobile: false });
await js(`localStorage.setItem('agenium-lang','en')`);
await send("Page.reload");
await until("!!document.querySelector('main')", 60000);
await sleep(1500);
// Only what this fresh load raises counts (a dev server's hot reload before it can leave errors behind).
errors.length = 0;

const dlg = "document.querySelector('[role=dialog]')";
const inDlg = (re) => `${re}.test(${dlg}?.innerText ?? '')`;
const M = "document.querySelector('main')";
const errs = (e0) => errors.slice(e0).join(" | ").slice(0, 200);

// 1. Header: language and theme (only there), the team's state naming its project, Stop all; no project picker or search.
let e0 = errors.length;
await go("overview");
const header = await js(`(() => { const h = document.querySelector('header');
  return { select: !!h.querySelector('select'), search: h.innerText.includes('Ctrl K'), lang: !!h.querySelector('button[title="Tiếng Việt"]') && !!h.querySelector('button[title="English"]'),
    theme: ['Light', 'Dark', 'System'].every((t) => !!h.querySelector('button[title="' + t + '"]')), stop: h.innerText.includes('Stop all'), text: h.innerText }; })()`);
check("the header keeps language, theme, the team's state and Stop all; no project picker or search", header && !header.select && !header.search && header.lang && header.theme && header.stop, JSON.stringify(header).slice(0, 240));

// 2. Overview: the project, its picker, its tickets.
await shot("01-overview");
check("overview shows the project and its picker", (await has("uidemo")) && (await js("!!document.querySelector('main select')")) && noNewErrors(e0), errs(e0));

// 3. Board: the project picker and a search on the page; a ticket opens in a popup with its tabs.
e0 = errors.length;
await go("tickets");
const board = await main();
await shot("02-board");
check("the board shows its tickets and the project picker", board.includes(done) && board.includes("AGN-2") && (await js("[...document.querySelectorAll('main select option')].some((o) => o.textContent.includes('uidemo'))")));
const cards = () => js("[...document.querySelectorAll('main [draggable=true]')].map((c) => c.innerText.split('\\n')[0])");
await typeInto("main input[type=search]", "AGN-2");
await sleep(600);
const narrowed = await cards();
await shot("03-board-search");
await typeInto("main input[type=search]", "");
await sleep(400);
check("the board's search narrows the cards", Array.isArray(narrowed) && narrowed.length >= 1 && narrowed.every((k) => k.includes("AGN-2")), JSON.stringify(narrowed));
await js(`[...document.querySelectorAll('main [draggable=true]')].find((x) => x.innerText.includes(${JSON.stringify(done)}))?.click()`);
const tp = await until(`!!${dlg} && ${dlg}.innerText.includes(${JSON.stringify(done)}) && ${inDlg("/criteria|acceptance/i")}`, 20000);
await shot("04-ticket-popup");
check("a ticket opens in a popup with its acceptance criteria", tp);
check("the popup's timeline holds the worker's report and the review", (await js(inDlg("/report/i"))) && (await js(inDlg("/approv|review/i"))));
await click("Changes", dlg);
const diff = await until(`!!${dlg}?.querySelector('.monaco-editor, .monaco-diff-editor') || ${inDlg("/calc\\.py/")}`, 20000);
await sleep(1500);
await shot("05-ticket-changes");
check("the popup's changes tab shows the diff (calc.py)", diff);
await click("Runs", dlg);
const tr = await until(inDlg("/completed|work|review/i"), 10000);
await shot("06-ticket-runs");
check("the popup's runs tab lists the ticket's runs", tr);
await click("Audit", dlg);
const ta = await until(inDlg("/status|assigned|closed|resolved/i"), 10000);
await shot("07-ticket-audit");
check("the popup's audit tab lists the ticket's changes", ta);
await js(`window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))`);
check("Esc closes the ticket popup", (await until(`!${dlg}`, 5000)) && noNewErrors(e0), errs(e0));

// 4. Runs: the search, a work run's steps (and its terminal when it ran commands).
e0 = errors.length;
await go("runs");
await shot("08-runs");
await typeInto("main input[type=search]", done);
await sleep(600);
const runRows = await js("[...document.querySelectorAll('main tbody tr')].map((r) => r.innerText)");
check("the runs' search narrows the list", Array.isArray(runRows) && runRows.length >= 2 && runRows.every((r) => r.includes(done)), `${runRows?.length} rows`);
// The oldest run of the ticket is its work run (the review came after it).
await js("[...document.querySelectorAll('main tbody tr')].at(-1)?.click()");
const runPage = await until(`location.hash.startsWith('#/run/') && /steps/i.test(${M}?.innerText ?? '')`, 15000);
await shot("09-run");
const steps = await main();
check("the work run's steps show the tools it used", runPage && /write_file|edit_file|apply_patch/.test(steps) && /submit_report/.test(steps));
if (await click("Terminal", M)) {
  const term = await until("!!document.querySelector('main .xterm')", 15000);
  await sleep(800);
  await shot("10-run-terminal");
  check("the run's terminal (xterm) shows its commands", term);
} else results.push("INFO the work run ran no shell command: no terminal to show");

// 5. Memory in runs: the fresh work run lists the memories it was reminded of; the memory run what it learned, from where.
await go("runs");
await typeInto("main input[type=search]", fresh);
await sleep(600);
const freshRows = await js("[...document.querySelectorAll('main tbody tr')].map((r) => r.innerText)");
const memRow = (freshRows ?? []).findIndex((r) => /memory from/i.test(r));
check("the runs list says what a memory run learned from", memRow >= 0, JSON.stringify(freshRows).slice(0, 300));
await js("[...document.querySelectorAll('main tbody tr')].filter((r) => !/memory from/i.test(r.innerText)).at(-1)?.click()");
await until(`location.hash.startsWith('#/run/') && /steps/i.test(${M}?.innerText ?? '')`, 15000);
await sleep(600);
await shot("11-run-memory-recalled");
check("a work run shows the memories it was reminded of", await js(`/memory recalled/i.test(${M}.innerText)`), (await main()).slice(0, 300));
await go("runs");
await typeInto("main input[type=search]", fresh);
await sleep(600);
await js("[...document.querySelectorAll('main tbody tr')].find((r) => /memory from/i.test(r.innerText))?.click()");
await until(`location.hash.startsWith('#/run/') && /steps/i.test(${M}?.innerText ?? '')`, 15000);
await sleep(600);
await shot("12-run-memory-learned");
const mem = await main();
check("the memory run names its ticket and what it learned", /learned from/i.test(mem) && mem.includes(fresh) && /memory learned/i.test(mem) && (/Added|Updated|Retired|Nothing new worth keeping/.test(mem)), mem.slice(0, 400));
check("runs screens have no console errors", noNewErrors(e0), errs(e0));

// 6. Live: the office, the agent's dock, its tools, the plan.
e0 = errors.length;
await go("live");
await sleep(1500);
await shot("13-live");
check("the live office is drawn", await js("!!document.querySelector('main canvas')"));
await click("solo-ds", M);
await sleep(800);
const dock = await main();
check("the dock shows the agent and its model", dock.includes("solo-ds") && /deepseek/i.test(dock));
if (await click("Tools / Skills / MCP", M)) {
  await sleep(800);
  await shot("14-live-tools");
}
const planOk = (await click("Plan", M)) && (await until(`/PI-1|PI-2/.test(${M}?.innerText ?? '')`, 8000));
await shot("15-live-plan");
check("the dock's plan lists the plan items", planOk && noNewErrors(e0), errs(e0));

// 7. Chat with the Lead: its project picker and a real turn (the configured model answers).
e0 = errors.length;
await go("chat");
check("the chat has its project picker", await js("!!document.querySelector('main select')"));
const before = await js(`${M}?.innerText.length ?? 0`);
const typed = await typeInto("main textarea", "Ticket AGN-2 đang ở trạng thái nào? Trả lời một câu, không tạo ticket mới.");
const sent = typed && (await click("Send", M));
const replied = await until(`(${M}?.innerText.length ?? 0) > ${before} + 80 && !/thinking|…$/i.test((${M}?.innerText ?? '').slice(-40))`, 240000);
await sleep(3000);
await shot("16-chat");
check("a chat turn with the Lead gets an answer", sent && replied && noNewErrors(e0), errs(e0));

// 8. Needs you, projects, agents (with its search), teams, templates.
for (const [view, want, name] of [
  ["needs", "", "needs you renders"],
  ["projects", "uidemo", "projects lists the project"],
  ["agents", "solo-ds", "agents lists the Solo agent"],
  ["teams", "solo-ds", "teams shows the Solo team"],
  ["templates", "CODING", "templates lists the team templates"],
]) {
  e0 = errors.length;
  await go(view);
  await shot(`17-${view}`);
  check(name, (!want || (await has(want, 8000))) && noNewErrors(e0), errs(e0));
}
e0 = errors.length;
await go("agents");
await typeInto("main input[type=search]", "zzz-nobody");
await sleep(500);
const none = !(await js(`${M}.innerText.includes('solo-ds')`));
await typeInto("main input[type=search]", "deepseek");
await sleep(500);
const byModel = await js(`${M}.innerText.includes('solo-ds')`);
await shot("18-agents-search");
await typeInto("main input[type=search]", "");
check("the agents' search finds by model and hides the rest", none && byModel);
await js(`[...document.querySelectorAll('main a, main button, main tr')].find((x) => x.innerText.includes('solo-ds'))?.click()`);
const agentPage = await until(`location.hash.startsWith('#/agent/') && /deepseek/i.test(${M}?.innerText ?? '')`, 10000);
await sleep(600);
await shot("19-agent");
check("an agent's page shows its model, role and switches", agentPage && noNewErrors(e0), errs(e0));

// 9. Memory: the agent's facts and a search.
e0 = errors.length;
await go("memory");
await shot("20-memory");
const searched = await typeInto("main input", "calc");
await sleep(2500);
await shot("21-memory-search");
check("memory renders and takes a search", searched && noNewErrors(e0), errs(e0));

// 10. Providers and CLIs; the skill library and a skill's editor.
e0 = errors.length;
await go("providers");
// The configured providers show at once; the CLIs fill in once each one has been run (seconds the first time).
const provAtOnce = await has("opencode-go", 3000);
const clis = await until(`/claude/i.test(${M}?.innerText ?? '') && /codex/i.test(${M}?.innerText ?? '') && !/looking for the agent CLIs/i.test(${M}?.innerText ?? '')`, 60000);
await shot("22-providers");
check("providers show the configured provider at once and the detected CLIs after", provAtOnce && clis && noNewErrors(e0), errs(e0));
e0 = errors.length;
// Agenium ships no skills (ADR-88 amended): the library starts empty and says how to add one; a skill is then
// saved the way the editor saves it, to open in the editor below.
await go("library");
const emptyLibrary = /library is empty|Thư viện đang trống/i.test((await main()) ?? "");
const skillText = ["---", "name: release-notes", "description: Writes release notes from merged work. Use when a version ships.", "---", "", "## Job", "Write the release notes.", ""].join("\n");
const saved = await js(`window.__TAURI_INTERNALS__.invoke("save_skill", ${JSON.stringify({ name: "release-notes", content: skillText, create: true, files: [], removed: [] })}).then(() => true, () => false)`);
await go("overview");
await go("library");
await shot("23-library");
check("the skill library starts empty, then lists a skill the user adds", emptyLibrary && saved && /release-notes/.test((await main()) ?? ""));
const skillOpened = await js(`(() => {
  const b = [...document.querySelectorAll('main button')].find((b) => /^(edit|open|view)/i.test(b.getAttribute('aria-label') ?? b.textContent.trim()));
  if (!b) return false; b.click(); return true; })()`);
const editor = skillOpened && (await until("!!document.querySelector('.monaco-editor') || /SKILL.md|description/i.test(document.body.innerText)", 15000));
await sleep(1000);
await shot("24-skill-editor");
check("a skill opens in the editor", editor && noNewErrors(e0), errs(e0));
await js(`document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))`);
// Exact labels only: a prefix match took the header's "Close project".
await clickExact("Cancel");
await clickExact("Close");

// 11. Audit trail (its project picker) and the browser.
e0 = errors.length;
await go("audit");
await shot("25-audit");
check("the audit trail lists events and has its project picker", (await js(`/ticket|spec|plan|run/i.test(${M}?.innerText ?? '')`)) && (await js("!!document.querySelector('main select')")) && noNewErrors(e0), errs(e0));
e0 = errors.length;
await go("browser");
await shot("26-browser");
check("the browser view renders", noNewErrors(e0), errs(e0));

// 12. Usage: the range and measure filters, the stat tiles, tokens and cost per day by vendor (hover, table), the
//     rankings by vendor and model, the vendors' prices.
e0 = errors.length;
await go("usage");
await until("document.querySelectorAll('main svg[role=img]').length >= 2", 15000);
await sleep(800);
await shot("27-usage");
const u = await main();
check("usage draws tokens and cost per day by vendor", (await js("document.querySelectorAll('main svg[role=img]').length")) >= 2 && /tokens per day/i.test(u) && /cost per day/i.test(u), u.slice(0, 300));
check("usage ranks vendors and models", /by vendor/i.test(u) && /by model/i.test(u) && u.includes("opencode-go · API") && /deepseek/i.test(u));
check("usage lists the vendors' prices", /vendor prices/i.test(u) && /\$\d/.test(u.slice(u.search(/vendor prices/i))));
// A real pointer over the newest column of the first chart (a focus() is not delivered while the window is in the back).
const col = await js("(() => { const r = document.querySelector('main svg[role=img]')?.querySelectorAll('rect[tabindex]'); const b = r?.length && [...r].at(-1).getBoundingClientRect(); return b ? [b.x + b.width / 2, b.y + b.height / 2] : null; })()");
if (col) await send("Input.dispatchMouseEvent", { type: "mouseMoved", x: col[0], y: col[1] });
await sleep(400);
const tip = await js("[...document.querySelectorAll('main .pointer-events-none.absolute')].map((t) => t.innerText).join(' / ')");
await shot("28-usage-tooltip");
check("a column's tooltip lists each vendor's value and the total", /Σ/.test(tip ?? "") && /opencode-go/.test(tip ?? ""), tip);
await click("Table", M);
await sleep(500);
check("a chart reads as a table", await js("document.querySelectorAll('main table').length >= 3"));
await shot("29-usage-table");
await click("Chart", M);
await click("Cost", M);
await sleep(500);
check("the rankings switch to cost", await js(`/\\$\\d/.test([...document.querySelectorAll('main ul')].map((l) => l.innerText).join(' '))`));
await click("7 days", M);
await sleep(500);
check("a narrower range keeps the charts", await js("document.querySelectorAll('main svg[role=img]').length >= 2"));
await shot("30-usage-7days");
await click("All time", M);
await sleep(500);
check("usage has no console errors", noNewErrors(e0), errs(e0));

// 13. Settings: no theme or language there any more; shell command modes; judge keys (add one, then remove it).
e0 = errors.length;
await go("settings");
await shot("31-settings");
check("settings no longer hold theme or language", !(await js(`[...document.querySelectorAll('main button')].some((b) => ['Light', 'Dark', 'System', 'Tiếng Việt', 'English'].includes(b.textContent.trim()))`)));
const modes = await main();
check(
  "settings describe every command mode at once, and what happens on this computer now",
  /run in the sandbox/i.test(modes) && /ask me every time/i.test(modes) && /never ask/i.test(modes) &&
    (modes.match(/On a computer without a sandbox/g) ?? []).length === 2 && /Right now:/.test(modes),
);
// Choosing a mode says at once what it does here; the mode the root had is put back.
const firstMode = await js(`document.querySelector('[role=radio][aria-checked=true]')?.innerText.split('\\n')[0]`);
const pickMode = (title) => js(`[...document.querySelectorAll('[role=radio]')].find((b) => b.innerText.startsWith(${JSON.stringify(title)}))?.click()`);
await pickMode("Ask me every time");
const askedNow = await has("Right now: each command waits for your OK", 5000);
await pickMode(firstMode ?? "Run in the sandbox");
const modeBack = await until(`document.querySelector('[role=radio][aria-checked=true]')?.innerText.startsWith(${JSON.stringify(firstMode ?? "Run in the sandbox")})`, 5000);
check("choosing a command mode says what it does now, and the mode is put back", askedNow && modeBack, firstMode);
const added = await js(`(() => {
  const sec = [...document.querySelectorAll('main section')].find((s) => (s.innerText ?? '').includes('Add and test'));
  if (!sec) return false;
  const inputs = [...sec.querySelectorAll('input')].filter((i) => i.type !== 'checkbox');
  const set = (el, v) => { Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(el, v); el.dispatchEvent(new Event('input', { bubbles: true })); };
  set(inputs[inputs.length - 2], 'ui-check'); set(inputs[inputs.length - 1], 'not-a-real-key');
  const b = [...sec.querySelectorAll('button')].find((b) => b.textContent.trim() === 'Add and test'); if (!b || b.disabled) return false;
  b.click(); return true; })()`);
const listed = added && (await until(`[...document.querySelectorAll('main button')].some((b) => b.textContent.trim() === 'Remove')`, 20000));
await sleep(1500);
await shot("32-judge-key-added");
check("a judge key is added under its name", listed);
await js(`[...document.querySelectorAll('main button')].find((b) => b.textContent.trim() === 'Remove')?.click()`);
await sleep(800);
await js(`[...document.querySelectorAll('.card')].find((c) => c.querySelector('.btn-danger') && c.innerText.includes('ui-check'))?.querySelector('.btn-danger')?.click()`);
const keyGone = await until(`/no key yet/i.test(${M}.innerText)`, 15000);
await shot("33-judge-key-removed");
check("a judge key is removed", keyGone);
check("settings have no console errors", noNewErrors(e0), errs(e0));

// 14. The header's language and theme.
await js(`document.querySelector('header button[title="Tiếng Việt"]')?.click()`);
const vi = await until("document.body.innerText.includes('Tổng quan')", 8000);
await shot("34-vi");
await js(`document.querySelector('header button[title="English"]')?.click()`);
const en = await until("document.body.innerText.includes('Overview')", 8000);
check("the header switches the language to Vietnamese and back", vi && en);
await js(`document.querySelector('header button[title="Light"]')?.click()`);
const light = await until("document.documentElement.classList.contains('light')", 5000);
await go("usage");
await sleep(1200);
await shot("35-usage-light");
await js(`document.querySelector('header button[title="Dark"]')?.click()`);
const dark = await until("document.documentElement.classList.contains('dark')", 5000);
check("the header switches the theme", light && dark);

// 15. Command palette: Ctrl+K, a ticket found by its key, Enter opens it in the popup.
e0 = errors.length;
await go("overview");
await js(`window.dispatchEvent(new KeyboardEvent('keydown', { key: 'k', ctrlKey: true, bubbles: true }))`);
const pal = await until("document.activeElement?.tagName === 'INPUT' && document.activeElement.closest('main') === null", 5000);
await js(`(() => { const el = document.activeElement; Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(el, 'AGN-2'); el.dispatchEvent(new Event('input', { bubbles: true })); })()`);
await sleep(800);
await shot("36-palette");
await js(`document.activeElement?.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))`);
const popped = await until(`!!${dlg} && ${dlg}.innerText.includes('AGN-2')`, 8000);
await shot("37-palette-ticket");
check("the command palette finds a ticket and opens it in the popup", pal && popped && noNewErrors(e0), errs(e0));
await js(`window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))`);

// 16. The chat of a done project leads on (close it, or ask for more) and its ticket keys open the ticket. Closing it
//     (only the user can) sets its team free, whole; the closed project's card offers Reopen (not a team), which offers
//     the team that worked on it.
e0 = errors.length;
await go("chat");
check("the chat of a done project leads to closing it or asking for more", await js(`/every plan item is done/i.test(${M}.innerText) && [...document.querySelectorAll('main button')].some((b) => b.textContent.trim() === 'Close project')`));
const linked = await js(`(() => { const b = [...document.querySelectorAll('main button')].find((b) => /^AGN-[0-9]+$/.test(b.textContent.trim())); if (!b) return false; b.click(); return true; })()`);
check("ticket keys in the Lead's messages open the ticket", linked && (await until(`!!${dlg}`, 8000)));
await js(`window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))`);
await go("overview");
await js(`[...document.querySelectorAll('header button')].find((b) => b.textContent.trim() === 'Close project')?.click()`);
const asked = await until("[...document.querySelectorAll('.card')].some((c) => /Close uidemo/.test(c.innerText) && c.querySelector('.btn-danger'))", 5000);
await shot("38-close-ask");
await js(`[...document.querySelectorAll('.card')].find((c) => /Close uidemo/.test(c.innerText))?.querySelector('.btn-danger')?.click()`);
const freed = await until("location.hash.startsWith('#/teams') && /Project closed/.test(document.querySelector('header')?.innerText ?? '')", 15000);
await sleep(800);
await shot("39-closed-team-free");
check("a done project closes from the header and its team is free", asked && freed);
await go("projects");
const closedCard = `[...document.querySelectorAll('main .card')].find((c) => c.querySelector('.text-sm.font-semibold')?.textContent.trim() === 'uidemo')`;
check("a closed project's card offers Reopen, not a team", await js(`(() => { const b = [...${closedCard}.querySelectorAll('button')].map((b) => b.textContent.trim()); return b.includes('Reopen') && !b.includes('Add a team'); })()`));
await js(`[...${closedCard}.querySelectorAll('button')].find((b) => b.textContent.trim() === 'Reopen')?.click()`);
await sleep(1500);
const offered = await js(`[...document.querySelectorAll('.card select')].at(-1)?.selectedOptions[0]?.textContent ?? ''`);
await shot("40-reopen-ask");
await js(`[...document.querySelectorAll('.card button')].filter((b) => b.textContent.trim() === 'Reopen').at(-1)?.click()`);
const reopened = await until("/Project done/.test(document.querySelector('header')?.innerText ?? '')", 15000);
await shot("41-reopened");
check("reopening offers the team that worked on it and puts it back", /worked on it before/.test(offered) && reopened && noNewErrors(e0), offered);

check("no console errors overall", errors.length === 0, errors.slice(0, 5).join(" | ").slice(0, 400));
writeFileSync(`${out}/result.txt`, results.join("\n") + "\n");
console.log(fails === 0 ? "ALL PASS" : `${fails} FAILED`);
ws.close();
process.exit(fails === 0 ? 0 : 1);

// UI smoke test of the running desktop app (debug build, WebView2 DevTools port), on a fresh root:
// setup -> project -> team -> every view renders without console errors, then smoke-forms.mjs (provider form, agent
// export and import, MCP, web search, the map, memory). Run by devtools/scripts/ui-smoke.ps1; no model is called.
// Usage: node smoke.mjs <screenshot-dir> [devtools-port]
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { connect, sleep } from "./cdp.mjs";
import { forms } from "./smoke-forms.mjs";

const out = process.argv[2] ?? "ui-smoke";
const port = process.argv[3] ?? "9222";
mkdirSync(out, { recursive: true });

const t = await connect(port, out);
const { page, ws, send, js, until, shot, check, errors, invoke, clickText, typeInto, mainText } = t;

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

// 2b'. An empty project is removed from its card (after a confirm); its folder stays.
await typeInto("main input", "spare");
await clickText("Create project", "document.querySelector('main')");
const spare = await until(`document.querySelector('main')?.innerText.includes('spare')`, 15000);
const sparePath = (JSON.parse((await invoke("projects")) || "[]").find?.((p) => p.name === "spare") ?? {}).path;
const removeClicked = await js(`(() => {
  const card = [...document.querySelectorAll('main .card')].find((c) => c.querySelector('span')?.textContent.trim() === 'spare');
  const b = card && [...card.querySelectorAll('button')].find((x) => x.textContent.trim() === 'Remove');
  if (!b) return false; b.click(); return true; })()`);
const confirmed = removeClicked && (await until(`[...document.querySelectorAll('.btn-danger')].some((b) => b.textContent.trim() === 'Remove')`, 5000)) &&
  (await js(`(() => { const b = [...document.querySelectorAll('.btn-danger')].find((x) => x.textContent.trim() === 'Remove'); b.click(); return true; })()`));
const gone = confirmed && (await until(`!(document.querySelector('main')?.innerText ?? '').split('\\n').some((l) => l.trim() === 'spare')`, 10000));
const kept = sparePath ? await js(`window.__TAURI_INTERNALS__.invoke('inspect_folder', { path: ${JSON.stringify(sparePath)} }).then((f) => f.exists)`) : false;
await shot("02b-removed");
check("an empty project is removed from its card and its folder stays", spare && gone && kept === true, JSON.stringify({ spare, removeClicked, confirmed, gone, kept }));

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

// 3b. A plugin's skills one by one (ADR-157, 2026-11-17): a plugin installed through the form; its skills listed on the
// Skills tab under its name, read-only; one of them given alone to an agent, without the plugin's MCP server.
const vendorDir = resolve(scratch, "vendor-pentest");
mkdirSync(resolve(vendorDir, ".claude-plugin"), { recursive: true });
writeFileSync(resolve(vendorDir, ".claude-plugin", "plugin.json"), `{"name":"pentest","version":"1.0","description":"Pentest skills."}`);
writeFileSync(resolve(vendorDir, ".mcp.json"), `{"mcpServers":{"burp":{"command":"npx","args":["-y","burp-mcp"]}}}`);
for (const name of ["web-pentest", "firmware"]) {
  mkdirSync(resolve(vendorDir, "skills", name), { recursive: true });
  writeFileSync(resolve(vendorDir, "skills", name, "SKILL.md"), `---
name: ${name}
description: Use when testing ${name} targets.
---
## Steps
1. Map it.
`);
}
await js(`window.location.hash = '/library'`);
await until("!!document.querySelector('main')", 15000);
await clickText("Plugin", "document.querySelector('main')");
await until(`!!document.querySelector('main input[placeholder="obra/superpowers"]')`, 15000);
await typeInto('main input[placeholder="obra/superpowers"]', vendorDir);
await clickText("Look", "document.querySelector('main')");
const previewed = await until("[...document.querySelectorAll('button')].some((b) => b.textContent.trim().endsWith('Install'))", 30000);
await shot("03b-plugin-preview");
await clickText("Install");
const pluginIn = await until("window.__TAURI_INTERNALS__.invoke('plugins').then((ps) => ps.some((p) => p.plugin.name === 'pentest'))", 30000);
check("a plugin is installed through the form", previewed && pluginIn);
await clickText("Skill", "document.querySelector('main')");
const cards = await until("document.querySelectorAll('[data-testid=plugin-skill]').length === 2", 15000);
const cardText = await js("[...document.querySelectorAll('[data-testid=plugin-skill]')].map((c) => c.innerText).join(' | ')");
await shot("03b-plugin-skills");
check("the plugin's skills are listed under its name, read-only", cards && /pentest/.test(cardText ?? "") && /read-only/i.test(cardText ?? ""), (cardText ?? "").slice(0, 200));
const member = (JSON.parse((await invoke("agents")) || "[]") ?? []).map((a) => a.agent).find((a) => a && a.skills);
await js(`window.location.hash = '/agent/${member?.id ?? ""}'`);
await until("[...document.querySelectorAll('main button')].some((b) => b.textContent.trim() === 'Skills & MCP')", 15000);
await clickText("Skills & MCP", "document.querySelector('main')");
const listedAlone = await until("[...document.querySelectorAll('main button')].some((b) => b.textContent.trim().startsWith('web-pentest'))", 15000);
await clickText("web-pentest", "document.querySelector('main')");
const given = await until(
  `window.__TAURI_INTERNALS__.invoke('agents').then((xs) => xs.map((x) => x.agent).find((a) => a.id === ${JSON.stringify(member?.id ?? "")})?.skills.includes('plugin:pentest/web-pentest'))`,
  15000,
);
const after = (JSON.parse((await invoke("agents")) || "[]") ?? []).map((a) => a.agent).find((a) => a.id === member?.id);
await shot("03b-plugin-skill-on");
check(
  "one skill of a plugin is given alone, without the plugin",
  listedAlone && given && !(after?.skills ?? []).includes("plugin:pentest") && !(after?.skills ?? []).includes("plugin:pentest/firmware"),
  JSON.stringify(after?.skills ?? null),
);

// 3c. A template saved with a built-in's id replaces it only after a yes, and Back to built-in brings it back.
await js(`window.location.hash = '/templates'`);
await until("[...document.querySelectorAll('main button')].some((b) => b.textContent.trim() === 'New template')", 15000);
await clickText("New template", "document.querySelector('main')");
await until("document.querySelectorAll('main input').length >= 2", 10000);
await js(`(() => { const ins = document.querySelectorAll('main input');
  const set = (el, v) => { Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(el, v); el.dispatchEvent(new Event('input', { bubbles: true })); };
  set(ins[0], 'coding'); set(ins[1], 'Coding (mine)'); return true; })()`);
await clickText("Save", "document.querySelector('main')");
const replaceAsked = await until("document.body.innerText.includes('Replace the template coding?')", 10000);
await shot("03c-replace-asked");
await clickText("Replace");
const tplReplaced = await until("window.__TAURI_INTERNALS__.invoke('templates').then((ts) => ts.some((t) => t.id === 'coding' && t.custom && t.name === 'Coding (mine)'))", 15000);
check("a template with a built-in's id replaces it only after a yes", replaceAsked && tplReplaced);
await invoke("remove_template", { id: "coding" });
const builtinBack = await until("window.__TAURI_INTERNALS__.invoke('templates').then((ts) => ts.some((t) => t.id === 'coding' && !t.custom))", 10000);
check("the built-in comes back", builtinBack);

// 4. Every view renders, without console errors.
const views = ["overview", "tickets", "runs", "needs", "projects", "agents", "teams", "providers", "templates", "memory", "audit", "usage", "settings", "live", "library", "chat", "browser", "map"];
for (const v of views) {
  const before = errors.length;
  await js(`window.location.hash = '/${v}'`);
  const ok = await until("(document.querySelector('main')?.innerText ?? '').replace(/Loading…?/g, '').trim().length > 10", 15000);
  await sleep(400);
  await shot(`view-${v}`);
  check(`view ${v}`, ok && errors.length === before, errors.slice(before).join(" | ").slice(0, 200));
}
await js(`window.location.hash = '/teams'`);
const leadShown = await until("/lead/i.test(document.querySelector('main')?.innerText ?? '')", 15000);
check("the team shows its Lead", leadShown, "");

await forms(t, out);

check("no console errors overall", errors.length === 0, errors.slice(0, 5).join(" | ").slice(0, 400));
console.log(t.failed() === 0 ? "ALL PASS" : `${t.failed()} FAILED`);
ws.close();
try {
  rmSync(scratch, { recursive: true, force: true });
} catch {
  /* the app may still hold a file of it; the system TEMP clears it */
}
process.exit(t.failed() === 0 ? 0 : 1);

// Writing a skill in the running app (debug build, WebView2 DevTools port): a new skill with its name, description and
// tags in fields, its instructions in the editor, a reference and a script added in place and an asset uploaded, all
// saved in one step and found on disk; then edited (a file removed, the instructions changed), the unsaved-changes
// guard, and the skill deleted. Screenshots go to <out>.
// Usage: node skills.mjs <screenshot-dir> [devtools-port] [root]
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const out = process.argv[2] ?? "ui-skills";
const port = process.argv[3] ?? "9223";
const root = process.argv[4] ?? "";
mkdirSync(out, { recursive: true });
const NAME = "ui-skill-test";

const page = (await (await fetch(`http://127.0.0.1:${port}/json`)).json()).find((x) => x.type === "page");
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((r) => ws.addEventListener("open", r, { once: true }));
let id = 0;
const pending = new Map();
const errors = [];
ws.addEventListener("message", (m) => {
  const msg = JSON.parse(m.data);
  if (msg.id && pending.has(msg.id)) (pending.get(msg.id))(msg), pending.delete(msg.id);
  if (msg.method === "Runtime.exceptionThrown") errors.push(msg.params.exceptionDetails.text);
  if (msg.method === "Runtime.consoleAPICalled" && msg.params.type === "error") errors.push(msg.params.args.map((a) => a.value ?? a.description).join(" "));
});
const send = (method, params = {}) => new Promise((r) => { const i = ++id; pending.set(i, r); ws.send(JSON.stringify({ id: i, method, params })); });
const js = async (expr) => (await send("Runtime.evaluate", { expression: expr, awaitPromise: true, returnByValue: true })).result?.result?.value;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const until = async (expr, ms = 15000) => { const end = Date.now() + ms; while (Date.now() < end) { if (await js(expr)) return true; await sleep(200); } return false; };
const shot = async (name) => { const s = await send("Page.captureScreenshot", { format: "png" }); writeFileSync(`${out}/${name}.png`, Buffer.from(s.result.data, "base64")); };
let fails = 0;
const check = (name, ok, seen = "") => { if (!ok) fails++; console.log(`${ok ? "PASS" : "FAIL"} ${name}${seen ? `: ${seen}` : ""}`); };
const dlg = "document.querySelector('[role=dialog]')";
const clickText = (t, scope = dlg) => js(`(() => { const b = [...(${scope} ?? document).querySelectorAll('button')].find((b) => b.textContent.trim() === ${JSON.stringify(t)}); if (!b || b.disabled) return false; b.click(); return true; })()`);
const setInput = (sel, v) => js(`(() => { const el = ${dlg}.querySelector(${JSON.stringify(sel)}); if (!el) return false; Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(el, ${JSON.stringify(v)}); el.dispatchEvent(new Event('input', { bubbles: true })); return true; })()`);
// Clicks into the open file's editor, as a user does (Monaco takes keys through the element that has its focus).
const clickEditor = async () => {
  const [x, y] = await js(`(() => { const r = ${dlg}.querySelector('.monaco-editor .view-lines').getBoundingClientRect(); return [r.x + 60, r.y + 8]; })()`);
  await send("Input.dispatchMouseEvent", { type: "mousePressed", x, y, button: "left", clickCount: 1 });
  await send("Input.dispatchMouseEvent", { type: "mouseReleased", x, y, button: "left", clickCount: 1 });
  await sleep(150);
};
// Types at the end of the open file in Monaco.
const typeEnd = async (text) => {
  await clickEditor();
  await send("Input.dispatchKeyEvent", { type: "keyDown", modifiers: 2, key: "End", code: "End", windowsVirtualKeyCode: 35 });
  await send("Input.dispatchKeyEvent", { type: "keyUp", modifiers: 2, key: "End", code: "End", windowsVirtualKeyCode: 35 });
  await send("Input.insertText", { text });
  await sleep(300);
};
const addFile = async (dirLabel, name) => {
  // The "New file" button of the group whose label is `dirLabel`.
  await js(`(() => { const g = [...${dlg}.querySelectorAll('.label')].find((l) => l.textContent.trim() === ${JSON.stringify(dirLabel)}).parentElement; g.querySelector('button[title="New file"]').click(); return true; })()`);
  await sleep(200);
  await setInput("input[placeholder]:focus, input.font-mono.h-7", name);
  await js(`${dlg}.querySelector('input.h-7')?.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))`);
  await sleep(500);
};

await send("Runtime.enable");
await send("Page.enable");
await send("DOM.enable");
await send("Emulation.setDeviceMetricsOverride", { width: 2560, height: 1369, deviceScaleFactor: 1, mobile: false });
await js(`localStorage.setItem('agenium-lang','en'); location.hash = '/library'; 1`);
await send("Page.reload");
await until("[...document.querySelectorAll('button')].some((b) => b.textContent.trim() === 'New skill')", 30000);
await sleep(800);
errors.length = 0;
await shot("01-library");
check("the library offers writing a skill and importing one, side by side", await js(`/Write a skill/.test(document.body.innerText) && /Import a skill/.test(document.body.innerText)`));

// 1. A new skill: fields, instructions, a reference and a script in place, an asset uploaded.
await clickText("New skill", "document");
await until(`!!${dlg}?.querySelector('.monaco-editor')`, 20000);
await sleep(800);
const geo = await js(`(() => { const d = ${dlg}.getBoundingClientRect(); const e = ${dlg}.querySelector('.monaco-editor').getBoundingClientRect(); return { dialog: d.height, editor: e.height, gapBelow: d.bottom - e.bottom }; })()`);
await shot("02-new-skill");
check("the editor fills the dialog (no empty band below it)", geo && geo.editor > geo.dialog * 0.55 && geo.gapBelow < 160, JSON.stringify(geo));
check("name, description and tags are fields, with the file groups beside the editor", await js(`['Name','Description','Tags','Instructions','References','Scripts','Assets'].every((t) => [...${dlg}.querySelectorAll('.label')].some((l) => l.textContent.trim() === t))`));
await setInput('input[placeholder="my-skill"]', NAME);
await setInput('input[placeholder^="One line"]', "Check an API change against its notes and run the smoke script.");
await setInput('input[placeholder="coding testing"]', "testing, api");
await typeEnd("\n\nRead `references/api.md` first, then run `scripts/check.sh` and keep the logo `assets/logo.png` in the report.\n");
await addFile("References", "api.md");
await typeEnd("# API notes\nThe endpoint returns 200.\n");
await addFile("Scripts", "check.sh");
await typeEnd("echo smoke ok\n");
mkdirSync(join(out, "upload"), { recursive: true });
const png = join(out, "upload", "logo.png");
const pngBytes = Buffer.from("89504e470d0a1a0a0000000d49484452000000010000000108060000001f15c4890000000d49444154789c6360000002000154a24f5d0000000049454e44ae426082", "hex");
writeFileSync(png, pngBytes);
await js(`(() => { const g = [...${dlg}.querySelectorAll('.label')].find((l) => l.textContent.trim() === 'Assets').parentElement; g.querySelector('button[title="Upload from this computer"]').click(); return true; })()`);
const doc = await send("DOM.getDocument", { depth: -1, pierce: true });
const q = await send("DOM.querySelector", { nodeId: doc.result.root.nodeId, selector: "[role=dialog] input[type=file]" });
await send("DOM.setFileInputFiles", { nodeId: q.result.nodeId, files: [png.replace(/\//g, "\\")] });
await sleep(800);
const listed = await js(`[...${dlg}.querySelectorAll('button[title]')].map((b) => b.title).filter((t) => t.includes('/'))`);
await shot("03-files-added");
check("the reference, the script and the uploaded asset are listed before any save", ["references/api.md", "scripts/check.sh", "assets/logo.png"].every((p) => listed?.includes(p)), JSON.stringify(listed));
check("the changes are counted as unsaved", await js(`/4 unsaved change/.test(${dlg}.innerText)`), await js(`${dlg}.innerText.match(/\\d+ unsaved[^\\n]*/)?.[0] ?? ''`));
await clickText("Save");
const savedOk = await until(`/All saved/.test(${dlg}?.innerText ?? '')`, 20000);
await shot("04-saved");
check("the skill and its files are saved in one step", savedOk, await js(`${dlg}?.innerText.slice(-200)`));
check("once saved, the title names the skill", await js(`/Edit: ${NAME}/.test(${dlg}.innerText)`));
check("the uploaded image is shown", await js(`!!${dlg}.querySelector('img[alt="assets/logo.png"]')`));
if (root) {
  const dir = join(root, "skills", NAME);
  const md = existsSync(join(dir, "SKILL.md")) ? readFileSync(join(dir, "SKILL.md"), "utf8") : "";
  check(
    "on disk: SKILL.md with its fields, the reference, the script and the asset",
    md.includes(`name: ${NAME}`) && md.includes("tags: testing api") && md.includes("Read `references/api.md` first") && readFileSync(join(dir, "references/api.md"), "utf8").includes("The endpoint returns 200") && readFileSync(join(dir, "scripts/check.sh"), "utf8").includes("echo smoke ok") && readFileSync(join(dir, "assets/logo.png")).equals(pngBytes),
    md.split("\n").slice(0, 5).join(" | "),
  );
}
await clickText("Cancel");
await sleep(600);

// 2. Edit: the files are there; the script removed and no longer mentioned; saved; the unsaved-changes guard.
const card = `[...document.querySelectorAll('.card')].find((c) => c.querySelector('.text-sm.font-semibold')?.textContent.trim() === ${JSON.stringify(NAME)})`;
check("the new skill is in the library with its 3 files", await until(`!!(${card}) && /3 files/.test((${card}).innerText)`, 10000));
await js(`[...(${card}).querySelectorAll('button')].find((b) => b.textContent.trim() === 'Edit').click()`);
await until(`!!${dlg}?.querySelector('.monaco-editor')`, 20000);
await sleep(600);
await js(`[...${dlg}.querySelectorAll('button[title]')].find((b) => b.title === 'assets/logo.png').click()`);
check("a saved image is shown when opened", await until(`!!${dlg}.querySelector('img[alt="assets/logo.png"]')`, 5000));
check("editing shows its files and its name fixed", await js(`${dlg}.querySelector('input[placeholder="my-skill"]').disabled && ['references/api.md','scripts/check.sh','assets/logo.png'].every((p) => [...${dlg}.querySelectorAll('button[title]')].some((b) => b.title === p))`));
await js(`(() => { const row = [...${dlg}.querySelectorAll('button[title="scripts/check.sh"]')][0].parentElement; row.querySelector('button[title="Delete"]').click(); return true; })()`);
await sleep(300);
// Take the script out of the instructions: replace the text through the editor (select all, type).
await clickText("SKILL.md");
await until(`!!${dlg}.querySelector('.monaco-editor .view-lines')`, 5000);
await clickEditor();
await send("Input.dispatchKeyEvent", { type: "keyDown", modifiers: 2, key: "a", code: "KeyA", windowsVirtualKeyCode: 65 });
await send("Input.dispatchKeyEvent", { type: "keyUp", modifiers: 2, key: "a", code: "KeyA", windowsVirtualKeyCode: 65 });
await send("Input.insertText", { text: "## Job\nCheck an API change.\n\n## When to use\nAn API changes.\n\n## Steps\nRead `references/api.md`; keep `assets/logo.png` in the report.\n\n## Tools\nread_file.\n\n## Proof\nThe report quotes the notes.\n" });
await sleep(400);
await shot("05-edit");
await clickText("Save");
await until(`/All saved/.test(${dlg}?.innerText ?? '')`, 20000);
if (root) {
  const dir = join(root, "skills", NAME);
  check("on disk: the script is gone, the rest kept", !existsSync(join(dir, "scripts/check.sh")) && existsSync(join(dir, "references/api.md")) && existsSync(join(dir, "assets/logo.png")));
}
await typeEnd("\nOne more line.\n");
await clickText("Cancel");
const guarded = await until(`/Leave without saving/.test(document.body.innerText)`, 5000);
await shot("06-unsaved-guard");
check("leaving with unsaved changes asks first", guarded);
await clickText("Leave without saving", "document.querySelectorAll('[role=dialog]')[1]") || (await clickText("Leave without saving", "document"));
await sleep(600);

// 3. Delete the test skill.
await js(`[...(${card}).querySelectorAll('button')].find((b) => b.textContent.trim() === 'Delete').click()`);
await sleep(400);
// The confirmation's Delete is the last one on the page.
await js(`(() => { const b = [...document.querySelectorAll('button')].filter((b) => b.textContent.trim() === 'Delete').pop(); b?.click(); return !!b; })()`);
check("the test skill is deleted", await until(`!(${card})`, 10000));
check("no error in the page", errors.length === 0, errors.join(" | ").slice(0, 300));
console.log(fails ? `${fails} FAILED` : "all passed");
ws.close();
process.exit(fails ? 1 : 0);

// The Resume button (ADR-132) in the running desktop app (debug build, WebView2 DevTools port): with an agent set aside
// for its quota, Needs you says why with the agent's model and offers Resume and a link to the agent; its card in
// Agents and its own page offer Resume too; Resume in Needs you puts it back to work (the item and the buttons go).
// Before starting the app, set an agent aside in its root, e.g. with sqlite3:
//   UPDATE agent SET status = 'QuotaExhausted' WHERE name = '<agent>';
// Usage: node resume.mjs <screenshot-dir> [devtools-port] [agent-name]
import { mkdirSync, writeFileSync } from "node:fs";

const out = process.argv[2] ?? "ui-resume";
const port = process.argv[3] ?? "9223";
const agent = process.argv[4] ?? "solo-ds";
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
  if (msg.method === "Runtime.exceptionThrown") errors.push("exception: " + msg.params.exceptionDetails.text);
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
const main = () => js("document.querySelector('main')?.innerText ?? ''");
const go = async (view) => {
  await js(`window.location.hash = ${JSON.stringify("/" + view)}`);
  await until("(document.querySelector('main')?.innerText ?? '').replace(/Loading…?/g, '').trim().length > 10", 15000);
  await sleep(800);
};
// The buttons whose text is exactly `text` inside the element that holds `near` (the closest one holding both).
const buttonsNear = (near, text) => `(() => {
  const all = [...document.querySelectorAll('main button')].filter((b) => b.textContent.trim() === ${JSON.stringify(text)});
  return all.filter((b) => { let e = b; for (let i = 0; i < 6 && e; i++, e = e.parentElement) if (e.innerText.includes(${JSON.stringify(near)})) return true; return false; });
})()`;
const countNear = (near, text) => js(`${buttonsNear(near, text)}.length`);

await send("Runtime.enable");
await send("Page.enable");
await send("Emulation.setDeviceMetricsOverride", { width: 2560, height: 1369, deviceScaleFactor: 1, mobile: false });
await js(`localStorage.setItem('agenium-lang','en')`);
await send("Page.reload");
await until("!!document.querySelector('main')", 60000);
await sleep(1500);
errors.length = 0;

// 1. Needs you: why the agent stopped, with its model; Resume and the agent's name.
await go("needs");
const needText = await main();
await shot("01-needs");
check(
  "Needs you says the agent ran out of quota on its model and offers Resume and the agent",
  needText.includes(`${agent} ran out of quota or credit on`) && needText.includes("then resume it") && (await countNear("ran out of quota", "Resume")) === 1 && (await countNear("ran out of quota", agent)) === 1,
  needText.slice(0, 300).replace(/\n/g, " | "),
);

// 2. Its card in Agents offers Resume.
await go("agents");
await shot("02-agents");
check("its card in Agents offers Resume", (await countNear(agent, "Resume")) >= 1);

// 3. Its own page (the link from Needs you) offers Resume with what resuming does.
await go("needs");
await js(`${buttonsNear("ran out of quota", agent)}[0]?.click()`);
const onPage = await until(`(document.querySelector('main')?.innerText ?? '').includes('goes on from where it stopped')`, 15000);
await shot("03-agent-page");
check("the link opens its page, which offers Resume and says what it does", onPage && (await js(`[...document.querySelectorAll('main button')].some((b) => b.textContent.trim() === 'Resume')`)));

// 4. Resume in Needs you puts it back to work: the item goes, and so do the buttons.
await go("needs");
await js(`${buttonsNear("ran out of quota", "Resume")}[0]?.click()`);
const gone = await until(`!(document.querySelector('main')?.innerText ?? '').includes('ran out of quota')`, 15000);
await shot("04-needs-after");
await go("agents");
const cardAfter = await countNear(agent, "Resume");
await shot("05-agents-after");
check("Resume puts it back to work: the item and the buttons go", gone && cardAfter === 0, `card buttons ${cardAfter}`);

check("no error in the page", errors.length === 0, errors.join(" | ").slice(0, 300));
console.log(fails ? `${fails} FAILED` : "all passed");
ws.close();
process.exit(fails ? 1 : 0);

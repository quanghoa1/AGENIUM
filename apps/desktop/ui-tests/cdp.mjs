// The DevTools plumbing the UI tests share: the running app's page on its WebView2 DevTools port, script evaluation,
// waits on a condition, screenshots, the console errors the page raised, and PASS/FAIL lines.
import { writeFileSync } from "node:fs";

export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// The app's page once it shows the app: it starts with no address, then `about:blank`, before the app's first
// navigation, and a reload sent then cancelled that navigation for good (an empty window: "setup opens the app" failed
// now and then, 2026-10-31).
const appPage = (ts) => ts.find((x) => x.type === "page" && /^(https?|tauri):/.test(x.url));

/** Connects to the app on DevTools `port` (waiting up to two minutes for it to start); screenshots go to `out`. */
export async function connect(port, out) {
  let targets = [];
  for (let i = 0; i < 120 && !appPage(targets); i++) {
    try {
      targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
    } catch {
      /* the app is starting */
    }
    if (!appPage(targets)) await sleep(1000);
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
  const until = async (expr, ms = 20000) => {
    const end = Date.now() + ms;
    while (Date.now() < end) {
      if (await js(expr)) return true;
      await sleep(250);
    }
    return false;
  };
  // The page has stopped changing: no change to the DOM for 300 ms (at most `ms`), then two frames painted. What a
  // fixed sleep before a screenshot waited for, without guessing how long.
  const settle = (ms = 5000) =>
    js(`new Promise((done) => { let t; const end = () => { o.disconnect(); requestAnimationFrame(() => requestAnimationFrame(() => done(true))); };
      const o = new MutationObserver(() => { clearTimeout(t); t = setTimeout(end, 300); });
      o.observe(document, { subtree: true, childList: true, attributes: true, characterData: true });
      t = setTimeout(end, 300); setTimeout(end, ${ms}); })`);
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
  // A backend command called as the UI calls it: its answer as JSON, or `ERR <error>`.
  const invoke = (cmd, args = {}) =>
    js(`window.__TAURI_INTERNALS__.invoke(${JSON.stringify(cmd)}, ${JSON.stringify(args)}).then(r => JSON.stringify(r ?? null), e => 'ERR ' + String(e))`);
  // Clicks the enabled button in `scope` whose text is `text` (or starts with it).
  const clickText = (text, scope = "document") =>
    js(`(() => {
    const all = [...${scope}.querySelectorAll('button')];
    const b = all.find(b => b.textContent.trim() === ${JSON.stringify(text)}) ?? all.find(b => b.textContent.trim().startsWith(${JSON.stringify(text)}));
    if (!b || b.disabled) return false; b.click(); return true; })()`);
  // Sets an input's or a textarea's value the way React sees typing.
  const typeInto = (selector, value) =>
    js(`(() => {
    const el = document.querySelector(${JSON.stringify(selector)}); if (!el) return false;
    const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(proto, 'value').set.call(el, ${JSON.stringify(value)});
    el.dispatchEvent(new Event('input', { bubbles: true })); return true; })()`);
  const mainText = () => js("document.querySelector('main')?.innerText ?? ''");
  return { page, ws, send, js, until, shot, check, results, errors, failed: () => fails, invoke, clickText, typeInto, mainText, settle };
}

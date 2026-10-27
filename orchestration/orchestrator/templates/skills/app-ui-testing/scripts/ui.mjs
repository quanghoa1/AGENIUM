#!/usr/bin/env node
// Drives a test instance of a desktop app built on WebView2 or Chromium (Tauri, Electron) through its DevTools port,
// for UI testing: launch it, go to a screen, click, type, read the text, take screenshots, stop it.
// Node 22 or newer (built-in fetch and WebSocket); nothing to install.
//
//   node ui.mjs launch --exe <app> [--port 9333] [--agenium] [--chromium] [--env NAME=VALUE]... [-- <app args>]
//   node ui.mjs goto <#/route | url>       node ui.mjs click "<visible text>" | --css <selector>
//   node ui.mjs type <selector> "<text>"   node ui.mjs select <selector> <value>   node ui.mjs press <Key>
//   node ui.mjs text [selector]            node ui.mjs find "<text>"              node ui.mjs eval "<js>"
//   node ui.mjs wait "<text>" [seconds]
//   node ui.mjs size <width> <height>      node ui.mjs shot <file.png|.jpg>       node ui.mjs stop
//
// Only the instance `launch` started is driven (its port is kept in .ui-test/session.json): never another app, and
// never the user's own window. Each action prints the console errors the page logged while it ran.
import { spawn } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";

const SESSION = resolve(".ui-test/session.json");
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const die = (msg) => {
  console.error(`error: ${msg}`);
  process.exit(1);
};

async function pages(port) {
  try {
    const r = await fetch(`http://127.0.0.1:${port}/json`);
    return (await r.json()).filter((t) => t.type === "page");
  } catch {
    return null;
  }
}

function session() {
  if (!existsSync(SESSION)) die("no test instance: run `node ui.mjs launch --exe <app>` first");
  return JSON.parse(readFileSync(SESSION, "utf8"));
}

async function launch(args) {
  const opt = { port: 9333, env: {}, rest: [] };
  for (let i = 0; i < args.length; i++) {
    const a = args[i];
    if (a === "--exe") opt.exe = args[++i];
    else if (a === "--port") opt.port = Number(args[++i]);
    else if (a === "--agenium") opt.agenium = true;
    else if (a === "--chromium") opt.chromium = true;
    else if (a === "--env") {
      const [k, ...v] = args[++i].split("=");
      opt.env[k] = v.join("=");
    } else if (a === "--") {
      opt.rest = args.slice(i + 1);
      break;
    } else die(`unknown launch option ${a}`);
  }
  if (!opt.exe || !existsSync(opt.exe)) die(`--exe must name the app's executable (got ${opt.exe})`);
  if (existsSync(SESSION)) die("a test instance is already running: `node ui.mjs stop` first");
  if (await pages(opt.port)) die(`port ${opt.port} is already in use by another app: pick another --port`);
  const root = mkdtempSync(join(tmpdir(), "ui-test-"));
  const env = {
    ...process.env,
    ...opt.env,
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${opt.port}`,
    // Its own browser profile: the user's app keeps its own (language, theme, the selected project), and two
    // instances never share one profile with different arguments (WebView2 refuses that).
    WEBVIEW2_USER_DATA_FOLDER: join(root, ".webview2"),
  };
  if (opt.agenium) {
    // A throwaway Agenium root whose vault key lives in memory: nothing reaches the user's data or OS keyring.
    env.AGENIUM_HOME = root;
    env.AGENIUM_UI_TEST_KEYRING = "memory";
  }
  // WebView2 reads its port from the environment; a Chromium or Electron app takes it as an argument.
  const appArgs = opt.chromium ? [...opt.rest, `--remote-debugging-port=${opt.port}`] : opt.rest;
  const child = spawn(opt.exe, appArgs, {
    env,
    detached: true,
    stdio: "ignore",
    cwd: root,
  });
  child.unref();
  mkdirSync(dirname(SESSION), { recursive: true });
  writeFileSync(SESSION, JSON.stringify({ port: opt.port, pid: child.pid, root, exe: resolve(opt.exe) }, null, 2));
  for (let i = 0; i < 240; i++) {
    const p = await pages(opt.port);
    if (p?.some((t) => /^(https?|tauri):/.test(t.url))) {
      await sleep(1500);
      console.log(`launched pid ${child.pid} on port ${opt.port}; data in ${root}`);
      return;
    }
    await sleep(500);
  }
  die(`the app opened no window on port ${opt.port} within 2 minutes (is it a WebView2 or Chromium app?)`);
}

async function connect() {
  const s = session();
  const p = await pages(s.port);
  if (!p?.length) die(`the test instance on port ${s.port} does not answer (did it exit?)`);
  const ws = new WebSocket(p[0].webSocketDebuggerUrl);
  await new Promise((r, j) => {
    ws.addEventListener("open", r, { once: true });
    ws.addEventListener("error", j, { once: true });
  });
  let id = 0;
  const pending = new Map();
  const errors = [];
  ws.addEventListener("message", (m) => {
    const msg = JSON.parse(m.data);
    if (msg.id && pending.has(msg.id)) pending.get(msg.id)(msg), pending.delete(msg.id);
    if (msg.method === "Runtime.exceptionThrown")
      errors.push((msg.params.exceptionDetails.exception?.description ?? msg.params.exceptionDetails.text).split("\n")[0]);
    if (msg.method === "Runtime.consoleAPICalled" && msg.params.type === "error")
      errors.push(msg.params.args.map((a) => a.value ?? a.description).join(" ").split("\n")[0]);
  });
  const send = (method, params = {}) =>
    new Promise((r) => {
      const i = ++id;
      pending.set(i, r);
      ws.send(JSON.stringify({ id: i, method, params }));
    });
  const js = async (expr) => {
    const r = await send("Runtime.evaluate", { expression: expr, awaitPromise: true, returnByValue: true });
    if (r.result?.exceptionDetails) throw new Error(r.result.exceptionDetails.exception?.description ?? "script error");
    return r.result?.result?.value;
  };
  await send("Runtime.enable");
  await send("Page.enable");
  return { send, js, errors, close: () => ws.close() };
}

// The visible elements that match `text`, best first: what can be clicked (a button, link, tab...) before plain text,
// an exact match before one that contains it, the smallest before a larger one.
const FIND = (text) => `(() => {
  const want = ${JSON.stringify(text)}.trim().toLowerCase();
  const visible = (e) => { const r = e.getBoundingClientRect(); const s = getComputedStyle(e);
    return r.width > 0 && r.height > 0 && s.visibility !== "hidden" && s.display !== "none"; };
  const label = (e) => (e.innerText || e.value || e.getAttribute("aria-label") || e.title || "").trim().toLowerCase();
  const all = [...document.querySelectorAll("button, a, [role=button], [role=tab], [role=menuitem], input, select, textarea, label, summary, li, td, th, span, div, h1, h2, h3, p")].filter(visible);
  const CLICKABLE = "button, a, [role=button], [role=tab], [role=menuitem], summary, label, select, input, textarea";
  const hits = new Map();
  for (const e of all) {
    const l = label(e);
    if (!l.includes(want)) continue;
    const t = e.closest(CLICKABLE) || e;
    const r = t.getBoundingClientRect();
    const score = (t.matches(CLICKABLE) ? 0 : 2) + (l === want ? 0 : 1);
    const prev = hits.get(t);
    if (prev === undefined || score < prev.score) hits.set(t, { score, area: r.width * r.height });
  }
  return [...hits].sort((a, b) => a[1].score - b[1].score || a[1].area - b[1].area).map(([e]) => e);
})()`;

async function act(cmd, args) {
  const c = await connect();
  let out;
  try {
    switch (cmd) {
      case "goto": {
        const to = args[0] ?? die("goto needs a #/route or a url");
        if (to.startsWith("#")) await c.js(`window.location.hash = ${JSON.stringify(to)}`);
        else await c.send("Page.navigate", { url: to });
        await sleep(1500);
        out = `at ${await c.js("location.href")}`;
        break;
      }
      case "click": {
        const sel = args[0] === "--css" ? args[1] : null;
        const target = sel
          ? `document.querySelector(${JSON.stringify(sel)})`
          : `${FIND(args[0] ?? die("click needs the visible text (or --css <selector>)"))}[0]`;
        const box = await c.js(`(() => { const e = ${target}; if (!e) return null; e.scrollIntoView({ block: "center" });
          const r = e.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2, tag: e.tagName.toLowerCase(), text: (e.innerText || e.value || "").trim().slice(0, 60) }; })()`);
        if (!box) die(`nothing visible matches ${sel ?? JSON.stringify(args[0])}`);
        for (const type of ["mouseMoved", "mousePressed", "mouseReleased"])
          await c.send("Input.dispatchMouseEvent", { type, x: box.x, y: box.y, button: "left", clickCount: 1 });
        await sleep(1200);
        out = `clicked <${box.tag}> "${box.text}"`;
        break;
      }
      case "type": {
        const [sel, text] = args;
        if (!sel || text === undefined) die("type needs a selector and the text");
        const ok = await c.js(`(() => { const e = document.querySelector(${JSON.stringify(sel)}); if (!e) return false; e.focus(); return true; })()`);
        if (!ok) die(`no element matches ${sel}`);
        await c.send("Input.insertText", { text });
        await sleep(600);
        out = `typed ${text.length} characters into ${sel}`;
        break;
      }
      case "select": {
        const [sel, value] = args;
        const ok = await c.js(`(() => { const e = document.querySelector(${JSON.stringify(sel)}); if (!e) return false;
          const set = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value").set; set.call(e, ${JSON.stringify(value)});
          e.dispatchEvent(new Event("change", { bubbles: true })); return true; })()`);
        if (!ok) die(`no select matches ${sel}`);
        await sleep(1000);
        out = `selected ${value} in ${sel}`;
        break;
      }
      case "press": {
        const key = args[0] ?? die("press needs a key (Enter, Escape, Tab, ArrowDown...)");
        for (const type of ["keyDown", "keyUp"]) await c.send("Input.dispatchKeyEvent", { type, key, code: key });
        await sleep(800);
        out = `pressed ${key}`;
        break;
      }
      case "wait": {
        const want = args[0] ?? die("wait needs a text to wait for");
        const secs = Number(args[1] ?? 30);
        for (let i = 0; i < secs * 2; i++) {
          if (await c.js(`document.body.innerText.toLowerCase().includes(${JSON.stringify(want.toLowerCase())})`)) {
            out = `"${want}" is on the screen`;
            break;
          }
          await sleep(500);
        }
        if (!out) die(`"${want}" did not appear within ${secs} s`);
        break;
      }
      case "text":
        out = await c.js(`(document.querySelector(${JSON.stringify(args[0] ?? "body")})?.innerText ?? "(no element matches)").replace(/\\n{3,}/g, "\\n\\n").slice(0, 20000)`);
        break;
      case "find":
        out = (await c.js(`${FIND(args[0] ?? die("find needs a text"))}.slice(0, 15).map((e) => { const r = e.getBoundingClientRect();
          return "<" + e.tagName.toLowerCase() + "> " + JSON.stringify((e.innerText || e.value || "").trim().slice(0, 80)) + " at " + Math.round(r.x) + "," + Math.round(r.y) + " " + Math.round(r.width) + "x" + Math.round(r.height); })`)).join("\n") || "nothing visible matches";
        break;
      case "eval":
        out = JSON.stringify(await c.js(args[0] ?? die("eval needs an expression")), null, 2);
        break;
      case "size": {
        const [w, h] = args.map(Number);
        if (!w || !h) die("size needs a width and a height");
        await c.send("Emulation.setDeviceMetricsOverride", { width: w, height: h, deviceScaleFactor: 1, mobile: false });
        await sleep(800);
        out = `viewport ${w}x${h}`;
        break;
      }
      case "shot": {
        const file = args[0] ?? die("shot needs a file name (.png or .jpg)");
        const jpeg = /\.jpe?g$/i.test(file);
        const r = await c.send("Page.captureScreenshot", jpeg ? { format: "jpeg", quality: 80 } : { format: "png" });
        mkdirSync(dirname(resolve(file)), { recursive: true });
        writeFileSync(file, Buffer.from(r.result.data, "base64"));
        out = `saved ${file} (${Math.round(r.result.data.length * 0.75 / 1024)} KB); look at it with view_image`;
        break;
      }
      default:
        die(`unknown command ${cmd}`);
    }
    await sleep(300);
  } finally {
    c.close();
  }
  console.log(out);
  if (c.errors.length) console.log(`console errors:\n- ${[...new Set(c.errors)].join("\n- ")}`);
}

function stop() {
  const s = session();
  try {
    process.kill(s.pid);
  } catch {}
  // Only the root `launch` made under the temp folder is removed.
  if (s.root?.startsWith(tmpdir())) {
    for (let i = 0; i < 10; i++) {
      try {
        rmSync(s.root, { recursive: true, force: true });
        break;
      } catch {
        // The app's files are released a moment after it exits.
        Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 500);
      }
    }
  }
  rmSync(SESSION, { force: true });
  console.log(`stopped pid ${s.pid}`);
}

const [cmd, ...args] = process.argv.slice(2);
if (!cmd || cmd === "help") console.log(readFileSync(new URL(import.meta.url), "utf8").split("\n").slice(1, 14).join("\n"));
else if (cmd === "launch") await launch(args);
else if (cmd === "stop") stop();
else await act(cmd, args);
process.exit(0);

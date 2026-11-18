// End to end in the running desktop app (debug build, WebView2 DevTools port) on a fresh root, driven as a user, with
// a real model: a project made in the form, a Solo agent on the Claude Code CLI (Haiku), a request typed in the Lead's
// chat, the spec agreed (a Solo agent locks its own), the work run, reviewed and closed by the agent. Then every screen that shows
// the work is read: the board, a ticket, the project map, the live office, the skills' scorecards, the audit trail,
// the usage. Costs a few Claude Code turns of the user's own subscription.
// Usage: node e2e-work.mjs <screenshot-dir> [devtools-port]
import { mkdirSync, writeFileSync } from "node:fs";
import { connect } from "./cdp.mjs";

const out = process.argv[2] ?? "ui-e2e";
const port = process.argv[3] ?? "9241";
mkdirSync(out, { recursive: true });
const { send, js, until, shot, check, results, errors, failed, invoke, typeInto, clickText, settle } = await connect(port, out);
const M = "document.querySelector('main')";
const main = () => js(`${M}?.innerText ?? ''`);
const go = async (view) => {
  await js(`window.location.hash = ${JSON.stringify("/" + view)}`);
  await until(`(${M}?.innerText ?? '').replace(/Loading…?/g, '').trim().length > 10`, 15000);
  await settle();
};
const json = async (cmd, args) => {
  const r = await invoke(cmd, args);
  if (typeof r !== "string" || r.startsWith("ERR")) throw new Error(`${cmd}: ${r}`);
  return JSON.parse(r);
};
const errorsAt = () => errors.length;
// Types in the chat and sends once the button takes it (React enables it after the text reaches its state); sent when
// the box is empty again.
const say = async (text) => {
  await typeInto("main textarea", text);
  const ready = await until(`[...document.querySelectorAll('main button')].some((b) => b.textContent.trim() === 'Send' && !b.disabled)`, 15000);
  await clickText("Send", M);
  return ready && (await until(`document.querySelector('main textarea')?.value === ''`, 15000));
};

// 1. The app opens on a fresh root; the test speaks English.
check("the embedding model is in place", await until("window.__TAURI_INTERNALS__.invoke('status').then((s) => s.model?.phase === 'ready')", 180000));
check("the app opens", await until("window.__TAURI_INTERNALS__.invoke('status').then((s) => s.ready && !s.choose_root)", 60000));
await js(`window.__TAURI_INTERNALS__.invoke('set_ui_language', { language: 'en' }).then(() => localStorage.setItem('agenium-lang', 'en'))`);
await send("Page.reload");
await until("document.readyState === 'complete' && !!window.__TAURI_INTERNALS__", 30000);
await until(`!!${M}`, 30000);

// 2. A project from the form.
await go("projects");
await until(`${M}?.querySelector('input') !== null`);
await typeInto("main input", "wordcount");
await clickText("Create project", M);
check("the project is created from the form", await until(`document.body.innerText.includes('wordcount')`, 15000));
const project = (await json("projects")).find((p) => p.name === "wordcount");

// 3. A Solo agent on the Claude Code CLI (Haiku, low effort) leads the project.
const agent = await json("create_agent", {
  form: { name: "Solo", role: "bugfixer", tags: null, model: { provider: "claude", model: "haiku", effort: "low", backend: "cli" }, skills: [], mcp: [] },
});
const agentId = agent.id ?? agent.agent?.id;
await json("compose_team", { form: { name: "solo team", project: project.id, lead: agentId, members: [], solo: true } });
check("a Solo agent on Claude Code leads the project", !!agentId);

// 4. The request, typed in the Lead's chat as a user does.
await go("chat");
await until(`!!document.querySelector('main textarea')`, 15000);
check(
  "the message is sent from the chat box",
  await say("Build a small Python command-line tool, wordcount.py, that prints the number of words in the file given as its argument, with a unittest file that tests it. Keep it to one ticket."),
);
const answered = await until(`(${M}?.innerText ?? '').length > 400`, 600000);
await settle();
await shot("01-chat-answered");
check("the Lead answers in the chat", answered);

// 5. The spec agreed: a Solo agent locks its own (a team's Lead waits for the user's Approve in Needs you). If the Lead
// asks before it writes one, the user says to go ahead.
const agreed = () => js(`!/Agreeing on requirements/.test(document.querySelector('header')?.innerText ?? '')`);
let past = false;
for (let i = 0; i < 40 && !past; i++) {
  past = await agreed();
  if (!past && i === 4) {
    await go("chat");
    await say("Yes, go ahead with what you proposed.");
  }
  if (!past) await new Promise((r) => setTimeout(r, 15000));
}
await go("chat");
await shot("02-chat");
check("the requirements are agreed and the project runs", past);

// 6. The agent plans, works, is reviewed and closes its tickets (rounds run by the app).
const closed = async () => {
  const cols = await json("board", { project: project.id }).catch(() => []);
  const all = cols.flatMap((c) => c.cards.map((k) => ({ ...k, status: c.status })));
  return { all, done: all.length > 0 && all.every((k) => ["Closed", "Cancelled"].includes(k.status)) && all.some((k) => k.status === "Closed") };
};
let state = { all: [], done: false };
const deadline = Date.now() + 40 * 60_000;
while (Date.now() < deadline && !state.done) {
  await new Promise((r) => setTimeout(r, 20000));
  state = await closed();
  // A plan to approve, a merge, a question: the user's buttons in Needs you.
  await go("needs");
  if (await js(`[...document.querySelectorAll('main button')].some((b) => b.textContent.trim() === 'Approve')`)) await clickText("Approve", M);
}
check("the agent's tickets are worked, reviewed and closed", state.done, JSON.stringify(state.all.map((k) => [k.key, k.status])));
const ticket = state.all.find((k) => k.status === "Closed")?.key;

// 7. The board and a closed ticket's popup (timeline, diff, runs).
let e0 = errorsAt();
await go("tickets");
await shot("03-board");
check("the board shows the closed tickets", ticket && (await main()).includes(ticket));
if (ticket) {
  await js(`[...document.querySelectorAll('main *')].find((x) => x.childElementCount === 0 && x.textContent.trim() === ${JSON.stringify(ticket)})?.click()`);
  const popup = await until(`!!document.querySelector('[role=dialog]')`, 10000);
  await settle();
  await shot("04-ticket");
  check("a closed ticket opens with its timeline", popup && /Report|Review|Closed/.test(await js("document.querySelector('[role=dialog]')?.innerText ?? ''")));
  await send("Input.dispatchKeyEvent", { type: "keyDown", key: "Escape", code: "Escape", windowsVirtualKeyCode: 27 });
}
check("the board and the ticket raise no console error", errors.length === e0, errors.slice(e0).join(" | "));

// 8. The project map: what the agent traced, checked by Agenium (ADR-166).
e0 = errorsAt();
const stats = await json("map_stats", { project: project.id }).catch(() => null);
await go("map");
await shot("05-map");
const mapText = await main();
check(
  "the Map page draws the project's map",
  stats && (stats.nodes > 0 ? (await js("document.querySelectorAll('main svg g, main canvas').length")) > 0 : /The map fills as agents work/.test(mapText)),
  JSON.stringify(stats),
);
check("the map page raises no console error", errors.length === e0, errors.slice(e0).join(" | "));

// 9. The live office and the agent's dock (its calls, read from the run's log).
e0 = errorsAt();
await go("live");
await js(`[...document.querySelectorAll('main *')].find((x) => x.childElementCount === 0 && x.textContent.trim() === 'Solo')?.click()`);
await settle();
await shot("06-live");
check("the live office shows the agent", (await main()).includes("Solo"));
check("the live page raises no console error", errors.length === e0, errors.slice(e0).join(" | "));

// 10. Skills: the library's scorecards (criterion 12 with its interval, ADR-152 amended).
e0 = errorsAt();
await go("library");
await shot("07-skills");
check("the skills page renders", (await main()).length > 50);
check("the skills page raises no console error", errors.length === e0, errors.slice(e0).join(" | "));

// 11. The audit trail (read by project in SQL) and the usage.
e0 = errorsAt();
await go("audit");
await shot("08-audit");
const audit = await main();
check("the audit trail lists the project's events", /TicketCreated|ticket|Spec|Plan/i.test(audit));
await go("usage");
await shot("09-usage");
check("the usage page renders", (await main()).length > 50);
check("audit and usage raise no console error", errors.length === e0, errors.slice(e0).join(" | "));

writeFileSync(`${out}/result.json`, JSON.stringify({ results, project: project.id, ticket, map: stats }, null, 2));
console.log(failed() === 0 ? "ALL PASS" : `${failed()} FAILED`);
console.log(`ROOT_TICKET ${ticket ?? ""}`);
process.exit(failed() === 0 ? 0 : 1);

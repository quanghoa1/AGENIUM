// The second half of the UI smoke test, run by smoke.mjs on the same app: the provider form (preset, placeholder
// refusal, save, delete through the confirm dialog), agent export and import, the MCP and web-search settings, the
// project map, settling memory, the chat's model dialog and how a call reads.
import { mkdirSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { sleep } from "./cdp.mjs";

/** Steps 5 to 11 with the connection `t` from cdp.mjs; screenshots and the exported agent go to `out`. */
export async function forms(t, out) {
  const { js, until, shot, check, errors, invoke, clickText, typeInto } = t;

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
  // 8b. Web search (ADR-163): Exa's free search alone; a Brave and a Tavily key added in Settings go before it (the
  // strongest first, the free search last), and one is removed on its own.
  await js(`window.location.hash = '/settings'`);
  await until("!!document.querySelector('[data-testid=web-search]')", 10000);
  const wsForm = "document.querySelector('[data-testid=web-search]')?.closest('.card, section')?.querySelector('form')";
  const wsAdd = async (k) => {
    await js(`(() => { const el = ${wsForm}?.querySelector('input'); if (!el) return false;
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(el, ${JSON.stringify(k)});
      el.dispatchEvent(new Event('input', { bubbles: true })); return true; })()`);
    await sleep(200);
    await js(`${wsForm}?.querySelector('button[type=submit]')?.click()`);
  };
  const wsRows = "[...document.querySelectorAll('[data-testid=web-search] li')].map(li => li.dataset.api).join(',')";
  const wsFree = await js(wsRows);
  await wsAdd("BSA-smoke-0123456789");
  await until(`${wsRows} === 'Brave Search,Exa'`, 10000);
  await wsAdd("tvly-smoke-0123456789");
  const wsBoth = await until(`${wsRows} === 'Tavily,Brave Search,Exa'`, 10000);
  const wsStatus = await invoke("web_search_status");
  await shot("settings-web-search");
  await js(`[...document.querySelector('[data-testid=web-search] li[data-api=Tavily]').querySelectorAll('button')].find(b => b.textContent.includes('Remove key'))?.click()`);
  const wsOne = await until(`${wsRows} === 'Brave Search,Exa'`, 10000);
  await invoke("remove_web_search_key", { api: "Brave Search" });
  check(
    "web search: Tavily and Brave keys go before the free Exa in Settings, and one is removed alone",
    wsFree === "Exa" && wsBoth && wsOne && !wsStatus.includes("smoke-0123"),
    wsStatus
  );
  // 8c. The project map (ADR-166): agents add to it, the user only looks. Rows written straight into the test root's
  // database (as a closed ticket's run leaves them); the page draws them, its search finds a node, the node's panel shows
  // its links grouped by type and the cited code as the project has it, and a right-click offers the path menu.
  {
    const { DatabaseSync } = await import("node:sqlite");
    const projects = JSON.parse((await invoke("projects")) || "[]");
    const shop = projects.find((x) => x.name === "smoke shop");
    mkdirSync(join(shop.path, "src"), { recursive: true });
    writeFileSync(join(shop.path, "src", "app.py"), "import config\n\ndef main():\n    cfg = config.load_config()\n    get('https://api.example.com')\n");
    const db = new DatabaseSync(join(process.env.AGENIUM_HOME, "db", "core.db"));
    db.exec("PRAGMA busy_timeout = 5000");
    const at = new Date().toISOString();
    const node = db.prepare("INSERT INTO map_node (project, key, kind, name, file, line_start, line_end, summary, evidence, status, ticket, agent, run, created_at, checked_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)");
    node.run(shop.id, "symbol:src/app.py#main", "symbol", "src/app.py#main", "src/app.py", 3, 5, "Entry point", "{}", "confirmed", "SMOK-1", "ada", "r1", at, at);
    node.run(shop.id, "symbol:src/config.py#load_config", "symbol", "src/config.py#load_config", "src/config.py", 1, 2, "", "{}", "confirmed", "SMOK-1", "ada", "r1", at, at);
    node.run(shop.id, "domain:api.example.com", "domain", "api.example.com", null, null, null, "The shop's API", '{"file":"src/app.py","line":5}', "confirmed", "SMOK-1", "ada", "r1", at, at);
    const link = db.prepare("INSERT INTO map_link (project, src, kind, dst, evidence, status, ticket, agent, run, created_at, checked_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)");
    link.run(shop.id, "symbol:src/app.py#main", "calls", "symbol:src/config.py#load_config", '{"file":"src/app.py","line":4}', "confirmed", "SMOK-1", "ada", "r1", at, at);
    link.run(shop.id, "symbol:src/app.py#main", "requests", "domain:api.example.com", '{"file":"src/app.py","line":5}', "stale", "SMOK-1", "ada", "r1", at, at);
    db.close();
    const before = errors.length;
    await js(`window.location.hash = '/map'`);
    await until("!!document.querySelector('main select.input')", 10000);
    await js(`(() => { const s = [...document.querySelectorAll('main select')].find(x => [...x.options].some(o => o.value === ${JSON.stringify(shop.id)}));
      Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, 'value').set.call(s, ${JSON.stringify(shop.id)}); s.dispatchEvent(new Event('change', { bubbles: true })); })()`);
    const drawn = await until("document.querySelectorAll('main svg[role=img] g[data-key]').length === 3", 15000);
    const stats = await js("document.querySelector('[data-testid=map-stats]')?.innerText ?? ''");
    await shot("map-overview");
    await typeInto("#map-search", "load");
    const hit = await until("!!document.querySelector('[data-testid=map-hits] button')", 10000);
    await js("document.querySelector('[data-testid=map-hits] button').click()");
    const panel = await until("(document.querySelector('[data-testid=map-node-panel]')?.innerText ?? '').includes('load_config')", 10000);
    // From load_config, its caller: main's panel lists both links, one stale.
    await js("[...document.querySelectorAll('[data-testid=map-node-panel] li button')].find(b => b.innerText.includes('main'))?.click()");
    const mainPanel = await until("(() => { const t = document.querySelector('[data-testid=map-node-panel]')?.innerText ?? ''; return t.includes('calls') && t.includes('requests') && /stale/i.test(t); })()", 10000);
    await js("[...document.querySelectorAll('[data-testid=map-node-panel] li')].find(li => li.innerText.includes('load_config'))?.querySelectorAll('button')[1]?.click()");
    const code = await until("[...document.querySelectorAll('[data-testid=map-node-panel] pre')].some(p => p.innerText.includes('>    4') && p.innerText.includes('load_config'))", 10000);
    await shot("map-node-panel");
    await js("document.querySelector('main svg[role=img] g[data-key]')?.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, clientX: 300, clientY: 300 }))");
    const menu = await until("(document.querySelector('[data-testid=map-menu]')?.innerText ?? '').includes('Start a path here')", 5000);
    const path = JSON.parse((await invoke("map_path", { project: shop.id, from: "symbol:src/app.py#main", to: "domain:api.example.com" })) || "null");
    check(
      "the project map draws what agents added, finds a node, shows its links with the cited code, and offers the path menu",
      drawn && stats.includes("3 nodes") && stats.includes("1 stale") && hit && panel && mainPanel && code && menu && path?.links?.length === 1 && errors.length === before,
      `${stats} drawn=${drawn} hit=${hit} panel=${panel} main=${mainPanel} code=${code} menu=${menu} ${errors.slice(before).join(" | ").slice(0, 200)}`
    );
  }

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
}

"""Claude Code's own tool search against Agenium's `tool_search` (ADR-139), same model, same task wording.

Each of the 58 requests of agent/mcp/data/many-tools-queries.json is given to `claude -p` with the 123 tools of
many-tools.json, and the tool it calls is checked against the one that does the request:

  native   Claude Code connected to the tools' server, its own ToolSearch (deferred tools, the default)
  upfront  the same with every tool listed (ENABLE_TOOL_SEARCH=false)
  agenium  Claude Code connected to Agenium's tool_search + call_tool over the same tools (tool_search_mcp example),
           the run's capabilities line appended to the system prompt; JEV_KEY in the environment turns the judge on
  agenium-nojev  the same without the judge (words + meaning)
  agenium-nonames  agenium without the line naming the tools (a catalogue past the 200 names a run lists)

Build first: cargo build -p agenium-mcp --bin agenium-mcp-many-tools && cargo build -p agenium-eval --example
tool_search_mcp. Run from the repository root:
  python quality/eval/examples/tool_search_vs_cli.py --models <root>/models [--arms native,agenium] [--only N]
    [--model haiku] [--parallel 4] [--out <dir>]
Each run's cwd is a fresh folder under --out (Claude Code keeps its session there); nothing of the user's MCP servers is
loaded (--strict-mcp-config) and only ToolSearch is enabled among Claude Code's own tools.
"""
import argparse
import concurrent.futures as cf
import json
import os
import shutil
import subprocess
import sys
import time

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", ".."))
EXE = ".exe" if os.name == "nt" else ""
MANY = os.path.join(ROOT, "target", "debug", "agenium-mcp-many-tools" + EXE)
CLAUDE = shutil.which("claude") or "claude"
TSMCP = os.path.join(ROOT, "target", "debug", "examples", "tool_search_mcp" + EXE)

PROMPT = (
    "Task: {q}\n\nThis is a test environment: the team's MCP tools are simulated and nothing they do is real. Do the "
    "task with them: call the one tool that does it, once, with plausible arguments (invent any ids or values you "
    "need). Do not call any other tool after it and do not ask questions. Then reply DONE."
)


def chosen(events, arm):
    """The MCP tool the run called (the server's own name), and the tool calls it made."""
    calls = []
    for e in events:
        if e.get("type") != "assistant":
            continue
        for c in e.get("message", {}).get("content", []):
            if c.get("type") == "tool_use":
                calls.append((c.get("name", ""), c.get("input", {})))
    pick = None
    for name, inp in calls:
        if arm.startswith("agenium") and name.endswith("__call_tool"):
            pick = str(inp.get("name", ""))
            break
        if not arm.startswith("agenium") and name.startswith("mcp__many__"):
            pick = name
            break
    if pick:
        pick = pick.split("__")[-1]
    return pick, [n for n, _ in calls]


def run(arm, i, q, expect, a, line):
    cwd = os.path.join(a.out, f"{arm}-{i:02d}")
    os.makedirs(cwd, exist_ok=True)
    env = dict(os.environ)
    if arm.startswith("agenium"):
        srv = {"command": TSMCP, "args": [], "env": {"MODELS": a.models, "MANY_TOOLS": MANY, **({"OPAQUE": "1"} if a.opaque else {}),
                                                      "TS_LOG": os.path.join(cwd, "calls.jsonl")}}
        if arm in ("agenium", "agenium-nonames") and os.environ.get("JEV_KEY"):
            srv["env"]["JEV_KEY"] = os.environ["JEV_KEY"]
        cfg = {"mcpServers": {"agenium": srv}}
        env["ENABLE_TOOL_SEARCH"] = "false"
    else:
        cfg = {"mcpServers": {"many": {"command": MANY, "args": ["--opaque"] if a.opaque else []}}}
        if arm == "upfront":
            env["ENABLE_TOOL_SEARCH"] = "false"
        else:
            env.pop("ENABLE_TOOL_SEARCH", None)
    env.pop("JEV_KEY", None)
    # The connectors of the user's claude.ai account come with the login: off, as in Agenium's runs (ADR-119).
    env["ENABLE_CLAUDEAI_MCP_SERVERS"] = "false"
    cfgp = os.path.join(cwd, "mcp.json")
    json.dump(cfg, open(cfgp, "w"))
    cmd = [CLAUDE, "-p", PROMPT.format(q=q), "--model", a.model, "--strict-mcp-config", "--mcp-config", cfgp,
           "--tools", "ToolSearch", "--allowedTools", "mcp__many", "mcp__agenium", "--output-format", "stream-json",
           "--verbose", "--max-turns", "8", "--setting-sources", "project", "--disable-slash-commands"]
    if arm.startswith("agenium") and arm != "agenium-nonames":
        cmd += ["--append-system-prompt", line]
    t = time.time()
    p = subprocess.run(cmd, cwd=cwd, env=env, capture_output=True, text=True, encoding="utf-8", errors="replace",
                       timeout=300)
    events = []
    for l in p.stdout.splitlines():
        try:
            events.append(json.loads(l))
        except ValueError:
            pass
    open(os.path.join(cwd, "stream.jsonl"), "w", encoding="utf-8").write(p.stdout)
    pick, calls = chosen(events, arm)
    res = next((e for e in events if e.get("type") == "result"), {})
    u = res.get("usage", {})
    return {
        "arm": arm, "i": i, "query": q, "expect": expect, "pick": pick, "ok": pick == expect, "calls": calls,
        "turns": res.get("num_turns"), "cost": res.get("total_cost_usd"), "s": round(time.time() - t, 1),
        "input": (u.get("input_tokens") or 0) + (u.get("cache_read_input_tokens") or 0)
        + (u.get("cache_creation_input_tokens") or 0),
        "output": u.get("output_tokens"), "error": None if res else p.stderr[-400:],
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--models", required=True)
    ap.add_argument("--arms", default="native,upfront,agenium,agenium-nojev")
    ap.add_argument("--only", type=int)
    ap.add_argument("--model", default="haiku")
    ap.add_argument("--opaque", action="store_true", help="tools named tool_000..., descriptions kept")
    ap.add_argument("--parallel", type=int, default=4)
    ap.add_argument("--out", default=os.path.join(ROOT, "..", "tmp", "tscmp"))
    a = ap.parse_args()
    a.out = os.path.abspath(a.out)
    a.models = os.path.abspath(a.models)
    cases = json.load(open(os.path.join(ROOT, "agent", "mcp", "data", "many-tools-queries.json"), encoding="utf-8"))
    idx = range(len(cases)) if a.only is None else [a.only - 1]
    names = [t["name"] for t in json.load(open(os.path.join(ROOT, "agent", "mcp", "data", "many-tools.json"), encoding="utf-8"))]
    want = (lambda n: "tool_%03d" % names.index(n)) if a.opaque else (lambda n: n)
    line = subprocess.run([TSMCP, "--line"], env={**os.environ, "MODELS": a.models, "MANY_TOOLS": MANY,
                                                  **({"OPAQUE": "1"} if a.opaque else {})},
                          capture_output=True, text=True).stdout.strip()
    jobs = [(arm, i, cases[i]["query"], want(cases[i]["expect"])) for arm in a.arms.split(",") for i in idx]
    out = []
    with cf.ThreadPoolExecutor(a.parallel) as ex:
        for r in ex.map(lambda j: run(*j, a, line), jobs):
            out.append(r)
            print(json.dumps(r), flush=True)
    json.dump(out, open(os.path.join(a.out, "results.json"), "w"), indent=1)
    print("\n| Arm | right tool | tool calls / run | input tokens / run | output tokens / run | s / run |")
    print("|---|---|---|---|---|---|")
    for arm in a.arms.split(","):
        rs = [r for r in out if r["arm"] == arm]
        n = len(rs)
        if not n:
            continue
        avg = lambda k: sum((r[k] or 0) for r in rs) / n
        print(f"| {arm} | {sum(r['ok'] for r in rs)}/{n} | {sum(len(r['calls']) for r in rs) / n:.1f} | "
              f"{avg('input'):.0f} | {avg('output'):.0f} | {avg('s'):.0f} |")


if __name__ == "__main__":
    sys.exit(main())

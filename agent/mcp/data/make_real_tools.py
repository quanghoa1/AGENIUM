"""Builds real-tools.json: the published tool definitions of six real MCP servers (330 tools: GitHub 127, Sentry 77,
Playwright 72, Notion 24, Slack 18, git 12), fetched from pinned commits and read as data (nothing is run). The
requests in real-tools-queries.json are ours. The catalogue is not kept in the repository (the servers' licenses
differ: MIT, Apache-2.0, FSL-1.1); fetch it before `examples/tool_search_sweep.rs` (ADR-139).

Usage, from the repository root: python -I agent/mcp/data/make_real_tools.py [out.json]
(default agent/mcp/data/real-tools.json, ignored by git)."""
import json
import os
import re
import sys
import urllib.request

PINS = {
    "github": ("github/github-mcp-server", "f2a07cd13243ea9773a0ef42692e5b0e156f0951"),
    "sentry": ("getsentry/toolkit", "3bbc00e40d422660f7c982d6059261720af4a76c"),
    "playwright": ("microsoft/playwright-mcp", "f6807605220f8c69f564feb3f83b5a3a330d0458"),
    "notion": ("makenotion/notion-mcp-server", "cf16e5c20bfbfb98ecec1227f210ef0ae3abebe5"),
    "git": ("modelcontextprotocol/servers", "194cc86e3ae6400958555bd4116849d822d69263"),
    "slack": ("korotovsky/slack-mcp-server", "b88c0de3f706f4f07337c9eda7133c736d1c9524"),
}


def fetch(url):
    req = urllib.request.Request(url, headers={"User-Agent": "agenium-eval"})
    with urllib.request.urlopen(req, timeout=60) as r:
        return r.read().decode("utf-8")


def raw(server, path):
    repo, sha = PINS[server]
    return fetch(f"https://raw.githubusercontent.com/{repo}/{sha}/{path}")


tools = []
seen = set()


def add(server, name, description, schema):
    if (server, name) in seen:
        return
    seen.add((server, name))
    tools.append({
        "server": server,
        "name": name,
        "description": (description or "").strip(),
        "inputSchema": schema or {"type": "object", "properties": {}},
    })


# GitHub: the server's own tool snapshots; variants for feature flags and output formats left out.
repo, sha = PINS["github"]
tree = json.loads(fetch(f"https://api.github.com/repos/{repo}/git/trees/{sha}?recursive=1"))["tree"]
for p in sorted(t["path"] for t in tree if t["path"].startswith("pkg/github/__toolsnaps__/") and t["path"].endswith(".snap")):
    base = os.path.basename(p)[:-5]
    if base.endswith("_output") or "_ff_" in base or base.endswith("_resolution_reason") or base.endswith("_with_intent"):
        continue
    t = json.loads(raw("github", p))
    add("github", t["name"], t.get("description"), t.get("inputSchema"))

# Sentry: the generated tool definitions.
for t in json.loads(raw("sentry", "packages/mcp-core/src/toolDefinitions.json")):
    add("sentry", t["name"], t.get("description"), t.get("inputSchema"))

# git (modelcontextprotocol/servers): the tools/list snapshot of its tests.
for t in json.loads(raw("git", "src/git/tests/snapshots/list_tools.json"))["tools"]:
    add("git", t["name"], t.get("description"), t.get("inputSchema"))

# Notion: one tool per OpenAPI operation, as notion-mcp-server makes them.
api = json.loads(raw("notion", "scripts/notion-openapi.json"))
def resolve(s, depth=0):
    if isinstance(s, dict) and "$ref" in s and depth < 4:
        node = api
        for part in s["$ref"].lstrip("#/").split("/"):
            node = node.get(part, {})
        return resolve(node, depth + 1)
    return s


for path, ops in api["paths"].items():
    for method, o in ops.items():
        if not isinstance(o, dict) or "operationId" not in o:
            continue
        props, required = {}, []
        for prm in o.get("parameters", []):
            prm = resolve(prm)
            if "name" in prm:
                props[prm["name"]] = {"type": resolve(prm.get("schema", {})).get("type", "string"),
                                      "description": prm.get("description", "")}
                if prm.get("required"):
                    required.append(prm["name"])
        body = resolve(o.get("requestBody", {}))
        js = resolve(body.get("content", {}).get("application/json", {}).get("schema", {}))
        for k, v in (js.get("properties") or {}).items():
            v = resolve(v)
            props[k] = {"type": v.get("type", "object"), "description": v.get("description", "")}
        required += js.get("required", [])
        desc = " ".join(x for x in [o.get("summary", ""), o.get("description", "")] if x)
        add("notion", "API-" + o["operationId"], f"Notion | {desc}",
            {"type": "object", "properties": props, "required": required})

# Playwright: the README's generated tool list.
md = raw("playwright", "README.md")
for m in re.finditer(r"^- \*\*(browser_[a-z_]+)\*\*\n((?:  .*\n)+)", md, re.M):
    name, body = m.group(1), m.group(2)
    d = re.search(r"- Description: (.*)", body)
    props, required = {}, []
    for pm in re.finditer(r"^    - `([A-Za-z_]+)` \(([a-z]+)(, optional)?\): (.*)$", body, re.M):
        props[pm.group(1)] = {"type": pm.group(2), "description": pm.group(4)}
        if not pm.group(3):
            required.append(pm.group(1))
    add("playwright", name, d.group(1) if d else "", {"type": "object", "properties": props, "required": required})

# Slack (korotovsky/slack-mcp-server): the README's tool sections.
md = raw("slack", "README.md")
tools_part = md.split("## Tools", 1)[1].split("\n## ", 1)[0]
for m in re.finditer(r"^### \d+\. ([a-z_]+):?\n(.*?)(?=^### |\Z)", tools_part, re.M | re.S):
    name, body = m.group(1), m.group(2)
    desc = body.split("- **Parameters:**")[0]
    desc = "\n".join(l for l in desc.splitlines() if not l.startswith(">")).strip()
    props, required = {}, []
    for pm in re.finditer(r"^  - `([A-Za-z_]+)` \(([a-z]+)([^)]*)\): (.*)$", body, re.M):
        props[pm.group(1)] = {"type": pm.group(2), "description": pm.group(4)}
        if "required" in pm.group(3):
            required.append(pm.group(1))
    add("slack", name, desc, {"type": "object", "properties": props, "required": required})

out = sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.path.dirname(__file__), "real-tools.json")
json.dump(tools, open(out, "w", encoding="utf-8"), indent=1)
by = {}
for t in tools:
    by[t["server"]] = by.get(t["server"], 0) + 1
print(len(tools), by)

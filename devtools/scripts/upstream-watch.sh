#!/usr/bin/env bash
# What changed upstream in the sandbox Agenium builds on (see docs/reference/SANDBOX-MAINTENANCE.md §2).
#
#   devtools/scripts/upstream-watch.sh
#
# Reports: the vendored Codex sandbox files against openai/codex main, the mxc-sdk pin against the newest release on
# crates.io, the latest microsoft/mxc release, and the state of the upstream issues/PRs Agenium waits on (WATCH below).
# Read-only: it changes nothing. Exit 0 = nothing new, 1 = something to look at, 2 = a lookup failed.
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
UA="User-Agent: agenium-upstream-watch"
# microsoft/mxc issues and PRs that decide an open item in Agenium (number: why).
WATCH=(
  "1277:Seatbelt: scope Unix sockets (ADR-55: a host unix socket is reachable with network on)"
  "1278:PR deniedUnixSocketPaths for Seatbelt (closed unmerged 2026-10-24)"
  "1568:Bubblewrap: keep host resolver sockets out of sandboxes (the 1.0.1 DNS forwarder, ADR-95)"
  "1443:Bubblewrap: seccomp default-deny profile"
)
status=0
json() { curl -fsSL -H "$UA" "$1"; }
py() { python3 -c "$1" 2>/dev/null || python -c "$1"; }

echo "== Codex sandbox (vendor/codex-sandbox) vs openai/codex main"
if out="$("$ROOT/devtools/scripts/sync-codex-sandbox.sh" main --summary 2>&1)"; then
  echo "$out" | head -1
  changed="$(echo "$out" | grep -E '^(changed|new|missing-upstream)' || true)"
  if [ -n "$changed" ]; then echo "$changed"; status=1; else echo "   all vendored files match"; fi
else
  echo "   lookup failed: $out" | head -3; status=2
fi

echo "== mxc-sdk"
pin="$(grep -m1 -E '^mxc-sdk *= *"' "$ROOT/security/sandbox/Cargo.toml" | grep -oE '[0-9]+\.[0-9]+\.[0-9]+')"
if meta="$(json https://crates.io/api/v1/crates/mxc-sdk)"; then
  latest="$(echo "$meta" | py "import json,sys; print(json.load(sys.stdin)['crate']['max_version'])")"
  echo "   pinned $pin, newest on crates.io $latest"
  [ "$pin" != "$latest" ] && { echo "   NEW: read the release notes, then bump only after the containment matrix passes (§2.5)"; status=1; }
else
  echo "   crates.io lookup failed"; status=2
fi
if rel="$(json https://api.github.com/repos/microsoft/mxc/releases/latest)"; then
  echo "$rel" | py "import json,sys; r=json.load(sys.stdin); print('   latest release', r['tag_name'], r['published_at'][:10], r['html_url'])"
fi

echo "== microsoft/mxc items Agenium waits on"
for w in "${WATCH[@]}"; do
  n="${w%%:*}"; why="${w#*:}"
  if it="$(json "https://api.github.com/repos/microsoft/mxc/issues/$n")"; then
    echo "$it" | N="$n" WHY="$why" py "import json,os,sys; i=json.load(sys.stdin); print(f\"   #{os.environ['N']} {i['state']:6} updated {i['updated_at'][:10]}  {os.environ['WHY']}\")"
  else
    echo "   #$n lookup failed"; status=2
  fi
done
exit $status

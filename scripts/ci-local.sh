#!/usr/bin/env bash
# Local CI (GitHub Actions is not used): fmt, clippy, tests, generated-schema check, scripted golden tasks.
# Usage: scripts/ci-local.sh [--quick] [--fail-fast]     Logs: target/ci-local/<step>.log
set -u
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
mkdir -p target/ci-local
quick=0; failfast=0
for a in "$@"; do case "$a" in --quick) quick=1;; --fail-fast) failfast=1;; esac; done
steps=("fmt|cargo fmt --all --check" "clippy|cargo clippy --workspace --all-targets -- -D warnings" "test|cargo test --workspace" "schemas|cargo xtask schemas --check" "vendor|cargo xtask vendor-check")
[ $quick -eq 0 ] && steps+=("golden-scripted|cargo run -q -p agenium-eval -- --out target/ci-local/eval")
fail=0; summary=""
for s in "${steps[@]}"; do
  name="${s%%|*}"; cmd="${s#*|}"; start=$(date +%s)
  if bash -c "$cmd" > "target/ci-local/$name.log" 2>&1; then r=PASS; else r=FAIL; fail=1; fi
  summary+=$(printf "%-16s %s %ss\n" "$name" "$r" "$(( $(date +%s) - start ))")$'\n'
  [ $fail -eq 1 ] && [ $failfast -eq 1 ] && break
done
printf "%s" "$summary"
exit $fail

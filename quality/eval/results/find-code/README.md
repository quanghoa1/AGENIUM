# `find_code` A/B on a pinned repository (2026-10-28)

Does `find_code` (ADR-128) take work off a coding agent? `quality/eval/examples/find_code_ab.rs`, 10 behavioural
questions about **pallets/flask at tag 3.1.0** (`quality/eval/data/find-code-cases-flask.json`, each with the file and
function that answer it), asked read-only twice per agent: with its usual read tools, and with `find_code` added. An
outside repository pinned at a tag keeps the answers fixed while Agenium changes. 100 runs, all completed.

- API: DeepSeek v4.1 Flash (OpenCode Go) with Agenium's read tools (read_file, list_dir, glob, grep).
- CLIs, each with its own file tools and `find_code` through Agenium's MCP bridge as in a real run, no shell: Claude
  Code with Haiku 5.5, Codex with gpt-6-luna, Grok with 4.7, agy with Gemini 3.8 Flash.

| Agent | right without / with | input tokens without → with | steps without → with | runs that used find_code | Jev input tokens (≈ $) |
|---|---|---|---|---|---|
| Codex gpt-6-luna | 10/10 / 10/10 | 1 053 383 → 704 761 (**−33.1 %**) | 64 → 34 | 8/10 | 555 831 (0.023) |
| Grok 4.7 | 10/10 / 10/10 | 1 172 533 → 835 698 (**−28.7 %**) | 77 → 48 | 10/10 | 617 566 (0.026) |
| agy Gemini 3.8 Flash | 9/10 / **10/10** | 982 065 → 806 955 (**−17.8 %**) | 90 → 74 | 10/10 | 592 011 (0.025) |
| DeepSeek v4.1 Flash (API) | 10/10 / 10/10 | 274 017 → 225 593 (**−17.7 %**) | 70 → 54 | 6/10 | 340 088 (0.014) |
| Claude Code Haiku 5.5 | 10/10 / 10/10 | 676 849 → 644 889 (**−4.7 %**) | 52 → 41 | 8/10 | 525 123 (0.022) |

Read:
- Every agent spent fewer input tokens and fewer steps with `find_code`, none answered worse; agy answered one more
  question right. jevgrep reports 25.8 % lower total cost on ten SWE-bench tasks; here −5 % to −33 % per agent.
- The judge's cost is small next to the agents' (about $0.002 per run that used it).
- Claude Code gains least: its own prompt is about 50 k tokens a turn, and it first looks the tool up (`ToolSearch`),
  so a saved step weighs less. agy's question 1 is the outlier the other way (22 steps with the tool: it read many
  files after it).
- The CLIs' input counts include their own system prompts, so the share saved on the search itself is larger.

Raw tables with every answer: `flask-3.1.0-<agent>-20261028.md`. Run again with
`cargo run -p agenium-eval --example find_code_ab -- --repo <flask checkout at 3.1.0> --cases
quality/eval/data/find-code-cases-flask.json --cli <claude|grok|codex|agy> --model <id>` (or `--provider`/`--root` for
an API model) and `JEV_KEY` set.

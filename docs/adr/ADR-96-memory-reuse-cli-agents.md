# ADR-96: Memory reuse on CLI agents — Agenium's own word is trusted, the shell's encoding trap is named

Status: Accepted (2026-10-26). Evidence: `docs/memory-eval.md` §4 (memory-reuse on Claude Haiku/Sonnet/Opus and Codex
gpt-6-luna/gpt-6.1-sol through their CLIs, memory on/off, n = 3 per arm). Amends ADR-94 (scenario) and the extraction
rules of the memory pipeline (ADR-51 §6).

## Context

The memory-reuse scenario (`quality/eval/src/memory_reuse.rs`) has a pre-merge check that rejects a team document
whose first line is not `Owner: docs-team`. No ticket states the rule. Ticket 1 fails on it, and the agent learns it.
Ticket 2 is measured: with memory on, the agent should pass the first merge. With DeepSeek it did (ADR-94).

The first CLI batch (eval binary at 20d6bfb, 30 runs) gave these first-merge passes on ticket 2, memory on vs off:
Haiku 2/3 vs 0/3, Sonnet 3/3 vs 0/3, Opus 2/3 vs 0/3, gpt-6-luna 0/3 vs 0/3, gpt-6.1-sol 0/3 vs 0/3. Memory saved
steps and tokens wherever it helped (Sonnet: 9 vs 15 steps, 152 k vs 264 k tokens). The Codex runs showed no effect.
The eval then recorded what the check rejected and what each agent held before ticket 2. A rerun of Codex on the same
code found three causes, and none of them is the model's:

1. **The rule was stored as untrusted.** The extraction rules mark content from outside (web pages, files the agent
   read) as `external`, which is stored `untrusted` and never recalled. Seen as plain text, a check's rejection in a
   system comment or a protocol tool's result looks just like outside content. The extractor marked the rule
   external in 3 of the 6 Codex reruns with memory on, e.g. `[Fact untrusted] The docs project's pre-merge check requires …`. The rule
   survived only when another lesson restated it.
2. **Windows PowerShell 5.1 writes a byte-order mark.** `Set-Content -Encoding utf8` and `Out-File -Encoding utf8` start
   the file with U+FEFF, and `>` writes UTF-16. Codex (the agent shell here is `powershell.exe`) wrote
   `\u{feff}Owner: docs-team`, so the rule was recalled and followed, and still rejected. Claude writes with its file
   tool and never met this.
3. **Lexical recall misses with the hashing embedder.** The hashing embedder (the default until `agenium models
   download`) has no stemming: "report" does not match "reports". A probe on the stored facts found that one rule
   phrasing was never recalled with hashing; with bge-small every rule fact ranked in the top 3.

The rerun itself (same code, diagnostics added) also showed how large the variance between batches is at n = 3:
Codex with memory on passed 5/6 first merges (luna 2/3, sol 3/3) against 0/6 in the first batch. Its only failure was
the byte-order mark. Codex with memory off stayed at 0/6.

## Decision

1. **Agenium's own word is never external.** The extraction rules say so explicitly: the ticket, its comments (system
   comments carry Agenium's check, merge and review results) and `[agenium result]` lines; a project rule stated there
   is trusted. The episode transcript tags a protocol tool's result `[agenium result]` / `[agenium error]` and any
   other tool's `[tool result]` / `[tool error]`, so the extractor can tell them apart. Content an agent fetched or
   read stays external.
2. **The work order names Windows PowerShell 5.1's encoding trap.** When the agent shell is `powershell.exe`, its line
   in the work order adds that `Set-Content`/`Out-File -Encoding utf8` write a byte-order mark and `>` writes UTF-16,
   and tells the agent to write files with its file tool or `[IO.File]::WriteAllText` instead. `pwsh` (7+) writes
   UTF-8 without a BOM by default and gets no note.
3. **The eval records why a run failed.** Each result now carries the embedder in use, what the check rejected (the raw
   first line), and the memories the agent held before ticket 2 (kind, trust, weight, scope). `--models <dir>` installs
   bge-small for a run. The published runs keep the hashing embedder, so they stay comparable with ADR-94 and match a
   fresh install.

## Result

After the fixes (same scenario, 5 models × 3 × memory on/off, 2026-10-26): ticket 2 passed its first merge in
**15/15 runs with memory and 0/15 without**. With memory, every model needed fewer steps (22–47 % fewer, medians) and
fewer tokens (23–44 % fewer). None of the 15 memory runs held the rule as untrusted, and none was rejected for a
byte-order mark. Details: `docs/memory-eval.md` §4.

## Consequences

- A rule learned from Agenium's own feedback reaches the next ticket. Prompt-injected content from a fetched page is
  still untrusted, because it never arrives as an Agenium result.
- The extraction transcript is a little longer (the tags).
- Hashing-embedder recall remains lexical. `doctor` already says so and recommends `agenium models download`.

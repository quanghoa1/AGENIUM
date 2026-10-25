# Learning loop on real models (ADR-91/92/93)

Does a real model use the skill-writing tools with good judgment, and do the gates hold on what it actually proposes?
Run 2026-10-25 with `cargo run -p agenium-eval -- --learning (--provider … | --cli …) --approve-commands`; raw results in
`quality/eval/results/learning-loop/` (pass 1 before the fixes of ADR-93, pass 2 after).

## Scenario (`quality/eval/src/learning.rs`)

A Solo agent holds three lessons, each held in two contexts, that call for three different right answers, then does an
ordinary ticket (create `deliverables/hello.txt`):

| Lesson | Right answer |
|---|---|
| `procedure` — every new deliverable gets a sorted entry in `deliverables/INDEX.md` | a new skill is reasonable (or nothing) |
| `covered` — commit subjects start with the ticket key | improve the enabled `commit-and-pr` skill, not a new one |
| `fact` — the demo project was made for an October review | no skill at all (only the model can judge this) |

Graded: ticket closed with correct work; no skill-writing before any work; nothing built from the fact; no new skill
for the covered rule. Using the tools is not required.

## Results — 12 runs, 6 models × 2 passes

| Model | Pass 1 | Pass 2 | Applied the procedure to the work |
|---|---|---|---|
| deepseek-v4.1-flash (API) | updated commit-and-pr ✓ | — | not recorded / ✗ |
| claude-haiku-5-5 (CLI) | — | — | ✓ / ✓ |
| claude-sonnet-5-5 (CLI) | — | updated commit-and-pr ✓ | ✓ / ✓ |
| claude-opus-5-5 (CLI) | new `deliverables-index` ✓ + updated commit-and-pr ✓ (2nd try) | new `deliverables-index` ✓ + updated commit-and-pr ✓ | ✓ / ✓ |
| gpt-6-luna (CLI) | new `deliverables-index` ✓ | new `deliverables-index` ✓ | ✓ / ✓ |
| gpt-6.1-sol (CLI) | new skill ✓; update **failed, gave up** | update refused once (real reason shown) → **retried ✓** | ✓ / ✓ |

- **12/12 passed, 0 violations.** Every ticket closed with correct work.
- **Judgment:** the tools were used in **8/12** runs, and every use was the right tool for the right lesson. No model
  built anything from the one-off fact, none made a new skill for the rule an existing skill covers, none wrote skills
  before working. Opus made both right moves in both passes.
- **Recall:** 10 of 11 recorded runs applied the recalled `procedure` lesson to the work itself; DeepSeek did not.
- **Bug found and fixed (ADR-93):** 3 of 6 update attempts failed to apply a patch. The error the agent saw came from a
  fallback (`b/SKILL.md: No such file or directory`) and hid the real reason, so Sol gave up in pass 1. The patch Sol
  sent in pass 2 shows the cause: a hunk with **no context lines** numbered from the body it had read (line 7; the real
  line is 14), which git cannot place. After the fixes the agent sees the real reason, may send the whole new body
  instead of a diff, and such a hunk applies when its text is unique.
- **Not exercised by real output:** no model proposed a duplicate or contradicting skill here, so the subject and
  contradiction gates (ADR-92) are verified by tests, not yet by a real model trying to get past them.

Tokens per run: 110 k–250 k in, 1.4 k–4.4 k out; 40–140 s (Codex CLI runs are the slowest).

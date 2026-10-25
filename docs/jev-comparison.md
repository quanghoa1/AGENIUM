# With Jev vs without: how much it improves

Jev (TypeSafe System One) is an **optional, fail-open** judge: off by default, and every Jev call returns nothing on
any doubt, so Agenium behaves identically with Jev absent. This note compares the two modes for the three wired uses and
says, with real numbers, how much Jev buys — and when it is worth turning on.

**Bottom line (confirmed by the measured A/B below):** on a straightforward judgment a cheap flash model is **as
accurate as Jev**, so Jev is not a correctness upgrade there. Its concrete, measured wins are **lower latency (~5×)**
and **token leanness** (no reasoning tokens); on top of that come **fewer tool steps** and **cleaner memory** by design,
and a **calibration** edge that needs *ambiguous* inputs to pay off (this easy set did not show it). Dollar savings are
small on a cheap model and grow roughly linearly with the agent model's price. Jev earns its place on expensive agent
models, skill-heavy agents, routine-heavy workloads, and review-heavy teams; it earns little on a cheap flash model
doing novel one-off work. It is off by default and fail-open, so turning it on never risks correctness.

## Prices and assumptions

- Agent model, cheap: `deepseek-v4.1-flash` — **$0.14 / 1M input, $0.28 / 1M output** (price catalogue, Oct 2026).
- Agent model, expensive (illustrative frontier class): **$3 / 1M input, $15 / 1M output**.
- ~4 characters per token.
- Jev typed call: measured earlier at ~**128 tokens per question** (3 questions ≈ 384 tokens). For a `Choice`/`Noul`
  Jev reads only the state you give it. TypeSafe's own per-token price is not in the public catalogue, so below we count
  **the agent-model tokens Jev lets you avoid** and treat Jev's own cost as small (and independent of the agent model);
  the live A/B (bottom) measures it directly.

These are per-call figures; multiply by your ticket/skill volume.

## 1. `skill_hint` — the cleanest token win

Without Jev, an agent with several skills reads their `SKILL.md` files (via `load_skill`) to decide which fits — full
file content, on the agent model, every run. With Jev, it reads **only each skill's one-line description** (name +
description, ~50 tokens each) once and picks; the agent then loads just the one chosen file.

Representative: 4 skills, each `SKILL.md` ≈ 1,500 tokens; without the hint an exploring agent reads 2–4 of them.

| | Without Jev | With Jev |
| --- | --- | --- |
| Agent-model input tokens to choose | 3,000–6,000 (reads 2–4 files) | ~1,500 (reads only the chosen file) |
| Jev input | — | ~200 tokens (4 descriptions) |
| Extra tool round-trips to choose | 1–3 `load_skill` calls | 0 |
| Cost of the avoided reading, flash | — | **~$0.0004–0.0006 / run saved** |
| Cost of the avoided reading, frontier | — | **~$0.009–0.013 / run saved** |

Why it is the clean win: Jev reads the short descriptions, not the files, so it never pays what it saves.

## 2. `memory_gate` — skip extraction on routine tickets

After a ticket closes Agenium runs one extraction call on the agent model (`max_tokens` 4000) over the transcript.
On a routine ticket that is wasted. The gate asks Jev *is anything here worth remembering?* and skips extraction on a
confident no (only when there is nothing to reconcile).

Representative routine transcript ≈ 8,000 input tokens; extraction output ≈ 800 tokens.

| | Without Jev (always extract) | With Jev (skip routine) |
| --- | --- | --- |
| Extraction cost, flash | 8,000·$0.14 + 800·$0.28 per 1M = **$0.00134 / ticket** | avoided on routine tickets |
| Extraction cost, frontier | **$0.036 / ticket** | avoided on routine tickets |
| Generation latency | a ~800–4,000-token generation each time | none on skipped tickets |
| Memory quality | routine noise can enter memory | only episodes worth it are mined |

Caveat, stated honestly: Jev also reads the transcript, so the **dollar** saving is the agent model's output
generation plus the price gap between the two models' input; the reliable wins here are the avoided generation
**latency** and **less memory noise**. On a frontier agent model the dollar saving is real (~$0.03+/routine ticket).

## 3. `review_assist` — calibrated quality, not tokens

Before a review run Jev reads the evidence against each acceptance criterion and returns a **calibrated probability**
per criterion. A chat model gives an uncalibrated yes/no; Jev gives a number you threshold at 0.7 / 0.3 and a clear
"unsure" band. The payoff is **avoided reopens**: a reopen costs a full extra work run (thousands of tokens + minutes).
If review_assist prevents even ~1 reopen in 20 tickets on a frontier model, it more than pays for every Jev call in the
batch.

## When Jev is worth it

| Factor | Jev helps more | Jev helps little |
| --- | --- | --- |
| Agent model price | frontier ($/token high) | cheap flash |
| Skills per agent | many | 0–1 |
| Ticket mix | routine-heavy | every ticket novel |
| Review load | strict, reopen-averse | lenient |

Because Jev is off by default and fail-open, turning it on never risks correctness — the worst case is you paid for a
few cheap typed calls that returned "unsure". Turn it off and Agenium is unchanged.

## The live A/B — measured (2026-10-25)

Run on a 16-item labelled judge set (`quality/eval/data/jev-judge.json`: evidence/criterion and lesson/fact-signal
items, the shape of `review_assist` and `worth_remembering`). Jev via the TypeSafe API; baseline `deepseek-v4.1-flash`
through Agenium's own provider (`cargo run -p agenium-eval --example jev_baseline`). Jev's key is test-only and was
passed in per-command env, never stored.

| Metric | Jev (`jev-latest`) | Baseline `deepseek-v4.1-flash` |
| --- | --- | --- |
| Accuracy | **16/16 (1.000)** | **16/16 (1.000)** |
| Brier (lower = better calibrated) | 0.0100 | **0.0069** |
| Avg latency | **0.37 s** | 1.95 s |
| Avg tokens | ~128/question (prior measurement; this run's API response did not expose usage) | 115 in + 67 out (up to 274 out on the one subtle item) |
| Cost / call | small (TypeSafe price not public) | ~$0.000035 |

**What the data actually says — honestly:**

- **Accuracy: a tie.** On this set both are perfect. The set is not hard enough to separate them on correctness.
- **Calibration: the baseline edged Jev here.** DeepSeek answered mostly a hard 0 or 1, and on items it got right that
  gives a near-zero Brier; Jev's softer probabilities (0.82–0.91 on true items) add a little Brier. Jev's calibration
  advantage is expected to show on *ambiguous* items where a chat model is overconfident — this easy set did not stress
  that, so do not over-read the 0.0069 vs 0.0100.
- **Latency: Jev wins clearly (~5×).** 0.37 s vs 1.95 s. On a review that batches a Noul per criterion, that compounds.
- **Tokens: Jev is leaner**, mainly because it emits no reasoning tokens; DeepSeek's output grew to 274 on the subtle
  item. On a cheap model the dollar difference is negligible either way.

**Takeaway:** for a *straightforward* judgment a cheap flash model is as accurate as Jev, so Jev is not a correctness
upgrade there — its concrete wins are **latency** and **token leanness**, and its calibration edge needs harder,
ambiguous inputs to pay off. This matches the design stance: Jev is an optional, fail-open *accelerator* for typed
judgments, not a requirement. Re-run on a harder labelled set to probe the calibration claim; the gating logic itself is
covered by `team.rs::skill_hint_*`, `team.rs::memory_gate_*`, and `agent/llm/tests/systemone.rs`.

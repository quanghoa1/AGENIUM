# ADR-127: The judge in modules; shadow mode, answer cache with prefetch, billed usage, output trimmer

Status: Accepted (2026-10-28; owner: "làm đi" on the research's next steps, and "chia nhỏ code ra từng module nhỏ cho dễ
maintain, ví dụ jev có nhiều chức năng thì chia ra các file, nếu chức năng nào lỗi thì sẽ dễ fix hơn"). Follows
ADR-126; research in [`docs/reference/JEV-OPPORTUNITIES.md`](../reference/JEV-OPPORTUNITIES.md).

## Decision

1. **One file per concern** under `orchestration/orchestrator/src/judge/` (was one 580-line `judge.rs`): `settings`
   (endpoint, key, switches, test call), `health` (when a failure turns it off), `client` (asking, activity log),
   `cache`, and one file per use — `skill_hint`, `memory_gate`, `skill_quality` (with contradictions),
   `review_assist`, `done_check`, `trim`. Paths callers use are re-exported unchanged.
2. **Shadow mode** (`[judge] shadow`, "observe only"): every use that is on asks and records what it would do —
   prefixed `[shadow]` in the activity, `shadow: true` — and changes nothing (no hint, no skip, no evidence note, no
   send-back, no trim, no refusal). A use's offload can then be measured on real runs before it is turned on, which
   is how a Jev use is judged (`docs/JEV.md`).
3. **Billed usage** (`usage` as the API reports it) is kept with each ask in `judge-activity.jsonl`.
4. **Answer cache** (`<root>/judge-answers.jsonl`, newest 2 000 kept): an answer is stored under a hash of the
   endpoint and model, the question, and the state entries the question names (`` `key` ``). The same question about
   the same text is not asked again; requests with images are never cached.
5. **Prefetch at submit**: when the done check has to ask (ADR-126), the review's per-criterion questions ride in the
   same request (`Judging::merge`) over the same report text (`judge::report_text`, used at submit and at review), so
   the review run finds them in the cache: one request instead of two, the report sent once.
6. **Output trimmer** (`[judge] trim_output`, off by default): in a work run, `shell` and `grep` are wrapped
   (`judge::trim::wrap`) so a result over 80 lines is split in 30-line chunks and the judge is asked, for each middle
   chunk, whether it holds something the ticket needs (at most 8 chunks, one request). The first and last chunk and
   every chunk with an error, failure, warning, panic or exit code are always kept; a chunk goes only when the judge is
   at most 30 % sure and it is in the lower half of the ranking. Each run of dropped chunks becomes one line, and the
   full output is saved beside it (`ToolCtx::save_output`) and linked. Trimming happens when the result is made, so the
   prompt-cache prefix never changes afterwards, and tool specs are unchanged (the wrapper delegates them).

## Consequences

- Tests: shadow changes nothing and keeps usage; the review reads prefetched answers with no request; a long log loses
  only a confidently unneeded, low-ranked chunk while head, tail and failures stay; cache keys cover only named state
  and change with the endpoint or model.
- **Measured with the real API** (`quality/eval/examples/jev_new_uses.rs`, raw `docs/reference/bench-raw/jev-new-uses-2026-10-28.md`):
  - *Done check*, 10 labelled reports: decisions as expected 9/9 (one case either way). The facts settle what they can
    — every criterion Met is the "done" claim, a check that failed after the last change sends back with no question,
    a passing check or an honest NotMet is accepted — and Jev is asked only whether a check would apply (≈ 430 input
    tokens, < $0.00002). A first version that also asked Jev "does the report claim done" got 8/9: it read
    "Implemented the CSV export." as 48 % done while the build had just failed.
  - *Output trimmer*, six real outputs of this repository's runs (cut to what the shell tool returns): 0 of 69 needed
    lines lost, but only one output trimmed (82 of 257 lines, ≈ 1 000 tokens) for 17 k Jev input tokens. Jev rates most
    chunks of a passing test log or a targeted grep 0.5–0.7 "needed" (they are the evidence the goal asks for), and the
    shell tool's own head + tail cap (256 lines) has already removed the bulk. A first version kept every chunk with
    the word "fail" — every `0 failed` result line — and trimmed nothing; key lines are now markers, kept line by line.
    The threshold is not lowered (that would drop chunks Jev reads as 50 % needed): the trimmer stays off by default,
    worth it only for long noisy outputs.
- Still to run: both uses in shadow mode on a real team.
- A CLI agent that brings its own shell (not hooked) is not trimmed: its tool results never pass through Agenium.

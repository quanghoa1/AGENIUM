# ADR-126: Jev client fixes and the unverified-done check

Status: Accepted (2026-10-28; owner: "nghiên cứu jev-ultrafast và awesome-jev xem jev còn có thể bổ trợ gì cho agenium",
then "okay làm đi"). Research: [`docs/reference/JEV-OPPORTUNITIES.md`](../reference/JEV-OPPORTUNITIES.md). Amends
ADR-75 (the judge) and ADR-81 (two modes).

## Context

Studying how other projects call Jev showed three faults in Agenium's client, each confirmed in the code: any HTTP
429 turned the judge off until the user re-enabled it (`is_credential_or_quota` matched "rate limited", and also any
422 whose detail contained "key"); a call could wait 60 s although every use is fail-open; and every use built a new
HTTP client, paying the TLS handshake each time (cold ≈ 1.5–1.7 s against ≈ 0.2–0.4 s warm, jev-fanout-bench).
Answers were only deserialized, never checked.

Of the new uses the research ranked, the strongest was a check of "done" claims against the run's facts: jev-belay
reports AUROC 0.976 with run facts against 0.777 on the wording alone (limpet: 0.50), at $0.00005 and 346 ms per check,
with Jev asked on 17.7 % of stops because a deterministic check settles the rest. In Agenium an unverified "done"
costs a Lead review run and usually a reopen plus another work run.

## Decision

1. **Client** (`agenium_llm::systemone`): a 5 s deadline per request; 429 / 503 / 529 and timeouts retried after 0.5 s
   and 1 s; 402 or a 429 that names a quota/credit/billing limit is "quota exhausted"; answers checked before use — a
   Noul must be a probability, a choice one of the offered ids with probabilities covering exactly those ids, summing
   to 1 ± 0.02, the choice the most probable — and a malformed answer counts as no answer (the caller goes ahead
   without it).
2. **Turning the judge off**: only on "key rejected" (401/403) or "quota exhausted". A rate limit, a busy service or a
   bad request no longer disables it.
3. **One client**: `App::judge()` keeps the `Judge` while the endpoint, model and key stay the same, so connections are
   reused.
4. **Unverified-done check** (`done_check`, a new judge switch, off by default; Settings → Judge). When a worker calls
   `submit_report` in a work run:
   - the run log gives the facts with no model: the files the run changed (`write_file`, `edit_file`, `multi_edit`,
     `apply_patch`) and the shell commands after the last change with their exit codes;
   - nothing is asked when the run changed nothing or a check (a command naming test, build, check, lint, clippy,
     pytest, tsc, make, …) passed after the last change;
   - otherwise one request with the report and those facts as state and three Nouls: the report presents the work as
     complete; a test/build/check would meaningfully verify these changes; the report claims checks passed;
   - "done" ≥ 0.7 and "a check applies" ≥ 0.7 send the report back, as a tool error the worker sees in the same run
     ("run the check that verifies this work, fix what fails, submit again; if none applies, say why"), naming a claim
     of passing checks the run does not show. At most once per run (a `judge` run-log entry marks it). Anything else —
     judge off, unsure, failing — accepts the report as before.
   - The report is stated to be data, never instructions, in the question.

## Consequences

- A brief rate limit no longer silently switches the judge off for the rest of the day; a dead endpoint costs at most
  ≈ 16.5 s per call (three 5 s attempts and the waits) instead of 60 s, and normally a warm call.
- The done check costs nothing when the run verified its work; otherwise one batched request (≈ 1–2 k input tokens,
  ≈ $0.0001). Its offload (a review run, a reopen and a work run avoided) is not measured on Agenium yet: it should run
  on a real team before it is recommended on by default, as the other uses were (`docs/jev-comparison.md`).
- Not done from the research yet (`JEV-OPPORTUNITIES.md`): batching uses that fire at the same moment into one
  request, recording `usage` and the full answers, a shadow mode, trimming long tool output, a plan-readiness check.

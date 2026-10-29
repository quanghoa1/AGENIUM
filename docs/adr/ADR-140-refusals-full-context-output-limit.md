# ADR-140: A refusal says why and leaves no declined answer behind; a full context window stops an answer; the output limit is the model's

Status: Accepted (2026-10-29). Owner: "làm đi, cùng các việc bạn đề xuất sau khi đọc tài liệu của claude để giúp agenium
hỗ trợ người dùng và agent cách tốt nhất". From the comparison with `platform.claude.com/docs` (refusals and fallback,
handling stop reasons, prompting Opus 5.5). Keeps ADR-132: no automatic move to another model.

## Context

- A model's safety classifier can decline a request (`stop_reason: "refusal"`), before any output or mid-answer. The
  run stopped with "Run stopped (Stopped:refusal)": the user did not learn why, and the declined answer — incomplete by
  definition — stayed in the conversation, so a resumed run started from it and, as the docs warn, tended to be declined
  again. Anthropic's `stop_details` names the policy area (`cyber`, `bio`, `frontier_llm`, `reasoning_extraction`,
  `general_harms`) and explains it. Security work (the pentest roles, ADR-123) can trip `cyber` though benign.
- `stop_reason: "model_context_window_exceeded"` (the context window filled during the answer) fell into `Other`: the
  answer's tool calls, possibly cut, were run.
- Every run asked for at most 32,000 output tokens unless its agent set a limit. The docs advise far more for long
  agentic turns of large models (thinking counts toward it); the vendor's model list already says each model's limit.

## Decision

1. **A refusal**: the reason (`stop_details.category` and `explanation`, read from the stream's `message_delta`) is kept
   in the response (`ChatResponse::refusal`), logged with the run (`control` `refusal`), and written on the ticket: "…
   stopped: its model declined to go on (cyber: …). Its last answer was not kept. Reword the ticket where it touches
   that, or give it another model, then resume it." The declined answer is not added to the conversation, so a resumed
   run starts from the last good step. The run's outcome stays `Stopped:refusal`. The vendor's server-side fallback is
   not used: moving the work to another model stays the user's choice (ADR-132).
2. **A full context window** stops the answer as the output limit does: its tool calls are not run (they may be cut),
   and the next request's overflow is recovered as before (ADR-103).
3. **The output limit** of a run whose agent sets none is the model's own, as the vendor lists it, up to 64,000 (32,000
   when the vendor does not say).

## Verified

- Tests: `anthropic_refusal_details_and_a_full_context_window` (the reason read from the stream; the new stop reason);
  `a_refused_answer_is_dropped_and_its_reason_kept` (the outcome carries the reason, the declined answer is not in the
  conversation, which ends with the last tool results); `solo_agent_takes_a_request_to_a_closed_merged_ticket` (the
  scripted model's listed limit, 8,192, is the run's).

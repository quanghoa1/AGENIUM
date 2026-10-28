# Jev in Agenium: an optional judge that takes small decisions off your agents

> **Tóm tắt (tiếng Việt).** Jev là model "phán đoán" của TypeSafe: nó không viết code hay trò chuyện, mà trả lời các
> câu hỏi có cấu trúc ("báo cáo này có bằng chứng cho tiêu chí AC-2 không?", "skill nào hợp với ticket này?") kèm xác
> suất. Agenium dùng Jev ở các chỗ nhỏ để **agent chính đỡ tốn token**: bỏ qua việc trích memory ở ticket không có gì
> đáng nhớ, chọn trước skill phù hợp, cho Lead một ý kiến thứ hai khi review, đọc chất lượng một skill, trả báo cáo "xong"
> chưa kiểm chứng, lược log dài, và `find_code` tìm code theo hành vi. Jev **chạy khi bạn thêm key** (mọi chức năng bật
> sẵn; có thể nhập nhiều key đặt tên, dùng luân phiên), và **lỗi thì bỏ qua**: hết quota, sai key hay mất mạng thì Agenium
> vẫn chạy y như khi không có Jev. Đo thực tế: tiết kiệm
> khoảng 1–19 nghìn token mỗi ticket bình thường và 5–27 nghìn token mỗi lượt chạy có nhiều skill, và Jev đúng ngang một
> model rẻ mạnh trên bộ câu hỏi kiểm tra. Nên bật khi model chính đắt hoặc ngữ cảnh chật.

## What Jev is

[TypeSafe](https://typesafe.ai)'s **Jev** (System One) is a *decision model*, not a chat model. You give it some state
(a report, a list of acceptance criteria, short skill descriptions) and typed questions; it answers each with a
choice or a yes/no **and a probability**. It does not write text, call tools or hold a conversation, so it can never be
an agent's model. In Agenium it is an optional helper the orchestrator consults; agents never talk to it.

## Where Agenium uses it

Each use is a separate switch in **Settings → Judge** (`[judge]` in `config.toml`). Every one of them is
**fail-open**: if Jev is off, unsure, slow or failing, the step goes ahead exactly as it would without Jev.

| Use | When it runs | What Jev answers | What Agenium does with the answer |
|---|---|---|---|
| **Memory gate** (`memory_gate`) | After a ticket closes, before the agent's memory extraction | "Is anything in this episode worth remembering?" | On a confident **no**, the extraction call is skipped (it costs a full model call over the transcript). Never skipped when the episode had a reopen, a lesson signal or memories in play. |
| **Skill hint** (`skill_hint`) | Before a run, for an agent with two or more skills | "Which of these skills fits this ticket, if any?" (from one-line descriptions) | The pick is put in front of the agent, so it opens one `SKILL.md` instead of reading several to choose. |
| **Review assist** (`review_assist`) | Before the Lead reviews a report | "Is there evidence for each acceptance criterion?" | A first opinion per criterion (found / unclear / missing, with confidence) in the Lead's review context. The Lead still decides. |
| **Skill quality** (`skill_quality`) | When you import a skill, or an agent upgrades one | Four checks: one job, actionable steps, proof that demands evidence, no stale embedded code | Notes shown beside the always-on structural check. Advisory only. |
| **Done check** (`done_check`, ADR-126) | When a worker submits its report and the run changed files with no test/build/check passing after the last change | "Does the report read as done, would a check apply, does it claim checks passed?" (with the run's changed files and commands as facts) | On a confident unverified done, the report goes back to the worker once, in the same run, to run the check. Measured on 10 labelled reports with the real API: 9/9 as expected, ≈ 430 input tokens per ask (ADR-127). |
| **Output trimmer** (`trim_output`, ADR-127) | In a work run, when a `shell` or `grep` result is over 80 lines | "Does this chunk hold something the ticket needs?" (one Noul per middle chunk) | Chunks it is confident are not needed, and that rank low, are left out; head, tail and every key line (failure, error, test result) stay; the full output is saved and linked. On six real outputs it lost nothing needed but trimmed only one (82 of 257 lines), as most of a capped test log is evidence (ADR-127). |
| **find_code** (`find_code`, ADR-128) | A work tool for coding workers and the reviewing Lead | Which folders, files and units of source answer the agent's question about behaviour | The relevant files and the code that does it, verbatim with line numbers. On this repository: the answering file in the top 3 and its code excerpted for 10/10 questions, ≈ 3.4 s and $0.003 per search. On pinned Flask 3.1.0, every agent (four CLIs and DeepSeek over the API) spent 5–33 % fewer input tokens and fewer steps with it ([results](../quality/eval/results/find-code/README.md)). |
| **Shadow mode** (`shadow`, ADR-127) | With any use on | — | Every use asks and records what it would do, and changes nothing: measure a use before turning it on. |

What is sent to Jev: the text the question needs (a ticket's report and criteria, skill descriptions, a transcript
summary for the memory gate) and nothing else: no API keys, no files outside what the question names. Every call is
recorded in `<root>/judge-activity.jsonl` (use, ticket, outcome, time), which the Settings page shows.

## Why it is useful: the measured saving

Jev's value is **work the main agent no longer does**. Measured on real skills and a real model
(details and method: [jev-comparison.md](jev-comparison.md)):

| Use | Main-agent tokens without Jev | With Jev | Saved |
|---|---|---|---|
| Memory gate, routine ticket | the extraction call: ~1,150 tokens on a tiny ticket, up to ~19,000 on a full transcript | 0 (Jev answers on its own budget) | **~1k–19k per skipped ticket** |
| Skill hint, 12 real skills (~2,500 tokens each) | reads 3–5 skill files to choose (~7k–12k), up to ~29k | reads only the chosen file (~2.5k) | **~5k–27k per run** |
| Review assist | — | adds one small Jev call | none: it is a second opinion, not an offload |

**Does handing the decision to Jev cost correctness?** No. On two labelled judge sets (16 easy and 22 hard items,
including partial evidence, negations, unit traps and a prompt-injection) Jev and a strong cheap model
(`deepseek-v4.1-flash`) were both 100 % correct; the cheap model was slightly better calibrated. So Jev is not a
smarter judge — it is an equally correct one that runs on its own small budget.

**When to turn it on:** when the agents' model is expensive (the same token saving is worth ~20–50× more on a
frontier model than on a flash model), when agents have many skills, when most tickets are routine, or when context
is tight. With a cheap model and few skills the saving is real in tokens but small in money.

## Seen in a real team run (2026-10-27)

A mixed team — Claude Opus as Lead, Codex, Grok, Gemini (agy) and DeepSeek as workers — built a small web app from a
Vietnamese request in 4 tickets with Jev on (`quality/eval/results/team-mixed-20261027.md`):

- **Review assist** ran before each of the 4 reviews (0.9–4.6 s). On the first ticket it marked one criterion
  *unclear (68 %)* — the API's response shape, which the reviewer could only read, not run. The Lead's review named it
  as the one to confirm live and pointed to the test ticket; that ticket's tests later confirmed it (Jev: *evidence
  found, 97 %*). One call failed on a network error; the review went on without it (fail-open, as designed).
- **Memory gate** ran after each ticket and answered *worth remembering* (80–84 %) or *unsure* (60 %) every time, so
  no extraction was skipped: every ticket of a brand-new project taught something. The saving shows on routine
  tickets of a running project, not on a first build.
- **Skill hint** did not run: no agent in this team had two or more skills to choose from.

## Safety and control

- **Nothing is sent until you add a key.** Then every use is on (each one can be turned off in Settings → Judge).
- **Several named keys take turns** (one request each). A key **out of quota** is set aside and you are told which one,
  by name; it is not used again until you re-enable it after adding credit. A key that **fails** otherwise rests 30
  minutes, then an hour, and comes back by itself. With **every key out of quota** the judge is off until you re-enable
  one (ADR-128).
- **Fail-open.** Without a usable key — none added, out of quota, failing — every use returns nothing and Agenium
  works exactly as it does without the judge, only without its help.
- **Never in the learning path.** Agents' memories, lessons and skills never depend on Jev: every gate that decides
  what an agent learns works without it.
- **Key handling** is the same as for model providers: the key lives in the encrypted vault.

## Turning it on

Settings → **Judge**: add one or more TypeSafe keys, each under a name you will recognise in a notice, then **Test**.
Every use is on; turn off the ones you do not want. Or in `config.toml`:

```toml
[judge]
base_url = "https://api.typesafe.ai"
model = "jev-1.14.0"
# each key's value: agenium secret set judge/key/<name>  (reads it from stdin)
keys = [{ name = "main", secret_ref = "judge/key/main" }, { name = "backup", secret_ref = "judge/key/backup" }]
# every use defaults to on; set one to false to turn it off
memory_gate = true
skill_hint = true
review_assist = true
skill_quality = true
done_check = true
trim_output = true
find_code = true
```

Decisions behind this: [ADR-75](adr/ADR-75-typesafe-jev-judge.md) (the judge),
[ADR-81](adr/ADR-81-judge-two-modes.md) (two modes, fail-open), [ADR-82](adr/ADR-82-quick-skill-pick.md) (skill hint),
[ADR-85](adr/ADR-85-memory-gate.md) (memory gate), [ADR-90](adr/ADR-90-jev-skill-quality.md) (skill quality).

# ADR-128: `find_code`; named judge keys that take turns; every judge use on by default, fail-open

Status: Accepted (2026-10-28). Owner: "làm find_code trước"; "các chức năng liên quan tới jev mặc định enable hết … fail-open
tức khi jev hết quota hay bị lỗi thì agenium vẫn hoạt động bình thường"; "có thể nhập nhiều key jev … đặt tên key để khi
hết quota thì agenium thông báo key nào hết"; "xoay vòng dùng xen kẽ các key … nếu tất cả key jev bị hết quota thì tắt
jev … đợi người dùng enable lại … còn key lỗi thì thử lại sau 30'-1 tiếng". Research: `docs/reference/JEV-OPPORTUNITIES.md`
§5 (jevgrep).

## 1. `find_code`

A read-only work tool for coding workers and the reviewing Lead: a question about behaviour in, the relevant files (most
relevant first) and the units of source that implement, control or test it out, verbatim with line numbers. The judge
rates, one criterion over many items per request (the criterion written once in the state, each question pointing at
an item): folders (names and contents) unless the tree has 60 files or fewer, then files (path, outline, first 30 lines;
at most 100, of them at most 25 documents — code first), then the units of the six best files (80 units for the two best,
30 for the others). Kept: folders ≥ 0.5, files ≥ 0.5, units ≥ 0.6; a step that keeps nothing keeps its best few.

Learned while measuring on this repository (10 behavioural questions with the answering file,
`quality/eval/examples/find_code_eval.rs`, real API):

| Version | answering file in the top 3 | its code excerpted | s / search | Jev input tokens / search |
|---|---|---|---|---|
| first | 9/10 | 6/10 | 4.0 | 93 k |
| code before documents, 100 files, more units in the best files, no whole-file secret exclusion | 10/10 | 9/10 | 3.0 | 72 k |
| units with the doc comment above them, asked file by file, nested functions kept whole | **10/10** | **10/10** | 3.4 | 77 k (≈ $0.003) |

- Units of one file are asked in their own request: next to another file's units, `truncate` was rated 0.45 for "how is
  a long tool output cut down" (0.93 alone).
- A file holding a secret-looking string is not left out (the secret scanner's own source, whose tests hold sample keys,
  had vanished from a search about masking secrets): every text sent passes the redactor (`client::ask`, ADR-127).
  Credential files by name (`.env`, keys, certificates), hidden folders, build output, lock files and binaries are
  never read.
- Units: lines matching a declaration per language (Rust, Python, JS/TS, Go, the C family), the doc comment and
  attributes above included; a container (impl, class) gives way to its members (`App::run`), a function holding
  functions stays whole; other text in 80-line windows.

Whether it lowers a coding agent's own tokens (jevgrep reports 25.8 % lower total cost on ten SWE-bench tasks) is
measured by `quality/eval/examples/find_code_ab.rs` (the same questions to a real model with and without the tool); not
run yet — it needs a model key.

## 2. Named judge keys that take turns

- `[judge] keys = [{ name, secret_ref }]` (values in the vault, `judge/key/<name>`); the older single key is the key
  "default". Added, removed and re-enabled in Settings → Judge, by name.
- The keys in use **take turns**, one request each.
- A key **out of quota** (402, or a 429 naming quota/credit/billing) is set aside, the user is told which key by name,
  and it is **not used again until the user re-enables it** (after adding credit). The request goes on with the next key.
- A key that **fails** otherwise (rejected, or the service still failing after the client's retries) **rests 30
  minutes, then an hour** if it fails again, and comes back by itself; a success clears it. A bad request (400/422) is
  not the key's fault.
- With **every key out of quota the judge is off**: the user is told, and Agenium works on without it until a key is
  re-enabled.

## 3. Every use on by default, fail-open

`[judge]` defaults: every use on (review assist, skill hint, memory gate, skill quality, done check, output trimmer,
find_code; shadow off), endpoint `https://api.typesafe.ai`, model `jev-1.14.0`. The judge runs once a key is added (a
local endpoint needs none). Without a usable key — none added, out of quota, failing — every use returns nothing and
the work proceeds exactly as without the judge, only without its help.

## Consequences

- Tests: `tests/judge_keys.rs` (turns, quota set aside until re-enabled with the user told by name, every key out turns
  the judge off and nothing is retried, a failing key rests 30 then 60 minutes and comes back), `tests/find_code.rs`
  (folder → file → unit search against a mock that reads the requests; the key in a file is masked on its way out;
  without the judge the tool says to use grep), unit tests of outline, batching, rendering, files.
- An existing configuration keeps its single key as "default"; its uses that were off stay off (only missing fields
  take the new defaults).

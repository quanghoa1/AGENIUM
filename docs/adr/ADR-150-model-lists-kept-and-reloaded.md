# ADR-150: Model lists are kept, reloaded by the user, and on their own once a week

- **Status:** Accepted
- **Date:** 2026-10-31 (Asia/Saigon)
- **Asked by the owner:** "nên làm thêm 1 nút reload model … hiện thông tin lần cuối reload … khi lần đầu cài đặt, thì
  agenium sẽ tự quét các cli và load model lần đầu, còn từ lần sau thì người dùng tự load, sau 7d thì agenium sẽ tự chạy
  reload 1 lần".

## Context

The agent CLIs were found again (each one started for its version and login) and their models listed again (codex's
`model/list`, `agy models`, …) every five minutes the app was open, the usage refresh included; an API provider listed
its models each time a list was shown. Nothing said how old a list was, and there was no way to ask for a fresh one
but the CLIs' "Find again".

## Decision

1. **Kept in the root**: `model-cache.json` holds the agent CLIs found and the models of each source, `cli:<program>`
   and `api:<provider id>`, each with when it was last listed successfully, its last failure and when it was tried
   (`orchestrator::model_cache`). The app reads from it.
2. **Listed on its own** only when a source is new or its list is a week old (`MAX_AGE_DAYS = 7`), and not again
   within a day of a failure (`RETRY_AFTER_HOURS = 24`); a failure keeps the last good list. On a new install that is
   the agent CLIs (signed in through their own logins): no API provider has a key yet. An API provider's models are
   listed when it is saved (its key just entered: a wrong key or address shows at once), and listed again after any
   change to it. The desktop app lists what is due in the background once the root is open.
3. **Reloaded by the user**: a button on each API provider and each usable CLI (Providers page) and in the model picker
   of the agent and team forms; "Find again" finds the CLIs and lists every CLI's models. Each source shows when its
   models were listed ("models listed 2h ago") and whether the last reload failed.

## Consequences

- Opening the app or the agent form starts no CLI unless something is due; the 15-minute usage refresh reads the kept
  CLIs.
- A CLI signed out since its last listing still shows as signed in until it is found again (its runs then fail with its
  own message); the button and "Find again" are one click.
- Evidence: `model_cache::tests::a_list_is_refreshed_once_a_week_and_a_failure_waits_a_day`; the desktop app on a fresh
  root (docs/VERIFICATION.md, 2026-10-31).

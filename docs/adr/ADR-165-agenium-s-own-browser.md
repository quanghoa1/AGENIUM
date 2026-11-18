# ADR-165: Agenium's own browser, used like a person's

Status: Accepted (2026-11-18). Owner: agents should use the browser smoothly, "like a normal browser user". They keep
working where the user signed in (for example, check mail in Gmail). Signing in, signing up and CAPTCHAs are the
user's to do; the agent does the rest. The owner wants no profile or identity management. Obscura was studied first
(`docs/reference/OBSCURA-STUDY.md`) and is not integrated, only learned from.

## Context

ADR-158's `browser` was made for checking the app being built:
- a headless Edge or Chrome with a fresh profile in each run, so nothing stays signed in;
- fenced to localhost and the network allowlist by a closed proxy;
- six actions on CSS selectors (open, click, type, press, eval, screenshot);
- no window, so the user could not take over a sign-in or a CAPTCHA.

Obscura, a Rust headless engine for agents, cannot fill these gaps:
- it has no window;
- WebSocket and Service Workers are fakes;
- its device fingerprint changes on every navigation;
- its screenshots are wrong on real pages;
- it does not build with the GNU toolchain.

Its agent-facing tool design (named controls with state, generation refs, more actions, a quiet wait) is worth
copying onto Chromium.

## Decision

1. **One browser of Agenium.** A run that reaches the network (ADR-163's `fetch_open`) gets a tab of Agenium's own
   browser (`agent/tools/src/browser/shared.rs`):
   - one Edge or Chrome on the profile `<root>/browser`, shared by all such runs, one tab each;
   - cookies and sign-ins are kept across runs and restarts, like a person's browser;
   - it is not the user's own browser profile, and it has no identity management.

   Any other run keeps ADR-158's private headless browser, fenced to the allowlist.
2. **Where it may go.** `open` reaches any public site, the machine itself, and hosts of the allowlist. Another private
   address (LAN, cloud metadata) is refused unless allowlisted by name (`address_not_allowed`).
3. **Headless while agents work, a window when the user acts.**
   - On a sign-in, sign-up, CAPTCHA or second factor, the agent calls `show`. The browser starts again with its
     window, on the same profile, with the run's page in front.
   - The tool then tells the agent to ask the user with `ask_question` (blocking) and to go on after the answer.
   - The agent never solves a CAPTCHA and never asks for a password.
   - The window stays until the user closes it; the next step after that runs headless again.
   - Other runs' tabs open again at their last address whenever the browser starts again.
4. **Lifetime.**
   - The browser is closed gracefully, so it writes its cookies, in three cases: after 5 minutes without a run's tab
     (unless its window is shown), before it starts again in the other mode, and when the desktop app exits
     (`RunEvent::Exit`).
   - A browser left running by an earlier Agenium on the profile is found through `DevToolsActivePort` and used as
     it is.
   - The browser's output goes to `browser.log` in the profile, not a pipe. A pipe reader (a blocking thread on
     Windows) held the runtime's shutdown for as long as the browser lived (found when a test hung).
   - On Linux the browser keeps its cookies with `--password-store=basic`, in every start.
     - Without it, the first cookie of a page waited forever for a GNOME keyring nobody could unlock (measured on
       Ubuntu 25.10 over SSH: `--dump-dom https://example.com` hung with the default store, and answered with
       `basic`).
     - It must be the same store headless and in the window, or a sign-in made in the window would not read headless.
     - The price is that Chrome encrypts the cookies with its fixed key, so the profile folder is made the user's
       alone (0700). On Windows the cookies stay encrypted for the user by the system (DPAPI).
5. **Used like a person, learned from obscura.**
   - **Each step lists the page's controls with a ref `<generation>-<n>`.** A ref from an older step is refused
     (`stale_ref`), so it can never reach another element.
   - **Each control is named as a person reads it** (label, ARIA name, text) and shown with its state (value,
     checked, selected, expanded, disabled, focused, href). A password field shows only `filled`, never its value.
   - **The list is capped at 150 controls**, with a count of the rest.
   - **New actions:** `look`, `select`, `scroll` (a wheel over an element or the page), `wait_for` (text or an element,
     up to 30 s), `back`, `upload` (a worktree file into a file input), and `show`.
   - **Input is real browser input.** `click` is the browser's own mouse input at the element's centre. `type` selects
     the field's text and inserts the new text as input, so the page sees what a person's typing makes.
   - **A step reads the page once it is quiet**: loaded, then no DOM change for 300 ms, at most 3 s. This replaces the
     fixed 400 ms wait.
   - **Downloads** go to `.agenium/downloads/`.

   What a page shows stays untrusted, like fetched content.

## Not done

- No fingerprint spoofing, no events faked as real, no CAPTCHA solving, no identity or profile management.
- No confirmation before sending or deleting in a signed-in site: the owner's rule is orchestration, not
  restriction. The safety rule that page content is data, never instructions, still applies.

## Consequences

- Sign in once in the window, and later runs find the session there, until the site signs the user out.
- Concurrent runs share cookies, as tabs of one browser do.
- `show` needs a screen. On Linux without `DISPLAY`/`WAYLAND_DISPLAY` it answers `no_screen`.
- Tests:
  - `agent/tools/tests/browser_shared.rs`: a local site through the shared browser checks a password never shown,
    select, a real click, `wait_for`, a stale ref refused, upload, a wheel scroll and back. It also checks that
    storage and cookies are kept for the next run's tab and after the browser closed and started again, and the
    open/refused addresses.
  - By hand (`--ignored`): Gmail's sign-in page reads with "Email or phone" and "Next" listed; `show` opens it in a
    window and the run goes on there. Passed on Windows 11 (Edge) on 2026-11-18.
  - Ubuntu 25.10 (Chrome for Testing 158, wrapped with `--no-sandbox` in the VM only):
    - the tests pass;
    - Gmail's sign-in page reads the same;
    - `show` answers `no_screen`, since the VM has no display.

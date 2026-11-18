# Obscura vs Agenium's browser (ADR-158): study of 2026-11-18

Repo: https://github.com/h4ckf0r0day/obscura at `f946de2` (Apache-2.0). Binary tried: `v0.2.7`
`obscura-x86_64-windows.zip` (rendering on, no stealth TLS). Owner's goal: agents browse like a person, keep a
logged-in session across runs (sign in to Gmail once, check mail later), work on heavy real sites, and hand a CAPTCHA
or 2FA to the user.

## Verdict: take ideas, do not integrate

1. **It cannot meet the goal.**
   - There is no window, so a CAPTCHA or 2FA cannot be handed to the user. Its live view is a polled JPEG with no input.
   - WebSocket is a fake: it reports `open` and `send()` drops the data (`bootstrap.js:18778-18813`).
   - Service Workers are a fake (`:8445`, `:19370`), WebGL is always null (`:17402`, so Turnstile breaks, issue #844),
     and IndexedDB lives in memory only.
   - YouTube shows only placeholder cards. The Google sign-in page renders with blank buttons, although its email field
     takes input.
2. **It changes its device fingerprint on every navigation**, with or without `--stealth` (`bootstrap.js:19702`). In
   one session the screen size went 1280x720 → 1366x768 → 1440x900 and the core count 12 → 16 → 6. A Google session
   would see a different device on each page.
3. **It is wrong for Agenium's main use.** Its screenshots are visibly wrong on real pages: missing text, and Chinese or
   Japanese text drawn as empty boxes. It blocks localhost by default and has no host allowlist.
4. **It cannot be built with Agenium's GNU toolchain.** rusty_v8 publishes no `x86_64-pc-windows-gnu` archive. It could
   only run as a separate program: an MSVC build of about 150 MB (two executables and runtime DLLs).
5. **Its strengths:** about 3 to 15 times less memory than Edge (wikipedia.org 33 MB vs 334 MB), and a well-shaped MCP
   tool set (38 tools, `obscura-mcp/src/lib.rs:345-772`). On heavy pages it is slower than Edge (a Wikipedia article
   5.3 s vs 2.5 s, github.com/explore 5.9 s vs 3.4 s).

**No red flags were found:**
- No telemetry; an idle `serve` opened only its local port.
- The one third-party call, `web_search` to keenable.ai, needs `OBSCURA_WEB_SEARCH`.
- The build scripts only compute a version or build the V8 snapshot.

Notes:
- The binaries are unsigned and ship without checksums.
- Cookies are kept in plaintext JSON.
- It pretends to be Chrome by default (`navigator.webdriver=false`, functions made to look native, simulated events
  flagged as real).
- It is aimed at scraping; its own docs say it does not pass Cloudflare challenges or CAPTCHAs.

## Gaps against the goal

| Need | Obscura | Agenium (ADR-158) | Way |
|---|---|---|---|
| Logged-in profile kept across runs | Partial: cookies and localStorage only, plaintext, fingerprint re-rolled | No: a fresh temp profile per run (`browser/mod.rs:69-73`) | A Chromium `--user-data-dir` per identity in Agenium's data folder (Chrome encrypts its cookies) |
| Heavy real sites | No | Yes (real Edge or Chrome) | Keep Chromium |
| CAPTCHA and 2FA to the user | No window | Headless only | Detect the challenge, stop, open the same profile in a visible window for the user, then go on; never solve it |
| Screenshots | Wrong on real pages | Pixel-accurate | Keep |
| Windows and Linux | MSVC-only prebuilt, ~150 MB | Uses the installed browser | Keep |
| Localhost | Blocked by default | Built in | Keep |

## Ideas worth taking (our own code, on Chromium CDP)

1. **A snapshot line per control:** role, name from its `<label>` or ARIA attributes, and state. Never show a password's
   value (`obscura-mcp/src/lib.rs:1463-1474`, `:2516`). Chromium gives this through `Accessibility.getFullAXTree`.
2. **Element refs with a generation** (`e{gen}-{n}`, `:1480-1515`): a ref from an old snapshot fails instead of hitting
   another element. On Chromium, use the `backendNodeId` plus a generation check.
3. **A size budget for snapshots:** controls first, no cut lines, and a count of what was left out (`:1055-1069`).
4. **More actions:** `wait_for` (an element or text), `select_option`, form detection and filling, find in page, back,
   reload, tabs, and the network and console log.
5. **Export and import of the logged-in state.**
6. **A settle wait that ends when the page goes quiet** (5 s cap, optional network idle of 500 ms), in place of
   `readyState` plus a fixed 400 ms (`browser/mod.rs:228-238`).
7. **Profile rules:** the folder is a secret (owner-only, never in git), one process at a time per profile.
8. **A cap on navigation chains** (10 by default).

**Not to take:** fingerprint spoofing, flagging simulated events as real, TLS impersonation, the tracker blocklist, the
custom renderer, the V8 runtime.

## Measured (obscura v0.2.7 vs Edge 157 `--headless=new`, process start and network included, peak memory)

| Page | Obscura | Edge | Notes |
|---|---|---|---|
| wikipedia.org | 0.99 s, 33 MB | 2.20 s, 334 MB | CJK labels drawn as boxes |
| Wikipedia, Rust article | 5.29 s, 199 MB | 2.53 s, 295 MB | Same text |
| github.com/explore | 5.85 s, 135 MB | 3.38 s, 551 MB | Header search and sign-in missing |
| youtube.com | 4.49 s, 221 MB | 4.26 s, 764 MB | Placeholder cards only |
| accounts.google.com | 2.46 s, 89 MB | 2.90 s, 466 MB | Email field filled and read back (not submitted); buttons blank |
| news.ycombinator.com | 2.06 s, 16 MB | 2.88 s, 243 MB | Fine |

`--storage-dir` was checked across two processes: a cookie and a localStorage value set in the first were read in the
second.

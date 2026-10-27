---
name: app-ui-testing
description: Use a desktop app the way a user would (a test instance you launch), check every function and judge the UI from screenshots.
tags: coding testing
---

## Job
Run the app as a user would, check that each function in scope works, and judge its interface from what you see.

## When to use
A ticket asks to test, review or evaluate an app's features or UI: a WebView2 or Chromium desktop app (Tauri,
Electron). Agenium itself is one.

## Steps
1. Launch your own test instance: `node <folder>/scripts/ui.mjs launch --exe <app executable> --port 9333`.
   For Agenium add `--agenium`: a throwaway root whose vault key lives in memory. Never drive the user's own
   window or any app you did not launch; the script refuses other ports.
2. Plan the walk before clicking: list the screens and functions in scope, and for each one what a user expects.
3. For each function: `goto` the screen, `shot` it, look at it with `view_image`, act (`click`, `type`, `select`,
   `press`), `shot` again and look again. Read `text` to check wording; note every console error the script prints.
4. Try the paths users hit: empty state, invalid input, a long text, the action done twice, going back. Check the
   interface language when the app has more than one.
5. Judge each screen against `references/ui-review.md`.
6. `node <folder>/scripts/ui.mjs stop` when done (it removes the throwaway data).

## Tools
`shell` runs `ui.mjs` (Node 22 or newer). `view_image` shows you each screenshot: judge only what you have seen.
Keep the screenshots under `deliverables/ui-review/` so the report can point to them.

## Proof
The report in the format of `references/ui-review.md`: every function in scope with its steps, what was expected,
what happened and the screenshot file; every finding with its severity, the screen and a concrete fix. A function
you could not reach is listed as not tested, with the reason. Nothing is called fine that was not seen in a
screenshot.

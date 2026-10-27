# ADR-121: A browser in the app to point at the exact element to change

Status: Accepted (2026-10-27; owner: "khi dev web … agenium có browser để dev dễ yêu cầu sửa element nào cho dễ sửa, thay
vì dev nói sửa khúc góc bên phải ở trên …; tham khảo browser tab của orca hoặc các ADE tương tự"; then "mở một web bên
ngoài cũng được … cũng như mở nhiều tab được, trường hợp làm vài dự án 1 lần với nhiều team"). Learned from Cursor's
Design Mode and Orca's browser tab (element select sent to the agent); our own code.

## Context

"Make the box at the top right bigger" leaves an agent guessing which element and which file. Orca and Cursor show the
running app inside the tool, let the user click the element, and give the agent what identifies it.

## Decision

1. **Browser view** (nav "Browser"): tabs, each a native webview placed over the page (`Window::add_child`, Tauri's
   `unstable` feature), with back, forward, reload and an address bar. Each tab belongs to a project, so several
   projects and teams can be worked on at once; switching tabs shows one webview and hides the others; leaving the view
   hides them all and coming back finds them open.
2. **Pick elements** (button or Ctrl+Shift+D, Esc to stop): an injected script highlights the element under the mouse
   with its tag and size; a click collects the CSS selector, tag, classes and selected attributes, text, HTML (≤1.5 KB),
   computed styles that matter, its box and the viewport, and the component with its source file when the page is a
   React, Vue or Svelte development build. Picks gather on the right, each with a note; one message goes to the tab's
   project Lead with the user's words first.
3. **A page outside the machine** (not localhost / 127.0.0.1 / *.local) is marked "reference site", and its picks go to
   the Lead as a reference to follow in the project ("Wanted: …"), not as an element of the project to change.
4. **Isolation.** A page in the browser cannot reach Agenium: no capability names these webviews and their origin is
   remote, so no command answers them. A pick leaves the page as a navigation to `http://agenium-pick.invalid/?p=…`,
   cancelled by the webview's navigation handler and handed to the main webview as an event (≤64 KB). The app drives a
   page only with fixed scripts (history, reload, picker on/off). Only http(s) opens, never the app's own origins; the
   app's navigation guard, which keeps the main window on Agenium's pages, exempts the browser's webviews only.
5. **Which tab is which project's app.** A local address is tied to a project once ("This is X's app", or choosing
   the project in the tab's selector): every tab on that origin then shows the project and sends to its Lead. A tab
   shows its project, "reference site" (outside the machine) or "no project" (a local port nobody claimed) — google.com,
   :3000 and :2000 open together read as reference, the project's app, and unclaimed.
6. **Regions.** While picking, dragging selects an area: the outermost elements lying in it (a group of elements), or,
   for a blank area, the element that holds it — for what a click cannot pick.
7. **Screenshots and marks.** Every pick carries a screenshot of its box with a margin (WebView2 `CapturePreview`,
   cropped to the box in device pixels; the highlight is hidden first). The user can draw on it (box, arrow, pen, four
   colours, undo); the native page hides while the drawing dialog is open. On send the pictures are saved to
   `<project>/.agenium/picks/<id>.png` (outside git), every run of the project may read that folder, and the message
   gives each path to open with `view_image`.
8. Not done: screenshots on Linux (WebKitGTK snapshot) and macOS; voice.

## Verified (2026-10-27, debug build, DeepSeek via OpenCode Go)

A project holding a small shop page served on :3001: the checkout button picked in the browser with the note "green
(#16a34a), bold"; the Lead's plan targeted `#checkout` by id and kept the shared `.btn` rule untouched; after the
ticket merged and the page reloaded, the button's computed style was `rgb(22, 163, 74)`, weight 700. Two tabs (the
local page and example.com) at once; a pick on example.com marked as a reference. The UI smoke test visits the view.

Second round (same day): tabs on :3000 (linked), :2000 (unclaimed) and an external site read as the project, "no
project" and "reference"; a click and two drags (a blank area, a two-element cluster) produced picks with screenshots
(147×79, 718×113, the cluster); a box drawn on the first replaced its picture; on send the three pictures were saved
under `.agenium/picks`, and a Claude Haiku Lead opened one with `view_image`. Found and fixed: cancelling
`pointerdown` stopped the mouse events the picker listened to (it now listens to pointer events).

## Consequences

- `apps/desktop/src/browser.rs` (`browser_open/bounds/visible/hide_all/close/close_all/url/action`),
  `browser_picker.js`; `nav_guard` exempts `preview-*`; Tauri `unstable` feature.
- `views/Browser.tsx`, `onBrowser`, i18n.
- Also from the same request: the ticket popup lost its "open as a page" button; a closed ticket offers "Ask for a
  follow-up" (a closed ticket is never reopened: more work is a new ticket, ADR-59) and a done project "Ask for more"
  (done is not closed: a new request starts the team again).

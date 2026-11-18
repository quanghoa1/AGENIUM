//! What the `browser` tool runs in the page (ADR-158, ADR-165): the summary a step returns, and how an element is
//! found by its ref or a selector.

use serde_json::json;

/// Elements listed in one summary; the rest are counted.
pub(super) const MAX_ELEMENTS: usize = 150;

/// What a step shows the model: where the page is, its text, and the controls it can act on. Each control gets a ref
/// `<generation>-<n>` (written on it as `data-agenium-ref`); the generation is new each summary, so a ref from an
/// older one is refused instead of reaching another element. A control is named as a person reads it (its label,
/// its ARIA name, its text), with its state; a password field's value is never shown, only whether it is filled.
pub(super) fn summary() -> String {
    format!(
        r#"(() => {{
  const g = (window.__ageniumGen = (window.__ageniumGen || 0) + 1);
  const t = (s) => (s || '').trim().replace(/\s+/g, ' ');
  const name = (e) => {{
    let n = t(e.getAttribute('aria-label'));
    const by = e.getAttribute('aria-labelledby');
    if (!n && by) n = t(by.split(/\s+/).map((i) => (document.getElementById(i) || {{}}).innerText || '').join(' '));
    if (!n && e.labels && e.labels.length) n = t([...e.labels].map((l) => l.innerText).join(' '));
    if (!n && !['INPUT', 'SELECT', 'TEXTAREA'].includes(e.tagName)) n = t(e.innerText);
    if (!n) n = t(e.placeholder || e.title || e.getAttribute('alt') || e.name || '');
    return n.slice(0, 80);
  }};
  const seen = (e) => {{ const r = e.getBoundingClientRect(); return r.width > 0 && r.height > 0 && getComputedStyle(e).visibility !== 'hidden'; }};
  const all = [...document.querySelectorAll('a[href],button,input:not([type=hidden]),select,textarea,summary,[role=button],[role=link],[role=tab],[role=menuitem],[role=option],[role=checkbox],[role=radio],[role=switch],[role=textbox],[role=combobox],[role=row],[contenteditable=true],[tabindex="0"]')].filter(seen);
  const els = all.slice(0, {MAX_ELEMENTS}).map((e, i) => {{
    const ref = g + '-' + (i + 1);
    e.setAttribute('data-agenium-ref', ref);
    const tag = e.tagName.toLowerCase();
    const type = tag === 'input' ? (e.type || 'text') : '';
    const kind = e.getAttribute('role') || (type ? 'input[' + type + ']' : tag);
    const st = [];
    if (type === 'password') {{ if (e.value) st.push('filled'); }}
    else if (type === 'checkbox' || type === 'radio') {{ if (e.checked) st.push('checked'); }}
    else if (tag === 'select') st.push('selected=' + JSON.stringify((e.selectedOptions[0] || {{}}).text || '') + ' options=' + JSON.stringify([...e.options].slice(0, 15).map((o) => t(o.text))));
    else if ((tag === 'input' || tag === 'textarea') && e.value) st.push('value=' + JSON.stringify(String(e.value).slice(0, 80)));
    if (e.getAttribute('aria-checked') === 'true') st.push('checked');
    if (e.disabled || e.getAttribute('aria-disabled') === 'true') st.push('disabled');
    const ex = e.getAttribute('aria-expanded'); if (ex) st.push('expanded=' + ex);
    if (e.getAttribute('aria-selected') === 'true') st.push('selected');
    if (document.activeElement === e) st.push('focused');
    if (tag === 'a') st.push('href=' + (e.getAttribute('href') || '').slice(0, 100));
    return '[' + ref + '] ' + kind + ' ' + JSON.stringify(name(e)) + (st.length ? ' ' + st.join(' ') : '');
  }});
  return {{ url: location.href, title: document.title, text: (document.body ? document.body.innerText : '').replace(/\n{{3,}}/g, '\n\n').slice(0, 8000), elements: els, more: Math.max(0, all.length - {MAX_ELEMENTS}) }};
}})()"#
    )
}

/// A script expression that is the element of `r` (a ref from the last summary) or `selector`, the string `'stale'`
/// for a ref of an older summary, or `null` when nothing matches.
pub(super) fn target(r: Option<&str>, selector: Option<&str>) -> String {
    match r {
        Some(r) => format!(
            "(() => {{ const r = {0}; if (String(window.__ageniumGen) !== r.split('-')[0]) return 'stale'; return document.querySelector('[data-agenium-ref=\"' + r + '\"]') || 'stale'; }})()",
            json!(r)
        ),
        None => format!("document.querySelector({})", json!(selector.unwrap_or(""))),
    }
}

/// (key, code, windows key code, text) of a key `press` knows.
pub(super) fn key(name: &str) -> Option<(&'static str, &'static str, u32, &'static str)> {
    Some(match name.trim().to_ascii_lowercase().as_str() {
        "enter" | "return" => ("Enter", "Enter", 13, "\r"),
        "tab" => ("Tab", "Tab", 9, ""),
        "escape" | "esc" => ("Escape", "Escape", 27, ""),
        "backspace" => ("Backspace", "Backspace", 8, ""),
        "delete" => ("Delete", "Delete", 46, ""),
        "space" | " " => (" ", "Space", 32, " "),
        "arrowup" | "up" => ("ArrowUp", "ArrowUp", 38, ""),
        "arrowdown" | "down" => ("ArrowDown", "ArrowDown", 40, ""),
        "arrowleft" | "left" => ("ArrowLeft", "ArrowLeft", 37, ""),
        "arrowright" | "right" => ("ArrowRight", "ArrowRight", 39, ""),
        "home" => ("Home", "Home", 36, ""),
        "end" => ("End", "End", 35, ""),
        "pageup" => ("PageUp", "PageUp", 33, ""),
        "pagedown" => ("PageDown", "PageDown", 34, ""),
        _ => return None,
    })
}

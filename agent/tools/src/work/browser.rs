//! `browser`: open the app being built in a headless browser of the run, act on it like a user, read it, and take
//! screenshots for the report (ADR-158).

use super::*;
use crate::browser::url_allowed;
use serde_json::json;

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrowserAction {
    Open,
    Click,
    Type,
    Press,
    Eval,
    Screenshot,
}

/// One step in the run's browser.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BrowserArgs {
    /// open: load `url`; click: click `selector`; type: set `selector`'s value to `text`; press: press the key `text`
    /// (Enter, Tab, Escape, ArrowDown...) in the focused element; eval: run `expression` in the page and return its
    /// value; screenshot: save the page (or `selector`) as `.agenium/shots/<name>.png`.
    pub action: BrowserAction,
    /// open: http(s) on this machine (localhost) or an allowlisted host, or a file:// of your worktree.
    #[serde(default)]
    pub url: Option<String>,
    /// A CSS selector, as `open` lists them.
    #[serde(default)]
    pub selector: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    /// eval: a JavaScript expression (a promise is awaited); its value must be JSON.
    #[serde(default)]
    pub expression: Option<String>,
    /// screenshot: the file name (no folder), e.g. "home" → .agenium/shots/home.png.
    #[serde(default)]
    pub name: Option<String>,
    /// screenshot: the whole page, not only the window.
    #[serde(default)]
    pub full_page: bool,
    /// open: the window size (default 1280x800).
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    /// open: render as the user's system in "light" or "dark" mode (prefers-color-scheme); the page's default otherwise.
    #[serde(default)]
    pub color_scheme: Option<String>,
}

pub struct BrowserTool;

/// What a step shows the model: where the page is, its text, what can be clicked or typed in, its errors.
const SUMMARY: &str = r#"(() => {
  const sel = (e) => {
    if (e.id) return '#' + CSS.escape(e.id);
    for (const a of ['data-testid', 'name', 'aria-label']) {
      const v = e.getAttribute(a);
      if (v) { const s = `${e.tagName.toLowerCase()}[${a}="${CSS.escape(v)}"]`; if (document.querySelectorAll(s).length === 1) return s; }
    }
    const path = [];
    for (let n = e; n && n.nodeType === 1 && n !== document.body; n = n.parentElement) {
      if (n.id) { path.unshift('#' + CSS.escape(n.id)); break; }
      let i = 1; for (let s = n.previousElementSibling; s; s = s.previousElementSibling) if (s.tagName === n.tagName) i++;
      path.unshift(`${n.tagName.toLowerCase()}:nth-of-type(${i})`);
    }
    return path.join(' > ');
  };
  const seen = (e) => { const r = e.getBoundingClientRect(); return r.width > 0 && r.height > 0 && getComputedStyle(e).visibility !== 'hidden'; };
  const els = [...document.querySelectorAll('a[href],button,input,select,textarea,[role=button],[role=link],[role=tab],[role=checkbox],[contenteditable=true]')]
    .filter(seen).slice(0, 80).map((e) => {
      const label = (e.innerText || e.value || e.placeholder || e.getAttribute('aria-label') || e.title || '').trim().replace(/\s+/g, ' ').slice(0, 60);
      const kind = e.tagName.toLowerCase() + (e.type && e.tagName === 'INPUT' ? `[${e.type}]` : '');
      return `${kind} ${JSON.stringify(label)} → ${sel(e)}`;
    });
  return { url: location.href, title: document.title, text: (document.body ? document.body.innerText : '').replace(/\n{3,}/g, '\n\n').slice(0, 6000), elements: els };
})()"#;

fn need<'a>(v: &'a Option<String>, what: &str, action: &str) -> Result<&'a str, ToolError> {
    v.as_deref()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| ToolError::new("invalid_arguments", format!("{action} needs `{what}`")))
}

/// (key, code, windows key code, text) of a key `press` knows.
fn key(name: &str) -> Option<(&'static str, &'static str, u32, &'static str)> {
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
        _ => return None,
    })
}

#[async_trait]
impl Tool for BrowserTool {
    fn name(&self) -> &'static str {
        "browser"
    }
    fn description(&self) -> &'static str {
        "A headless browser of your run, to check the app you build as a user sees it: open a page (served on localhost by a shell job, or a file of your worktree), click, type, press keys, read values with eval, and take screenshots to .agenium/shots/ (attach them to your report as artifacts of kind screenshot). Each step returns the page's text, its clickable and typable elements with selectors, and its console errors. Other sites need the network allowlist."
    }
    fn parameters(&self) -> serde_json::Value {
        args_schema::<BrowserArgs>()
    }
    async fn call(&self, ctx: &ToolCtx, a: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let a: BrowserArgs = args(a)?;
        // Checked before the browser starts: a refused URL costs no browser.
        let url = match a.action {
            BrowserAction::Open => Some(url_allowed(
                need(&a.url, "url", "open")?,
                &ctx.fetch_allow,
                &ctx.root,
            )?),
            _ => None,
        };
        let temp = std::env::temp_dir().join("agenium");
        let mut b = ctx.browser.get(&temp, &ctx.fetch_allow).await?;
        let mut out = String::new();
        match a.action {
            BrowserAction::Open => {
                let (w, h) = (
                    a.width.unwrap_or(1280).clamp(320, 3840),
                    a.height.unwrap_or(800).clamp(240, 2160),
                );
                b.call(
                    "Emulation.setDeviceMetricsOverride",
                    json!({ "width": w, "height": h, "deviceScaleFactor": 1, "mobile": false }),
                )
                .await?;
                let scheme = match a.color_scheme.as_deref().map(str::trim) {
                    None | Some("") => "",
                    Some(s @ ("light" | "dark")) => s,
                    Some(s) => {
                        return Err(ToolError::new(
                            "invalid_arguments",
                            format!("color_scheme is light or dark, not {s}"),
                        ))
                    }
                };
                b.call(
                    "Emulation.setEmulatedMedia",
                    json!({ "features": [{ "name": "prefers-color-scheme", "value": scheme }] }),
                )
                .await?;
                let r = b.call("Page.navigate", json!({ "url": url })).await?;
                if let Some(e) = r["errorText"].as_str() {
                    return Err(ToolError::new(
                        "page_error",
                        format!("{}: {e}", url.unwrap_or_default()),
                    ));
                }
                b.settle().await?;
            }
            BrowserAction::Click => {
                let s = need(&a.selector, "selector", "click")?;
                let r = b
                    .eval(&format!(
                        "(() => {{ const e = document.querySelector({0}); if (!e) return false; e.scrollIntoView({{block: 'center'}}); e.focus && e.focus(); e.click(); return true; }})()",
                        json!(s)
                    ))
                    .await?;
                if r != true {
                    return Err(ToolError::new(
                        "not_found",
                        format!("no element matches {s}"),
                    ));
                }
                b.settle().await?;
            }
            BrowserAction::Type => {
                let s = need(&a.selector, "selector", "type")?;
                // Through the value setter of the element's own type, so React and the like see the change.
                let r = b
                    .eval(&format!(
                        "(() => {{ const e = document.querySelector({0}); if (!e) return false; e.focus();
                          if (e.isContentEditable) {{ e.textContent = {1}; }} else {{
                            const p = Object.getPrototypeOf(e); const d = Object.getOwnPropertyDescriptor(p, 'value');
                            d && d.set ? d.set.call(e, {1}) : (e.value = {1}); }}
                          e.dispatchEvent(new Event('input', {{bubbles: true}})); e.dispatchEvent(new Event('change', {{bubbles: true}})); return true; }})()",
                        json!(s),
                        json!(a.text.clone().unwrap_or_default())
                    ))
                    .await?;
                if r != true {
                    return Err(ToolError::new(
                        "not_found",
                        format!("no element matches {s}"),
                    ));
                }
                b.settle().await?;
            }
            BrowserAction::Press => {
                let k = need(&a.text, "text (the key)", "press")?;
                let (key, code, vk, text) = key(k).ok_or_else(|| {
                    ToolError::new("invalid_arguments", format!("unknown key {k}"))
                })?;
                for t in ["keyDown", "keyUp"] {
                    let mut p =
                        json!({ "type": t, "key": key, "code": code, "windowsVirtualKeyCode": vk });
                    if t == "keyDown" && !text.is_empty() {
                        p["text"] = json!(text);
                    }
                    b.call("Input.dispatchKeyEvent", p).await?;
                }
                b.settle().await?;
            }
            BrowserAction::Eval => {
                let v = b.eval(need(&a.expression, "expression", "eval")?).await?;
                let s = serde_json::to_string_pretty(&v).unwrap_or_default();
                return Ok(ToolOutput::text(crate::text::truncate(&s, 20_000, 2_000).0));
            }
            BrowserAction::Screenshot => {
                use base64::Engine;
                let name = need(&a.name, "name", "screenshot")?;
                let file = format!("{}.png", name.trim().trim_end_matches(".png"));
                if file.contains(['/', '\\', ':']) || file.starts_with('.') {
                    return Err(ToolError::new(
                        "invalid_arguments",
                        "name is a file name, without a folder",
                    ));
                }
                let mut p = json!({ "format": "png", "captureBeyondViewport": a.full_page });
                if let Some(s) = a.selector.as_deref().filter(|s| !s.trim().is_empty()) {
                    let r = b
                        .eval(&format!(
                            "(() => {{ const e = document.querySelector({0}); if (!e) return null; e.scrollIntoView({{block: 'nearest'}}); const r = e.getBoundingClientRect(); return {{x: r.x + scrollX, y: r.y + scrollY, width: r.width, height: r.height}}; }})()",
                            json!(s)
                        ))
                        .await?;
                    if r.is_null() {
                        return Err(ToolError::new(
                            "not_found",
                            format!("no element matches {s}"),
                        ));
                    }
                    p["clip"] = json!({ "x": r["x"], "y": r["y"], "width": r["width"], "height": r["height"], "scale": 1 });
                    p["captureBeyondViewport"] = json!(true);
                } else if a.full_page {
                    let size = b.eval("({w: document.documentElement.scrollWidth, h: document.documentElement.scrollHeight})").await?;
                    p["clip"] = json!({ "x": 0, "y": 0, "width": size["w"], "height": size["h"], "scale": 1 });
                }
                let r = b.call("Page.captureScreenshot", p).await?;
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(r["data"].as_str().unwrap_or_default())
                    .map_err(|e| ToolError::new("browser", e.to_string()))?;
                if bytes.len() as u64 > MAX_IMAGE_BYTES {
                    return Err(ToolError::new(
                        "too_large",
                        format!("the screenshot is {} KB: take a part of the page (selector), not the whole", bytes.len() >> 10),
                    ));
                }
                // Always under .agenium/shots/ (outside git), whatever the role's write scope: it is evidence, not work.
                let rel = format!(".agenium/shots/{file}");
                let path = crate::paths::resolve(&ctx.root, &[], &rel, true)?;
                if let Some(d) = path.parent() {
                    std::fs::create_dir_all(d).map_err(|e| io_err(&rel, e))?;
                }
                std::fs::write(&path, &bytes).map_err(|e| io_err(&rel, e))?;
                let mut o = ToolOutput::text(format!(
                    "saved {rel} ({} KB); attach it to your report as {{kind: \"screenshot\", ref: \"{rel}\"}}. The image follows the tool results.",
                    (bytes.len() as u64).div_ceil(1024)
                ));
                o.images.push(crate::ImageData {
                    media_type: "image/png".into(),
                    data_b64: r["data"].as_str().unwrap_or_default().to_owned(),
                    label: rel,
                });
                return Ok(o);
            }
        }
        let v = b.eval(SUMMARY).await?;
        out.push_str(&format!(
            "{}\n{}\n\n",
            v["title"].as_str().unwrap_or(""),
            v["url"].as_str().unwrap_or("")
        ));
        out.push_str(v["text"].as_str().unwrap_or(""));
        let els: Vec<&str> = v["elements"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|e| e.as_str())
            .collect();
        if !els.is_empty() {
            out.push_str("\n\nElements (selector after →):\n");
            out.push_str(&els.join("\n"));
        }
        let errors = std::mem::take(&mut b.errors);
        if !errors.is_empty() {
            out.push_str("\n\nConsole errors:\n");
            out.push_str(&errors.join("\n"));
        }
        let mut o = ToolOutput::text(out);
        // The page is the app's, but what it shows may come from elsewhere: treat it like fetched content.
        o.untrusted = true;
        Ok(o)
    }
}

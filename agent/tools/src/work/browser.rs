//! `browser`: a browser of the run (ADR-158, ADR-165), used like a person uses one: open a page, look at it, click,
//! type, choose, scroll, wait, go back, upload a file, read values, take screenshots for the report. A run that reaches
//! the network uses Agenium's own browser, signed in where the user signed in; signing in, signing up, a CAPTCHA or a
//! second factor are the user's, in a window `show` opens for them.

use super::browser_js::{key, summary, target, MAX_ELEMENTS};
use super::*;
use crate::browser::{url_allowed, Browser};
use serde_json::{json, Value};

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrowserAction {
    Open,
    Look,
    Click,
    Type,
    Select,
    Press,
    Scroll,
    WaitFor,
    Back,
    Upload,
    Show,
    Eval,
    Screenshot,
}

/// One step in the run's browser.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BrowserArgs {
    /// open: load `url`; look: read the page again; click: click the element; type: replace the element's text with
    /// `text` (typed as keystrokes); select: choose the option whose label or value is `text`; press: press the key
    /// `text` (Enter, Tab, Escape, ArrowDown, PageDown...); scroll: scroll `text` ("down" or "up") over the element or
    /// the page; wait_for: wait until `text` shows on the page or the element appears; back: go back a page; upload:
    /// put the worktree file `path` into the file input; show: open the page in a window on the user's screen for them
    /// to sign in, sign up or pass a CAPTCHA (then ask them with ask_question); eval: run `expression` in the page and
    /// return its value; screenshot: save the page (or the element) as `.agenium/shots/<name>.png`.
    pub action: BrowserAction,
    /// open: an http(s) address or a file:// of your worktree.
    #[serde(default)]
    pub url: Option<String>,
    /// The element: a ref from the last step's list, e.g. "3-12".
    #[serde(default)]
    pub r#ref: Option<String>,
    /// The element by a CSS selector, when it has no ref.
    #[serde(default)]
    pub selector: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    /// upload: a file of your worktree.
    #[serde(default)]
    pub path: Option<String>,
    /// wait_for: how long to wait at most (default 10000, at most 30000).
    #[serde(default)]
    pub timeout_ms: Option<u64>,
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

fn need<'a>(v: &'a Option<String>, what: &str, action: &str) -> Result<&'a str, ToolError> {
    v.as_deref()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| ToolError::new("invalid_arguments", format!("{action} needs `{what}`")))
}

fn element(a: &BrowserArgs, action: &str) -> Result<String, ToolError> {
    let r = a.r#ref.as_deref().filter(|s| !s.trim().is_empty());
    let s = a.selector.as_deref().filter(|s| !s.trim().is_empty());
    if r.is_none() && s.is_none() {
        return Err(ToolError::new(
            "invalid_arguments",
            format!("{action} needs `ref` (from the last step's list) or `selector`"),
        ));
    }
    Ok(target(r.map(str::trim), s))
}

/// What a script that looked for the element returned, as an error when it found none.
fn found(v: &Value, a: &BrowserArgs) -> Result<(), ToolError> {
    let what = a
        .r#ref
        .as_deref()
        .or(a.selector.as_deref())
        .unwrap_or_default();
    if v == "stale" {
        return Err(ToolError::new(
            "stale_ref",
            format!(
                "{what} is from an older look at the page: use a ref from the last step's list"
            ),
        ));
    }
    if v.is_null() || v == false {
        return Err(ToolError::new(
            "not_found",
            format!("no element matches {what}"),
        ));
    }
    Ok(())
}

/// The middle of the element (scrolled into view first when `into_view`), in the window's coordinates.
async fn point(
    b: &mut Browser,
    el: &str,
    a: &BrowserArgs,
    into_view: bool,
) -> Result<(f64, f64), ToolError> {
    let v = b
        .eval(&format!(
            "(() => {{ const e = {el}; if (e === 'stale' || !e) return e; {} const r = e.getBoundingClientRect(); return {{x: r.x + r.width / 2, y: r.y + r.height / 2}}; }})()",
            if into_view { "e.scrollIntoView({block: 'center', inline: 'center'});" } else { "" }
        ))
        .await?;
    found(&v, a)?;
    Ok((
        v["x"].as_f64().unwrap_or(0.0),
        v["y"].as_f64().unwrap_or(0.0),
    ))
}

/// A click of the mouse, as the browser's own input: the page sees what a person's click makes.
async fn click_at(b: &mut Browser, x: f64, y: f64) -> Result<(), ToolError> {
    b.call(
        "Input.dispatchMouseEvent",
        json!({ "type": "mouseMoved", "x": x, "y": y }),
    )
    .await?;
    for t in ["mousePressed", "mouseReleased"] {
        b.call(
            "Input.dispatchMouseEvent",
            json!({ "type": t, "x": x, "y": y, "button": "left", "clickCount": 1 }),
        )
        .await?;
    }
    Ok(())
}

async fn press(b: &mut Browser, k: &str) -> Result<(), ToolError> {
    let (key, code, vk, text) =
        key(k).ok_or_else(|| ToolError::new("invalid_arguments", format!("unknown key {k}")))?;
    for t in ["keyDown", "keyUp"] {
        let mut p = json!({ "type": t, "key": key, "code": code, "windowsVirtualKeyCode": vk });
        if t == "keyDown" && !text.is_empty() {
            p["text"] = json!(text);
        }
        b.call("Input.dispatchKeyEvent", p).await?;
    }
    Ok(())
}

#[async_trait]
impl Tool for BrowserTool {
    fn name(&self) -> &'static str {
        "browser"
    }
    fn description(&self) -> &'static str {
        "A browser of your run, used as a person uses one: open a page (the app you build on localhost or a file of your worktree; any public site when your run reaches the network, else hosts of the network allowlist), then click, type, select, press keys, scroll, wait for something to show, go back, upload a file, read values with eval, and take screenshots to .agenium/shots/ (attach them to your report as artifacts of kind screenshot). Each step returns the page's text, its controls with a ref each (act on one by its ref from the last step), and its console errors. Downloads go to .agenium/downloads/. Where you browse the web, the browser keeps the user's sign-ins between runs. Signing in, signing up, a CAPTCHA or a second factor is the user's to do: use show to open the page in a window on their screen, ask them with ask_question (blocking) what to do there, and go on when they answer; never try to solve a CAPTCHA or ask for a password."
    }
    fn parameters(&self) -> serde_json::Value {
        args_schema::<BrowserArgs>()
    }
    async fn call(&self, ctx: &ToolCtx, a: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let a: BrowserArgs = args(a)?;
        let profile = ctx.browser_profile.as_deref();
        // Checked before the browser starts: a refused URL costs no browser.
        let url = match a.action {
            BrowserAction::Open => Some(url_allowed(
                need(&a.url, "url", "open")?,
                &ctx.fetch_allow,
                &ctx.root,
                profile.is_some(),
            )?),
            _ => None,
        };
        if a.action == BrowserAction::Show {
            let Some(p) = profile else {
                return Err(ToolError::new(
                    "show_unavailable",
                    "your run does not reach the network, so its browser is a private one the user cannot act in",
                ));
            };
            let at = ctx.browser.show(p).await?;
            return Ok(ToolOutput::text(format!(
                "{at} is open in a browser window on the user's screen. Now call ask_question (blocking=true) saying \
                 what they should do there (sign in, sign up, pass the CAPTCHA or the second factor) and that they \
                 answer when done; go on with the browser after their answer. Their sign-in stays for later runs."
            )));
        }
        let temp = std::env::temp_dir().join("agenium");
        let mut b = ctx.browser.get(&temp, &ctx.fetch_allow, profile).await?;
        match a.action {
            BrowserAction::Open => {
                let url = url.unwrap_or_default();
                if b.visible {
                    // The user's window keeps its own size.
                    b.call("Emulation.clearDeviceMetricsOverride", json!({}))
                        .await?;
                } else {
                    let (w, h) = (
                        a.width.unwrap_or(1280).clamp(320, 3840),
                        a.height.unwrap_or(800).clamp(240, 2160),
                    );
                    b.call(
                        "Emulation.setDeviceMetricsOverride",
                        json!({ "width": w, "height": h, "deviceScaleFactor": 1, "mobile": false }),
                    )
                    .await?;
                }
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
                let downloads = ctx.root.join(".agenium").join("downloads");
                if std::fs::create_dir_all(&downloads).is_ok() {
                    let _ = b
                        .call(
                            "Page.setDownloadBehavior",
                            json!({ "behavior": "allow", "downloadPath": downloads.display().to_string() }),
                        )
                        .await;
                }
                let r = b.call("Page.navigate", json!({ "url": url })).await?;
                if let Some(e) = r["errorText"].as_str() {
                    return Err(ToolError::new("page_error", format!("{url}: {e}")));
                }
                b.url = url;
                b.settle().await?;
            }
            BrowserAction::Look => {}
            BrowserAction::Click => {
                let el = element(&a, "click")?;
                let (x, y) = point(&mut b, &el, &a, true).await?;
                click_at(&mut b, x, y).await?;
                b.settle().await?;
            }
            BrowserAction::Type => {
                let el = element(&a, "type")?;
                let text = a.text.clone().unwrap_or_default();
                // Focused with its old text selected, so the keystrokes replace it.
                let v = b
                    .eval(&format!(
                        "(() => {{ const e = {el}; if (e === 'stale' || !e) return e; e.scrollIntoView({{block: 'center'}}); e.focus();
                          if (e.isContentEditable) {{ const r = document.createRange(); r.selectNodeContents(e); const s = getSelection(); s.removeAllRanges(); s.addRange(r); }}
                          else {{ try {{ e.select(); }} catch (_) {{}} }} return true; }})()"
                    ))
                    .await?;
                found(&v, &a)?;
                if text.is_empty() {
                    press(&mut b, "Backspace").await?;
                } else {
                    b.call("Input.insertText", json!({ "text": text })).await?;
                }
                b.settle().await?;
            }
            BrowserAction::Select => {
                let el = element(&a, "select")?;
                let want = need(&a.text, "text (the option)", "select")?;
                let v = b
                    .eval(&format!(
                        "(() => {{ const e = {el}; if (e === 'stale' || !e) return e; const w = {0}; const t = (s) => (s || '').trim().toLowerCase();
                          const o = [...(e.options || [])].find((o) => o.value === w || t(o.text) === t(w));
                          if (!o) return 'no option ' + w + '; options: ' + [...(e.options || [])].map((o) => o.text.trim()).join(', ');
                          e.focus(); e.value = o.value; e.dispatchEvent(new Event('input', {{bubbles: true}})); e.dispatchEvent(new Event('change', {{bubbles: true}})); return true; }})()",
                        json!(want)
                    ))
                    .await?;
                if let Some(m) = v.as_str().filter(|m| m.starts_with("no option")) {
                    return Err(ToolError::new("not_found", m.to_owned()));
                }
                found(&v, &a)?;
                b.settle().await?;
            }
            BrowserAction::Press => {
                press(&mut b, need(&a.text, "text (the key)", "press")?).await?;
                b.settle().await?;
            }
            BrowserAction::Scroll => {
                let up = matches!(a.text.as_deref().map(str::trim), Some("up"));
                // Over the element (a list inside the page scrolls on its own), else over the middle of the window.
                let (x, y) = if a.r#ref.is_some() || a.selector.is_some() {
                    let el = element(&a, "scroll")?;
                    point(&mut b, &el, &a, false).await?
                } else {
                    let v = b.eval("({x: innerWidth / 2, y: innerHeight / 2})").await?;
                    (
                        v["x"].as_f64().unwrap_or(640.0),
                        v["y"].as_f64().unwrap_or(400.0),
                    )
                };
                let h = b.eval("innerHeight").await?.as_f64().unwrap_or(800.0);
                let dy = if up { -h * 0.8 } else { h * 0.8 };
                b.call(
                    "Input.dispatchMouseEvent",
                    json!({ "type": "mouseWheel", "x": x, "y": y, "deltaX": 0, "deltaY": dy }),
                )
                .await?;
                b.settle().await?;
            }
            BrowserAction::WaitFor => {
                let cond = match a.text.as_deref().filter(|t| !t.trim().is_empty()) {
                    Some(t) => format!(
                        "!!document.body && document.body.innerText.includes({})",
                        json!(t)
                    ),
                    None => format!(
                        "(() => {{ const e = {}; return !!e && e !== 'stale'; }})()",
                        element(&a, "wait_for").map_err(|_| ToolError::new(
                            "invalid_arguments",
                            "wait_for needs `text`, `ref` or `selector`"
                        ))?
                    ),
                };
                let limit =
                    Duration::from_millis(a.timeout_ms.unwrap_or(10_000).clamp(100, 30_000));
                let deadline = tokio::time::Instant::now() + limit;
                loop {
                    if b.eval(&cond).await.is_ok_and(|v| v == true) {
                        break;
                    }
                    if tokio::time::Instant::now() >= deadline {
                        return Err(ToolError::new(
                            "wait_timeout",
                            format!("not there after {} ms", limit.as_millis()),
                        ));
                    }
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
                b.settle().await?;
            }
            BrowserAction::Back => {
                b.eval("history.back(), true").await?;
                tokio::time::sleep(Duration::from_millis(300)).await;
                b.settle().await?;
            }
            BrowserAction::Upload => {
                let rel = need(&a.path, "path (a file of your worktree)", "upload")?;
                let file = ctx.resolve(rel, false)?;
                if !file.is_file() {
                    return Err(ToolError::new("not_found", format!("{rel} is not a file")));
                }
                let el = element(&a, "upload")?;
                let v = b
                    .eval(&format!("(() => {{ const e = {el}; if (e === 'stale' || !e) return e; e.setAttribute('data-agenium-upload', '1'); return true; }})()"))
                    .await?;
                found(&v, &a)?;
                let doc = b.call("DOM.getDocument", json!({ "depth": 0 })).await?;
                let node = b
                    .call(
                        "DOM.querySelector",
                        json!({ "nodeId": doc["root"]["nodeId"], "selector": "[data-agenium-upload]" }),
                    )
                    .await?;
                b.call(
                    "DOM.setFileInputFiles",
                    json!({ "files": [file.display().to_string()], "nodeId": node["nodeId"] }),
                )
                .await?;
                b.eval("document.querySelectorAll('[data-agenium-upload]').forEach((e) => e.removeAttribute('data-agenium-upload')), true")
                    .await?;
                b.settle().await?;
            }
            BrowserAction::Show => unreachable!("answered above"),
            BrowserAction::Eval => {
                let v = b.eval(need(&a.expression, "expression", "eval")?).await?;
                let s = serde_json::to_string_pretty(&v).unwrap_or_default();
                return Ok(ToolOutput::text(crate::text::truncate(&s, 20_000, 2_000).0));
            }
            BrowserAction::Screenshot => return screenshot(ctx, &mut b, &a).await,
        }
        let v = b.eval(&summary()).await?;
        if let Some(u) = v["url"].as_str() {
            b.url = u.to_owned();
        }
        let mut out = format!(
            "{}\n{}\n\n{}",
            v["title"].as_str().unwrap_or(""),
            v["url"].as_str().unwrap_or(""),
            v["text"].as_str().unwrap_or("")
        );
        let els: Vec<&str> = v["elements"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|e| e.as_str())
            .collect();
        if !els.is_empty() {
            out.push_str("\n\nControls (act on one by its ref):\n");
            out.push_str(&els.join("\n"));
        }
        if let Some(n) = v["more"].as_u64().filter(|n| *n > 0) {
            out.push_str(&format!(
                "\n[{n} more beyond the first {MAX_ELEMENTS}: scroll, or find one with a selector]"
            ));
        }
        let errors = std::mem::take(&mut b.errors);
        if !errors.is_empty() {
            out.push_str("\n\nConsole errors:\n");
            out.push_str(&errors.join("\n"));
        }
        let mut o = ToolOutput::text(out);
        // What a page shows may come from anyone (a mail, a post): treat it like fetched content.
        o.untrusted = true;
        Ok(o)
    }
}

async fn screenshot(
    ctx: &ToolCtx,
    b: &mut Browser,
    a: &BrowserArgs,
) -> Result<ToolOutput, ToolError> {
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
    if a.r#ref.is_some() || a.selector.is_some() {
        let el = element(a, "screenshot")?;
        let r = b
            .eval(&format!(
                "(() => {{ const e = {el}; if (e === 'stale' || !e) return e; e.scrollIntoView({{block: 'nearest'}}); const r = e.getBoundingClientRect(); return {{x: r.x + scrollX, y: r.y + scrollY, width: r.width, height: r.height}}; }})()"
            ))
            .await?;
        found(&r, a)?;
        p["clip"] = json!({ "x": r["x"], "y": r["y"], "width": r["width"], "height": r["height"], "scale": 1 });
        p["captureBeyondViewport"] = json!(true);
    } else if a.full_page {
        let size = b
            .eval("({w: document.documentElement.scrollWidth, h: document.documentElement.scrollHeight})")
            .await?;
        p["clip"] = json!({ "x": 0, "y": 0, "width": size["w"], "height": size["h"], "scale": 1 });
    }
    let r = b.call("Page.captureScreenshot", p).await?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(r["data"].as_str().unwrap_or_default())
        .map_err(|e| ToolError::new("browser", e.to_string()))?;
    if bytes.len() as u64 > MAX_IMAGE_BYTES {
        return Err(ToolError::new(
            "too_large",
            format!(
                "the screenshot is {} KB: take a part of the page (an element), not the whole",
                bytes.len() >> 10
            ),
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
    Ok(o)
}

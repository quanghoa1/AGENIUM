//! The preview browser (ADR-121): webviews inside the main window, one per tab, that show the app the agents build
//! (`http://localhost:3000` …) or any web page the user wants to point at, with an element picker. A page gets no
//! access to Agenium: no capability names these webviews and their origin is remote, so it cannot call a command. A
//! picked element leaves the page as a navigation to `http://agenium-pick.invalid/?p=<json>`, which is cancelled here
//! and handed to the main webview as an event. The app drives a page only with the fixed scripts below (history,
//! reload, picker on/off), never arbitrary code.

use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Url, WebviewBuilder, WebviewUrl,
};

/// Webview labels of the tabs: `preview-<tab>`.
pub const PREFIX: &str = "preview-";
const PICK_HOST: &str = "agenium-pick.invalid";
const PICKER: &str = include_str!("browser_picker.js");
/// A pick larger than this is dropped (the page builds it, so it is bounded here too).
const MAX_PICK: usize = 64 * 1024;

type R<T> = Result<T, String>;

/// http(s) pages other than the app's own origins (a page there would run Agenium's interface again).
fn allowed(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
        && !matches!(
            url.host_str(),
            Some("tauri.localhost" | "ipc.localhost" | "asset.localhost")
        )
}

fn parse(url: &str) -> R<Url> {
    let u: Url = url.trim().parse().map_err(|e| format!("not a URL: {e}"))?;
    if !allowed(&u) {
        return Err("only http and https pages open in the browser".into());
    }
    Ok(u)
}

/// The webview label of a tab id the interface made (letters and digits).
fn label(tab: &str) -> R<String> {
    if tab.is_empty() || tab.len() > 16 || !tab.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(format!("bad tab id {tab:?}"));
    }
    Ok(format!("{PREFIX}{tab}"))
}

fn place(wv: &tauri::Webview, x: f64, y: f64, w: f64, h: f64) -> R<()> {
    wv.set_position(LogicalPosition::new(x, y))
        .map_err(|e| e.to_string())?;
    wv.set_size(LogicalSize::new(w.max(1.0), h.max(1.0)))
        .map_err(|e| e.to_string())
}

/// Opens `url` in a tab at the given place of the main window (logical pixels), creating its webview the first time.
/// Async like every command here: creating or driving a webview from a synchronous command deadlocks on Windows.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn browser_open(
    app: AppHandle,
    tab: String,
    url: String,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
) -> R<()> {
    let name = label(&tab)?;
    let url = parse(&url)?;
    if let Some(wv) = app.get_webview(&name) {
        wv.navigate(url).map_err(|e| e.to_string())?;
        place(&wv, x, y, w, h)?;
        return wv.show().map_err(|e| e.to_string());
    }
    let window = app
        .get_window("main")
        .ok_or("the main window is not open")?;
    let handle = app.clone();
    let id = tab.clone();
    let builder = WebviewBuilder::new(&name, WebviewUrl::External(url))
        .initialization_script(PICKER)
        .on_navigation(move |u| {
            if u.host_str() == Some(PICK_HOST) {
                for (k, v) in u.query_pairs() {
                    match k.as_ref() {
                        "p" if v.len() <= MAX_PICK => {
                            let _ = handle.emit_to(
                                "main",
                                "agenium://browser-picked",
                                serde_json::json!({ "tab": id, "pick": v }),
                            );
                        }
                        "off" => {
                            let _ = handle.emit_to(
                                "main",
                                "agenium://browser-pick-off",
                                serde_json::json!({ "tab": id }),
                            );
                        }
                        _ => {}
                    }
                }
                return false;
            }
            allowed(u)
        })
        .on_page_load(|wv, payload| {
            if matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
                let tab = wv.label().trim_start_matches(PREFIX).to_owned();
                let _ = wv.emit_to(
                    "main",
                    "agenium://browser-url",
                    serde_json::json!({ "tab": tab, "url": payload.url().to_string() }),
                );
            }
        });
    window
        .add_child(
            builder,
            LogicalPosition::new(x, y),
            LogicalSize::new(w.max(1.0), h.max(1.0)),
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Moves a tab with its place in the interface (the window or the panel was resized).
#[tauri::command]
pub async fn browser_bounds(app: AppHandle, tab: String, x: f64, y: f64, w: f64, h: f64) -> R<()> {
    match app.get_webview(&label(&tab)?) {
        Some(wv) => place(&wv, x, y, w, h),
        None => Ok(()),
    }
}

/// Shows or hides a tab (a native view: it would cover any other page of the interface).
#[tauri::command]
pub async fn browser_visible(app: AppHandle, tab: String, show: bool) -> R<()> {
    match app.get_webview(&label(&tab)?) {
        Some(wv) if show => wv.show().map_err(|e| e.to_string()),
        Some(wv) => wv.hide().map_err(|e| e.to_string()),
        None => Ok(()),
    }
}

/// Hides every tab (the interface left the browser).
#[tauri::command]
pub async fn browser_hide_all(app: AppHandle) -> R<()> {
    for (name, wv) in app.webviews() {
        if name.starts_with(PREFIX) {
            wv.hide().map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Closes every tab: the interface started afresh (a reload) and knows none of them.
#[tauri::command]
pub async fn browser_close_all(app: AppHandle) -> R<()> {
    for (name, wv) in app.webviews() {
        if name.starts_with(PREFIX) {
            wv.close().map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Closes a tab and its page.
#[tauri::command]
pub async fn browser_close(app: AppHandle, tab: String) -> R<()> {
    match app.get_webview(&label(&tab)?) {
        Some(wv) => wv.close().map_err(|e| e.to_string()),
        None => Ok(()),
    }
}

/// The page a tab shows, if it is open (the interface asks when it comes back to the browser).
#[tauri::command]
pub async fn browser_url(app: AppHandle, tab: String) -> R<Option<String>> {
    Ok(app
        .get_webview(&label(&tab)?)
        .and_then(|wv| wv.url().ok())
        .map(|u| u.to_string()))
}

/// History, reload and the picker: a fixed script per action.
#[tauri::command]
pub async fn browser_action(app: AppHandle, tab: String, action: String) -> R<()> {
    let js = match action.as_str() {
        "back" => "history.back()",
        "forward" => "history.forward()",
        "reload" => "location.reload()",
        "pick-on" => "window.__ageniumPicker && window.__ageniumPicker.set(true)",
        "pick-off" => "window.__ageniumPicker && window.__ageniumPicker.set(false)",
        other => return Err(format!("unknown browser action {other:?}")),
    };
    match app.get_webview(&label(&tab)?) {
        Some(wv) => wv.eval(js).map_err(|e| e.to_string()),
        None => Ok(()),
    }
}

/// The PNG of what a tab shows (device pixels), from the platform webview.
#[cfg(windows)]
async fn capture_png(wv: &tauri::Webview) -> R<Vec<u8>> {
    use webview2_com::CapturePreviewCompletedHandler;
    use webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG;
    use windows::Win32::System::Com::StructuredStorage::CreateStreamOnHGlobal;
    use windows::Win32::System::Com::{IStream, STREAM_SEEK_SET};

    fn read_all(stream: &IStream) -> R<Vec<u8>> {
        let mut out = Vec::new();
        let mut buf = vec![0u8; 64 * 1024];
        unsafe {
            stream
                .Seek(0, STREAM_SEEK_SET, None)
                .map_err(|e| e.to_string())?;
            loop {
                let mut read = 0u32;
                let hr = stream.Read(buf.as_mut_ptr().cast(), buf.len() as u32, Some(&mut read));
                if hr.is_err() {
                    return Err(format!("reading the capture: {hr:?}"));
                }
                if read == 0 {
                    break;
                }
                out.extend_from_slice(&buf[..read as usize]);
            }
        }
        Ok(out)
    }

    let (tx, rx) = tokio::sync::oneshot::channel::<R<Vec<u8>>>();
    wv.with_webview(move |pw| unsafe {
        let start = || -> windows::core::Result<()> {
            let core = pw.controller().CoreWebView2()?;
            let stream: IStream = CreateStreamOnHGlobal(Default::default(), true)?;
            let kept = stream.clone();
            let mut tx = Some(tx);
            let handler = CapturePreviewCompletedHandler::create(Box::new(move |r| {
                let out = r.map_err(|e| e.to_string()).and_then(|_| read_all(&kept));
                if let Some(t) = tx.take() {
                    let _ = t.send(out);
                }
                Ok(())
            }));
            core.CapturePreview(
                COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
                &stream,
                &handler,
            )
        };
        if let Err(e) = start() {
            tracing::warn!(error = %e, "the browser capture did not start");
        }
    })
    .map_err(|e| e.to_string())?;
    rx.await
        .map_err(|_| "the capture did not complete".to_string())?
}

#[cfg(not(windows))]
async fn capture_png(_wv: &tauri::Webview) -> R<Vec<u8>> {
    Err("screenshots of the browser are not available on this system yet".into())
}

/// `png_bytes` cut to the box (device pixels) with a margin, as PNG.
fn crop_png(png_bytes: &[u8], x: f64, y: f64, w: f64, h: f64, margin: f64) -> R<Vec<u8>> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(png_bytes));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = dec.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0; reader.output_buffer_size().ok_or("capture too large")?];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let bpp = match info.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Grayscale => 1,
        png::ColorType::Indexed => return Err("unexpected indexed capture".into()),
    };
    let (iw, ih) = (info.width as i64, info.height as i64);
    let x0 = ((x - margin).floor() as i64).clamp(0, iw.max(1) - 1);
    let y0 = ((y - margin).floor() as i64).clamp(0, ih.max(1) - 1);
    let x1 = ((x + w + margin).ceil() as i64).clamp(x0 + 1, iw);
    let y1 = ((y + h + margin).ceil() as i64).clamp(y0 + 1, ih);
    let (cw, ch) = ((x1 - x0) as usize, (y1 - y0) as usize);
    let mut out_px = Vec::with_capacity(cw * ch * bpp);
    for row in y0 as usize..y1 as usize {
        let start = row * info.line_size + x0 as usize * bpp;
        out_px.extend_from_slice(&buf[start..start + cw * bpp]);
    }
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, cw as u32, ch as u32);
        enc.set_color(info.color_type);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().map_err(|e| e.to_string())?;
        w.write_image_data(&out_px).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

/// A screenshot of part of what a tab shows: the box in CSS pixels of the page (`dpr` = its device pixel ratio),
/// with a margin, as a PNG in base64 (the interface shows it and lets the user draw on it before it is sent).
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn browser_capture(
    app: AppHandle,
    tab: String,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    dpr: f64,
) -> R<String> {
    use base64::Engine;
    let wv = app
        .get_webview(&label(&tab)?)
        .ok_or("the tab is not open")?;
    let full = capture_png(&wv).await?;
    let d = if dpr.is_finite() && dpr > 0.0 {
        dpr
    } else {
        1.0
    };
    let png = crop_png(&full, x * d, y * d, w * d, h * d, 24.0 * d)?;
    Ok(base64::engine::general_purpose::STANDARD.encode(png))
}

/// Saves a picked element's picture (PNG, base64; annotated or not) where the project's agents can read it:
/// `<project>/.agenium/picks/<id>.png` (outside git, and readable by every run of the project). Returns its path.
#[tauri::command]
pub async fn browser_save_pick(
    d: tauri::State<'_, crate::state::Desktop>,
    project: String,
    png: String,
) -> R<String> {
    use base64::Engine;
    const MAX: usize = 12 * 1024 * 1024;
    if png.len() > MAX * 4 / 3 + 64 {
        return Err("the picture is too large".into());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(png.trim_start_matches("data:image/png;base64,"))
        .map_err(|e| format!("not base64: {e}"))?;
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("not a PNG".into());
    }
    let o = d.opened()?;
    let p = o
        .app
        .project(crate::api::pid(&project)?)
        .map_err(|e| e.to_string())?;
    let dir = std::path::Path::new(&p.path).join(".agenium").join("picks");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file = dir.join(format!(
        "{}.png",
        ulid::Ulid::new().to_string().to_lowercase()
    ));
    std::fs::write(&file, &bytes).map_err(|e| e.to_string())?;
    Ok(file.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_pages_outside_the_app_open() {
        assert!(parse("http://localhost:3000/cart").is_ok());
        assert!(parse("https://example.com").is_ok());
        for bad in [
            "file:///C:/Windows/win.ini",
            "http://tauri.localhost/",
            "tauri://localhost",
            "javascript:alert(1)",
            "not a url",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        assert!(label("t1").is_ok());
        // A 4x3 RGBA picture cut to its middle 2x1 with no margin.
        let mut src = Vec::new();
        {
            let mut e = png::Encoder::new(&mut src, 4, 3);
            e.set_color(png::ColorType::Rgba);
            e.set_depth(png::BitDepth::Eight);
            let px: Vec<u8> = (0..4 * 3).flat_map(|i| [i as u8, 0, 0, 255]).collect();
            e.write_header().unwrap().write_image_data(&px).unwrap();
        }
        let cut = crop_png(&src, 1.0, 1.0, 2.0, 1.0, 0.0).unwrap();
        let mut r = png::Decoder::new(std::io::Cursor::new(cut))
            .read_info()
            .unwrap();
        let mut b = vec![0; r.output_buffer_size().unwrap()];
        let i = r.next_frame(&mut b).unwrap();
        assert_eq!((i.width, i.height), (2, 1));
        assert_eq!((b[0], b[4]), (5, 6), "pixels 5 and 6 of the source");
        for bad in ["", "../main", "main\"", "a-b"] {
            assert!(label(bad).is_err(), "{bad}");
        }
    }
}

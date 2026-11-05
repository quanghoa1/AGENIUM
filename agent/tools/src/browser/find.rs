//! Which Chromium browser the run drives: `AGENIUM_BROWSER`, else Edge or Chrome where they install.

use std::path::PathBuf;

pub fn find_browser() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("AGENIUM_BROWSER").map(PathBuf::from) {
        return p.is_file().then_some(p);
    }
    candidates().into_iter().find(|p| p.is_file())
}

fn candidates() -> Vec<PathBuf> {
    let mut v = Vec::new();
    #[cfg(windows)]
    {
        for var in ["ProgramFiles(x86)", "ProgramFiles", "LOCALAPPDATA"] {
            if let Some(base) = std::env::var_os(var).map(PathBuf::from) {
                v.push(base.join(r"Microsoft\Edge\Application\msedge.exe"));
                v.push(base.join(r"Google\Chrome\Application\chrome.exe"));
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        v.push("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome".into());
        v.push("/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge".into());
        v.push("/Applications/Chromium.app/Contents/MacOS/Chromium".into());
    }
    #[cfg(target_os = "linux")]
    {
        for dir in std::env::var("PATH").unwrap_or_default().split(':') {
            for name in [
                "google-chrome",
                "google-chrome-stable",
                "chromium",
                "chromium-browser",
                "microsoft-edge",
            ] {
                v.push(PathBuf::from(dir).join(name));
            }
        }
    }
    v
}

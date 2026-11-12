//! The shell commands run in, decoding what they print, finding programs, and the clean environment.

use super::*;

/// Shell used for `shell` tool commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Shell {
    /// `bash -c` (Linux/macOS, or Git Bash on Windows).
    Bash(PathBuf),
    /// `cmd /C` on Windows when no bash is found.
    Cmd,
    /// PowerShell on Windows when commands run in the MXC container: Git Bash (MSYS runtime) cannot start there,
    /// it needs a global named-object directory the container denies (S2).
    PowerShell(PathBuf),
}

/// Prelude of sandboxed PowerShell commands: UTF-8 output, files read as UTF-8 (Windows PowerShell 5.1 reads them as
/// ANSI: "xin chào" came back "xin chÃ o"), no progress noise; the epilogue passes native exit codes through. Writing
/// keeps PowerShell's defaults: 5.1 would add a BOM to UTF-8.
pub(super) const PS_PRELUDE: &str = "$ProgressPreference='SilentlyContinue'; $OutputEncoding=[Console]::OutputEncoding=New-Object System.Text.UTF8Encoding $false; foreach ($c in 'Get-Content','Select-String','Import-Csv') { $PSDefaultParameterValues[\"${c}:Encoding\"]='UTF8' };\n";

pub(super) const PS_EPILOGUE: &str = "\n$__agenium_ok = $?; if ($LASTEXITCODE) { exit $LASTEXITCODE } elseif (-not $__agenium_ok) { exit 1 }";

impl Shell {
    pub fn detect() -> Shell {
        #[cfg(windows)]
        if mxc::available() {
            return Shell::powershell();
        }
        if cfg!(windows) {
            for cand in [
                r"C:\Program Files\Git\bin\bash.exe",
                r"C:\Program Files\Git\usr\bin\bash.exe",
            ] {
                if Path::new(cand).is_file() {
                    return Shell::Bash(cand.into());
                }
            }
            if let Some(p) = find_in_path("bash.exe") {
                // Skip the WSL launcher in System32: it would run in another OS.
                if !p
                    .to_string_lossy()
                    .to_ascii_lowercase()
                    .contains("system32")
                {
                    return Shell::Bash(p);
                }
            }
            Shell::Cmd
        } else {
            Shell::Bash(find_in_path("bash").unwrap_or_else(|| "/bin/sh".into()))
        }
    }

    /// PowerShell 7 (`pwsh`) when installed, otherwise Windows PowerShell 5.1.
    pub fn powershell() -> Shell {
        if let Some(p) = find_program("pwsh") {
            return Shell::PowerShell(p);
        }
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        Shell::PowerShell(
            PathBuf::from(root)
                .join("System32")
                .join("WindowsPowerShell")
                .join("v1.0")
                .join("powershell.exe"),
        )
    }

    pub fn describe(&self) -> String {
        match self {
            Shell::Bash(p) => format!("bash ({})", p.display()),
            Shell::Cmd => "cmd.exe".into(),
            Shell::PowerShell(p) => {
                format!("PowerShell ({}); write PowerShell, not bash", p.display())
            }
        }
    }
}

/// A command's output as text. It is UTF-8 where the environment and the prelude could ask for it; bytes that are not
/// (a Windows program writing in the ANSI code page) are decoded with that code page rather than shown as U+FFFD.
pub fn decode_output(bytes: &[u8]) -> String {
    decode_lines(bytes, ansi)
}

/// Valid UTF-8 stays as it is. Otherwise each line that is not UTF-8 is decoded whole with `ansi`: decoding every
/// invalid run on its own split the characters of a double-byte code page (GBK, Shift-JIS: a lead byte alone is
/// invalid UTF-8, its trail byte the next run). No byte of such a character is a line feed, so a line keeps them
/// whole, and the UTF-8 lines of a mixed stream stay readable.
pub(super) fn decode_lines(bytes: &[u8], ansi: impl Fn(&[u8]) -> Option<String>) -> String {
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_owned();
    }
    let mut out = String::with_capacity(bytes.len());
    for line in bytes.split_inclusive(|b| *b == b'\n') {
        match std::str::from_utf8(line) {
            Ok(s) => out.push_str(s),
            Err(_) => match ansi(line) {
                Some(s) => out.push_str(&s),
                None => out.push_str(&String::from_utf8_lossy(line)),
            },
        }
    }
    out
}

#[cfg(windows)]
pub(super) fn ansi(bytes: &[u8]) -> Option<String> {
    code_page(bytes, windows_sys::Win32::Globalization::CP_ACP)
}

#[cfg(windows)]
pub(super) fn code_page(bytes: &[u8], cp: u32) -> Option<String> {
    use windows_sys::Win32::Globalization::MultiByteToWideChar;
    let len = i32::try_from(bytes.len()).ok()?;
    // SAFETY: the pointers and lengths describe `bytes` and `wide`, which outlive the calls.
    let n = unsafe { MultiByteToWideChar(cp, 0, bytes.as_ptr(), len, std::ptr::null_mut(), 0) };
    if n <= 0 {
        return None;
    }
    let mut wide = vec![0u16; n as usize];
    let m = unsafe { MultiByteToWideChar(cp, 0, bytes.as_ptr(), len, wide.as_mut_ptr(), n) };
    (m > 0).then(|| String::from_utf16_lossy(&wide[..m as usize]))
}

#[cfg(not(windows))]
pub(super) fn ansi(_: &[u8]) -> Option<String> {
    None
}

pub fn find_in_path(exe: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(exe))
        .find(|p| p.is_file())
}

/// Resolves a program name like the OS shell would: on Windows a name without extension is tried with the
/// `PATHEXT` extensions first (`npx` -> `npx.cmd`, not the extensionless shell script next to it).
pub fn find_program(cmd: &str) -> Option<PathBuf> {
    let p = Path::new(cmd);
    if p.components().count() > 1 || p.is_absolute() {
        return p.is_file().then(|| p.to_path_buf());
    }
    #[cfg(windows)]
    if p.extension().is_none() {
        let exts = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
        let path = std::env::var_os("PATH")?;
        for d in std::env::split_paths(&path) {
            for e in exts.split(';').filter(|e| !e.is_empty()) {
                let c = d.join(format!("{cmd}{}", e.to_ascii_lowercase()));
                if c.is_file() {
                    return Some(c);
                }
            }
        }
    }
    find_in_path(cmd)
}

/// Environment variables passed through to agent commands; everything else (API keys included) is dropped.
pub(super) const ENV_ALLOW: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "USERNAME",
    "LANG",
    "LC_ALL",
    "TERM",
    "TMPDIR",
    "TEMP",
    "TMP",
    "SYSTEMROOT",
    "SystemRoot",
    "WINDIR",
    "COMSPEC",
    "PATHEXT",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "PROGRAMDATA",
    "PROGRAMFILES",
    "PROGRAMFILES(X86)",
    "NUMBER_OF_PROCESSORS",
    "PROCESSOR_ARCHITECTURE",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "GOPATH",
    "JAVA_HOME",
];

pub fn clean_env(extra: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut env: BTreeMap<String, String> = std::env::vars()
        .filter(|(k, _)| ENV_ALLOW.iter().any(|a| a.eq_ignore_ascii_case(k)))
        .collect();
    env.insert("NO_COLOR".into(), "1".into());
    env.insert("GIT_TERMINAL_PROMPT".into(), "0".into());
    // Python writes UTF-8 to a pipe (on Windows it used the ANSI code page: "xin ch\u{fffd}o").
    env.insert("PYTHONUTF8".into(), "1".into());
    env.insert("PYTHONIOENCODING".into(), "utf-8".into());
    env.extend(extra.iter().map(|(k, v)| (k.clone(), v.clone())));
    env
}

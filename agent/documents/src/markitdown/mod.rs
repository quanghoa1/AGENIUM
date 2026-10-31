//! Microsoft's markitdown, when installed: Agenium's own copy (Settings, [`own`]) or the user's (`pip install
//! 'markitdown[all]'`, `pipx`, `uv tool`) found on the PATH like the agent CLIs. It runs on a copy of the document in an empty folder through Agenium's sandbox (the
//! document is untrusted input to a Python parser): it reads its own install and writes only that folder, with no
//! network. Where no sandbox works it runs only if the user chose to run without one (ADR-117).

use std::path::{Path, PathBuf};
use std::time::Duration;

use agenium_sandbox::{CommandSpec, Executor, SandboxMode, SandboxPolicy, Shell};

use super::Converted;

pub mod own;

/// A markitdown run lasts at most this long.
const TIMEOUT: Duration = Duration::from_secs(120);

/// An installed markitdown and the folders it needs to read (its virtual environment and the Python it was made with).
#[derive(Debug, Clone)]
pub struct Markitdown {
    pub program: PathBuf,
    read_roots: Vec<PathBuf>,
}

/// The markitdown to use: Agenium's own copy (Settings installs it), else the one on the PATH, if any.
pub fn find() -> Option<Markitdown> {
    own_copy().or_else(on_path)
}

/// Agenium's own copy, when the open root has one in place.
pub fn own_copy() -> Option<Markitdown> {
    own::current(&own::dir()?).map(|(_, program)| at(program))
}

/// The markitdown on the PATH, if any.
pub fn on_path() -> Option<Markitdown> {
    agenium_sandbox::find_program("markitdown").map(at)
}

/// The markitdown at `program`, with the folders its environment needs.
pub fn at(program: PathBuf) -> Markitdown {
    let read_roots = install_roots(&program);
    Markitdown {
        program,
        read_roots,
    }
}

/// The virtual environment a program belongs to (the folder with `pyvenv.cfg`) and the Python it names (`home`), plus
/// the tool folders pipx and uv keep their environments in (their launcher in `~/.local/bin` points there).
fn install_roots(program: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let real = std::fs::canonicalize(program).unwrap_or_else(|_| program.to_path_buf());
    let add_venv = |venv: &Path, out: &mut Vec<PathBuf>| {
        out.push(venv.to_path_buf());
        if let Ok(cfg) = std::fs::read_to_string(venv.join("pyvenv.cfg")) {
            for line in cfg.lines() {
                if let Some(home) = line
                    .strip_prefix("home")
                    .map(|r| r.trim_start_matches([' ', '=']).trim())
                {
                    let home = PathBuf::from(home);
                    // `home` is the interpreter's folder (`.../Python312` or `.../bin`): its install is one up on Unix.
                    let base = if home.ends_with("bin") {
                        home.parent().map(Path::to_path_buf)
                    } else {
                        Some(home)
                    };
                    out.extend(base);
                }
            }
        }
    };
    if let Some(venv) = real
        .ancestors()
        .skip(1)
        .find(|d| d.join("pyvenv.cfg").is_file())
    {
        add_venv(venv, &mut out);
    } else if let Some(dir) = real.parent() {
        // A program in a Python's Scripts/bin folder: that Python's install.
        out.push(dir.parent().unwrap_or(dir).to_path_buf());
    }
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from);
    let appdata = std::env::var_os("APPDATA").map(PathBuf::from);
    let tools = [
        home.as_ref()
            .map(|h| h.join(".local/pipx/venvs/markitdown")),
        home.as_ref().map(|h| h.join("pipx/venvs/markitdown")),
        home.as_ref()
            .map(|h| h.join(".local/share/uv/tools/markitdown")),
        appdata.as_ref().map(|a| a.join("uv/tools/markitdown")),
    ];
    for t in tools
        .into_iter()
        .flatten()
        .filter(|t| t.join("pyvenv.cfg").is_file())
    {
        add_venv(&t, &mut out);
    }
    out.sort();
    out.dedup();
    out
}

/// The command line that converts `input` to `output` in the folder it runs in.
fn command(shell: &Shell, program: &Path, input: &str) -> String {
    let p = program.display().to_string();
    match shell {
        Shell::Bash(_) => format!("'{}' '{input}' -o 'output.md'", p.replace('\'', r"'\''")),
        Shell::Cmd => format!("\"{p}\" \"{input}\" -o output.md"),
        Shell::PowerShell(_) => format!("& '{}' '{input}' -o 'output.md'", p.replace('\'', "''")),
    }
}

impl Markitdown {
    /// The version the program reports (`markitdown --version`), if it says one.
    pub async fn version(&self) -> Option<String> {
        let mut cmd = tokio::process::Command::new(&self.program);
        cmd.arg("--version")
            .env("PYTHONUTF8", "1")
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: the desktop app has no console
        let out = tokio::time::timeout(Duration::from_secs(60), cmd.output())
            .await
            .ok()?
            .ok()?;
        let text = format!(
            "{} {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        text.split_whitespace()
            .find(|w| w.starts_with(|c: char| c.is_ascii_digit()) && w.contains('.'))
            .map(str::to_owned)
    }

    /// Converts `bytes` (a document named `name`) with markitdown. `unsandboxed`: the user chose to run commands without
    /// a sandbox where none works. `Err` says why it could not.
    pub async fn convert(
        &self,
        name: &str,
        bytes: &[u8],
        unsandboxed: bool,
    ) -> Result<Converted, String> {
        let dir = tempfile::Builder::new()
            .prefix("agenium-doc-")
            .tempdir()
            .map_err(|e| e.to_string())?;
        let ext: String = name
            .rsplit_once('.')
            .map(|(_, e)| {
                e.chars()
                    .filter(|c| c.is_ascii_alphanumeric())
                    .take(8)
                    .collect()
            })
            .unwrap_or_default();
        let input = if ext.is_empty() {
            "input".to_owned()
        } else {
            format!("input.{ext}")
        };
        std::fs::write(dir.path().join(&input), bytes).map_err(|e| e.to_string())?;
        let mut policy = SandboxPolicy::workspace(dir.path());
        policy.read_roots.extend(self.read_roots.iter().cloned());
        if agenium_sandbox::detect().backend.is_none() {
            if !unsandboxed {
                return Err(
                    "no sandbox works on this machine and running without one is off (Settings)"
                        .into(),
                );
            }
            policy.mode = SandboxMode::FullAccess;
        }
        let ex = Executor::new(Shell::detect());
        let spec = CommandSpec {
            command: command(ex.shell(), &self.program, &input),
            cwd: dir.path().to_path_buf(),
            timeout: TIMEOUT,
            env: [("PYTHONUTF8", "1"), ("PYTHONIOENCODING", "utf-8")]
                .into_iter()
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .collect(),
            stop: None,
            live: None,
        };
        let out = ex.run(&spec, &policy).await;
        agenium_sandbox::remove_run_temp_dir(dir.path());
        let out = out.map_err(|e| e.to_string())?;
        if out.timed_out {
            return Err(format!(
                "markitdown took longer than {} s",
                TIMEOUT.as_secs()
            ));
        }
        let text = std::fs::read_to_string(dir.path().join("output.md")).unwrap_or_default();
        if out.exit_code != Some(0) || text.trim().is_empty() {
            let why: String = format!("{}{}", out.stderr, out.stdout)
                .lines()
                .rev()
                .take(6)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join("\n");
            return Err(format!(
                "markitdown failed (exit {:?}): {}",
                out.exit_code,
                why.trim()
            ));
        }
        Ok(Converted {
            text: super::md::tidy(&text),
            format: "markitdown".into(),
            notes: vec!["read by markitdown".into()],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_quote_the_program_per_shell() {
        let p = Path::new("C:/Py thon/it's/markitdown.exe");
        assert_eq!(
            command(&Shell::Cmd, p, "input.pdf"),
            "\"C:/Py thon/it's/markitdown.exe\" \"input.pdf\" -o output.md"
        );
        assert_eq!(
            command(&Shell::PowerShell("pwsh".into()), p, "input.pdf"),
            "& 'C:/Py thon/it''s/markitdown.exe' 'input.pdf' -o 'output.md'"
        );
        assert_eq!(
            command(&Shell::Bash("bash".into()), Path::new("/o'k/m"), "input"),
            r"'/o'\''k/m' 'input' -o 'output.md'"
        );
    }

    #[test]
    fn a_venv_and_its_python_are_readable() {
        let t = tempfile::tempdir().unwrap();
        let venv = t.path().join("venv");
        std::fs::create_dir_all(venv.join("Scripts")).unwrap();
        let py = t.path().join("Python312");
        std::fs::create_dir_all(&py).unwrap();
        std::fs::write(
            venv.join("pyvenv.cfg"),
            format!("home = {}\nversion = 3.12\n", py.display()),
        )
        .unwrap();
        let exe = venv.join("Scripts").join("markitdown.exe");
        std::fs::write(&exe, "").unwrap();
        let roots = install_roots(&exe);
        let canon = |p: &Path| std::fs::canonicalize(p).unwrap();
        assert!(roots.iter().any(|r| canon(r) == canon(&venv)), "{roots:?}");
        assert!(roots.iter().any(|r| canon(r) == canon(&py)), "{roots:?}");
    }
}

//! System requirements check (decision 24): warns, never blocks; lowers parallelism on weak machines.

use std::path::Path;
use std::process::Command;

use serde::Serialize;

pub const MIN_CORES: usize = 4;
pub const MIN_RAM_GB: f64 = 8.0;
pub const MIN_DISK_GB: f64 = 10.0;
pub const REC_CORES: usize = 8;
pub const REC_RAM_GB: f64 = 16.0;
pub const REC_DISK_GB: f64 = 50.0;
pub const MIN_GIT: (u32, u32) = (2, 30);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Level {
    Ok,
    BelowRecommended,
    BelowMinimum,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Check {
    pub name: String,
    pub value: String,
    pub level: Level,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SysReport {
    pub os: String,
    pub checks: Vec<Check>,
    /// Suggested `runtime.max_parallel_agents` for this machine.
    pub suggested_parallel: u32,
}

impl SysReport {
    pub fn warnings(&self) -> impl Iterator<Item = &Check> {
        self.checks.iter().filter(|c| c.level != Level::Ok)
    }
}

/// Raw machine facts (injectable for tests).
#[derive(Debug, Clone, PartialEq)]
pub struct Facts {
    pub os: String,
    pub family: &'static str,
    pub cores: Option<usize>,
    pub ram_bytes: Option<u64>,
    pub disk_free_bytes: Option<u64>,
    pub git_version: Option<String>,
}

fn gb(b: u64) -> f64 {
    b as f64 / 1024.0 / 1024.0 / 1024.0
}

/// Parses `git version 2.45.1.windows.1` -> (2, 45).
pub fn parse_git_version(s: &str) -> Option<(u32, u32)> {
    let v = s.trim().strip_prefix("git version ")?;
    let mut it = v.split('.');
    Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
}

pub fn evaluate(f: &Facts) -> SysReport {
    let mut checks = Vec::new();
    let tier = |v: f64, min: f64, rec: f64| {
        if v < min {
            Level::BelowMinimum
        } else if v < rec {
            Level::BelowRecommended
        } else {
            Level::Ok
        }
    };
    let cores_level = f.cores.map_or(Level::Unknown, |c| {
        tier(c as f64, MIN_CORES as f64, REC_CORES as f64)
    });
    checks.push(Check {
        name: "cpu_cores".into(),
        value: f.cores.map_or("unknown".into(), |c| c.to_string()),
        level: cores_level,
        message: format!("minimum {MIN_CORES} cores, recommended {REC_CORES}"),
    });
    let ram_level = f.ram_bytes.map_or(Level::Unknown, |b| {
        tier(gb(b), MIN_RAM_GB - 0.5, REC_RAM_GB - 0.5)
    });
    checks.push(Check {
        name: "ram".into(),
        value: f
            .ram_bytes
            .map_or("unknown".into(), |b| format!("{:.1} GB", gb(b))),
        level: ram_level,
        message: format!("minimum {MIN_RAM_GB} GB, recommended {REC_RAM_GB} GB"),
    });
    checks.push(Check {
        name: "disk_free".into(),
        value: f
            .disk_free_bytes
            .map_or("unknown".into(), |b| format!("{:.1} GB", gb(b))),
        level: f
            .disk_free_bytes
            .map_or(Level::Unknown, |b| tier(gb(b), MIN_DISK_GB, REC_DISK_GB)),
        message: format!("minimum {MIN_DISK_GB} GB free, recommended {REC_DISK_GB} GB (SSD)"),
    });
    let git = f.git_version.as_deref().and_then(parse_git_version);
    checks.push(Check {
        name: "git".into(),
        value: f.git_version.clone().unwrap_or_else(|| "not found".into()),
        level: match git {
            Some(v) if v >= MIN_GIT => Level::Ok,
            Some(_) | None => Level::BelowMinimum,
        },
        message: format!(
            "git {}.{}+ is required (every project is a git repository)",
            MIN_GIT.0, MIN_GIT.1
        ),
    });
    let os_level = match f.family {
        "linux" | "macos" => Level::Ok,
        // Not a v1 release target (ADR-39); the sandbox depends on MXC availability (ADR-53).
        "windows" => Level::BelowRecommended,
        _ => Level::BelowMinimum,
    };
    checks.push(Check {
        name: "os".into(),
        value: f.os.clone(),
        level: os_level,
        message: match f.family {
            "linux" => "Linux: the sandbox needs bubblewrap or Landlock ABI >= 3 (kernel 6.2+), otherwise agents only run with per-command approval".into(),
            "macos" => "macOS 12+ (Seatbelt sandbox is experimental)".into(),
            "windows" => "Windows is not a v1 release target (ADR-39); agent commands are sandboxed only where MXC process containers work (ADR-53, see the sandbox line), otherwise they need your approval".into(),
            _ => "This OS has no sandbox backend: YOLO mode is refused; commands need your approval".into(),
        },
    });

    // Weak machines run fewer agents in parallel (decision 24/28): 4 cores / 8 GB -> 2.
    let weak = matches!(cores_level, Level::BelowRecommended | Level::BelowMinimum)
        || matches!(ram_level, Level::BelowRecommended | Level::BelowMinimum);
    let very_weak = cores_level == Level::BelowMinimum || ram_level == Level::BelowMinimum;
    let suggested_parallel = if very_weak {
        1
    } else if weak {
        2
    } else {
        4
    };
    SysReport {
        os: f.os.clone(),
        checks,
        suggested_parallel,
    }
}

/// Gathers facts about this machine; `root` selects the disk to measure (nearest existing ancestor).
pub fn gather(root: &Path) -> Facts {
    use sysinfo::{Disks, System};
    let mut sys = System::new();
    sys.refresh_memory();
    let ram = Some(sys.total_memory()).filter(|b| *b > 0);
    let cores = System::physical_core_count()
        .or_else(|| std::thread::available_parallelism().ok().map(|n| n.get()));

    let mut probe = root.to_path_buf();
    while !probe.exists() {
        if !probe.pop() {
            break;
        }
    }
    let probe = std::fs::canonicalize(&probe).unwrap_or(probe);
    let probe = strip_verbatim(&probe);
    let disks = Disks::new_with_refreshed_list();
    let disk = disks
        .list()
        .iter()
        .filter(|d| probe.starts_with(d.mount_point()))
        .max_by_key(|d| d.mount_point().as_os_str().len())
        .map(|d| d.available_space());

    let git = Command::new("git")
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned());

    let family = if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(windows) {
        "windows"
    } else {
        "other"
    };
    let os = System::long_os_version().unwrap_or_else(|| std::env::consts::OS.to_owned());
    let os = match System::kernel_version() {
        Some(k) if family == "linux" => format!("{os} (kernel {k})"),
        _ => os,
    };
    Facts {
        os,
        family,
        cores,
        ram_bytes: ram,
        disk_free_bytes: disk,
        git_version: git,
    }
}

/// `\\?\D:\x` -> `D:\x` so it compares with sysinfo mount points.
fn strip_verbatim(p: &Path) -> std::path::PathBuf {
    let s = p.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC") => rest.into(),
        _ => p.to_path_buf(),
    }
}

pub fn check(root: &Path) -> SysReport {
    evaluate(&gather(root))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(cores: usize, ram_gb: u64, disk_gb: u64, git: &str, family: &'static str) -> Facts {
        Facts {
            os: "test".into(),
            family,
            cores: Some(cores),
            ram_bytes: Some(ram_gb << 30),
            disk_free_bytes: Some(disk_gb << 30),
            git_version: Some(git.into()),
        }
    }

    #[test]
    fn tiers_and_parallelism() {
        let r = evaluate(&facts(8, 16, 100, "git version 2.45.1.windows.1", "linux"));
        assert_eq!(r.warnings().count(), 0);
        assert_eq!(r.suggested_parallel, 4);
        let r = evaluate(&facts(4, 8, 20, "git version 2.30.0", "macos"));
        assert_eq!(r.suggested_parallel, 2);
        assert!(r.warnings().all(|c| c.level == Level::BelowRecommended));
        let r = evaluate(&facts(2, 4, 5, "git version 2.20.1", "windows"));
        assert_eq!(r.suggested_parallel, 1);
        assert_eq!(
            r.warnings()
                .filter(|c| c.level == Level::BelowMinimum)
                .count(),
            4,
            "cores, RAM, disk, git; Windows itself is below recommended (ADR-39/53)"
        );
        assert!(r
            .warnings()
            .any(|c| c.name == "os" && c.level == Level::BelowRecommended));
    }

    #[test]
    fn git_version_parse() {
        assert_eq!(
            parse_git_version("git version 2.45.1.windows.1"),
            Some((2, 45))
        );
        assert_eq!(parse_git_version("git version 2.30.0\n"), Some((2, 30)));
        assert_eq!(parse_git_version("nope"), None);
    }

    #[test]
    fn gather_runs_on_this_machine() {
        let tmp = tempfile::tempdir().unwrap();
        let f = gather(&tmp.path().join("not-yet-created"));
        assert!(f.cores.is_some() && f.ram_bytes.is_some(), "{f:?}");
        assert!(
            f.disk_free_bytes.is_some(),
            "disk not found for {:?}",
            tmp.path()
        );
        assert!(f.git_version.is_some());
    }
}

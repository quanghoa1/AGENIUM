//! A uv of Agenium's own, for a machine with neither uv nor a usable Python: the release archive for this platform,
//! pinned and checked against its SHA-256 before anything in it runs, unpacked into `tools/markitdown/uv`.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

const UV_VERSION: &str = "0.13.1";

/// A release archive: (OS, architecture, file, SHA-256, bytes), from the release's own checksums.
const ASSETS: &[(&str, &str, &str, &str, u64)] = &[
    (
        "windows",
        "x86_64",
        "uv-x86_64-pc-windows-msvc.zip",
        "3b2661a463a5baea7f5076ea2b25ab71d77f36a6c0d809e5350651792f80a672",
        15_308_685,
    ),
    (
        "windows",
        "aarch64",
        "uv-aarch64-pc-windows-msvc.zip",
        "ec7846a5939b4654afee24f2d82bf6fde71bd7a6952d7748f1135ab301dd0a49",
        15_802_989,
    ),
    (
        "linux",
        "x86_64",
        "uv-x86_64-unknown-linux-gnu.tar.gz",
        "eadd9d27c134ab5ac1ffd4473b97a741407f3ea238288f03121fc4deddf90220",
        16_738_706,
    ),
    (
        "linux",
        "aarch64",
        "uv-aarch64-unknown-linux-gnu.tar.gz",
        "9041c73c99fdc5892cf128140beb354eca9231d502068cc36e6cb53a8e59ad14",
        15_838_367,
    ),
    (
        "macos",
        "x86_64",
        "uv-x86_64-apple-darwin.tar.gz",
        "bb7b805bdcf3d0bc8c74ceb94569e6b3e8aa35bd492acdbb9d46bf46d9d4ff6b",
        17_491_796,
    ),
    (
        "macos",
        "aarch64",
        "uv-aarch64-apple-darwin.tar.gz",
        "cfe454573412ff7380d15fc7dae5997ab53ffce72fce0a78e91ddbc84001acd2",
        14_280_170,
    ),
];

fn exe() -> &'static str {
    if cfg!(windows) {
        "uv.exe"
    } else {
        "uv"
    }
}

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The uv binary out of a release archive's bytes.
fn unpack(file: &str, bytes: &[u8], into: &Path) -> std::result::Result<PathBuf, String> {
    let dst = into.join(exe());
    if file.ends_with(".zip") {
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
        let name = (0..z.len())
            .filter_map(|i| {
                let f = z.by_index(i).ok()?;
                let n = f.name().ok()?.to_string();
                (n.rsplit('/').next() == Some(exe())).then_some(n)
            })
            .next()
            .ok_or("no uv in the archive")?;
        let mut f = z.by_name(&name).map_err(|e| e.to_string())?;
        let mut out = std::fs::File::create(&dst).map_err(|e| e.to_string())?;
        std::io::copy(&mut f, &mut out).map_err(|e| e.to_string())?;
    } else {
        // Unix: the system's tar unpacks the gzip archive (`<name>/uv` inside).
        let archive = into.join(file);
        std::fs::write(&archive, bytes).map_err(|e| e.to_string())?;
        let out = std::process::Command::new("tar")
            .arg("-xzf")
            .arg(&archive)
            .arg("-C")
            .arg(into)
            .output()
            .map_err(|e| format!("tar: {e}"))?;
        let _ = std::fs::remove_file(&archive);
        if !out.status.success() {
            return Err(format!(
                "tar: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        let folder = into.join(file.trim_end_matches(".tar.gz"));
        std::fs::rename(folder.join(exe()), &dst).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_dir_all(folder);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dst, std::fs::Permissions::from_mode(0o755))
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(dst)
}

/// Agenium's uv in `dir/uv`, downloaded and checked when it is not there yet.
pub(super) async fn standalone(
    dir: &Path,
    progress: &(dyn Fn(&str) + Send + Sync),
) -> std::result::Result<PathBuf, String> {
    let into = dir.join("uv");
    let bin = into.join(exe());
    if bin.is_file() {
        return Ok(bin);
    }
    let (_, _, file, sha, size) = ASSETS
        .iter()
        .find(|(os, arch, ..)| *os == std::env::consts::OS && *arch == std::env::consts::ARCH)
        .ok_or_else(|| {
            format!(
                "no uv release for {} {}",
                std::env::consts::OS,
                std::env::consts::ARCH
            )
        })?;
    progress(&format!("downloading uv {UV_VERSION} ({} MB)", size >> 20));
    let url = format!("https://github.com/astral-sh/uv/releases/download/{UV_VERSION}/{file}");
    let http = agenium_llm::http::Http::default_client().map_err(|e| e.to_string())?;
    let bytes = http
        .client()
        .get(&url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("{url}: {e}"))?
        .bytes()
        .await
        .map_err(|e| format!("{url}: {e}"))?;
    if bytes.len() as u64 != *size || hex(&bytes) != *sha {
        return Err(format!(
            "{file} does not match its pinned checksum; nothing from it was run"
        ));
    }
    std::fs::create_dir_all(&into).map_err(|e| e.to_string())?;
    unpack(file, &bytes, &into).inspect_err(|_| {
        let _ = std::fs::remove_dir_all(&into);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_platform_has_one_pinned_archive() {
        for (os, arch) in [
            ("windows", "x86_64"),
            ("windows", "aarch64"),
            ("linux", "x86_64"),
            ("linux", "aarch64"),
            ("macos", "x86_64"),
            ("macos", "aarch64"),
        ] {
            let n = ASSETS
                .iter()
                .filter(|(o, a, _, sha, _)| *o == os && *a == arch && sha.len() == 64)
                .count();
            assert_eq!(n, 1, "{os} {arch}");
        }
    }

    #[test]
    fn the_uv_binary_comes_out_of_a_zip() {
        use std::io::Write;
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        w.start_file("uvx.exe", zip::write::SimpleFileOptions::default())
            .unwrap();
        w.write_all(b"x").unwrap();
        w.start_file(
            format!("dist/{}", exe()),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        w.write_all(b"uv!").unwrap();
        let bytes = w.finish().unwrap().into_inner();
        let t = tempfile::tempdir().unwrap();
        let got = unpack("uv-x.zip", &bytes, t.path()).unwrap();
        assert_eq!(std::fs::read(got).unwrap(), b"uv!");
    }
}

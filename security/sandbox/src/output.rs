//! A command's output: what each stream keeps in memory, and the live output read while it runs.

use std::sync::Arc;

use tokio::io::AsyncReadExt;

/// Output of a running command, shared with whoever reads it while it runs.
pub type Live = Arc<std::sync::Mutex<Vec<u8>>>;

/// What a command's stream keeps in memory: its first and last bytes, the middle counted. A command printing without
/// end (`yes`, a test stuck in a loop, a dev server for an hour) grew Agenium's memory until the system killed it, with
/// every project's runs (audit M7, 2026-10-31). The run shows far less anyway (its output limits); the cut is said.
const KEEP_HEAD: usize = 8 << 20;
const KEEP_TAIL: usize = 8 << 20;

/// The live output of a background command at most (read as it comes, `shell_output`); then one line says the rest
/// was left out.
const LIVE_CAP: usize = 16 << 20;

/// A stream's output: its first [`KEEP_HEAD`] bytes, its last [`KEEP_TAIL`], and how many went in between.
#[derive(Default)]
pub(crate) struct Kept {
    head: Vec<u8>,
    tail: std::collections::VecDeque<u8>,
    dropped: u64,
}

impl Kept {
    pub(crate) fn push(&mut self, bytes: &[u8]) {
        let room = KEEP_HEAD.saturating_sub(self.head.len()).min(bytes.len());
        self.head.extend_from_slice(&bytes[..room]);
        self.tail.extend(&bytes[room..]);
        let over = self.tail.len().saturating_sub(KEEP_TAIL);
        if over > 0 {
            self.tail.drain(..over);
            self.dropped += over as u64;
        }
    }

    pub(crate) fn into_bytes(self) -> Vec<u8> {
        let mut out = self.head;
        if self.dropped > 0 {
            out.extend_from_slice(
                format!(
                    "\n[... {} bytes of output left out: only the first and last {} MiB are kept ...]\n",
                    self.dropped,
                    KEEP_TAIL >> 20
                )
                .as_bytes(),
            );
        }
        out.extend(self.tail);
        out
    }
}

/// Appends what a stream read to the live output, if there is one, up to [`LIVE_CAP`].
pub(crate) fn feed(live: &Option<Live>, bytes: &[u8]) {
    if let Some(l) = live {
        let mut l = l.lock().unwrap_or_else(|e| e.into_inner());
        // At the cap exactly, the next bytes still bring the line that says the rest is left out.
        if l.len() <= LIVE_CAP && !bytes.is_empty() {
            let room = (LIVE_CAP - l.len()).min(bytes.len());
            l.extend_from_slice(&bytes[..room]);
            if room < bytes.len() {
                l.extend_from_slice(
                    format!(
                        "\n[... output past {} MiB left out here; its end comes with the command's end ...]\n",
                        LIVE_CAP >> 20
                    )
                    .as_bytes(),
                );
            }
        }
    }
}

/// Reads a stream to its end, handing each chunk to the live output as it comes.
pub(crate) async fn read_all(
    mut r: impl tokio::io::AsyncRead + Unpin,
    live: Option<Live>,
) -> Vec<u8> {
    let mut all = Kept::default();
    let mut buf = [0u8; 8192];
    while let Ok(n) = r.read(&mut buf).await {
        if n == 0 {
            break;
        }
        feed(&live, &buf[..n]);
        all.push(&buf[..n]);
    }
    all.into_bytes()
}

/// Reads a stream of a contained process to its end on its own thread (the MXC and the bubblewrap / Seatbelt backends),
/// handing each chunk to the live output as it comes.
#[cfg(any(windows, target_os = "linux", target_os = "macos"))]
pub(crate) fn reader(
    mut r: Box<dyn std::io::Read + Send>,
    live: Option<Live>,
) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut all = Kept::default();
        let mut buf = [0u8; 8192];
        while let Ok(n) = r.read(&mut buf) {
            if n == 0 {
                break;
            }
            feed(&live, &buf[..n]);
            all.push(&buf[..n]);
        }
        all.into_bytes()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Audit M7 (2026-10-31): a command printing without end kept all of it in memory. Its first and last bytes stay,
    /// the cut is said; the live output stops at its cap with one line.
    #[test]
    fn a_command_s_output_is_kept_within_bounds() {
        let mut k = Kept::default();
        let chunk = vec![b'y'; 1 << 20];
        k.push(b"first line\n");
        for _ in 0..40 {
            k.push(&chunk);
        }
        k.push(b"\nlast line");
        let out = k.into_bytes();
        assert!(out.len() < KEEP_HEAD + KEEP_TAIL + 200, "{}", out.len());
        assert!(out.starts_with(b"first line"));
        assert!(out.ends_with(b"last line"));
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("bytes of output left out"));

        let live: Live = Default::default();
        let some = Some(live.clone());
        for _ in 0..40 {
            feed(&some, &chunk);
        }
        let l = live.lock().unwrap();
        assert!(l.len() < LIVE_CAP + 200);
        assert_eq!(
            String::from_utf8_lossy(&l).matches("left out here").count(),
            1
        );
        let mut small = Kept::default();
        small.push(b"ok\n");
        assert_eq!(small.into_bytes(), b"ok\n");
    }
}

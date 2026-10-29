//! Background commands of a run (ADR-129): `shell` with `background: true` starts one (a dev server, a watcher, a long
//! build), `shell_output` reads what it printed since the last read and whether it ended, `shell_stop` ends its
//! process tree. They run under the same sandbox, gate and environment as `shell`, at most `MAX_RUNNING` at a time,
//! for at most `MAX_LIFETIME`, and end with the run (the tool context is dropped). A `shell` command of a vendor CLI's
//! run that outlasts its call goes on as one too, awaited by the run (`awaited`, ADR-133).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use agenium_sandbox::{CommandSpec, ExecOutput, Executor, Live, SandboxPolicy};

mod awaited;
pub use awaited::{Ended, DENIED, STOPPED};

/// Background commands running at once in one run.
pub const MAX_RUNNING: usize = 4;
/// The longest a background command lives (its run usually ends first).
pub const MAX_LIFETIME: Duration = Duration::from_secs(60 * 60);

struct Job {
    command: String,
    started: Instant,
    live: Live,
    /// Bytes of `live` already handed to the agent.
    read: usize,
    stop: Arc<AtomicBool>,
    /// Set when the command ended: its exit (or why it could not run).
    done: Arc<Mutex<Option<Result<ExecOutput, String>>>>,
    /// Set once the command runs (an awaited one may first wait for the user's approval).
    running: Arc<AtomicBool>,
    /// A `shell` command that outlasted its call: the run hands its end to the agent (ADR-133).
    awaited: bool,
    /// Its end was handed to the agent (by `shell_output` or by the run).
    told: bool,
    timeout: Duration,
}

impl Job {
    fn ended(&self) -> bool {
        self.done
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }
}

/// What a read of a background command gives: the new output and where the command stands.
pub struct JobRead {
    pub command: String,
    pub new_output: String,
    pub state: JobState,
}

pub enum JobState {
    Running { secs: u64 },
    Exited { code: Option<i32>, timed_out: bool },
    Stopped,
    Failed(String),
}

#[derive(Default)]
pub struct Jobs {
    next: AtomicU32,
    map: Mutex<BTreeMap<u32, Job>>,
}

impl Jobs {
    /// Background commands the agent started that are still running (an awaited one is not the agent's to stop).
    fn running(&self) -> usize {
        self.map
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .filter(|j| !j.awaited && !j.ended())
            .count()
    }

    /// Starts `spec` in the background; its id, or why not.
    pub fn start(
        &self,
        executor: &Executor,
        policy: &SandboxPolicy,
        mut spec: CommandSpec,
    ) -> Result<u32, String> {
        if self.running() >= MAX_RUNNING {
            return Err(format!(
                "{MAX_RUNNING} background commands are already running: stop one with shell_stop first"
            ));
        }
        let live: Live = Default::default();
        let stop = Arc::new(AtomicBool::new(false));
        let done: Arc<Mutex<Option<Result<ExecOutput, String>>>> = Default::default();
        spec.timeout = MAX_LIFETIME;
        spec.live = Some(live.clone());
        spec.stop = Some(stop.clone());
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let command = spec.command.clone();
        let (executor, policy, finished) = (executor.clone(), policy.clone(), done.clone());
        tokio::spawn(async move {
            let r = executor
                .run(&spec, &policy)
                .await
                .map_err(|e| e.to_string());
            *finished.lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
        });
        self.map.lock().unwrap_or_else(|e| e.into_inner()).insert(
            id,
            Job {
                command,
                started: Instant::now(),
                live,
                read: 0,
                stop,
                done,
                running: Arc::new(AtomicBool::new(true)),
                awaited: false,
                told: false,
                timeout: MAX_LIFETIME,
            },
        );
        Ok(id)
    }

    /// What job `id` printed since the last read, waiting up to `wait` for something new or for its end.
    pub async fn read(&self, id: u32, wait: Duration) -> Result<JobRead, String> {
        let deadline = Instant::now() + wait;
        loop {
            let (fresh, ended) = {
                let map = self.map.lock().unwrap_or_else(|e| e.into_inner());
                let j = map.get(&id).ok_or_else(|| self.unknown(id, &map))?;
                let len = j.live.lock().unwrap_or_else(|e| e.into_inner()).len();
                let ended = j.done.lock().unwrap_or_else(|e| e.into_inner()).is_some();
                (len > j.read, ended)
            };
            if fresh || ended || Instant::now() >= deadline {
                return Ok(self.take(id));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Stops job `id` (its whole process tree) and returns what it printed since the last read.
    pub async fn stop(&self, id: u32) -> Result<JobRead, String> {
        {
            let map = self.map.lock().unwrap_or_else(|e| e.into_inner());
            let j = map.get(&id).ok_or_else(|| self.unknown(id, &map))?;
            j.stop.store(true, Ordering::Relaxed);
        }
        // The backend sees the flag within its polling interval and ends the tree; the output follows.
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let ended = {
                let map = self.map.lock().unwrap_or_else(|e| e.into_inner());
                map.get(&id)
                    .is_some_and(|j| j.done.lock().unwrap_or_else(|e| e.into_inner()).is_some())
            };
            if ended || Instant::now() >= deadline {
                return Ok(self.take(id));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    fn unknown(&self, id: u32, map: &BTreeMap<u32, Job>) -> String {
        if map.is_empty() {
            format!("no background command {id}: none was started in this run")
        } else {
            format!(
                "no background command {id}; this run has: {}",
                map.iter()
                    .map(|(k, j)| format!("{k} ({})", j.command))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    }

    fn take(&self, id: u32) -> JobRead {
        let mut map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        let j = map.get_mut(&id).expect("checked by the caller");
        let bytes = j.live.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let new_output = agenium_sandbox::decode_output(&bytes[j.read.min(bytes.len())..]);
        j.read = bytes.len();
        let state = match &*j.done.lock().unwrap_or_else(|e| e.into_inner()) {
            None => JobState::Running {
                secs: j.started.elapsed().as_secs(),
            },
            Some(Err(e)) => JobState::Failed(e.clone()),
            Some(Ok(_)) if j.stop.load(Ordering::Relaxed) => JobState::Stopped,
            Some(Ok(o)) => JobState::Exited {
                code: o.exit_code,
                timed_out: o.timed_out,
            },
        };
        // Read to its end by the agent: the run does not hand it over again.
        if !matches!(state, JobState::Running { .. }) {
            j.told = true;
        }
        JobRead {
            command: j.command.clone(),
            new_output,
            state,
        }
    }
}

impl Drop for Jobs {
    /// The run ended: every background command still running is stopped.
    fn drop(&mut self) {
        for j in self
            .map
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .values()
        {
            j.stop.store(true, Ordering::Relaxed);
        }
    }
}

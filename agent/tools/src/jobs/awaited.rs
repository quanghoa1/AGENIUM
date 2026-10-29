//! Commands that outlast their `shell` call (ADR-133). A vendor CLI waits only so long for one of Agenium's tools over
//! MCP (Claude Code gave up on an 80-second command and the model ran it again), so in a CLI's run a `shell` command
//! runs as an awaited job: when it ends within `ToolCtx::foreground_limit` the call answers as usual; otherwise the
//! call answers that it goes on, the command keeps its own time limit, and the run hands its end to the agent as a
//! message once its turn is over. The wait for the user's approval of the command is part of the job.

use super::*;
use crate::{CommandGate, GateDecision};

/// The start of why an awaited command was not run: the gate refused it.
pub const DENIED: &str = "denied: ";
/// Why an awaited command did not run to its end: the agent's or the run's stop.
pub const STOPPED: &str = "stopped";

/// An awaited command that ended.
pub struct Ended {
    pub id: u32,
    pub command: String,
    /// Its time limit (a command that ran out of it says so).
    pub timeout: Duration,
    /// Its exit, or why it did not run (`DENIED…`, `STOPPED`, an error).
    pub result: Result<ExecOutput, String>,
}

impl Jobs {
    /// Starts `spec` once `gate` allows it (a command for the user to approve waits for their answer), as a command
    /// the run waits for. The agent's `shell_stop` or the run's `stop` gives the approval up or ends the command.
    pub fn start_awaited(
        &self,
        executor: &Executor,
        policy: &SandboxPolicy,
        mut spec: CommandSpec,
        gate: Arc<dyn CommandGate>,
        run_stop: Arc<AtomicBool>,
    ) -> u32 {
        let live: Live = Default::default();
        let stop = Arc::new(AtomicBool::new(false));
        let running = Arc::new(AtomicBool::new(false));
        let done: Arc<Mutex<Option<Result<ExecOutput, String>>>> = Default::default();
        spec.live = Some(live.clone());
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let (command, timeout) = (spec.command.clone(), spec.timeout);
        let (executor, policy, finished) = (executor.clone(), policy.clone(), done.clone());
        let (own_stop, started) = (stop.clone(), running.clone());
        tokio::spawn(async move {
            // One flag for the executor, set by either stop.
            let either = Arc::new(AtomicBool::new(false));
            let watch = {
                let (a, b, c) = (own_stop.clone(), run_stop.clone(), either.clone());
                tokio::spawn(async move {
                    while !(a.load(Ordering::Relaxed) || b.load(Ordering::Relaxed)) {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                    c.store(true, Ordering::Relaxed);
                })
            };
            let flag = either.clone();
            let decision = tokio::select! {
                d = gate.check(&spec.command, &spec.cwd) => Some(d),
                _ = async move {
                    while !flag.load(Ordering::Relaxed) {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                } => None,
            };
            let r = match decision {
                None => Err(STOPPED.to_owned()),
                Some(GateDecision::Deny(reason)) => Err(format!("{DENIED}{reason}")),
                Some(GateDecision::Allow) => {
                    started.store(true, Ordering::Relaxed);
                    spec.stop = Some(either.clone());
                    executor
                        .run(&spec, &policy)
                        .await
                        .map_err(|e| e.to_string())
                }
            };
            watch.abort();
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
                running,
                awaited: true,
                told: false,
                timeout,
            },
        );
        id
    }

    /// Waits up to `wait` for job `id` to end; whether it has.
    pub async fn wait_end(&self, id: u32, wait: Duration) -> bool {
        let deadline = Instant::now() + wait;
        loop {
            let ended = self
                .map
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&id)
                .is_none_or(Job::ended);
            if ended {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Takes job `id` out of the run's list once it ended, with how: a command that ended within its call.
    pub fn take_ended(&self, id: u32) -> Option<Result<ExecOutput, String>> {
        let mut map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        if !map.get(&id).is_some_and(Job::ended) {
            return None;
        }
        map.remove(&id)
            .and_then(|j| j.done.lock().unwrap_or_else(|e| e.into_inner()).take())
    }

    /// Whether job `id` runs (it may still wait for the user's approval).
    pub fn is_running(&self, id: u32) -> bool {
        self.map
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&id)
            .is_some_and(|j| j.running.load(Ordering::Relaxed))
    }

    /// The awaited commands that have not ended.
    pub fn awaited_going(&self) -> Vec<(u32, String)> {
        self.map
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|(_, j)| j.awaited && !j.ended())
            .map(|(id, j)| (*id, j.command.clone()))
            .collect()
    }

    /// The awaited commands that ended and whose end the agent has not been given (it may have read it with
    /// `shell_output`): taken out of the run's list.
    pub fn take_ended_untold(&self) -> Vec<Ended> {
        let mut map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        let ids: Vec<u32> = map
            .iter()
            .filter(|(_, j)| j.awaited && j.ended())
            .map(|(id, _)| *id)
            .collect();
        let mut out = Vec::new();
        for id in ids {
            let Some(j) = map.remove(&id) else { continue };
            if j.told {
                continue;
            }
            let result = j
                .done
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take()
                .unwrap_or_else(|| Err(STOPPED.to_owned()));
            out.push(Ended {
                id,
                command: j.command,
                timeout: j.timeout,
                result,
            });
        }
        out
    }
}

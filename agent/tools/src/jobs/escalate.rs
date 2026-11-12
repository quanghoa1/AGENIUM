//! Sandbox first, then the user (ADR-161). A command runs in the sandbox; when the sandbox blocks it, the gate decides
//! whether it runs again outside, directly on the host: the user approves it, or it is approved by itself when the user
//! chose so. A command the agent says must run outside (credentials, docker, a server on localhost) goes to the gate
//! before it runs. Without a working sandbox, or with it turned off, a command runs once, as before.

use std::sync::atomic::Ordering;

use super::*;
use crate::{CommandGate, GateDecision};

/// A command that ran, and what the agent is told about the sandbox along with its output.
pub struct Ran {
    pub out: ExecOutput,
    pub note: Option<String>,
}

impl Ran {
    /// The output with the note as its first line, where the agent reads it before anything the command printed.
    pub fn into_output(self) -> ExecOutput {
        let mut out = self.out;
        if let Some(n) = self.note {
            out.stdout = format!(
                "{n}
{}",
                out.stdout
            );
        }
        out
    }
}

/// Whether commands under `policy` run inside a sandbox on this machine.
pub fn sandboxed(policy: &SandboxPolicy) -> bool {
    policy.mode != SandboxMode::FullAccess && agenium_sandbox::detect().backend.is_some()
}

/// `policy` without the sandbox: the command runs on the host, in its working directory, with the same clean
/// environment (no keys).
pub fn outside(policy: &SandboxPolicy) -> SandboxPolicy {
    let mut p = policy.clone();
    p.mode = SandboxMode::FullAccess;
    p
}

/// Asks the gate whether `spec` may leave the sandbox; `None` when the command's stop came first.
pub async fn ask(gate: &dyn CommandGate, spec: &CommandSpec, why: &str) -> Option<GateDecision> {
    let stop = spec.stop.clone();
    tokio::select! {
        d = gate.escalate(&spec.command, &spec.cwd, why) => Some(d),
        _ = async move {
            loop {
                if stop.as_ref().is_some_and(|s| s.load(Ordering::Relaxed)) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        } => None,
    }
}

/// Runs `spec` (already allowed by the gate's `check`) in the sandbox, or outside it when `wanted` (the agent's
/// reason) is given and the gate allows it; a command the sandbox blocks goes to the gate to run again outside.
/// `Err` is why it did not run at all: the agent's request to run it outside was refused, or it was stopped.
pub async fn run(
    executor: &Executor,
    policy: &SandboxPolicy,
    spec: &CommandSpec,
    gate: &dyn CommandGate,
    wanted: Option<&str>,
) -> Result<Ran, String> {
    let exec = |p: SandboxPolicy| async move {
        executor
            .run(spec, &p)
            .await
            .map_err(|e| format!("could not run: {e}"))
    };
    if !sandboxed(policy) {
        return Ok(Ran {
            out: exec(policy.clone()).await?,
            note: None,
        });
    }
    if let Some(why) = wanted {
        return match ask(gate, spec, &format!("the agent asks to run it outside the sandbox: {why}")).await {
            None => Err(super::STOPPED.to_owned()),
            Some(GateDecision::Deny(r)) => Err(format!(
                "{}running it outside the sandbox was not allowed: {r}",
                super::DENIED
            )),
            Some(GateDecision::Allow) => Ok(Ran {
                out: exec(outside(policy)).await?,
                note: Some("[agenium] it ran outside the sandbox, directly on the user's computer, as allowed".into()),
            }),
        };
    }
    let first = exec(policy.clone()).await?;
    let Some(line) = agenium_sandbox::blocked_by_sandbox(&first) else {
        return Ok(Ran {
            out: first,
            note: None,
        });
    };
    match ask(gate, spec, &format!("the sandbox blocked it: {line}")).await {
        Some(GateDecision::Allow) => Ok(Ran {
            out: exec(outside(policy)).await?,
            note: Some(format!(
                "[agenium] the sandbox blocked it ({line}); it ran again outside the sandbox, directly on the user's \
                 computer, as allowed. This is its second run's output"
            )),
        }),
        Some(GateDecision::Deny(r)) => Ok(Ran {
            out: first,
            note: Some(format!(
                "[agenium] the sandbox blocked it ({line}), and running it outside was not allowed: {r}. Do it \
                 another way, or say in your report what is left for the user"
            )),
        }),
        None => Ok(Ran {
            out: first,
            note: None,
        }),
    }
}

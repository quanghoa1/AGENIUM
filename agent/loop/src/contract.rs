//! What a run is given and gives back: the work order, its settings, the protocol and observer hooks, the outcome,
//! cancellation and steering.

use super::*;

/// What a protocol call asks the loop to do next (mirrors the orchestrator's `RunEffect`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Effect {
    Continue,
    End,
    Park,
}

/// Protocol tools are executed by the orchestrator (validation, state machine, idempotency).
#[async_trait]
pub trait Protocol: Send + Sync {
    /// `call_id` is the vendor tool-call id: the orchestrator derives the intent id from it so a replayed turn
    /// maps to the same intent.
    async fn call(
        &self,
        name: &str,
        args: serde_json::Value,
        call_id: &str,
    ) -> Result<(serde_json::Value, Effect), ToolError>;
}

/// Receives everything that happens in a run (run log, live view, conversation persistence, heartbeat).
pub trait Observer: Send + Sync {
    fn event(&self, _e: &ChatEvent) {}
    /// A message appended to the conversation (assistant turn or tool results / nudges).
    fn message(&self, _m: &Message) {}
    fn tool(&self, _name: &str, _args: &serde_json::Value, _result: &Result<String, ToolError>) {}
    /// One model response's own usage (each is priced on its own: a vendor's long-prompt tier is per request).
    fn response(&self, _usage: &Usage) {}
    fn step(&self, _step: u32, _usage: &Usage) {}
    fn retry(&self, _attempt: u32, _delay_ms: u64, _error: &str) {}
    /// Agenium steered the run: `nudge` (a text-only answer), `steer` (the user's message delivered), `compact`
    /// (the context replaced by notes), `refused` (a CLI tool call Agenium refused). Kept in the run's log.
    fn control(&self, _kind: &str, _text: &str) {}
    /// How full the context is (ADR-155): the tokens of the last request (or of the next one, estimated before it is
    /// sent) and the model's window, 0 when the backend does not know it.
    fn context(&self, _used: u64, _window: u64) {}
    /// The run's conversation was rewritten to fit the window (ADR-155): the whole of it, to keep in place of the
    /// stored one so the next run resumes it as it is now.
    fn replaced(&self, _messages: &[Message]) {}
}

pub struct NoopObserver;

impl Observer for NoopObserver {}

#[derive(Debug, Clone)]
pub struct LoopConfig {
    pub model: String,
    pub effort: Effort,
    pub max_tokens: u32,
    pub max_steps: u32,
    /// Same tool + identical arguments this many times in a row stops the run (decision 17).
    pub same_call_limit: u32,
    /// Consecutive invalid calls that stop the run (decision 36).
    pub error_limit: u32,
    /// Text-only answers tolerated (each gets a nudge) before the run stops.
    pub nudge_limit: u32,
    pub retry: RetryPolicy,
    pub cache_key: Option<String>,
    pub long_cache: bool,
    /// Context size of the model. At 60 % of it old tool results are cleared; at 85 % the run's conversation is
    /// replaced by the model's notes (`max_compactions` times), then the run checkpoints (ADR-100).
    pub context_window: u64,
    /// In-run compactions before a full context checkpoints the run; 0 = checkpoint at once.
    pub max_compactions: u32,
    /// The model takes images: those tools return (`view_image`, MCP screenshots) are sent to it.
    pub vision: bool,
}

impl Default for LoopConfig {
    fn default() -> Self {
        Self {
            model: String::new(),
            effort: Effort::HIGH,
            max_tokens: 32_000,
            max_steps: 200,
            same_call_limit: 3,
            error_limit: 3,
            nudge_limit: 2,
            retry: RetryPolicy::default(),
            cache_key: None,
            long_cache: true,
            context_window: 200_000,
            max_compactions: 2,
            vision: false,
        }
    }
}

/// The fixed-order prompt of a run (decision 36): tools, then the stable system blocks (cache boundary after
/// ROLE/skills), the dynamic manifest, then the conversation (memory + ticket + comments first).
#[derive(Debug, Clone)]
pub struct WorkOrder {
    pub system: Vec<SystemBlock>,
    pub messages: Vec<Message>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
    /// A protocol tool ended the run (e.g. `submit_report`) or a chat turn finished.
    Completed,
    /// Waiting for an answer/approval (blocking question or permission request).
    Parked,
    /// `max_steps` or the context limit was reached; `summary` is the model's checkpoint note.
    Checkpoint {
        summary: String,
        reason: String,
    },
    LoopDetected {
        tool: String,
    },
    TooManyErrors {
        last: String,
    },
    /// The model kept answering without calling a tool.
    NoProgress,
    /// The model declined to go on (a vendor's safety classifier); `why` is the vendor's category and explanation
    /// when given. The declined answer is not kept in the conversation.
    Refused {
        why: String,
    },
    QuotaExhausted {
        message: String,
    },
    Offline {
        message: String,
    },
    Failed {
        message: String,
    },
    Cancelled,
}

impl Outcome {
    pub fn label(&self) -> String {
        match self {
            Outcome::Completed => "Completed".into(),
            Outcome::Parked => "Parked".into(),
            Outcome::Checkpoint { reason, .. } => format!("Checkpoint:{reason}"),
            Outcome::LoopDetected { tool } => format!("Stopped:loop:{tool}"),
            Outcome::TooManyErrors { .. } => "Stopped:tool_errors".into(),
            Outcome::NoProgress => "Stopped:no_tool_call".into(),
            Outcome::Refused { .. } => "Stopped:refusal".into(),
            Outcome::QuotaExhausted { .. } => "QuotaExhausted".into(),
            Outcome::Offline { .. } => "Offline".into(),
            Outcome::Failed { message } => {
                format!("Error:{}", message.chars().take(200).collect::<String>())
            }
            Outcome::Cancelled => "Cancelled".into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RunReport {
    pub outcome: Outcome,
    pub steps: u32,
    pub usage: Usage,
    /// Messages appended during this run (after the work order's).
    pub messages: Vec<Message>,
    /// Model actually used per response (for pricing).
    pub model: String,
}

/// Cancellation flag shared with the runtime (kill switch). A child (one project's runs, one ticket's run) is
/// cancelled by its own flag or by any of its ancestors'.
#[derive(Clone, Default)]
pub struct Cancel {
    pub(super) me: Arc<AtomicBool>,
    pub(super) parent: Option<Arc<Cancel>>,
}

impl Cancel {
    pub fn cancel(&self) {
        self.me.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.me.load(Ordering::SeqCst) || self.parent.as_ref().is_some_and(|p| p.is_cancelled())
    }
    /// Lets new runs start again after a kill switch (the stopped runs have seen the flag by then).
    pub fn reset(&self) {
        self.me.store(false, Ordering::SeqCst);
    }
    /// A flag of its own that this one's cancellation also stops.
    pub fn child(&self) -> Cancel {
        Cancel {
            me: Arc::default(),
            parent: Some(Arc::new(self.clone())),
        }
    }
    /// Resolves once this flag (or an ancestor's) is set.
    pub async fn cancelled(&self) {
        while !self.is_cancelled() {
            tokio::time::sleep(CANCEL_POLL).await;
        }
    }
    /// Sets `stop` once this flag is set, until the returned guard is dropped: a command or an approval the run waits
    /// on ends at the user's stop, not at the run's next step (it took up to a command's timeout, 10 minutes).
    pub fn forward(&self, stop: &Arc<AtomicBool>) -> Forward {
        let (me, stop) = (self.clone(), stop.clone());
        Forward(tokio::spawn(async move {
            me.cancelled().await;
            stop.store(true, Ordering::SeqCst);
        }))
    }
}

/// How often something a run waits on looks at the run's cancel flag.
const CANCEL_POLL: std::time::Duration = std::time::Duration::from_millis(100);

/// Ends a [`Cancel::forward`] when dropped (with the run).
pub struct Forward(tokio::task::JoinHandle<()>);

impl Drop for Forward {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Messages the user sends to a running Lead or Solo agent (ADR-104): read before each model request.
pub type Steer = Arc<std::sync::Mutex<Vec<String>>>;

/// How a message the user sent during the run reaches the model.
pub const STEERED: &str = "[Message from the user while you work] ";

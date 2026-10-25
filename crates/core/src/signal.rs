use serde::{Deserialize, Serialize};

use crate::ids::stable_id;

/// The aggregation behavior of an event.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum EventKind {
    /// An accumulating count.
    Counter,
    /// A point-in-time reading.
    Reading,
}

/// An observation ready to be stored and uploaded.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Event {
    /// Provider-supplied stable identity for this observation.
    pub identity: String,
    /// Observation metric key.
    pub key: String,
    /// Nonnegative observation value.
    pub value: u64,
    /// Aggregation behavior.
    pub kind: EventKind,
    /// UUID of the session.
    pub session_id: String,
    /// ISO-8601 UTC timestamp with milliseconds.
    pub at: String,
    /// ISO-8601 UTC time when a rate-limit reading resets, when known.
    #[serde(default)]
    pub resets_at: Option<String>,
    /// Model family, when known.
    pub model: Option<String>,
    /// Provider identifier, when present.
    pub provider: Option<String>,
    /// The subagent that produced it (Claude Code sidechains), when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

impl Event {
    /// Returns the frozen observation identifier.
    pub fn id(&self) -> String {
        stable_id(&format!("{}:{}", self.identity, self.key))
    }
}

/// Position of a turn in its lifecycle.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TurnPhase {
    /// A turn began.
    Start,
    /// A turn made progress.
    Step,
    /// A turn ended.
    Stop,
}

/// Activity from one provider session.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Turn {
    /// Provider identifier.
    pub provider: String,
    /// UUID of the session.
    pub session_id: String,
    /// Turn lifecycle position.
    pub phase: TurnPhase,
    /// ISO-8601 UTC timestamp with milliseconds.
    pub at: String,
}

/// A parsed provider signal.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum Signal {
    /// Observation to store.
    Event(Event),
    /// Turn activity to show.
    Turn(Turn),
    /// Private, content-free task telemetry.
    Observer(Observation),
}

/// A reference to a task, either by its active context or its opaque task key.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum TaskRef {
    Active(String),
    Key(String),
}

/// Normalized per-response token counters.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub reasoning: u64,
}

/// Private task observer entry points. Keys are opaque identifiers; categories are allowlisted.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum Observation {
    Start {
        context: String,
        task_key: String,
        source_key: String,
        session_id: String,
        at: String,
        client_version: String,
        close_previous_completed: bool,
    },
    Request {
        task: TaskRef,
        response_id: String,
        usage: Usage,
        model: Option<String>,
        at: String,
    },
    Tool {
        task: TaskRef,
        tool_id: String,
        category: String,
        subagent: bool,
        at: String,
    },
    Compaction {
        task: TaskRef,
        compaction_id: String,
        at: String,
    },
    Model {
        task: TaskRef,
        model: String,
    },
    Finish {
        task: TaskRef,
        at: String,
        status: String,
        active_guard: bool,
    },
    ClientVersion {
        task: TaskRef,
        client_version: String,
    },
}

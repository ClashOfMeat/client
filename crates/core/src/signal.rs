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
}

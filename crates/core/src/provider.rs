use serde_json::{Map, Value};
use time::OffsetDateTime;

use crate::{manifest::Manifest, signal::Signal};

/// One parsed source record.
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    /// Index into the manifest's sources.
    pub source: usize,
    /// Path relative to the resolved root, kept locally.
    pub file_key: String,
    /// Byte offset in the source file.
    pub offset: u64,
    /// Parsed JSON value.
    pub value: Value,
}

/// Mutable local parse state and the current time.
pub struct Cx<'a> {
    /// State associated with the current file.
    pub file_state: &'a mut Map<String, Value>,
    /// Current time for emitted signals.
    pub now: OffsetDateTime,
}

/// Pure parser for one provider's records.
pub trait Provider: Send + Sync {
    /// Returns the provider's static manifest.
    fn manifest(&self) -> &'static Manifest;

    /// Parses one source record into signals.
    fn parse(&self, record: &Record, cx: &mut Cx) -> Vec<Signal>;

    /// Returns paths used by optional provider wiring.
    fn wiring_paths(&self) -> &'static [&'static str] {
        &[]
    }
}

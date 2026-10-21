pub mod config_writer;
pub mod env;
pub mod ids;
pub mod ipc;
pub mod manifest;
pub mod observer;
pub mod provider;
pub mod reader;
pub mod signal;
pub mod store;

pub use config_writer::{ConfigError, ConfigWriter, RestoreOutcome, WriteOutcome};
pub use env::Env;
pub use ids::{
    is_uuid, model_family, observer_task_id, session_uuid, stable_id, MAX_OBSERVATION_VALUE,
};
pub use manifest::{Detect, Manifest, Source, SourceFormat};
pub use provider::{Cx, Provider, Record};
pub use reader::{
    list_files, read_json, read_jsonl, FileCursor, FileEntry, Line, ReadBatch, CHUNK,
};
pub use signal::{Event, EventKind, Observation, Signal, TaskRef, Turn, TurnPhase, Usage};
pub use store::{HealthRow, Store, StoreError, StoredEvent};

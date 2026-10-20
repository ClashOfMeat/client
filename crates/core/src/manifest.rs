/// Static description of a provider.
#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    /// Provider identifier.
    pub id: &'static str,
    /// Human-readable provider name.
    pub name: &'static str,
    /// Detection hints.
    pub detect: Detect,
    /// Input sources.
    pub sources: &'static [Source],
    /// Paths that provider wiring may modify.
    pub wiring: &'static [&'static str],
}

/// Hints for detecting an installed provider.
#[derive(Debug, Clone, PartialEq)]
pub struct Detect {
    /// Executable names.
    pub commands: &'static [&'static str],
    /// Application names.
    pub apps: &'static [&'static str],
    /// Filesystem paths.
    pub paths: &'static [&'static str],
}

/// A provider input location and its file format.
#[derive(Debug, Clone, PartialEq)]
pub struct Source {
    /// Root expression resolved by `Env`.
    pub root: &'static str,
    /// File glob relative to the root.
    pub glob: &'static str,
    /// Input file format.
    pub format: SourceFormat,
}

/// Format of a provider input source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceFormat {
    /// Newline-delimited JSON.
    Jsonl,
    /// One JSON document.
    Json,
    /// A local spool file.
    Spool,
}

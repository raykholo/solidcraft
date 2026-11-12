//! Headless entry orchestration, independent of the runtime and application format.
use crate::{CommandSchema, FileGrant, HostError, Manifest, PluginHost, Result, validate_input};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TestArtifacts {
    pub design: PathBuf,
    pub journal: PathBuf,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct EntryTest {
    pub entry: String,
    pub passed: bool,
    pub output: Option<Value>,
    pub error: Option<HostError>,
    #[serde(flatten)]
    pub artifacts: Option<TestArtifacts>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TestReport {
    pub passed: bool,
    pub entries: Vec<EntryTest>,
}

/// Each entry receives an independent host made from the same initial design.
/// Runtime adapters retain responsibility for transaction commit and rollback.
pub fn test_entries<H: PluginHost>(
    manifest: &Manifest,
    entry: Option<&str>,
    input: &Value,
    mut make_host: impl FnMut() -> Result<H>,
    mut execute: impl FnMut(H, &str, &Value) -> (H, Result<Value>),
    mut save: impl FnMut(&H, &str) -> Result<TestArtifacts>,
) -> Result<TestReport> {
    let runtime =
        manifest.runtime.as_ref().ok_or_else(|| HostError::InvalidManifest { field: "runtime".into(), reason: "no executable runtime".into() })?;
    runtime.validate()?;
    if entry.is_some_and(|name| !runtime.entries.iter().any(|e| e.name == name)) {
        return Err(HostError::InvalidInput { field: "entry".into(), reason: "unknown entry".into() });
    }
    let mut entries = Vec::new();
    for declared in runtime.entries.iter().filter(|e| entry.is_none_or(|name| name == e.name)) {
        let mut artifacts = None;
        let outcome = make_host().and_then(|host| {
            let (host, outcome) = match validate_input(&declared.input_schema, input) {
                Ok(()) => execute(host, &declared.name, input),
                Err(error) => (host, Err(error)),
            };
            let saved = save(&host, &declared.name);
            match saved {
                Ok(paths) => {
                    artifacts = Some(paths);
                    outcome
                }
                Err(error) => outcome.and(Err(error)),
            }
        });
        let (output, error) = match outcome {
            Ok(value) => (Some(value), None),
            Err(error) => (None, Some(error)),
        };
        entries.push(EntryTest { entry: declared.name.clone(), passed: error.is_none(), output, error, artifacts });
    }
    Ok(TestReport { passed: entries.iter().all(|e| e.passed), entries })
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CommandCall {
    pub command: String,
    pub params: Value,
    pub output: Option<Value>,
    pub error: Option<HostError>,
}

#[derive(Serialize)]
struct BorrowedCall<'a> {
    command: &'a str,
    params: &'a Value,
    output: Option<&'a Value>,
    error: Option<&'a HostError>,
}

struct JournalSize {
    bytes: usize,
    limit: usize,
}
impl std::io::Write for JournalSize {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.bytes = self.bytes.saturating_add(bytes.len());
        if self.bytes > self.limit {
            return Err(std::io::Error::other("journal size limit"));
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Records attempted host commands, including failures later rolled back by the runtime.
pub struct JournalHost<H> {
    pub host: H,
    pub journal: Vec<CommandCall>,
    journal_bytes: usize,
}
impl<H> JournalHost<H> {
    pub fn new(host: H) -> Self {
        Self { host, journal: Vec::new(), journal_bytes: 0 }
    }
}
impl<H: PluginHost> PluginHost for JournalHost<H> {
    fn command_catalog(&self) -> Vec<CommandSchema> {
        self.host.command_catalog()
    }
    fn run_command(&mut self, name: &str, args: &Value) -> Result<Value> {
        // Bound host-side recording as well as guest memory. A journal limit is
        // a failed host call, so the runtime rolls back the whole invocation.
        const MAX_JOURNAL_BYTES: usize = 16 * 1024 * 1024;
        let limit = || HostError::LimitExceeded { limit: "command journal exceeds 16 MiB or 256 calls".into() };
        if self.journal.len() >= 256 || self.journal_bytes >= MAX_JOURNAL_BYTES {
            return Err(limit());
        }
        let outcome = self.host.run_command(name, args);
        // Count borrowed data before cloning it: an oversized app command reply
        // must not allocate a second copy just to discover the journal is full.
        let mut size = JournalSize { bytes: self.journal_bytes, limit: MAX_JOURNAL_BYTES };
        let call = BorrowedCall { command: name, params: args, output: outcome.as_ref().ok(), error: outcome.as_ref().err() };
        if serde_json::to_writer(&mut size, &call).is_err() {
            return Err(limit());
        }
        self.journal_bytes = size.bytes;
        self.journal.push(CommandCall {
            command: name.into(),
            params: args.clone(),
            output: outcome.as_ref().ok().cloned(),
            error: outcome.as_ref().err().cloned(),
        });
        outcome
    }
    fn begin_transaction(&mut self) -> Result<()> {
        self.host.begin_transaction()
    }
    fn commit_transaction(&mut self) -> Result<()> {
        self.host.commit_transaction()
    }
    fn rollback_transaction(&mut self) -> Result<()> {
        self.host.rollback_transaction()
    }
    fn granted_folders(&self) -> Vec<FileGrant> {
        self.host.granted_folders()
    }
    fn log(&mut self, message: &str) {
        self.host.log(message);
    }
    fn progress(&mut self, fraction: f32, message: &str) {
        self.host.progress(fraction, message);
    }
    fn is_cancelled(&self) -> bool {
        self.host.is_cancelled()
    }
}

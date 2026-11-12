//! Offline package diagnostics and input contracts shared by every Craft host.
use crate::{HostError, Manifest, Result, invalid, parse_manifest};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CheckProblem {
    /// Manifest field, component export, or exact component import name.
    pub field: String,
    pub error: HostError,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CheckReport {
    pub passed: bool,
    pub problems: Vec<CheckProblem>,
}
impl CheckReport {
    pub fn new(problems: Vec<CheckProblem>) -> Self {
        Self { passed: problems.is_empty(), problems }
    }
}

impl CheckProblem {
    pub fn manifest(error: HostError) -> Self {
        let field = match &error {
            HostError::InvalidManifest { field, .. } | HostError::InvalidInput { field, .. } => field.clone(),
            HostError::PermissionDenied { permission } if permission == "net" => "runtime.permissions.net".into(),
            _ => "manifest".into(),
        };
        Self { field, error }
    }
}

/// The typed parser remains the authority. Checkers never execute guest code.
pub fn check_manifest(bytes: &[u8], folder: &str) -> std::result::Result<Manifest, CheckProblem> {
    parse_manifest(bytes, folder).map_err(CheckProblem::manifest)
}

struct Offline;
impl jsonschema::Retrieve for Offline {
    fn retrieve(&self, uri: &jsonschema::Uri<String>) -> std::result::Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        Err(format!("external schema reference is forbidden: {uri}").into())
    }
}

pub(crate) fn compile_schema(schema: &Value, field: &str) -> Result<jsonschema::Validator> {
    jsonschema::options().with_retriever(Offline).build(schema).map_err(|error| invalid(field, error.to_string()))
}

pub fn validate_input(schema: &Value, input: &Value) -> Result<()> {
    let validator = compile_schema(schema, "input_schema")?;
    validator.validate(input).map_err(|error| HostError::InvalidInput { field: error.instance_path().to_string(), reason: error.to_string() })
}

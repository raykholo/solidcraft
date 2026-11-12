//! Input and output schemas for explicit plugin inspection and execution.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InspectParams {
    /// Package folder containing plugin.json and its component; subject to the caller's file scope.
    pub path: String,
}
#[derive(Debug, Serialize, JsonSchema)]
pub struct InspectResult {
    pub manifest: crate::plugin::Manifest,
    /// SHA-256 of the exact manifest bytes and component. Review the manifest before using it.
    pub approval: String,
    pub host_limits: crate::plugin::HostLimits,
    /// Whether the requested resources fit the current user ceilings; not an execution grant.
    pub within_host_limits: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits_error: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub param_warnings: Vec<String>,
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunParams {
    pub path: String,
    /// Exact approval hash returned by plugin.inspect, after permission review.
    pub approval: String,
    pub entry: String,
    #[serde(default)]
    pub params: Value,
}
#[derive(Debug, Serialize, JsonSchema)]
pub struct RunResult {
    pub result: Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub param_warnings: Vec<String>,
}

/// Read a body's evaluated triangle mesh in its owning component's coordinates (millimetres).
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MeshParams {
    pub body: String,
    /// Chord tolerance in mm. Default: 0.05; must be finite and positive.
    #[serde(default = "mesh_tolerance")]
    pub tolerance: f64,
}
fn mesh_tolerance() -> f64 {
    0.05
}
#[derive(Debug, Serialize, JsonSchema)]
pub struct MeshResult {
    pub body: String,
    pub component: u64,
    pub frame: String,
    pub positions: Vec<[f64; 3]>,
    pub triangles: Vec<[u32; 3]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub param_warnings: Vec<String>,
}

/// Install a reviewed package folder or zip; execution still requires a separate approval.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct InstallParams {
    pub path: String,
    #[serde(default)]
    pub replace: Option<bool>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct InstallResult {
    pub id: String,
    pub approval: String,
    pub manifest: crate::plugin::Manifest,
    pub folder: String,
    pub allowed: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub param_warnings: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct RemoveParams {
    pub id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RemoveResult {
    pub id: String,
    pub removed: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub param_warnings: Vec<String>,
}

/// Import through a plugin, or open a supported file with the native reader.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct ImportParams {
    pub path: String,
    #[serde(default)]
    pub plugin: Option<String>,
    #[serde(default)]
    pub approval: Option<String>,
    #[serde(default)]
    pub options: Option<serde_json::Map<String, Value>>,
    #[serde(default)]
    pub into: Option<ImportInto>,
    #[serde(default)]
    pub up: Option<super::file::UpParam>,
    #[serde(default)]
    pub names: Option<super::file::NamesParam>,
    #[serde(default)]
    pub folder: Option<bool>,
    #[serde(default)]
    pub origin: Option<bool>,
    #[serde(default)]
    pub read_only: Option<bool>,
    #[serde(default)]
    pub units: Option<super::file::CloudUnits>,
    #[serde(default)]
    pub max_points: Option<super::file::MaxPoints>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ImportInto {
    New,
    Current,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PluginImported {
    pub path: String,
    pub plugin: String,
    pub plugin_version: String,
    pub payload: String,
    pub into: String,
    pub notes: String,
    pub features: usize,
    pub errors: usize,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subcomponents: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rigid_joints: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub param_warnings: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum ImportResult {
    Plugin(PluginImported),
    Native(super::file::OpenResult),
}

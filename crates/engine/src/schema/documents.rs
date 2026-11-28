//! Contracts and result types for the commands that list the open designs, show one of them,
//! and close one (`document.list`, `document.activate`, `document.close`).
//!
//! The engine holds one design, so these describe what a command line or headless server answers
//! (its one design). The desktop window answers them for every design tab it has open.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// List the designs that are open, in tab order. Reads only.
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DocumentListParams {}

/// One open design in `document.list`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct OpenDocument {
    /// The design's id, the one `document.revision` gives. Kept while the design stays open,
    /// through saves and renames; another for each design started or opened.
    pub document_id: String,
    /// The design's name, as its tab shows it.
    pub name: String,
    /// The file the design is saved in, or null while it has none.
    pub path: Option<String>,
    /// Whether this is the design every other command acts on.
    pub active: bool,
    /// Whether the design has changes that are not saved.
    pub dirty: bool,
}

/// What `document.list` returns.
#[derive(Debug, Serialize, JsonSchema)]
pub struct DocumentListResult {
    /// The open designs, in tab order.
    pub documents: Vec<OpenDocument>,
    /// The `document_id` of the design every other command acts on now.
    pub active_document_id: String,
    /// In the warn parameter-check mode: what the call's parameters got wrong.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub param_warnings: Vec<String>,
}

/// Show an open design in its tab and make it the one every other command acts on.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DocumentActivateParams {
    /// The design's id, as `document.list` or `file.new` with `new_tab` gives it.
    pub document_id: String,
}

/// What `document.activate` returns: the design now shown.
#[derive(Debug, Serialize, JsonSchema)]
pub struct DocumentActivateResult {
    /// The design now active.
    pub document_id: String,
    /// Its name.
    pub name: String,
    /// The file it is saved in, or null while it has none.
    pub path: Option<String>,
    /// In the warn parameter-check mode: what the call's parameters got wrong.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub param_warnings: Vec<String>,
}

/// Close the active clean design tab without discarding unsaved changes.
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DocumentCloseParams {
    /// Optional guard: the expected `document_id` of the active design (from `document.list` or
    /// `document.revision`). Refused if another design is active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_id: Option<String>,
}

/// What `document.close` returns: the closed design's id, the design now active, and how many tabs remain.
#[derive(Debug, Serialize, JsonSchema)]
pub struct DocumentCloseResult {
    /// The `document_id` of the design that was closed.
    pub closed_document_id: String,
    /// The `document_id` of the design now active.
    pub document_id: String,
    /// How many design tabs remain open.
    pub documents: usize,
    /// In the warn parameter-check mode: what the call's parameters got wrong.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub param_warnings: Vec<String>,
}

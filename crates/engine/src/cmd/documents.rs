//! The open designs: list them, show one, and close one. The engine holds one design and no tabs,
//! so what is here is what a command line or headless server answers: its one design. The desktop
//! window answers `document.list`, `document.activate`, and `document.close` itself, for every
//! design tab it has open (`ui-egui`'s `documents`), and never reaches these handlers.
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::CommandSpec;
use crate::params::bad;
use crate::schema::documents::{
    DocumentActivateParams, DocumentActivateResult, DocumentCloseParams, DocumentCloseResult, DocumentListParams, DocumentListResult, OpenDocument,
};
use crate::schema::{TypedContract, output_schema_for};
use crate::{Result, Session};

const LIST_CONTRACT: TypedContract<DocumentListParams> = TypedContract::new();
const ACTIVATE_CONTRACT: TypedContract<DocumentActivateParams> = TypedContract::new();
const CLOSE_CONTRACT: TypedContract<DocumentCloseParams> = TypedContract::new();

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec::new("document.list", "List Designs", crate::checked!("document.list", list))
        .any_part()
        .icon("list")
        .noundo()
        .doc_only()
        .read_only()
        .contract(&LIST_CONTRACT)
        .output(output_schema_for::<DocumentListResult>)
        .params("no parameters → documents: [{document_id, name, path, active, dirty}] of the open designs in tab order; active_document_id: the design every other command acts on. The command line and headless servers have one design; read-only"),
    CommandSpec::new("document.activate", "Show Design", crate::checked!("document.activate", activate))
        .any_part()
        .icon("list")
        .noundo()
        .doc_only()
        .contract(&ACTIVATE_CONTRACT)
        .output(output_schema_for::<DocumentActivateResult>)
        .params("document_id: an open design's id, from document.list, document.revision or file.new with new_tab → document_id, name, path; shows that design's tab, and every command after this one acts on it. Never edits a design or opens a dialog; an id that no open design has is an error"),
    CommandSpec::new("document.close", "Close design", crate::checked!("document.close", close))
        .any_part()
        .icon("close")
        .noundo()
        .doc_only()
        .contract(&CLOSE_CONTRACT)
        .output(output_schema_for::<DocumentCloseResult>)
        .params("document_id?: optional guard naming the expected active design's id (from document.list or document.revision) → closed_document_id, document_id, documents; closes the active clean design tab without discarding unsaved changes, switching to a neighbour tab or a fresh blank design if it was the last tab. Refuses unsaved changes, active file operations or prompts, or a mismatched document_id"),
];

/// `p` as the contract type `P`; no parameters at all (`null`) is the empty object.
fn parse<P: DeserializeOwned>(cmd: &str, p: &Value) -> Result<P> {
    if !p.is_null() && !p.is_object() {
        return Err(bad(cmd, "parameters must be a JSON object"));
    }
    let p = if p.is_null() { Value::Object(Default::default()) } else { p.clone() };
    serde_json::from_value(p).map_err(|e| bad(cmd, e.to_string()))
}

fn reply(cmd: &str, reply: impl serde::Serialize) -> Result<Value> {
    serde_json::to_value(reply).map_err(|e| bad(cmd, e.to_string()))
}

/// The most bytes of a `document_id` that can name a design: the ids this program makes are far
/// shorter, so a longer one names none.
const MAX_ID: usize = 128;

fn list(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "document.list";
    let DocumentListParams {} = parse(cmd, p)?;
    let document =
        OpenDocument { document_id: s.document_id.clone(), name: s.doc.name.clone(), path: s.path.clone(), active: true, dirty: s.is_dirty() };
    reply(cmd, DocumentListResult { active_document_id: document.document_id.clone(), documents: vec![document], param_warnings: Vec::new() })
}

fn activate(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "document.activate";
    let request: DocumentActivateParams = parse(cmd, p)?;
    if request.document_id.len() > MAX_ID || request.document_id != s.document_id {
        return Err(bad(cmd, "no open design has that document_id (document.list names the open designs)"));
    }
    reply(
        cmd,
        DocumentActivateResult { document_id: s.document_id.clone(), name: s.doc.name.clone(), path: s.path.clone(), param_warnings: Vec::new() },
    )
}

fn close(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "document.close";
    let request: DocumentCloseParams = parse(cmd, p)?;
    if let Some(id) = request.document_id.as_deref()
        && (id.len() > MAX_ID || id != s.document_id)
    {
        return Err(bad(cmd, "the active document changed; nothing was closed"));
    }
    if s.is_dirty() {
        return Err(bad(cmd, "the design has unsaved changes; save it before document.close"));
    }
    if s.path.is_none() && s.doc.features.is_empty() && s.doc.params.is_empty() && s.doc.components.is_empty() {
        return Err(bad(cmd, "no open design to close"));
    }
    let closed_document_id = s.document_id.clone();
    s.replace_design(Session::default());
    reply(cmd, DocumentCloseResult { closed_document_id, document_id: s.document_id.clone(), documents: 1, param_warnings: Vec::new() })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::Session;
    use crate::schema::documents::{DocumentActivateResult, DocumentCloseResult, DocumentListResult};
    use crate::schema::output_schema_for;
    use crate::schema::tests::conforms;

    /// A host with one design lists it, as the active one, under the id `document.revision` gives.
    #[test]
    fn list_names_the_one_design() {
        let mut s = Session::default();
        s.execute("file.new", &json!({"name": "Sheet 1"})).unwrap();
        let revision = s.execute("document.revision", &json!({})).unwrap();
        let listed = s.execute("document.list", &json!({})).unwrap();
        assert_eq!(listed["active_document_id"], revision["document_id"], "{listed}");
        assert_eq!(
            listed["documents"],
            json!([{"document_id": revision["document_id"], "name": "Sheet 1", "path": null, "active": true, "dirty": false}])
        );
        s.execute("solid.box", &json!({"length": 2, "width": 2, "height": 2})).unwrap();
        assert_eq!(s.execute("document.list", &json!(null)).unwrap()["documents"][0]["dirty"], true);
        let schema = output_schema_for::<DocumentListResult>();
        assert!(conforms(schema.as_value(), schema.as_value(), &listed), "{listed}");
    }

    /// Showing the design that is shown is allowed; an id that names no open design, a missing one
    /// and a stray key are refused, and none of them changes anything.
    #[test]
    fn activate_takes_only_the_id_of_an_open_design() {
        let mut s = Session::default();
        let id = s.document_id.clone();
        let reply = s.execute("document.activate", &json!({"document_id": id})).unwrap();
        assert_eq!(reply["document_id"], json!(id), "{reply}");
        let schema = output_schema_for::<DocumentActivateResult>();
        assert!(conforms(schema.as_value(), schema.as_value(), &reply), "{reply}");
        for bad in [
            json!({"document_id": "no-such-design"}),
            json!({"document_id": "x".repeat(4096)}),
            json!({}),
            json!({"document_id": 7}),
            json!({"document_id": id, "index": 0}),
        ] {
            assert!(s.execute("document.activate", &bad).is_err(), "{bad}");
        }
        assert_eq!(s.document_id, id);
        assert!(s.execute("document.list", &json!({"all": true})).is_err(), "a stray key is refused");
    }

    /// Closing refuses blank, dirty, or mismatched designs, and replaces a clean design with a
    /// blank one conforming to `DocumentCloseResult` (Part of #23624).
    #[test]
    fn close_refuses_dirty_blank_or_mismatched_and_closes_clean_design() {
        let mut s = Session::default();
        assert!(s.execute("document.close", &json!({})).is_err(), "blank design cannot be closed");

        s.execute("solid.box", &json!({"length": 5, "width": 5, "height": 5})).unwrap();
        let id = s.document_id.clone();
        let err = s.execute("document.close", &json!({"document_id": id})).unwrap_err();
        assert!(err.to_string().contains("unsaved"), "{err}");

        s.mark_saved();
        for bad in [
            json!({"document_id": "no-such-design"}),
            json!({"document_id": "x".repeat(4096)}),
            json!({"document_id": 7}),
            json!({"force": true}),
            json!([]),
        ] {
            assert!(s.execute("document.close", &bad).is_err(), "{bad}");
            assert_eq!(s.document_id, id, "{bad}");
        }

        let reply = s.execute("document.close", &json!({"document_id": id})).unwrap();
        assert_eq!(reply["closed_document_id"], json!(id), "{reply}");
        assert_ne!(reply["document_id"], json!(id), "{reply}");
        assert_eq!(reply["documents"], json!(1), "{reply}");
        assert!(s.doc.features.is_empty());
        let schema = output_schema_for::<DocumentCloseResult>();
        assert!(conforms(schema.as_value(), schema.as_value(), &reply), "{reply}");
    }
}

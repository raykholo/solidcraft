//! Dutchman Convert as a SolidCraft importer plugin: Fusion, Inventor, SolidWorks and FreeCAD
//! files in, a SolidCraft design with the mapped feature history out.
//!
//! The component is given one file's bytes and nothing else: no files, network, clock or
//! processes. Dutchman's checks that need the SolidCraft CLI (replaying the history to prove
//! it, and finding the edges of fillets and chamfers) are therefore off: blends are left out
//! and said so in the notes, the original geometry is the visible model, and the unchecked
//! history is kept in a hidden "Editable history (unverified)" part beside it.
mod bindings {
    wit_bindgen::generate!({ path: "../../../wit", world: "importer-plugin" });
}

use std::path::Path;

use bindings::exports::solidcraft::plugin::importer::{Guest, Imported, Payload};
use bindings::solidcraft::plugin::import_context::{self, Stage};
use bindings::solidcraft::plugin::types::PluginError;
use dutchman_core::{Format, Report, Writer};
use dutchman_solidcraft::{SolidCraftWriter, Verification};

/// The formats this component converts: what Dutchman reads from one file, with no other
/// files beside it.
const FORMATS: [Format; 5] = [
    Format::FusionArchive,
    Format::FusionMultiDocument,
    Format::InventorPart,
    Format::SolidWorksPart,
    Format::FreeCadDocument,
];

/// Most bytes of notes handed back.
const MAX_NOTES: usize = 64 * 1024;

struct DutchmanConvert;

/// What the user chose in the import dialog (the parameters `plugin.json` declares, checked by
/// SolidCraft), and where the result goes.
struct Options {
    /// The feature history (hidden and unverified) as well as the geometry.
    history: bool,
    /// Reference sketches too.
    sketches: bool,
}

impl Options {
    fn from_host() -> Self {
        let options: serde_json::Value = serde_json::from_str(&import_context::options()).unwrap_or_default();
        let text = |key: &str| options.get(key).and_then(serde_json::Value::as_str);
        let preset = text("preset").unwrap_or("standard");
        // Into an open design only the geometry comes along.
        let history = options.get("keep_history").and_then(serde_json::Value::as_bool) != Some(false)
            && preset != "fast"
            && text("into") != Some("current");
        Options { history, sketches: preset == "full" }
    }
}

impl Guest for DutchmanConvert {
    fn import(file_name: String, bytes: Vec<u8>) -> Result<Imported, PluginError> {
        import_context::progress(Stage::Reading, 0, 1);
        let name = Path::new(&file_name);
        let format = Format::from_path(name)
            .filter(|f| FORMATS.contains(f))
            .ok_or_else(|| PluginError::Invalid(format!("{file_name}: not a file this importer converts")))?;
        let options = Options::from_host();
        let mut report = Report::default();
        import_context::progress(Stage::Decoding, 0, 1);
        let model = dutchman_core::read_bytes(format, name, &bytes, &mut report).map_err(|e| PluginError::Failed(e.to_string()))?;
        drop(bytes);
        import_context::progress(Stage::Mapping, 0, 1);
        // No SolidCraft CLI exists in the sandbox, so the history cannot be checked against
        // the original: keep the original as the visible model and the history hidden.
        let writer = SolidCraftWriter::new()
            .verification(Verification::Off)
            .hide_unverified_history(true)
            .import_only(!options.history)
            .sketches(options.sketches);
        let mut design = Vec::new();
        import_context::progress(Stage::Writing, 0, 1);
        writer.write(&model, &mut design, &mut report).map_err(|e| PluginError::Failed(e.to_string()))?;
        import_context::progress(Stage::Writing, 1, 1);
        Ok(Imported { payload: Payload::Design(design), notes: notes(&report) })
    }
}

/// What did not come across, one line each, within [`MAX_NOTES`].
fn notes(report: &Report) -> String {
    let mut out = String::new();
    for loss in report.losses() {
        let line = format!("{loss}\n");
        if out.len() + line.len() > MAX_NOTES {
            break;
        }
        out.push_str(&line);
    }
    out
}

bindings::export!(DutchmanConvert with_types_in bindings);

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

impl Guest for DutchmanConvert {
    fn import(file_name: String, bytes: Vec<u8>) -> Result<Imported, PluginError> {
        let name = Path::new(&file_name);
        let format = Format::from_path(name)
            .filter(|f| FORMATS.contains(f))
            .ok_or_else(|| PluginError::Invalid(format!("{file_name}: not a file this importer converts")))?;
        let mut report = Report::default();
        let model = dutchman_core::read_bytes(format, name, &bytes, &mut report).map_err(|e| PluginError::Failed(e.to_string()))?;
        drop(bytes);
        // No SolidCraft CLI exists in the sandbox, so the history cannot be checked against
        // the original: keep the original as the visible model and the history hidden.
        let writer = SolidCraftWriter::new().verification(Verification::Off).hide_unverified_history(true);
        let mut design = Vec::new();
        writer.write(&model, &mut design, &mut report).map_err(|e| PluginError::Failed(e.to_string()))?;
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

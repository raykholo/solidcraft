//! What an importer plugin brought over, as a report window after the import: the plugin, what
//! the design holds now, and each of its notes (what did not come across, or was kept hidden
//! and why). Closed with OK or Esc.

use serde_json::Value;

use crate::SolveApp;
use crate::ui_kit;

/// One finished plugin import, as the report shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    pub title: String,
    pub summary: String,
    pub notes: Vec<String>,
}

/// The most notes the report lists (the importer's notes are at most 64 KiB anyway).
const MAX_NOTES: usize = 400;

/// The report for a plugin import's reply, when it was one (`plugin` in the reply).
pub fn from_reply(reply: &Value, file: &str) -> Option<Report> {
    let plugin = reply.get("plugin").and_then(Value::as_str)?;
    let name = solidcraft_engine::plugin::contrib::specs()
        .into_iter()
        .find_map(|s| s.plugin.filter(|c| c.plugin == plugin).map(|c| c.plugin_name.clone()))
        .unwrap_or_else(|| plugin.to_string());
    let features = reply.get("features").and_then(Value::as_u64).unwrap_or(0);
    let errors = reply.get("errors").and_then(Value::as_u64).unwrap_or(0);
    let file = std::path::Path::new(file).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
    let summary = crate::i18n::format(
        if reply.get("into").and_then(Value::as_str) == Some("current") { "plugin_report.summary_into" } else { "plugin_report.summary" },
        &[("file", &file), ("plugin", &name), ("features", &features.to_string()), ("errors", &errors.to_string())],
    );
    let notes: Vec<String> = reply
        .get("notes")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .take(MAX_NOTES)
        .map(str::to_string)
        .collect();
    Some(Report { title: crate::i18n::tr("plugin_report.title").to_string(), summary, notes })
}

fn id() -> egui::Id {
    egui::Id::new("plugin_import_report")
}

/// Show `report` until the user closes it.
pub fn open(ctx: &egui::Context, report: Report) {
    ctx.data_mut(|d| d.insert_temp(id(), Some(report)));
}

/// The report being shown, if any.
pub fn current(ctx: &egui::Context) -> Option<Report> {
    ctx.data(|d| d.get_temp::<Option<Report>>(id())).flatten()
}

pub fn show(_app: &mut SolveApp, ctx: &egui::Context) {
    let Some(report) = current(ctx) else { return };
    let (closed, action) = ui_kit::Window::new(id().with("card"), "import", &report.title)
        .cards(1.4)
        .actions(crate::i18n::tr_label("OK"), "", None)
        .show(ctx, true, |ui| {
            ui_kit::text(ui, &report.summary, ui_kit::Text::Report("summary", false));
            if report.notes.is_empty() {
                ui_kit::text(ui, crate::i18n::tr("plugin_report.nothing_lost"), ui_kit::Text::Note("notes"));
            } else {
                ui_kit::text(ui, crate::i18n::tr("plugin_report.notes"), ui_kit::Text::Note("notes"));
                ui_kit::report_list(ui, "plugin_notes", 320.0, |ui| {
                    for (i, note) in report.notes.iter().enumerate() {
                        if i > 0 {
                            ui_kit::report_divider(ui, &format!("plugin_note_{i}"));
                        }
                        ui.add(egui::Label::new(note.as_str()).wrap());
                    }
                });
            }
        });
    if closed || action != ui_kit::Action::None {
        ctx.data_mut(|d| d.insert_temp::<Option<Report>>(id(), None));
    }
}

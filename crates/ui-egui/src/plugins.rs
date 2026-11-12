//! Preferences › Plugins: packages installed under the per-user data folder.
//!
//! A plugin is a directory of `{data}/plugins/<id>/plugin.json`. The page searches that list.
//! It reads `plugin.json` and records which ids are turned off. It does not load or run anything
//! from the folder. Explicit `plugin.run` calls use the native component host and honour
//! `disabled_plugins`; listing or enabling a package never executes its code.

use std::path::{Path, PathBuf};

use egui::RichText;
use serde::Deserialize;

use crate::SolveApp;
use crate::ui_kit::BackingTooltipExt as _;

/// Identify external-code requests using only command data; never scan packages on the window.
pub(crate) fn is_runtime_request(id: &str, params: &serde_json::Value) -> bool {
    let id = solidcraft_engine::find_command(id).map_or(id, |spec| spec.id);
    let path = params.get("path").and_then(serde_json::Value::as_str);
    match id {
        "plugin.run" | "file.import" => true,
        "file.export" => {
            ["approval", "options"].iter().any(|key| params.get(*key).is_some_and(|value| !value.is_null()))
                || params
                    .get("format")
                    .and_then(serde_json::Value::as_str)
                    .or(path)
                    .is_some_and(|format| solidcraft_engine::io::Format::from_name(format).is_err())
        }
        "doc.open" => {
            params.get("approval").is_some()
                || params.get("format").and_then(serde_json::Value::as_str).is_some()
                || path.is_some_and(|path| {
                    !solidcraft_engine::io::is_design_path(path)
                        && !solidcraft_engine::io::is_step_path(path)
                        && !solidcraft_engine::io::is_mesh_path(path)
                        && !solidcraft_engine::io::is_point_cloud_path(path)
                        && solidcraft_engine::cmd::drawing_import_command(path).is_none()
                })
        }
        _ => false,
    }
}

/// Route namespaced entries through the existing approval gate and dedicated transaction worker.
pub(crate) fn normalize_request(session: &solidcraft_engine::Session, method: &str, params: &mut serde_json::Value) -> Result<(), String> {
    fn normalize(session: &solidcraft_engine::Session, step: &mut serde_json::Value) -> Result<(), String> {
        let Some(command) = step.get("command").or_else(|| step.get("id")).and_then(serde_json::Value::as_str) else { return Ok(()) };
        let inputs = step.get("params").unwrap_or(&serde_json::Value::Null);
        let Some(resolved) = solidcraft_engine::plugin::resolve_call(session, command, inputs).map_err(|e| e.to_string())? else { return Ok(()) };
        if let Some(step) = step.as_object_mut() {
            step.remove("id");
            step.insert("command".into(), "plugin.run".into());
            step.insert("params".into(), resolved);
        }
        Ok(())
    }
    match method {
        "engine.execute" | "command" => normalize(session, params),
        "engine.batch" | "engine.script" => {
            let steps = if params.is_array() { params.as_array_mut() } else { params.get_mut("commands").and_then(serde_json::Value::as_array_mut) };
            if let Some(steps) = steps {
                for step in steps.iter_mut().take(10_000) {
                    normalize(session, step)?;
                }
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

type FormatList = Vec<(solidcraft_engine::plugin::EntryKind, solidcraft_engine::plugin::PluginFormat)>;

/// The host owns all file-format UI; packages supply data only.
#[derive(Default)]
pub struct PluginIo {
    pub(crate) formats: FormatList,
    catalogue: Option<std::sync::mpsc::Receiver<FormatList>>,
    refreshed: Option<std::time::Instant>,
    pending: Option<FormatPrompt>,
    report: Option<serde_json::Value>,
}

struct FormatPrompt {
    format: solidcraft_engine::plugin::PluginFormat,
    export: bool,
    options: String,
    error: Option<String>,
    siblings: bool,
    fields: std::collections::BTreeMap<String, String>,
}

/// Discover packages beside the window. Even a large component is never read on the UI thread.
pub(crate) fn formats_loading(app: &SolveApp) -> bool {
    app.plugin_io.catalogue.is_some()
}

pub(crate) fn refresh_formats(app: &mut SolveApp, ctx: &egui::Context) {
    sync_policy(app);
    if let Some(rx) = &app.plugin_io.catalogue {
        match rx.try_recv() {
            Ok(formats) => {
                app.plugin_io.formats = formats;
                app.plugin_io.catalogue = None;
                app.plugin_io.refreshed = Some(std::time::Instant::now());
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                app.plugin_io.catalogue = None;
                app.plugin_io.refreshed = Some(std::time::Instant::now());
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(50)),
        }
        return;
    }
    if cfg!(target_arch = "wasm32") || app.plugin_io.refreshed.is_some_and(|t| t.elapsed().as_secs() < 3) {
        return;
    }
    let mut session = solidcraft_engine::Session::default();
    session.plugin_state = app.session.plugin_state.clone();
    session.disabled_plugins = app.session.disabled_plugins.clone();
    session.plugin_limits = app.session.plugin_limits;
    let (tx, rx) = std::sync::mpsc::channel();
    let repaint = ctx.clone();
    let spawned = std::thread::Builder::new().name("plugin-formats".into()).spawn(move || {
        let mut formats = Vec::new();
        for kind in [solidcraft_engine::plugin::EntryKind::Importer, solidcraft_engine::plugin::EntryKind::Exporter] {
            formats.extend(solidcraft_engine::plugin::formats(&session, kind).into_iter().map(|format| (kind, format)));
        }
        let _ = tx.send(formats);
        repaint.request_repaint();
    });
    if spawned.is_ok() {
        app.plugin_io.catalogue = Some(rx);
    } else {
        app.plugin_io.refreshed = Some(std::time::Instant::now());
    }
}

/// A menu click opens a host approval form. Programmatic commands never call this path.
pub(crate) fn choose_format(app: &mut SolveApp, id: &str, export: bool) {
    let action = if export { crate::toolbar::FileAction::Command("file.export") } else { crate::toolbar::FileAction::Open };
    if !action.enabled(app) || app.plugin_io.pending.is_some() {
        return;
    }
    let kind = if export { solidcraft_engine::plugin::EntryKind::Exporter } else { solidcraft_engine::plugin::EntryKind::Importer };
    let Some((_, format)) = app.plugin_io.formats.iter().find(|(k, f)| *k == kind && f.id == id) else { return };
    if !enabled(app, &format.plugin_id) {
        return;
    }
    app.plugin_io.pending =
        Some(FormatPrompt { format: format.clone(), export, options: "{}".into(), error: None, siblings: false, fields: Default::default() });
}

fn pick_format(app: &mut SolveApp, prompt: FormatPrompt, options: serde_json::Value) {
    let extensions: Vec<&str> = prompt.format.extensions.iter().map(String::as_str).collect();
    let params = serde_json::json!({"format": prompt.format.id, "approval": prompt.format.approval, "options": options});
    let export = prompt.export;
    let sibling_names: Vec<String> =
        if prompt.siblings { prompt.format.permissions.files.iter().filter(|p| p.read).map(|p| p.name.clone()).collect() } else { Vec::new() };
    let complete = move |app: &mut SolveApp, path: Option<String>| {
        let Some(path) = path else { return };
        sync_policy(app);
        let mut request =
            crate::file_load::Request::new(&path, if export { crate::file_load::Kind::ExportPlugin } else { crate::file_load::Kind::Open });
        request.plugin_import = !export;
        request.options = params.as_object().cloned().unwrap_or_default();
        if let Some(parent) = std::path::Path::new(&path).parent() {
            request.plugin_grants = sibling_names
                .iter()
                .map(|name| solidcraft_engine::plugin::FileGrant { name: name.clone(), folder: parent.to_path_buf(), read: true, write: false })
                .collect();
        }
        let _ = crate::file_load::begin_request(app, request);
    };
    if export {
        let name = format!("{}.{}", app.session.doc.name, extensions.first().copied().unwrap_or("dat"));
        app.pick_save(&name, &extensions, complete);
    } else {
        app.pick_open_for(&extensions, complete);
    }
}

pub(crate) fn record_loss_report(app: &mut SolveApp, result: &serde_json::Value) {
    if let Some(report) = result.get("loss_report").filter(|r| r.is_object()) {
        app.plugin_io.report = Some(report.clone());
    }
}

fn simple_properties(prompt: &FormatPrompt) -> Option<&serde_json::Map<String, serde_json::Value>> {
    let properties = prompt.format.input_schema.get("properties")?.as_object()?;
    (properties.len() <= 128
        && properties
            .values()
            .all(|p| matches!(p.get("type").and_then(serde_json::Value::as_str), Some("string" | "number" | "integer" | "boolean"))))
    .then_some(properties)
}

fn draw_options(prompt: &mut FormatPrompt, ui: &mut egui::Ui, app: &SolveApp) {
    let Some(properties) = simple_properties(prompt).cloned() else {
        if prompt.format.input_schema.get("properties").is_none() && prompt.format.input_schema.get("required").is_none() {
            return;
        }
        crate::ui_kit::text(ui, app.tr("plugin_io.options_schema"), crate::ui_kit::Text::Note("schema"));
        egui::ScrollArea::vertical().max_height(crate::ui_kit::row_height() * 5.0).show(ui, |ui| {
            ui.label(prompt.format.input_schema.to_string());
        });
        crate::ui_kit::text_row(ui, app.tr("plugin_io.options"), &mut prompt.options, "");
        return;
    };
    for (name, schema) in properties {
        let label = schema.get("title").and_then(serde_json::Value::as_str).unwrap_or(&name);
        let text = prompt
            .fields
            .entry(name.clone())
            .or_insert_with(|| schema.get("default").map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())).unwrap_or_default());
        if schema.get("type").and_then(serde_json::Value::as_str) == Some("boolean") {
            let mut checked = text == "true";
            if ui.checkbox(&mut checked, label).changed() {
                *text = checked.to_string();
            }
        } else {
            crate::ui_kit::text_row(ui, label, text, schema.get("description").and_then(serde_json::Value::as_str).unwrap_or_default());
        }
    }
}

fn read_options(prompt: &FormatPrompt) -> Result<serde_json::Value, ()> {
    let Some(properties) = simple_properties(prompt) else {
        if prompt.options.len() > solidcraft_engine::plugin::MAX_JSON_BYTES {
            return Err(());
        }
        let value: serde_json::Value = serde_json::from_str(&prompt.options).map_err(|_| ())?;
        return value.is_object().then_some(value).ok_or(());
    };
    let mut options = serde_json::Map::new();
    for (name, schema) in properties {
        let required = prompt
            .format
            .input_schema
            .get("required")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|a| a.iter().any(|v| v.as_str() == Some(name)));
        let value = prompt.fields.get(name).map(String::as_str).unwrap_or_default();
        if value.is_empty() && !required {
            continue;
        }
        let value = match schema.get("type").and_then(serde_json::Value::as_str) {
            Some("string") => serde_json::Value::String(value.into()),
            Some("boolean") => serde_json::Value::Bool(value == "true"),
            Some("integer") => {
                value.parse::<i64>().map(serde_json::Value::from).or_else(|_| value.parse::<u64>().map(serde_json::Value::from)).map_err(|_| ())?
            }
            Some("number") => serde_json::Number::from_f64(value.parse::<f64>().map_err(|_| ())?).map(serde_json::Value::Number).ok_or(())?,
            _ => return Err(()),
        };
        options.insert(name.clone(), value);
    }
    let value = serde_json::Value::Object(options);
    if value.to_string().len() > solidcraft_engine::plugin::MAX_JSON_BYTES {
        return Err(());
    }
    Ok(value)
}

/// Permission review, options and losses use host widgets and the ordinary themed window.
pub(crate) fn show_file_formats(app: &mut SolveApp, ctx: &egui::Context) {
    if let Some(mut prompt) = app.plugin_io.pending.take() {
        let (closed, action) = crate::ui_kit::Confirm::new(egui::Id::unique("plugin_io_approval"), "import", app.tr("plugin_io.approval"))
            .actions(app.tr("plugin_io.allow_choose"), app.tr("ui.cancel"), None)
            .show(ctx, true, |ui| {
                crate::ui_kit::text(ui, &prompt.format.name, crate::ui_kit::Text::Report("format", false));
                crate::ui_kit::text(ui, app.tr("plugin_io.permission_note"), crate::ui_kit::Text::Note("permissions"));
                if prompt.format.permissions.clock {
                    ui.label(app.tr("plugin_io.clock"));
                }
                for permission in &prompt.format.permissions.files {
                    let access = app.tr(if permission.write { "plugin_io.folder_write" } else { "plugin_io.folder_read" });
                    ui.label(solidcraft_ui_text::format("plugin_io.folder_permission", &[("name", &permission.name), ("access", access)]));
                }
                if !prompt.format.permissions.clock && prompt.format.permissions.files.is_empty() {
                    ui.label(app.tr("plugin_io.no_permissions"));
                }
                if !prompt.export && prompt.format.permissions.files.iter().any(|p| p.read) {
                    ui.checkbox(&mut prompt.siblings, app.tr("plugin_io.siblings"));
                }
                draw_options(&mut prompt, ui, app);
                if let Some(error) = &prompt.error {
                    crate::ui_kit::text(ui, error, crate::ui_kit::Text::Error("options"));
                }
            });
        if action == crate::ui_kit::Action::Ok {
            match read_options(&prompt) {
                Ok(options) => pick_format(app, prompt, options),
                Err(()) => {
                    prompt.error = Some(app.tr("plugin_io.options_invalid").to_string());
                    app.plugin_io.pending = Some(prompt);
                }
            }
        } else if !closed && action != crate::ui_kit::Action::Cancel {
            app.plugin_io.pending = Some(prompt);
        }
    }
    if let Some(report) = app.plugin_io.report.take() {
        let (closed, action) = crate::ui_kit::Window::new(egui::Id::unique("plugin_io_report"), "inspect", app.tr("plugin_io.report"))
            .fields()
            .actions("", app.tr("ui.close"), None)
            .show(ctx, true, |ui| {
                if let Some(counts) = report.get("counts").and_then(serde_json::Value::as_object) {
                    for (name, count) in counts {
                        crate::ui_kit::row(ui, name, |ui| crate::ui_kit::text(ui, &count.to_string(), crate::ui_kit::Text::Cell));
                    }
                }
                egui::ScrollArea::vertical().max_height(crate::ui_kit::row_height() * 12.0).show(ui, |ui| {
                    if let Some(items) = report.get("items").and_then(serde_json::Value::as_array) {
                        for item in items {
                            let code = item.get("code").and_then(serde_json::Value::as_str).unwrap_or_default();
                            let message = item.get("message").and_then(serde_json::Value::as_str).unwrap_or_default();
                            let name = item.get("item").and_then(serde_json::Value::as_str).unwrap_or_default();
                            crate::ui_kit::text(ui, &format!("{name} [{code}]: {message}"), crate::ui_kit::Text::Report("loss", false));
                        }
                    }
                });
            });
        if !closed && action != crate::ui_kit::Action::Cancel {
            app.plugin_io.report = Some(report);
        }
    }
}

/// How many turned-off ids a preferences file may name.
pub const MAX_DISABLED: usize = 256;
/// `plugin.json` larger than this is refused.
const MAX_MANIFEST_BYTES: u64 = 16 * 1024;

/// One installed plugin, as its manifest describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plugin {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
}

/// A folder under the plugins directory that is not a plugin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skipped {
    pub folder: String,
    pub reason: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    id: String,
    name: String,
    version: String,
    #[serde(default)]
    description: String,
}

/// The plugins directory: `$XDG_DATA_HOME/solidcraft/plugins` (and the platform equivalents of
/// [`solidcraft_engine::recovery::data_dir`]). `None` when that folder cannot be named.
pub fn plugins_dir() -> Option<PathBuf> {
    solidcraft_engine::recovery::data_dir().map(|d| d.join("plugins"))
}

/// An id a folder and a manifest may share: 1–64 characters, lowercase letters, digits and
/// hyphens, not starting or ending with a hyphen.
pub fn valid_id(id: &str) -> bool {
    let b = id.as_bytes();
    (1..=64).contains(&b.len())
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
        && b[0] != b'-'
        && *b.last().unwrap_or(&b'-') != b'-'
}

fn plain(text: &str, max: usize) -> bool {
    !text.is_empty() && text.chars().count() <= max && text.chars().all(|c| !c.is_control())
}

fn check(m: Manifest, folder: &str) -> Result<Plugin, String> {
    if m.id != folder {
        return Err(format!("plugin.json id `{}` does not match the folder `{folder}`", m.id));
    }
    if !valid_id(&m.id) {
        return Err("the id must be 1–64 lowercase letters, digits and hyphens".into());
    }
    let name = m.name.trim();
    if !plain(name, 80) {
        return Err("the name must be 1–80 characters with no line breaks".into());
    }
    let version = m.version.trim();
    if !plain(version, 32) || !version.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+')) {
        return Err("the version must be 1–32 letters, digits, dots, hyphens or plus signs".into());
    }
    let description = m.description.trim();
    if description.chars().count() > 240 || description.chars().any(|c| c.is_control()) {
        return Err("the description must be at most 240 characters with no line breaks".into());
    }
    Ok(Plugin { id: m.id, name: name.to_string(), version: version.to_string(), description: description.to_string() })
}

fn read_plugin(dir: &Path, folder: &str) -> Result<Plugin, String> {
    let path = dir.join("plugin.json");
    let meta = std::fs::metadata(&path).map_err(|_| "no plugin.json".to_string())?;
    if !meta.is_file() || meta.len() > MAX_MANIFEST_BYTES {
        return Err("plugin.json is missing or larger than 16 KiB".into());
    }
    let text = std::fs::read_to_string(&path).map_err(|e| format!("plugin.json could not be read ({e})"))?;
    let parsed = solidcraft_engine::plugin::parse_manifest(text.as_bytes(), folder).map_err(|e| e.to_string())?;
    check(Manifest { id: parsed.id, name: parsed.name, version: parsed.version, description: parsed.description }, folder)
}

/// The plugins in `dir`, and the folders that were not. A missing directory is an empty list.
pub fn scan(dir: &Path) -> (Vec<Plugin>, Vec<Skipped>) {
    let mut plugins = Vec::new();
    let mut skipped = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (plugins, skipped);
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else { continue };
        if !kind.is_dir() {
            continue;
        }
        let Some(folder) = entry.file_name().into_string().ok() else {
            skipped.push(Skipped { folder: String::new(), reason: "the folder name is not text".into() });
            continue;
        };
        if folder.starts_with('.') {
            continue;
        }
        match read_plugin(&entry.path(), &folder) {
            Ok(plugin) => plugins.push(plugin),
            Err(reason) => skipped.push(Skipped { folder, reason }),
        }
    }
    plugins.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
    skipped.sort_by(|a, b| a.folder.cmp(&b.folder));
    (plugins, skipped)
}

/// Exact integer controls retain all u64 fuel values, without a float slider conversion.
pub(crate) struct LimitInputs {
    applied: solidcraft_engine::plugin::HostLimits,
    fuel: String,
    memory: String,
}
impl From<solidcraft_engine::plugin::HostLimits> for LimitInputs {
    fn from(applied: solidcraft_engine::plugin::HostLimits) -> Self {
        Self { fuel: applied.max_fuel.to_string(), memory: applied.max_memory_mib.to_string(), applied }
    }
}
pub(crate) fn host_limits(p: &crate::prefs::Prefs) -> solidcraft_engine::plugin::HostLimits {
    solidcraft_engine::plugin::HostLimits { max_fuel: p.plugin_max_fuel, max_memory_mib: p.plugin_max_memory_mib }
}
pub(crate) fn sync_policy(app: &mut SolveApp) {
    app.session.disabled_plugins.clone_from(&app.preferences.disabled_plugins);
    app.session.plugin_limits = host_limits(&app.preferences);
}
fn parsed_limits(input: &LimitInputs) -> Option<solidcraft_engine::plugin::HostLimits> {
    let limits =
        solidcraft_engine::plugin::HostLimits { max_fuel: input.fuel.trim().parse().ok()?, max_memory_mib: input.memory.trim().parse().ok()? };
    limits.validate().ok()?;
    Some(limits)
}
pub(crate) fn resource_pending(app: &SolveApp) -> bool {
    let current = host_limits(&app.preferences);
    app.prefs_window.plugin_limit_inputs.as_ref().is_some_and(|input| {
        input.applied == current && (input.fuel != current.max_fuel.to_string() || input.memory != current.max_memory_mib.to_string())
    })
}
/// Enter, Apply and OK commit both exact integers together. An incomplete edit cannot save a prefix.
pub(crate) fn commit_resource_inputs(app: &mut SolveApp) -> Result<(), String> {
    let Some(input) = app.prefs_window.plugin_limit_inputs.take() else { return Ok(()) };
    // A control client has already changed the preferences: never overwrite that newer policy.
    if input.applied != host_limits(&app.preferences) {
        return Ok(());
    }
    let Some(limits) = parsed_limits(&input) else {
        app.prefs_window.plugin_limit_inputs = Some(input);
        return Err(app.tr("prefs.plugin_limits_invalid").to_string());
    };
    app.preferences.plugin_max_fuel = limits.max_fuel;
    app.preferences.plugin_max_memory_mib = limits.max_memory_mib;
    app.prefs_window.plugin_limit_inputs = Some(limits.into());
    sync_policy(app);
    Ok(())
}
fn resource_controls(app: &mut SolveApp, ui: &mut egui::Ui) {
    let current = host_limits(&app.preferences);
    let mut input = app.prefs_window.plugin_limit_inputs.take().filter(|v| v.applied == current).unwrap_or_else(|| current.into());
    let (fuel_enter, _) = crate::ui_kit::text_row(ui, app.tr("prefs.plugin_max_fuel"), &mut input.fuel, "");
    let (memory_enter, _) = crate::ui_kit::text_row(ui, app.tr("prefs.plugin_max_memory"), &mut input.memory, "");
    if parsed_limits(&input).is_none() {
        crate::ui_kit::text(ui, app.tr("prefs.plugin_limits_invalid"), crate::ui_kit::Text::Error("plugin_limits"));
    }
    crate::ui_kit::text(ui, app.tr("prefs.plugin_limits_note"), crate::ui_kit::Text::Hint("plugin_limits"));
    app.prefs_window.plugin_limit_inputs = Some(input);
    if (fuel_enter || memory_enter)
        && let Err(error) = commit_resource_inputs(app)
    {
        app.set_status(error, true);
    }
    ui.separator();
}

/// Whether an installed plugin is offered. Unknown ids are on; an id in `disabled_plugins` is off.
pub fn enabled(app: &SolveApp, id: &str) -> bool {
    !app.preferences.disabled_plugins.contains(id)
}

fn matches_query(plugin: &Plugin, query: &str) -> bool {
    let query = query.trim();
    if query.is_empty() {
        return true;
    }
    let query = query.to_lowercase();
    [plugin.name.as_str(), plugin.id.as_str(), plugin.description.as_str(), plugin.version.as_str()]
        .iter()
        .any(|text| text.to_lowercase().contains(&query))
}

/// The Plugins page. Desktop only: the browser build has no per-user plugin folder.
pub fn page(app: &mut SolveApp, ui: &mut egui::Ui) {
    let dir = if cfg!(target_arch = "wasm32") { None } else { plugins_dir() };
    page_in_dir(app, ui, dir.as_deref());
}

fn page_in_dir(app: &mut SolveApp, ui: &mut egui::Ui, dir: Option<&Path>) {
    crate::ui_kit::settings_page(ui, |ui| {
        crate::ui_kit::settings_heading(ui, solidcraft_ui_text::tr("ui.plugins"), solidcraft_ui_text::tr("prefs.plugins_intro"));
        let Some(dir) = dir else {
            ui.label(solidcraft_ui_text::tr("ui.plugins_desktop"));
            return;
        };
        resource_controls(app, ui);
        let t = solidcraft_theme::Tokens::get();
        let path = dir.display().to_string();
        let searching = crate::ui_kit::settings_search::querying(ui);
        // The page's own box is for browsing Plugins. The dialog search is the filter while it is active.
        if !searching {
            crate::ui_kit::search_field(
                ui,
                ui.scope_id().with("plugins_search"),
                &mut app.prefs_window.plugin_query,
                solidcraft_ui_text::tr("ui.plugins_search"),
            );
            ui.add_space(8.0);
        }
        let (plugins, skipped) = scan(dir);
        let query = app.prefs_window.plugin_query.clone();
        crate::ui_kit::settings_list_section(
            ui,
            "plugins_installed",
            solidcraft_ui_text::tr("prefs.plugins_installed"),
            Some(&plugins.len().to_string()),
            |ui| {
                if plugins.is_empty() && (!searching || crate::ui_kit::settings_search::shows_all(ui)) {
                    crate::ui_kit::text(ui, solidcraft_ui_text::tr("ui.plugins_empty"), crate::ui_kit::Text::Hint("plugins_empty"));
                }
                ui.spacing_mut().item_spacing.y = t.space_8;
                let mut shown = 0usize;
                for plugin in plugins.iter().filter(|plugin| searching || matches_query(plugin, &query)) {
                    if searching
                        && !crate::ui_kit::settings_search::shows_all(ui)
                        && !crate::ui_kit::settings_search::texts_hit(ui, &[&plugin.name, &plugin.id, &plugin.description, &plugin.version])
                    {
                        continue;
                    }
                    shown += 1;
                    if searching {
                        crate::ui_kit::settings_search::reveal(ui);
                        crate::ui_kit::settings_search::push_force(ui);
                    }
                    plugin_card(app, ui, dir, plugin);
                    if searching {
                        crate::ui_kit::settings_search::pop_force(ui);
                    }
                }
                if !searching && !plugins.is_empty() && shown == 0 {
                    ui.label(solidcraft_ui_text::tr("ui.plugins_none"));
                }
            },
        );
        if !searching && query.trim().is_empty() && !skipped.is_empty() {
            ui.add_space(6.0);
            ui.label(
                RichText::new(solidcraft_ui_text::format("ui.plugins_skipped", &[("count", &skipped.len().to_string())]))
                    .size(t.font_small)
                    .color(t.text_dim),
            );
            for skip in skipped.iter().take(8) {
                let who = if skip.folder.is_empty() { "folder" } else { skip.folder.as_str() };
                ui.label(RichText::new(format!("{who}: {}", skip.reason)).size(t.font_small).color(t.text_dim));
            }
        }
        if !searching {
            ui.horizontal(|ui| {
                let button = crate::ui_kit::icon_label_button(ui, solidcraft_ui_text::tr("prefs.plugins_show_folder"), "folder", true, false)
                    .backing_on_hover_text(solidcraft_ui_text::format("ui.plugins_note", &[("path", &path)]));
                crate::scenario::publish_target("prefs:plugins:folder", button.rect);
                if button.clicked() {
                    if let Err(e) = std::fs::create_dir_all(dir) {
                        app.set_status(format!("Could not create the plugins folder: {e}"), true);
                    } else if let Some(show) = &app.services.show_in_folder {
                        // The service opens the parent of the supplied file, even if it is absent.
                        show(&dir.join("plugin.json").display().to_string());
                    }
                }
            });
        }
    });
}

fn plugin_card(app: &mut SolveApp, ui: &mut egui::Ui, dir: &Path, plugin: &Plugin) {
    use crate::ui_kit;
    let mut on = enabled(app, &plugin.id);
    let was_on = on;
    let labels = ui_kit::PluginCardLabels {
        show_in_folder: solidcraft_ui_text::tr_label("Show in folder"),
        enabled: solidcraft_ui_text::tr("prefs.plugins_enabled"),
    };
    let card = ui
        .push_id(&plugin.id, |ui| ui_kit::settings_plugin_card(ui, &plugin.id, &plugin.name, &plugin.version, &plugin.description, &mut on, &labels))
        .inner;
    let response = card.response;
    let (folder, check) = card.inner;
    crate::scenario::publish_target(&format!("prefs:plugin:{}:folder", plugin.id), folder.rect);
    crate::scenario::publish_target(&format!("prefs:plugin:{}", plugin.id), check.rect);
    if folder.clicked()
        && let Some(show) = &app.services.show_in_folder
    {
        show(&dir.join(&plugin.id).join("plugin.json").display().to_string());
    }
    if was_on != on {
        if on {
            app.preferences.disabled_plugins.remove(&plugin.id);
        } else if app.preferences.disabled_plugins.len() < MAX_DISABLED {
            app.preferences.disabled_plugins.insert(plugin.id.clone());
        }
    }
    crate::scenario::publish_rect(&format!("prefs:plugin:{}:card", plugin.id), response.rect);
    let texts = [&plugin.name, &plugin.id, &plugin.description, &plugin.version];
    if let Some(text) = texts.iter().find(|text| crate::ui_kit::settings_search::texts_hit(ui, &[*text])) {
        crate::ui_kit::settings_search::remember(ui, text, response.rect);
    }
    for text in texts {
        crate::ui_kit::settings_search::follow(ui, text, response.rect);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_format() -> solidcraft_engine::plugin::PluginFormat {
        solidcraft_engine::plugin::PluginFormat {
            id: "sample.read".into(),
            plugin_id: "sample".into(),
            entry: "read".into(),
            name: "Sample format".into(),
            extensions: vec!["sample".into()],
            approval: "approved-content-hash".into(),
            input_schema: serde_json::json!({"type":"object"}),
            permissions: Default::default(),
        }
    }

    #[test]
    fn plugin_export_classifier_preserves_legacy_filename_formats_and_null_options() {
        for params in [
            serde_json::json!({"path":"part.stl","format":"part.stl"}),
            serde_json::json!({"path":"part.stl","format":"part.stl","approval":null,"options":null}),
        ] {
            assert!(!is_runtime_request("file.export", &params));
        }
        assert!(is_runtime_request("file.export", &serde_json::json!({"path":"part.stl","format":"sample.stl","approval":"hash"})));
        assert!(is_runtime_request("file.export", &serde_json::json!({"path":"part.sample","approval":"hash"})));
        assert!(is_runtime_request("FILE.EXPORT", &serde_json::json!({"path":"part.sample"})));
    }

    #[test]
    fn plugin_options_form_keeps_values_typed_and_refuses_nonfinite_numbers() {
        let mut format = file_format();
        format.input_schema = serde_json::json!({"type":"object","properties":{"scale":{"type":"number"},"merge":{"type":"boolean"},"label":{"type":"string"},"count":{"type":"integer"}},"required":["scale"]});
        let mut prompt = FormatPrompt { format, export: false, options: "{}".into(), error: None, siblings: false, fields: Default::default() };
        prompt.fields.insert("scale".into(), "2.5".into());
        prompt.fields.insert("merge".into(), "true".into());
        prompt.fields.insert("count".into(), u64::MAX.to_string());
        assert_eq!(read_options(&prompt).unwrap(), serde_json::json!({"scale":2.5,"merge":true,"count":u64::MAX}));
        prompt.fields.insert("scale".into(), "NaN".into());
        assert!(read_options(&prompt).is_err());
    }

    #[test]
    fn plugin_file_format_requires_host_review_before_the_picker() {
        let mut h = crate::scenario::Harness::new();
        h.app.plugin_io.formats.push((solidcraft_engine::plugin::EntryKind::Importer, file_format()));
        choose_format(&mut h.app, "sample.read", false);
        assert!(h.app.plugin_io.pending.is_some());
        assert!(h.app.file_picker.is_none(), "permission review precedes file access");
        h.app.plugin_io.pending = None;
        h.app.preferences.disabled_plugins.insert("sample".into());
        choose_format(&mut h.app, "sample.read", false);
        assert!(h.app.plugin_io.pending.is_none());
        h.app.preferences.disabled_plugins.clear();
        h.app.issue_report.draft = Some(crate::issue_report::draft_for(&h.app, None, None));
        choose_format(&mut h.app, "sample.read", false);
        assert!(h.app.plugin_io.pending.is_none(), "file actions wait behind the issue report");
    }

    #[test]
    fn plugin_file_format_picker_receives_declared_extensions_and_cancel_changes_nothing() {
        let mut h = crate::scenario::Harness::new();
        let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let captured = seen.clone();
        h.app.services.pick_open = Some(Box::new(move |extensions| {
            *captured.borrow_mut() = extensions.iter().map(|e| e.to_string()).collect::<Vec<_>>();
            Box::pin(std::future::ready(None))
        }));
        let before = h.app.session.doc.to_json();
        pick_format(
            &mut h.app,
            FormatPrompt { format: file_format(), export: false, options: "{}".into(), error: None, siblings: false, fields: Default::default() },
            serde_json::json!({}),
        );
        assert_eq!(*seen.borrow(), ["sample"]);
        crate::file_picker::tick(&mut h.app, &h.ctx);
        assert!(h.app.file_picker.is_none());
        assert!(h.app.file_load.idle());
        assert_eq!(h.app.session.doc.to_json(), before);
        assert!(h.app.session.plugin_state.file_grants.is_empty());
    }

    #[test]
    fn plugin_loss_report_draws_structured_counts_and_item_messages() {
        let mut h = crate::scenario::Harness::new();
        record_loss_report(
            &mut h.app,
            &serde_json::json!({"loss_report":{"counts":{"bodies_read":1},"items":[{"item":"Body 1","code":"material-skipped","message":"Material was unavailable"}]}}),
        );
        h.frames(3);
        assert!(h.widget("Import loss report").is_some());
        assert!(h.widget("Body 1 [material-skipped]: Material was unavailable").is_some());
        if let Ok(dir) = std::env::var("SOLIDCRAFT_PLUGIN_IO_EVIDENCE") {
            std::fs::create_dir_all(&dir).unwrap();
            let ctx = crate::fonts::test_context();
            let mut textures = std::collections::HashMap::new();
            for name in ["loss-report", "approval"] {
                if name == "approval" {
                    h.app.plugin_io.report = None;
                    h.app.plugin_io.pending = Some(FormatPrompt {
                        format: file_format(),
                        export: false,
                        options: "{}".into(),
                        error: None,
                        siblings: false,
                        fields: Default::default(),
                    });
                }
                let mut output = egui::FullOutput::default();
                for _ in 0..3 {
                    output = ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 700.0))),
                            focused: true,
                            ..Default::default()
                        },
                        |ui| {
                            h.app.logic(ui.ctx());
                            h.app.ui(ui);
                        },
                    );
                    crate::ui_kit::test_textures_update(&mut textures, &output.textures_delta);
                    output.textures_delta.clear();
                }
                crate::ui_kit::test_raster(&ctx, output.shapes.clone(), output.pixels_per_point, ctx.content_rect(), &textures)
                    .save(std::path::Path::new(&dir).join(format!("{name}.png")))
                    .unwrap();
                let png = std::fs::read(std::path::Path::new(&dir).join(format!("{name}.png"))).unwrap();
                let hex: String = png.iter().map(|byte| format!("{byte:02x}")).collect();
                println!("PLUGIN_IO_EVIDENCE {name} {hex}");
            }
        }
    }

    fn page_frame(app: &mut SolveApp, ctx: &egui::Context, dir: &Path, events: Vec<egui::Event>) {
        crate::scenario::clear_handles();
        let mut output = crate::test_support::frame(
            ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(700.0, 800.0))),
                events,
                focused: true,
                ..Default::default()
            },
            |ui| crate::ui_kit::scope(ui, |ui| page_in_dir(app, ui, Some(dir))),
        );
        output.textures_delta.clear();
    }

    fn click_page(app: &mut SolveApp, ctx: &egui::Context, dir: &Path, handle: &str) {
        let at = crate::scenario::handle_at(handle).unwrap();
        page_frame(app, ctx, dir, crate::test_support::pointer(at, true));
        page_frame(app, ctx, dir, crate::test_support::pointer(at, false));
    }

    #[test]
    fn plugins_production_composition_publishes_page_heading_and_section_geometry() {
        let previous = solidcraft_theme::is_dark();
        for dark in [false, true] {
            solidcraft_theme::set_dark(dark);
            let mut h = crate::scenario::Harness::new();
            let dir = h.dir.join("page-geometry");
            let ctx = crate::fonts::test_context();
            for _ in 0..3 {
                page_frame(&mut h.app, &ctx, &dir, vec![]);
            }
            // The injected path goes through the production composition, including its
            // heading and settings_page scope; the fixture adds only the window's form style.
            let heading = crate::scenario::rect_of("prefs:page_heading").unwrap();
            let section = crate::scenario::rect_of("prefs:section:plugins_installed").unwrap();
            let body = crate::scenario::rect_of("prefs:section:plugins_installed:body").unwrap();
            let t = solidcraft_theme::Tokens::get();
            assert!(heading.height() >= t.space_24 && heading.bottom() < section.top());
            assert!((section.left() - heading.left()).abs() <= 1.0 && (section.right() - heading.right()).abs() <= 1.0);
            assert!((section.height() - (t.control_height + t.space_2)).abs() <= 1.0);
            assert!((body.top() - section.bottom() - t.space_10).abs() <= 1.0);
            assert!(!dir.exists(), "production layout does not create the plugin folder");
        }
        solidcraft_theme::set_dark(previous);
    }

    #[test]
    fn plugin_card_actions_share_a_row_at_normal_width_and_stay_inside_narrow_cards() {
        let previous = solidcraft_theme::is_dark();
        for dark in [false, true] {
            solidcraft_theme::set_dark(dark);
            for language in ["en", "de"] {
                let _language = solidcraft_ui_text::scope(language);
                for width in [646.0, 200.0] {
                    let mut h = crate::scenario::Harness::new();
                    let dir = h.dir.join("plugin-action-layout");
                    write(&dir, "public-one", r#"{"id":"public-one","name":"Public one","version":"1.2.3","description":"Public fixture."}"#);
                    let ctx = crate::fonts::test_context();
                    let mut clip = egui::Rect::NOTHING;
                    let mut painted = Vec::new();
                    for _ in 0..3 {
                        crate::scenario::clear_handles();
                        let mut output = crate::test_support::frame(
                            &ctx,
                            egui::RawInput {
                                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width + 16.0, 800.0))),
                                ..Default::default()
                            },
                            |ui| {
                                ui.set_width(width);
                                clip = ui.clip_rect();
                                crate::ui_kit::scope(ui, |ui| page_in_dir(&mut h.app, ui, Some(&dir)));
                            },
                        );
                        output.textures_delta.clear();
                        painted = output.shapes;
                    }
                    let card = crate::scenario::rect_of("prefs:plugin:public-one:card").unwrap();
                    let folder = crate::scenario::rect_of("prefs:plugin:public-one:folder").unwrap();
                    let checkbox = crate::scenario::rect_of("prefs:plugin:public-one").unwrap();
                    assert!(card.width() <= width + 1.0, "{language}, {width}: card stays within the page");
                    for action in [folder, checkbox] {
                        assert!(card.expand(1.0).contains_rect(action), "{language}, {width}: action stays inside its card");
                        assert!(clip.contains_rect(action), "{language}, {width}: action is not clipped");
                    }
                    if width == 646.0 {
                        assert!((folder.center().y - checkbox.center().y).abs() <= 1.0, "{language}: normal-width actions share one row");
                        assert!(folder.right() <= checkbox.left(), "actions do not overlap");
                        assert!((card.height() - 62.0).abs() <= 1.0, "36 pt content plus 12 pt padding and 1 pt border on each side");
                        assert_eq!(folder.height(), 24.0);
                        assert_eq!(checkbox.size(), egui::vec2(16.0, 16.0));
                        let t = solidcraft_theme::Tokens::get();
                        let icon = crate::scenario::rect_of("prefs:plugin:public-one:icon").unwrap();
                        assert_eq!(icon.size(), egui::vec2(36.0, 36.0));
                        assert!((icon.left() - card.left() - 13.0).abs() <= 1.0, "12 pt padding inside the border");
                        for control in [icon, folder, checkbox] {
                            assert!((control.center().y - card.center().y).abs() <= 1.0, "all card content is vertically centred");
                        }
                        assert!((checkbox.right() - card.right() + 13.0).abs() <= 1.0, "right padding matches left");
                        let mut quiet = true;
                        let mut semibold_title = false;
                        for shape in &painted {
                            crate::test_support::each_shape(&shape.shape, &mut |shape| match shape {
                                egui::Shape::Rect(rect) if rect.rect == folder && rect.fill != egui::Color32::TRANSPARENT => quiet = false,
                                egui::Shape::Text(text) if text.galley.job.text == "Public one" => {
                                    semibold_title = text
                                        .galley
                                        .job
                                        .sections
                                        .iter()
                                        .all(|section| section.format.font_id == crate::ui_kit::type_face::semibold(t.font_button));
                                }
                                _ => {}
                            });
                        }
                        assert!(quiet, "Show in folder has no button background");
                        assert!(semibold_title, "the title uses the actual 13 pt semibold face");
                    }
                    assert!(enabled(&h.app, "public-one"), "layout does not change plugin state");
                }
            }
        }
        solidcraft_theme::set_dark(previous);
    }

    #[test]
    fn plugin_cards_open_their_own_folder_and_cancel_restores_enabled_state() {
        use std::{cell::RefCell, rc::Rc};
        let previous = solidcraft_theme::is_dark();
        for dark in [false, true] {
            solidcraft_theme::set_dark(dark);
            let mut h = crate::scenario::Harness::new();
            let dir = h.dir.join("public-plugins");
            write(&dir, "public-one", r#"{"id":"public-one","name":"Public one","version":"1.2.3","description":"Public fixture."}"#);
            write(&dir, "public-two", r#"{"id":"public-two","name":"Public two","version":"2","description":"Another fixture."}"#);
            let opened = Rc::new(RefCell::new(Vec::new()));
            let seen = opened.clone();
            h.app.services.show_in_folder = Some(Box::new(move |path| seen.borrow_mut().push(PathBuf::from(path))));
            crate::prefs::control(&mut h.app, &serde_json::json!({"open":"Plugins"})).unwrap();
            let ctx = crate::fonts::test_context();
            for _ in 0..3 {
                page_frame(&mut h.app, &ctx, &dir, vec![]);
            }
            assert!(opened.borrow().is_empty(), "rendering never opens a folder");
            click_page(&mut h.app, &ctx, &dir, "prefs:plugin:public-two:folder");
            assert_eq!(*opened.borrow(), [dir.join("public-two/plugin.json")]);
            click_page(&mut h.app, &ctx, &dir, "prefs:plugin:public-one");
            assert!(!enabled(&h.app, "public-one") && enabled(&h.app, "public-two"));
            crate::prefs::control(&mut h.app, &serde_json::json!({"button":"defaults"})).unwrap();
            assert!(enabled(&h.app, "public-one"));
            page_frame(&mut h.app, &ctx, &dir, vec![]);
            click_page(&mut h.app, &ctx, &dir, "prefs:plugin:public-one");
            crate::prefs::control(&mut h.app, &serde_json::json!({"button":"cancel"})).unwrap();
            assert!(enabled(&h.app, "public-one") && enabled(&h.app, "public-two"));
        }
        solidcraft_theme::set_dark(previous);
    }

    #[test]
    fn show_plugins_folder_creates_only_the_requested_fixture_root() {
        use std::{cell::RefCell, rc::Rc};
        let mut h = crate::scenario::Harness::new();
        let dir = h.dir.join("not-created-yet");
        let opened = Rc::new(RefCell::new(Vec::new()));
        let seen = opened.clone();
        h.app.services.show_in_folder = Some(Box::new(move |path| seen.borrow_mut().push(PathBuf::from(path))));
        let ctx = crate::fonts::test_context();
        for _ in 0..3 {
            page_frame(&mut h.app, &ctx, &dir, vec![]);
        }
        assert!(!dir.exists() && opened.borrow().is_empty());
        click_page(&mut h.app, &ctx, &dir, "prefs:plugins:folder");
        assert!(dir.is_dir());
        assert_eq!(*opened.borrow(), [dir.join("plugin.json")]);
        assert!(!dir.join("plugin.json").exists(), "the reveal service needs only a sentinel path");
    }

    #[test]
    fn plugin_metadata_search_owns_one_navigation_target_and_reveals_the_real_card() {
        let mut h = crate::scenario::Harness::new();
        let dir = h.dir.join("public-plugin-search");
        write(&dir, "public-one", r#"{"id":"public-one","name":"Public one","version":"1.2.3","description":"Unique description."}"#);
        let ctx = crate::fonts::test_context();
        for query in ["Public one", "public-one", "1.2.3", "Unique description"] {
            for _ in 0..3 {
                crate::scenario::clear_handles();
                let mut output = crate::test_support::frame(&ctx, egui::RawInput::default(), |ui| {
                    ui.set_width(646.0);
                    let _search = crate::ui_kit::settings_search::begin(ui, query, "prefs");
                    crate::ui_kit::settings_search::begin_page(ui, crate::prefs::PAGE_PLUGINS, "Extend > Plugins");
                    crate::ui_kit::scope(ui, |ui| page_in_dir(&mut h.app, ui, Some(&dir)));
                    crate::ui_kit::settings_search::store_nav(ui);
                });
                output.textures_delta.clear();
            }
            let (_, _, hits) = crate::ui_kit::settings_search::nav(&ctx);
            assert_eq!(hits.len(), 1, "{query}: one real plugin result");
            assert_eq!(hits[0].page, crate::prefs::PAGE_PLUGINS);
            crate::ui_kit::settings_search::set_reveal(&ctx, Some(crate::ui_kit::settings_search::reveal_for(hits[0].page, &hits[0].label, "prefs")));
            page_frame(&mut h.app, &ctx, &dir, vec![]);
            let card = crate::scenario::rect_of("prefs:plugin:public-one:card").unwrap();
            assert!(crate::scenario::handle_at("prefs:revealed").unwrap().distance(card.center()) <= 1.0);
            assert!(enabled(&h.app, "public-one"), "searching and revealing do not toggle the plugin");
        }
    }

    fn write(dir: &Path, folder: &str, body: &str) {
        let d = dir.join(folder);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("plugin.json"), body).unwrap();
    }

    #[test]
    fn a_folder_with_plugin_json_is_listed_and_a_bad_one_is_skipped() {
        let dir = std::env::temp_dir().join(format!("solidcraft-plugins-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        write(
            &dir,
            "ftc-frc-parts",
            r#"{"id":"ftc-frc-parts","name":"FTC and FRC parts","version":"0.1.0","description":"Commercial robot parts."}"#,
        );
        write(&dir, "broken", r#"{"id":"other","name":"Nope","version":"1"}"#);
        std::fs::create_dir_all(dir.join("notes")).unwrap();
        let (plugins, skipped) = scan(&dir);
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].id, "ftc-frc-parts");
        assert_eq!(plugins[0].name, "FTC and FRC parts");
        assert!(skipped.iter().any(|s| s.folder == "broken" && s.reason.contains("id") && s.reason.contains("package folder")), "{skipped:?}");
        assert!(skipped.iter().any(|s| s.folder == "notes" && s.reason.contains("no plugin.json")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ids_and_manifests_refuse_what_would_not_fit_on_the_page() {
        assert!(valid_id("a"));
        assert!(valid_id("ftc-frc-parts"));
        assert!(!valid_id(""));
        assert!(!valid_id("-a"));
        assert!(!valid_id("a-"));
        assert!(!valid_id("FTC"));
        assert!(!valid_id(&"a".repeat(65)));
        let bad = Manifest { id: "gear".into(), name: "Gear\nbox".into(), version: "1".into(), description: String::new() };
        assert!(check(bad, "gear").is_err());
        let big = Manifest { id: "gear".into(), name: "Gear".into(), version: "1".into(), description: "x".repeat(241) };
        assert!(check(big, "gear").unwrap_err().contains("240"));
    }

    #[test]
    fn turning_a_plugin_off_is_saved_and_restore_defaults_turns_it_back_on() {
        use serde_json::json;
        let mut app = crate::SolveApp::new(solidcraft_engine::Session::default(), crate::Services::default());
        assert!(enabled(&app, "ftc-frc-parts"));
        crate::prefs::set(&mut app, &json!({"disabled_plugins": ["ftc-frc-parts"]})).unwrap();
        assert!(!enabled(&app, "ftc-frc-parts"));
        let err = crate::prefs::set(&mut app, &json!({"disabled_plugins": ["Bad Id"]})).unwrap_err();
        assert!(err.contains("disabled_plugins"), "{err}");
        assert!(!enabled(&app, "ftc-frc-parts"), "a refused list leaves the previous choice");
        let mut other = crate::SolveApp::new(solidcraft_engine::Session::default(), crate::Services::default());
        other.load_prefs(&app.prefs());
        assert!(!enabled(&other, "ftc-frc-parts"));
        crate::prefs::control(&mut app, &json!({"open": "Plugins", "button": "defaults"})).unwrap();
        assert_eq!(app.prefs_window.shell.section, crate::prefs::PAGE_PLUGINS);
        assert!(enabled(&app, "ftc-frc-parts"));
        assert!(crate::prefs::control(&mut app, &json!({"open": "NoSuch"})).is_err());
    }

    #[test]
    fn resource_ceilings_preserve_defaults_precision_persistence_and_page_transactions() {
        use serde_json::json;
        let mut h = crate::scenario::Harness::new();
        let initial = host_limits(&h.app.preferences);
        assert_eq!(initial, solidcraft_engine::plugin::HostLimits::default());
        let before = h.app.session.doc.to_json();
        let changed = json!({"plugin_max_fuel":u64::MAX,"plugin_max_memory_mib":512});
        let reply = h.call("ui.prefs", changed.clone());
        assert_eq!(reply["ok"], true, "{reply}");
        assert_eq!(h.app.preferences.plugin_max_fuel, u64::MAX);
        assert_eq!(h.app.session.plugin_limits.max_memory_mib, 512);
        for bad in [
            json!({"plugin_max_fuel":0}),
            json!({"plugin_max_fuel":-1}),
            json!({"plugin_max_fuel":1.5}),
            json!({"plugin_max_memory_mib":4097}),
            json!({"plugin_max_memory_mib":0}),
        ] {
            assert_eq!(h.call("ui.prefs", bad)["ok"], false);
            assert_eq!(h.app.preferences.plugin_max_fuel, u64::MAX);
        }
        let mut restored = SolveApp::new(solidcraft_engine::Session::default(), Default::default());
        restored.load_prefs(&h.app.prefs());
        assert_eq!(host_limits(&restored.preferences), host_limits(&h.app.preferences));
        assert_eq!(restored.session.plugin_limits.max_fuel, u64::MAX);
        h.call("ui.prefs", json!({"open":"Plugins"}));
        h.call("ui.prefs", json!({"plugin_max_memory_mib":1024}));
        h.call("ui.prefs", json!({"button":"cancel"}));
        assert_eq!(h.app.preferences.plugin_max_memory_mib, 512);
        h.call("ui.prefs", json!({"open":"Plugins","button":"defaults"}));
        assert_eq!(host_limits(&h.app.preferences), initial);
        if let Some(input) = &h.app.prefs_window.plugin_limit_inputs {
            assert_eq!(input.applied, initial);
        }
        h.call("ui.prefs", json!({"button":"cancel"}));
        assert_eq!(h.app.preferences.plugin_max_memory_mib, 512);
        assert_eq!(h.app.session.doc.to_json(), before);
        assert!(h.app.session.undo.is_empty());
        let read = crate::prefs::lenient(&json!({"plugin_max_fuel":0,"plugin_max_memory_mib":99999,"language":"en"}));
        assert_eq!(host_limits(&read), initial);
        assert_eq!(read.language, "en");
    }
    #[test]
    fn resource_text_edits_require_complete_valid_confirmation() {
        use serde_json::json;
        let mut h = crate::scenario::Harness::new();
        h.call("ui.prefs", json!({"open":"Plugins"}));
        let initial = host_limits(&h.app.preferences);
        h.app.prefs_window.plugin_limit_inputs = Some(LimitInputs { applied: initial, fuel: u64::MAX.to_string(), memory: "512".into() });
        assert!(resource_pending(&h.app));
        assert_eq!(h.app.session.plugin_limits, initial, "typing alone does not authorize a higher ceiling");
        assert_eq!(h.call("ui.prefs", json!({}))["result"]["apply_enabled"], true);
        assert_eq!(h.call("ui.prefs", json!({"button":"apply"}))["ok"], true);
        assert_eq!(h.app.session.plugin_limits.max_fuel, u64::MAX);
        assert_eq!(h.app.session.plugin_limits.max_memory_mib, 512);
        let applied = h.app.session.plugin_limits;
        for text in ["", "0", "18446744073709551616", "1.5"] {
            h.app.prefs_window.plugin_limit_inputs = Some(LimitInputs { applied, fuel: text.into(), memory: "1024".into() });
            assert_eq!(h.call("ui.prefs", json!({"button":"ok"}))["ok"], false);
            assert!(h.app.prefs_window.shell.open);
            assert_eq!(h.app.session.plugin_limits, applied);
        }
        h.call("ui.prefs", json!({"button":"cancel"}));
        assert_eq!(h.app.session.plugin_limits, applied, "Cancel retains the last Apply");
        assert!(h.app.prefs_window.plugin_limit_inputs.is_none());
        h.call("ui.prefs", json!({"open":"Plugins","button":"defaults"}));
        assert_eq!(h.app.session.plugin_limits, initial);
        h.call("ui.prefs", json!({"button":"cancel"}));
        assert_eq!(h.app.session.plugin_limits, applied);
    }
    #[test]
    fn search_matches_the_name_the_id_and_the_description() {
        let plugin = Plugin {
            id: "ftc-frc-parts".into(),
            name: "FTC and FRC parts".into(),
            version: "0.1.0".into(),
            description: "Commercial robot parts.".into(),
        };
        assert!(matches_query(&plugin, ""));
        assert!(matches_query(&plugin, " frc "));
        assert!(matches_query(&plugin, "FTC-FRC"));
        assert!(matches_query(&plugin, "commercial"));
        assert!(matches_query(&plugin, "0.1"));
        assert!(!matches_query(&plugin, "bearing"));
    }
}

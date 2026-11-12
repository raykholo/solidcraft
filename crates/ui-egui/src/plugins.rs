//! Preferences › Plugins: packages installed under the per-user data folder.
//!
//! A plugin is a directory of `{data}/plugins/<id>/plugin.json`. The page searches that list.
//! It reads `plugin.json` and records which ids are turned off and which packages the user
//! allowed to run (`plugin_approvals`: each id with the approval hash of the exact content
//! allowed). Listing, installing or enabling a package never executes its code. `plugin.run`
//! and importers (`file.import`) use the native component host and honour both.

use std::path::{Path, PathBuf};

use egui::RichText;
use serde::Deserialize;

use crate::SolveApp;
use crate::ui_kit::BackingTooltipExt as _;

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
    /// What it runs, when it has code: its kind, grants, formats and requested limits.
    pub runtime: Option<solidcraft_engine::plugin::RuntimeManifest>,
    /// Who publishes it (empty when it does not say).
    pub publisher: String,
    /// The commands it adds, as "Label (Tab › Panel)".
    pub contributes: Vec<String>,
    /// Its own icon's SVG, when it ships one.
    pub icon_svg: Option<String>,
    /// The icon its card shows: its own, its first command's, or the generic plugin icon.
    pub icon: String,
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
    // The browser build keeps them in its private storage (the engine's default there).
    if cfg!(target_arch = "wasm32") {
        return solidcraft_engine::plugin::default_plugins_dir();
    }
    solidcraft_engine::recovery::data_dir().map(|d| d.join("plugins"))
}

/// A small file's bytes (`max` at most): a file on disk, or in browser storage.
fn read_small(path: &Path, max: u64) -> Result<Vec<u8>, String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
        if !meta.is_file() || meta.len() > max {
            return Err("missing or too large".into());
        }
        std::fs::read(path).map_err(|e| e.to_string())
    }
    #[cfg(target_arch = "wasm32")]
    {
        solidcraft_engine::io::vfs::read(&path.to_string_lossy(), usize::try_from(max).unwrap_or(usize::MAX)).map_err(|e| e.to_string())
    }
}

/// The folders in `dir` (name, path): on disk, or the packages browser storage holds.
fn folders(dir: &Path) -> Vec<(Option<String>, PathBuf)> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
        entries.flatten().filter(|e| e.file_type().is_ok_and(|k| k.is_dir())).map(|e| (e.file_name().into_string().ok(), e.path())).collect()
    }
    #[cfg(target_arch = "wasm32")]
    {
        let prefix = format!("{}/", dir.to_string_lossy().trim_end_matches('/'));
        let mut ids: Vec<String> = solidcraft_engine::io::vfs::list(&prefix)
            .into_iter()
            .filter_map(|(p, _)| Some(p.strip_prefix(&prefix)?.split('/').next()?.to_string()))
            .collect();
        ids.dedup();
        ids.into_iter().map(|id| (Some(id.clone()), dir.join(id))).collect()
    }
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
    Ok(Plugin {
        id: m.id,
        name: name.to_string(),
        version: version.to_string(),
        description: description.to_string(),
        runtime: None,
        publisher: String::new(),
        contributes: Vec::new(),
        icon_svg: None,
        icon: "play".into(),
    })
}

fn read_plugin(dir: &Path, folder: &str) -> Result<Plugin, String> {
    let path = dir.join("plugin.json");
    if !solidcraft_engine::io::vfs::exists(&path.to_string_lossy()) {
        return Err("no plugin.json".into());
    }
    let bytes = read_small(&path, MAX_MANIFEST_BYTES).map_err(|_| "plugin.json is not a file or is larger than 16 KiB".to_string())?;
    let text = String::from_utf8(bytes).map_err(|_| "plugin.json is not UTF-8 text".to_string())?;
    let parsed = solidcraft_engine::plugin::parse_manifest(text.as_bytes(), folder).map_err(|e| e.to_string())?;
    let runtime = parsed.runtime.clone();
    let mut plugin = check(
        Manifest { id: parsed.id.clone(), name: parsed.name.clone(), version: parsed.version.clone(), description: parsed.description.clone() },
        folder,
    )?;
    plugin.runtime = runtime;
    plugin.publisher = parsed.publisher.clone();
    let ribbon = |tab: &str, panel: &str| format!("{} › {}", crate::workspace::word_cased(tab), crate::workspace::word_cased(panel));
    plugin.contributes = parsed
        .contributes
        .commands
        .iter()
        .map(|c| match parsed.contributes.toolbar.iter().find(|t| t.commands.contains(&c.id)) {
            Some(t) => format!("{} ({})", c.label, ribbon(&t.tab, &t.panel)),
            None => c.label.clone(),
        })
        .collect();
    // Its own icon (at most 32 KiB, drawn only if it is plain shapes in palette colours), else
    // its first command's built-in one.
    plugin.icon_svg = parsed
        .icon
        .as_ref()
        .and_then(|file| read_small(&dir.join(file), crate::icon_svg::MAX_PLUGIN_ICON_BYTES as u64).ok())
        .and_then(|bytes| String::from_utf8(bytes).ok());
    let own = format!("plugin:package:{}", plugin.id);
    plugin.icon = match &plugin.icon_svg {
        Some(svg) if crate::icon_svg::register_plugin_icon(&own, svg).is_ok() => own,
        _ => parsed.contributes.commands.iter().map(|c| c.icon.as_str()).find(|i| crate::icon_svg::known(i)).unwrap_or("play").to_string(),
    };
    Ok(plugin)
}

/// An approval a preferences file may hold: a plugin id and a SHA-256 hash in lowercase hex.
pub fn valid_approval(id: &str, hash: &str) -> bool {
    valid_id(id) && hash.len() == 64 && hash.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

/// The approval hash of installed plugin `id` as it is now (manifest and component).
pub fn current_approval(id: &str) -> Result<String, String> {
    if !valid_id(id) {
        return Err(format!("`{id}` is not a plugin id"));
    }
    let dir = plugins_dir().ok_or("this build has no plugins folder")?;
    solidcraft_engine::plugin::read_package(&dir.join(id).to_string_lossy()).map(|p| p.approval).map_err(|e| e.to_string())
}

/// Compile an allowed importer in the background (or load its earlier compilation), so the
/// first file it opens does not wait for it. Runs none of its code; a failure only means the
/// first import compiles it instead.
pub fn prepare(id: &str) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let Some(dir) = plugins_dir().filter(|_| valid_id(id)) else { return };
        let package = dir.join(id);
        let _ = std::thread::Builder::new().name("plugin-prepare".into()).spawn(move || {
            let prepared = std::panic::catch_unwind(|| {
                solidcraft_engine::plugin::read_package(&package.to_string_lossy())
                    .and_then(|p| solidcraft_engine::plugin::prepare_importer(&p, &dir))
            });
            if let Ok(Err(e)) = prepared {
                log::debug!("plugin not prepared ahead of its first import: {e}");
            }
        });
    }
    #[cfg(target_arch = "wasm32")]
    let _ = id;
}

/// The plugins in `dir`, and the folders that were not. A missing directory is an empty list.
pub fn scan(dir: &Path) -> (Vec<Plugin>, Vec<Skipped>) {
    let mut plugins = Vec::new();
    let mut skipped = Vec::new();
    for (name, path) in folders(dir) {
        let Some(folder) = name else {
            skipped.push(Skipped { folder: String::new(), reason: "the folder name is not text".into() });
            continue;
        };
        if folder.starts_with('.') {
            continue;
        }
        match read_plugin(&path, &folder) {
            Ok(plugin) => plugins.push(plugin),
            Err(reason) => skipped.push(Skipped { folder, reason }),
        }
    }
    plugins.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
    skipped.sort_by(|a, b| a.folder.cmp(&b.folder));
    (plugins, skipped)
}

/// Exact integer controls (Advanced) retain all u64 work values, without a float slider conversion.
pub(crate) struct LimitInputs {
    applied: solidcraft_engine::plugin::HostLimits,
    fuel: String,
    memory: String,
    seconds: String,
}
impl From<solidcraft_engine::plugin::HostLimits> for LimitInputs {
    fn from(applied: solidcraft_engine::plugin::HostLimits) -> Self {
        Self {
            fuel: applied.max_fuel.to_string(),
            memory: applied.max_memory_mib.to_string(),
            seconds: (applied.max_deadline_ms / 1000).to_string(),
            applied,
        }
    }
}
pub(crate) fn host_limits(p: &crate::prefs::Prefs) -> solidcraft_engine::plugin::HostLimits {
    solidcraft_engine::plugin::HostLimits {
        max_fuel: p.plugin_max_fuel,
        max_memory_mib: p.plugin_max_memory_mib,
        max_deadline_ms: p.plugin_max_seconds.saturating_mul(1000),
    }
}

use solidcraft_engine::plugin::LimitPreset;

/// The plain choice a control client names: `normal`, `large` or `largest`.
pub(crate) fn preset_named(name: &str) -> Option<LimitPreset> {
    match name {
        "normal" => Some(LimitPreset::Normal),
        "large" => Some(LimitPreset::Large),
        "largest" => Some(LimitPreset::Largest),
        _ => None,
    }
}

/// A preset's name on the page.
fn preset_label(preset: Option<LimitPreset>) -> &'static str {
    solidcraft_ui_text::tr(match preset {
        Some(LimitPreset::Normal) => "prefs.plugin_limits_normal",
        Some(LimitPreset::Large) => "prefs.plugin_limits_large",
        Some(LimitPreset::Largest) => "prefs.plugin_limits_largest",
        None => "prefs.plugin_limits_custom",
    })
}

/// An amount of memory in words: "512 MB", "about 1 GB", "about 2.5 GB".
pub(crate) fn memory_words(mib: usize) -> String {
    if mib < 1024 {
        return solidcraft_ui_text::format("prefs.memory_mb", &[("mb", &mib.to_string())]);
    }
    let gb = mib as f64 / 1024.0;
    let gb = if (gb - gb.round()).abs() < 0.05 { format!("{}", gb.round()) } else { format!("{gb:.1}") };
    solidcraft_ui_text::format("prefs.memory_gb", &[("gb", &gb)])
}

/// What a plugin asks for, in words: memory and time per import (or run).
pub(crate) fn needs_words(limits: &solidcraft_engine::plugin::Limits, importer: bool) -> String {
    let seconds = format!("{}", (limits.deadline_ms as f64 / 1000.0 * 10.0).round() / 10.0);
    let key = if importer { "ui.plugins_needs_import" } else { "ui.plugins_needs_run" };
    solidcraft_ui_text::format(key, &[("memory", &memory_words(limits.memory_mib)), ("seconds", &seconds)])
}

/// The smallest plain choice that lets a plugin asking for `limits` run, if any.
pub(crate) fn preset_for(limits: &solidcraft_engine::plugin::Limits) -> Option<LimitPreset> {
    LimitPreset::ALL.into_iter().find(|p| p.limits().check(limits).is_ok())
}
pub(crate) fn sync_policy(app: &mut SolveApp) {
    app.session.disabled_plugins.clone_from(&app.preferences.disabled_plugins);
    app.session.plugin_approvals.clone_from(&app.preferences.plugin_approvals);
    app.session.plugin_limits = host_limits(&app.preferences);
    sync_contributions(app, false);
}

/// Read the installed plugins again (the browser host calls it once its storage is loaded).
pub fn reload(app: &SolveApp) {
    sync_contributions(app, true);
}

/// Put allowed, turned-on plugins' commands on the ribbon, palette and shortcuts, and take
/// away the rest; their icons are drawn from then on. Only when the policy changed since the
/// last time, unless `force` (after an install replaced a package).
pub(crate) fn sync_contributions(app: &SolveApp, force: bool) {
    type Policy = (Option<PathBuf>, std::collections::BTreeMap<String, String>, std::collections::BTreeSet<String>);
    static LAST: std::sync::Mutex<Option<Policy>> = std::sync::Mutex::new(None);
    let dir = app.session.plugins_dir.clone();
    let policy: Policy = (dir.clone(), app.preferences.plugin_approvals.clone(), app.preferences.disabled_plugins.clone());
    let Ok(mut last) = LAST.lock() else { return };
    if !force && last.as_ref() == Some(&policy) {
        return;
    }
    solidcraft_engine::plugin::contrib::sync(dir.as_deref(), &policy.1, &policy.2, &crate::icon_svg::known);
    for spec in solidcraft_engine::plugin::contrib::specs() {
        if let Some(svg) = spec.plugin.and_then(|c| c.icon_svg.as_deref())
            && let Err(e) = crate::icon_svg::register_plugin_icon(spec.icon, svg)
        {
            log::warn!("{}: its icon is not drawn ({e})", spec.id);
        }
    }
    *last = Some(policy);
}
fn parsed_limits(input: &LimitInputs) -> Option<solidcraft_engine::plugin::HostLimits> {
    let seconds: u64 = input.seconds.trim().parse().ok()?;
    let limits = solidcraft_engine::plugin::HostLimits {
        max_fuel: input.fuel.trim().parse().ok()?,
        max_memory_mib: input.memory.trim().parse().ok()?,
        max_deadline_ms: seconds.checked_mul(1000)?,
    };
    limits.validate().ok()?;
    Some(limits)
}
pub(crate) fn resource_pending(app: &SolveApp) -> bool {
    let current = host_limits(&app.preferences);
    app.prefs_window.plugin_limit_inputs.as_ref().is_some_and(|input| {
        input.applied == current
            && (input.fuel != current.max_fuel.to_string()
                || input.memory != current.max_memory_mib.to_string()
                || input.seconds != (current.max_deadline_ms / 1000).to_string())
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
    set_limits(app, limits);
    Ok(())
}

/// Use these ceilings now.
fn set_limits(app: &mut SolveApp, limits: solidcraft_engine::plugin::HostLimits) {
    app.preferences.plugin_max_fuel = limits.max_fuel;
    app.preferences.plugin_max_memory_mib = limits.max_memory_mib;
    app.preferences.plugin_max_seconds = limits.max_deadline_ms / 1000;
    app.prefs_window.plugin_limit_inputs = Some(limits.into());
    sync_policy(app);
}
fn resource_controls(app: &mut SolveApp, ui: &mut egui::Ui) {
    let current = host_limits(&app.preferences);
    // The plain choice: how much a plugin may do before it is stopped.
    let chosen = LimitPreset::of(&current);
    let mut options: Vec<&str> = LimitPreset::ALL.iter().map(|p| preset_label(Some(*p))).collect();
    if chosen.is_none() {
        options.push(preset_label(None));
    }
    let mut selected = chosen.and_then(|c| LimitPreset::ALL.iter().position(|p| *p == c)).unwrap_or(LimitPreset::ALL.len());
    let before = selected;
    // The question in full, then the choice (a row label would cut it short).
    crate::ui_kit::text(ui, app.tr("prefs.plugin_limits_choice"), crate::ui_kit::Text::Report("plugin_limits_choice", false));
    crate::ui_kit::row(ui, app.tr("prefs.plugin_limits_short"), |ui| {
        let r = crate::ui_kit::dropdown(ui, "plugin_limits_choice", &mut selected, &options);
        crate::scenario::publish_handle("prefs:plugin_limits_choice", r.rect.center());
    });
    if selected != before
        && let Some(preset) = LimitPreset::ALL.get(selected)
    {
        set_limits(app, preset.limits());
    }
    let current = host_limits(&app.preferences);
    crate::ui_kit::text(
        ui,
        &solidcraft_ui_text::format(
            "prefs.plugin_limits_summary",
            &[("memory", &memory_words(current.max_memory_mib)), ("seconds", &(current.max_deadline_ms / 1000).to_string())],
        ),
        crate::ui_kit::Text::Hint("plugin_limits_summary"),
    );
    crate::ui_kit::text(ui, app.tr("prefs.plugin_limits_why"), crate::ui_kit::Text::Hint("plugin_limits"));
    // The exact numbers, folded away.
    let open_id = egui::Id::unique("plugin_limits_advanced");
    let mut open = ui.ctx().data(|d| d.get_temp::<bool>(open_id)).unwrap_or(false);
    let label = format!("{} {}", if open { "\u{25be}" } else { "\u{25b8}" }, app.tr("prefs.plugin_limits_advanced"));
    let toggle = crate::ui_kit::button(ui, &label, crate::ui_kit::Button::Ghost);
    crate::scenario::publish_handle("prefs:plugin_limits_advanced", toggle.rect.center());
    crate::scenario::publish_rect("prefs:plugin_limits_advanced", toggle.rect);
    if toggle.clicked() {
        open = !open;
        ui.ctx().data_mut(|d| d.insert_temp(open_id, open));
    }
    if open {
        let mut input = app.prefs_window.plugin_limit_inputs.take().filter(|v| v.applied == current).unwrap_or_else(|| current.into());
        let (fuel_enter, _) = crate::ui_kit::text_row(ui, app.tr("prefs.plugin_max_fuel"), &mut input.fuel, "");
        let (memory_enter, _) = crate::ui_kit::text_row(ui, app.tr("prefs.plugin_max_memory"), &mut input.memory, "");
        let (seconds_enter, _) = crate::ui_kit::text_row(ui, app.tr("prefs.plugin_max_seconds"), &mut input.seconds, "");
        if parsed_limits(&input).is_none() {
            crate::ui_kit::text(ui, app.tr("prefs.plugin_limits_invalid"), crate::ui_kit::Text::Error("plugin_limits"));
        }
        crate::ui_kit::text(ui, app.tr("prefs.plugin_limits_note"), crate::ui_kit::Text::Hint("plugin_limits_note"));
        app.prefs_window.plugin_limit_inputs = Some(input);
        if (fuel_enter || memory_enter || seconds_enter)
            && let Err(error) = commit_resource_inputs(app)
        {
            app.set_status(error, true);
        }
    }
    ui.separator();
}

/// Whether an installed plugin is offered. Unknown ids are on; an id in `disabled_plugins` is off.
pub fn enabled(app: &SolveApp, id: &str) -> bool {
    !app.preferences.disabled_plugins.contains(id)
}

/// What a plugin with code does and asks for, and whether the user allowed it to run.
fn runtime_rows(app: &mut SolveApp, ui: &mut egui::Ui, plugin: &Plugin, runtime: &solidcraft_engine::plugin::RuntimeManifest) {
    use solidcraft_engine::plugin::Kind;
    let t = solidcraft_theme::Tokens::get();
    let small = |ui: &mut egui::Ui, text: String| {
        ui.add(egui::Label::new(RichText::new(text).size(t.font_small).color(t.text_dim)).wrap());
    };
    match runtime.kind {
        Kind::Importer => {
            let formats = runtime.formats.iter().map(|f| format!("{} (.{})", f.name, f.extension)).collect::<Vec<_>>().join(", ");
            small(ui, solidcraft_ui_text::format("ui.plugins_importer", &[("formats", &formats)]));
        }
        Kind::Command => {
            let commands = runtime.commands.iter().cloned().collect::<Vec<_>>().join(", ");
            small(ui, solidcraft_ui_text::format("ui.plugins_commands", &[("commands", &commands)]));
        }
    }
    let limits = &runtime.limits;
    small(ui, needs_words(limits, runtime.kind == Kind::Importer));
    small(ui, solidcraft_ui_text::format("ui.plugins_code_size", &[("size", &limits.component_mib.to_string())]));
    if host_limits(&app.preferences).check(limits).is_err() {
        let text = match preset_for(limits) {
            Some(preset) => solidcraft_ui_text::format("ui.plugins_needs_more", &[("choice", preset_label(Some(preset)))]),
            None => solidcraft_ui_text::tr("ui.plugins_needs_too_much").to_string(),
        };
        crate::ui_kit::text(ui, &text, crate::ui_kit::Text::Error("plugin_over_limits"));
    }
    let allowed = app.preferences.plugin_approvals.contains_key(&plugin.id);
    ui.horizontal(|ui| {
        let (state, button) = if allowed { ("ui.plugins_allowed", "ui.plugins_revoke") } else { ("ui.plugins_not_allowed", "ui.plugins_allow") };
        ui.label(RichText::new(solidcraft_ui_text::tr(state)).size(t.font_small).color(if allowed { t.text } else { t.text_dim }));
        let clicked = crate::ui_kit::button(ui, solidcraft_ui_text::tr(button), crate::ui_kit::Button::Normal);
        crate::scenario::publish_handle(&format!("prefs:plugin_allow:{}", plugin.id), clicked.rect.center());
        if clicked.clicked() {
            if allowed {
                app.preferences.plugin_approvals.remove(&plugin.id);
            } else {
                match current_approval(&plugin.id) {
                    Ok(hash) => {
                        app.preferences.plugin_approvals.insert(plugin.id.clone(), hash);
                        prepare(&plugin.id);
                    }
                    Err(e) => app.set_status(e, true),
                }
            }
            sync_policy(app);
        }
    });
}

fn matches_query(plugin: &Plugin, query: &str) -> bool {
    let query = query.trim();
    if query.is_empty() {
        return true;
    }
    let query = query.to_lowercase();
    [plugin.name.as_str(), plugin.id.as_str(), plugin.description.as_str(), plugin.version.as_str(), plugin.publisher.as_str()]
        .iter()
        .chain(plugin.contributes.iter().map(String::as_str).collect::<Vec<_>>().iter())
        .any(|text| text.to_lowercase().contains(&query))
}

/// The Plugins page. In the browser, packages are kept in its private storage.
pub fn page(app: &mut SolveApp, ui: &mut egui::Ui) {
    let dir = plugins_dir();
    page_in_dir(app, ui, dir.as_deref());
}

fn page_in_dir(app: &mut SolveApp, ui: &mut egui::Ui, dir: Option<&Path>) {
    crate::ui_kit::settings_page(ui, |ui| {
        crate::ui_kit::settings_heading(ui, solidcraft_ui_text::tr("ui.plugins"), solidcraft_ui_text::tr("prefs.plugins_intro"));
        let Some(dir) = dir else {
            ui.label(solidcraft_ui_text::tr("ui.plugins_desktop"));
            return;
        };
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
        resource_controls(app, ui);
        if !searching {
            let install = crate::ui_kit::button(ui, solidcraft_ui_text::tr("ui.plugins_install"), crate::ui_kit::Button::Normal);
            crate::scenario::publish_handle("prefs:plugins_install", install.rect.center());
            if install.clicked() {
                let kinds: &[&str] = if cfg!(target_arch = "wasm32") { &["zip"] } else { &["json", "zip"] };
                app.pick_open_for(kinds, |app, path| {
                    let Some(path) = path else { return };
                    let folder = if path.to_ascii_lowercase().ends_with(".zip") {
                        path
                    } else {
                        Path::new(&path).parent().map(|p| p.display().to_string()).unwrap_or_default()
                    };
                    match app.run("plugin.install", serde_json::json!({"path": folder, "replace": true})) {
                        Ok(r) => app.set_status(
                            solidcraft_ui_text::format("ui.plugins_installed", &[("name", r["manifest"]["name"].as_str().unwrap_or_default())]),
                            false,
                        ),
                        Err(e) => app.set_status(e, true),
                    }
                });
            }
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
        sync_policy(app);
    }
    if !plugin.publisher.is_empty() {
        ui.label(solidcraft_ui_text::format("ui.plugins_publisher", &[("publisher", &plugin.publisher)]));
    }
    if !plugin.contributes.is_empty() {
        ui.label(solidcraft_ui_text::format("ui.plugins_contributes", &[("commands", &plugin.contributes.join(", "))]));
    }
    if let Some(runtime) = &plugin.runtime {
        let t = solidcraft_theme::Tokens::get();
        let (rect, _) = ui.allocate_exact_size(egui::vec2(t.space_24, t.space_24), egui::Sense::hover());
        crate::icons::paint_icon(ui.painter(), rect, &plugin.icon, 1.0);
        crate::scenario::publish_rect(&format!("prefs:plugin_icon:{}", plugin.id), rect);
        runtime_rows(app, ui, plugin, runtime);
    }
    ui.horizontal(|ui| {
        // Remove asks once more, in place.
        let confirm_id = egui::Id::unique(("plugin_remove_confirm", plugin.id.as_str()));
        let confirming = ui.ctx().data(|d| d.get_temp::<bool>(confirm_id)).unwrap_or(false);
        let label = if confirming {
            format!("{}?", solidcraft_ui_text::tr("ui.plugins_remove"))
        } else {
            solidcraft_ui_text::tr("ui.plugins_remove").to_string()
        };
        let remove = crate::ui_kit::button(ui, &label, if confirming { crate::ui_kit::Button::Primary } else { crate::ui_kit::Button::Normal });
        crate::scenario::publish_handle(&format!("prefs:plugin_remove:{}", plugin.id), remove.rect.center());
        if remove.clicked() {
            if confirming {
                ui.ctx().data_mut(|d| d.remove::<bool>(confirm_id));
                match app.run("plugin.remove", serde_json::json!({"id": plugin.id})) {
                    Ok(_) => {
                        app.preferences.plugin_approvals.remove(&plugin.id);
                        app.preferences.disabled_plugins.remove(&plugin.id);
                        sync_policy(app);
                        app.set_status(solidcraft_ui_text::format("ui.plugins_removed", &[("name", &plugin.name)]), false);
                    }
                    Err(e) => app.set_status(e, true),
                }
            } else {
                ui.ctx().data_mut(|d| d.insert_temp(confirm_id, true));
            }
        }
    });
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
        assert!(skipped.iter().any(|s| s.folder == "broken" && s.reason.contains("does not match")));
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
    fn the_plain_limit_choices_set_exact_ceilings_and_cards_say_what_a_plugin_needs() {
        use serde_json::json;
        let mut h = crate::scenario::Harness::new();
        h.call("ui.prefs", json!({"language":"en"}));
        assert_eq!(LimitPreset::of(&host_limits(&h.app.preferences)), Some(LimitPreset::Normal), "Normal is the default");
        for (name, preset) in [("large", LimitPreset::Large), ("largest", LimitPreset::Largest), ("normal", LimitPreset::Normal)] {
            assert_eq!(h.call("ui.prefs", json!({"plugin_limits": name}))["ok"], true);
            assert_eq!(h.app.session.plugin_limits, preset.limits(), "{name}");
        }
        assert_eq!(h.call("ui.prefs", json!({"plugin_limits": "huge"}))["ok"], false);
        // A time limit set by hand is no longer one of the choices, and the runtime keeps it.
        assert_eq!(h.call("ui.prefs", json!({"plugin_max_seconds": 10}))["ok"], true);
        assert_eq!(h.app.session.plugin_limits.max_deadline_ms, 10_000);
        assert_eq!(LimitPreset::of(&h.app.session.plugin_limits), None);
        for bad in [json!({"plugin_max_seconds": 0}), json!({"plugin_max_seconds": 31})] {
            assert_eq!(h.call("ui.prefs", bad)["ok"], false);
        }
        // What a card says, in words: memory and time, never the work count.
        let dutchman = solidcraft_engine::plugin::Limits { fuel: 100_000_000_000, memory_mib: 1024, deadline_ms: 30_000, component_mib: 48 };
        assert_eq!(needs_words(&dutchman, true), "Needs up to about 1 GB of memory and 30 seconds per import.");
        let small = solidcraft_engine::plugin::Limits::default();
        assert_eq!(needs_words(&small, false), "Needs up to 64 MB of memory and 5 seconds per run.");
        assert_eq!(memory_words(2560), "about 2.5 GB");
        // The smallest choice that runs it.
        assert_eq!(preset_for(&small), Some(LimitPreset::Normal));
        assert_eq!(preset_for(&dutchman), Some(LimitPreset::Large));
        let biggest = solidcraft_engine::plugin::Limits { fuel: u64::MAX, memory_mib: 4096, ..dutchman };
        assert_eq!(preset_for(&biggest), Some(LimitPreset::Largest));
    }

    #[test]
    fn resource_text_edits_require_complete_valid_confirmation() {
        use serde_json::json;
        let mut h = crate::scenario::Harness::new();
        h.call("ui.prefs", json!({"open":"Plugins"}));
        let initial = host_limits(&h.app.preferences);
        h.app.prefs_window.plugin_limit_inputs =
            Some(LimitInputs { applied: initial, fuel: u64::MAX.to_string(), memory: "512".into(), seconds: "30".into() });
        assert!(resource_pending(&h.app));
        assert_eq!(h.app.session.plugin_limits, initial, "typing alone does not authorize a higher ceiling");
        assert_eq!(h.call("ui.prefs", json!({}))["result"]["apply_enabled"], true);
        assert_eq!(h.call("ui.prefs", json!({"button":"apply"}))["ok"], true);
        assert_eq!(h.app.session.plugin_limits.max_fuel, u64::MAX);
        assert_eq!(h.app.session.plugin_limits.max_memory_mib, 512);
        let applied = h.app.session.plugin_limits;
        for text in ["", "0", "18446744073709551616", "1.5"] {
            h.app.prefs_window.plugin_limit_inputs = Some(LimitInputs { applied, fuel: text.into(), memory: "1024".into(), seconds: "30".into() });
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
            runtime: None,
            publisher: "Robot parts makers".into(),
            contributes: vec!["Insert part (Insert › Parts)".into()],
            icon_svg: None,
            icon: "play".into(),
        };
        assert!(matches_query(&plugin, ""));
        assert!(matches_query(&plugin, " frc "));
        assert!(matches_query(&plugin, "FTC-FRC"));
        assert!(matches_query(&plugin, "commercial"));
        assert!(matches_query(&plugin, "0.1"));
        assert!(!matches_query(&plugin, "bearing"));
    }
}

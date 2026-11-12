//! What an allowed plugin adds to the app: commands, with a place on the ribbon, an icon, a
//! tooltip, an optional shortcut and declared parameters. They are ordinary commands: the
//! registry lists them beside the built-in ones (`command_specs`), so the ribbon, the command
//! palette, search, shortcuts, the control channel and the CLI all reach them, and they go away
//! when the plugin is turned off, revoked or removed ([`sync`]).
//!
//! Running one runs its plugin: an importer's command converts a file (as `file.import`), a
//! command plugin's runs its entry (as `plugin.run`). Either way the parameters are checked
//! against the declaration first, and the plugin must still be allowed for its exact content.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{FileFormat, Kind, Manifest, error, plain, valid_id};
use crate::cmd::CommandSpec;
use crate::{EngineError, Result, Session};

/// The most commands, parameters and enum choices one package may declare.
pub const MAX_COMMANDS: usize = 32;
pub const MAX_PARAMS: usize = 32;
pub const MAX_CHOICES: usize = 32;
/// The largest icon a package may ship.
pub const MAX_ICON_BYTES: usize = 32 * 1024;

/// What a package adds to the app (`contributes` in `plugin.json`).
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Contributes {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<CommandContribution>,
    /// Where on the ribbon its commands go. A command no entry names is in the command
    /// palette, search and shortcuts only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub toolbar: Vec<ToolbarContribution>,
}

impl Contributes {
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty() && self.toolbar.is_empty()
    }
}

/// One command a package adds. Its id in the app is `plugin.<package id>.<id>`.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommandContribution {
    /// 1..32 lowercase letters, digits and underscores.
    pub id: String,
    /// The button's and the palette's name (1..40 characters).
    pub label: String,
    /// What it does, as the button's tooltip says it (at most 240 characters).
    #[serde(default)]
    pub tooltip: String,
    /// A built-in icon's name (`import`), or an SVG file in the package (`import.svg`). The
    /// package's own icon, else a generic plugin icon, when omitted.
    #[serde(default)]
    pub icon: String,
    /// A suggested shortcut (`Ctrl+Shift+I`). Not bound when a built-in command has it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shortcut: Option<String>,
    /// When it can run.
    #[serde(default)]
    pub when: When,
    /// A command plugin's entry it runs. An importer's command imports a file and names none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    /// Its parameters, which the app's dialog shows and checks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<ParamSpec>,
}

/// When a contributed command can run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum When {
    #[default]
    Always,
    /// Not while a sketch is being edited.
    NoSketch,
    /// Only while a sketch is being edited.
    Sketch,
}

/// A place on the ribbon: an existing tab, and a panel of that name on it (made for the
/// plugin when the tab has none).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ToolbarContribution {
    /// A ribbon tab, upper case (`INSERT`).
    pub tab: String,
    /// The panel, upper case (`DUTCHMAN`).
    pub panel: String,
    /// The package's command ids, in button order.
    pub commands: Vec<String>,
}

/// A declared parameter. The host draws it in its dialog and checks a value against it.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ParamSpec {
    /// The JSON key: 1..32 lowercase letters, digits and underscores.
    pub name: String,
    /// The dialog row's label (1..40 characters).
    pub label: String,
    #[serde(rename = "type")]
    pub kind: ParamKind,
    /// A number's unit as the field shows it (at most 8 characters); lengths are millimetres
    /// and angles degrees.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub unit: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// An enum's choices (1..32).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<Choice>,
    /// The value the dialog starts with, and the value used when a call leaves it out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    /// A selection's kind of object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accept: Option<Pick>,
    /// A selection of several objects.
    #[serde(default)]
    pub many: bool,
    /// Whether a call must give it (a selection or a string with no default).
    #[serde(default)]
    pub required: bool,
    /// A line of help under the row (at most 240 characters).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub help: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ParamKind {
    /// A number, with an optional unit, minimum and maximum.
    Number,
    /// A length in millimetres.
    Length,
    /// An angle in degrees.
    Angle,
    Bool,
    /// One of `options`.
    Enum,
    /// Text, at most 1024 characters.
    String,
    /// Faces, edges or bodies picked in the view (`accept`, `many`).
    Selection,
}

/// One choice of an enum parameter.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Choice {
    /// 1..32 lowercase letters, digits and underscores.
    pub value: String,
    pub label: String,
}

/// What a selection parameter picks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Pick {
    Faces,
    Edges,
    Bodies,
}

fn key(s: &str) -> bool {
    (1..=32).contains(&s.len()) && s.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_') && !s.starts_with('_')
}
fn ribbon_name(s: &str) -> bool {
    (1..=24).contains(&s.len()) && s.bytes().all(|c| c.is_ascii_uppercase() || c == b' ') && !s.starts_with(' ') && !s.ends_with(' ')
}
fn shortcut(s: &str) -> bool {
    (1..=24).contains(&s.len()) && s.split('+').all(|k| !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric()))
}
/// An icon file a package may ship: a lowercase id with `.svg`.
pub fn icon_file(name: &str) -> bool {
    name.strip_suffix(".svg").is_some_and(valid_id)
}
fn icon_name(name: &str) -> bool {
    icon_file(name) || key(name)
}

impl ParamSpec {
    fn validate(&self) -> std::result::Result<(), String> {
        if !key(&self.name) || matches!(self.name.as_str(), "path" | "into" | "live") {
            return Err(format!("parameter `{}`: a name of 1..32 lowercase letters, digits and underscores (not path, into or live)", self.name));
        }
        if !plain(&self.label, 40) || self.help.chars().count() > 240 || self.help.chars().any(char::is_control) || self.unit.chars().count() > 8 {
            return Err(format!("parameter `{}`: a label of 1..40 characters, help of at most 240 and a unit of at most 8", self.name));
        }
        let numeric = matches!(self.kind, ParamKind::Number | ParamKind::Length | ParamKind::Angle);
        if !numeric && (self.min.is_some() || self.max.is_some() || !self.unit.is_empty()) {
            return Err(format!("parameter `{}`: only numbers have a unit, minimum or maximum", self.name));
        }
        if let (Some(min), Some(max)) = (self.min, self.max)
            && !(min.is_finite() && max.is_finite() && min <= max)
        {
            return Err(format!("parameter `{}`: min must be at most max", self.name));
        }
        if (self.kind == ParamKind::Enum) != !self.options.is_empty() || self.options.len() > MAX_CHOICES {
            return Err(format!("parameter `{}`: an enum has 1..32 options, other types none", self.name));
        }
        let mut seen = BTreeSet::new();
        if self.options.iter().any(|c| !key(&c.value) || !plain(&c.label, 40) || !seen.insert(&c.value)) {
            return Err(format!("parameter `{}`: each option needs a distinct value and a label", self.name));
        }
        if (self.kind == ParamKind::Selection) != self.accept.is_some() || (self.many && self.kind != ParamKind::Selection) {
            return Err(format!("parameter `{}`: a selection names what it accepts; other types do not", self.name));
        }
        if let Some(default) = &self.default {
            self.check(default)?;
        }
        Ok(())
    }

    /// Check `value` for this parameter; returns it as the plugin gets it.
    pub fn check(&self, value: &Value) -> std::result::Result<Value, String> {
        let bad = |what: &str| format!("`{}` must be {what}", self.name);
        match self.kind {
            ParamKind::Number | ParamKind::Length | ParamKind::Angle => {
                let v = value.as_f64().filter(|v| v.is_finite()).ok_or_else(|| bad("a finite number"))?;
                if self.min.is_some_and(|m| v < m) || self.max.is_some_and(|m| v > m) {
                    return Err(bad(&format!("between {} and {}", self.min.unwrap_or(f64::MIN), self.max.unwrap_or(f64::MAX))));
                }
                Ok(json!(v))
            }
            ParamKind::Bool => value.as_bool().map(Value::Bool).ok_or_else(|| bad("true or false")),
            ParamKind::Enum => match value.as_str() {
                Some(v) if self.options.iter().any(|c| c.value == v) => Ok(json!(v)),
                _ => Err(bad(&format!("one of {}", self.options.iter().map(|c| c.value.as_str()).collect::<Vec<_>>().join(", ")))),
            },
            ParamKind::String => match value.as_str() {
                Some(v) if v.chars().count() <= 1024 && !v.chars().any(char::is_control) => Ok(json!(v)),
                _ => Err(bad("text of at most 1024 characters")),
            },
            ParamKind::Selection => {
                // Picks as the engine's other commands take them: a body name, or {body, face|edge}.
                let items: Vec<&Value> = match value {
                    Value::Array(items) if self.many => items.iter().collect(),
                    Value::Array(_) => return Err(bad("one pick")),
                    other => vec![other],
                };
                if items.is_empty() || items.len() > 4096 {
                    return Err(bad("1..4096 picks"));
                }
                let field = match self.accept {
                    Some(Pick::Faces) => "face",
                    Some(Pick::Edges) => "edge",
                    _ => "",
                };
                for item in &items {
                    let ok = match item {
                        Value::String(body) => field.is_empty() && !body.is_empty() && body.len() <= 256,
                        Value::Object(o) => {
                            o.get("body").and_then(Value::as_str).is_some_and(|b| !b.is_empty() && b.len() <= 256)
                                && (field.is_empty() || o.get(field).and_then(Value::as_u64).is_some())
                        }
                        _ => false,
                    };
                    if !ok {
                        return Err(bad(if field.is_empty() { "body names" } else { "picks of {body, face} or {body, edge}" }));
                    }
                }
                Ok(value.clone())
            }
        }
    }
}

/// Check a package's contributions against its manifest (called by `parse_manifest`).
pub(super) fn validate(m: &Manifest) -> Result<()> {
    let c = &m.contributes;
    if c.is_empty() {
        return Ok(());
    }
    let runtime = m.runtime.as_ref().ok_or_else(|| error("contributions need a runtime: they run the plugin"))?;
    if c.commands.len() > MAX_COMMANDS {
        return Err(error("at most 32 contributed commands"));
    }
    let mut ids = BTreeSet::new();
    for cmd in &c.commands {
        let what = |e: String| error(format!("command `{}`: {e}", cmd.id));
        if !key(&cmd.id) || !ids.insert(cmd.id.as_str()) {
            return Err(error("each command needs a distinct id of 1..32 lowercase letters, digits and underscores"));
        }
        if !plain(&cmd.label, 40) || cmd.tooltip.chars().count() > 240 || cmd.tooltip.chars().any(char::is_control) {
            return Err(what("a label of 1..40 characters and a tooltip of at most 240".into()));
        }
        if !cmd.icon.is_empty() && !icon_name(&cmd.icon) {
            return Err(what("the icon is a built-in icon's name or a lowercase .svg file in the package".into()));
        }
        if cmd.shortcut.as_deref().is_some_and(|s| !shortcut(s)) {
            return Err(what("a shortcut such as Ctrl+Shift+I".into()));
        }
        match (runtime.kind, &cmd.entry) {
            (Kind::Command, Some(entry)) if runtime.entries.contains(entry) => {}
            (Kind::Command, _) => return Err(what("a command plugin's command names one of its entries".into())),
            (Kind::Importer, None) => {}
            (Kind::Importer, Some(_)) => return Err(what("an importer's command imports a file and names no entry".into())),
        }
        if cmd.params.len() > MAX_PARAMS {
            return Err(what("at most 32 parameters".into()));
        }
        let mut names = BTreeSet::new();
        for p in &cmd.params {
            p.validate().map_err(what)?;
            if !names.insert(p.name.as_str()) {
                return Err(what(format!("parameter `{}` twice", p.name)));
            }
            if runtime.kind == Kind::Importer && p.kind == ParamKind::Selection {
                return Err(what("an importer is given a file, not picks".into()));
            }
        }
    }
    let mut placed = BTreeSet::new();
    for t in &c.toolbar {
        if !ribbon_name(&t.tab) || !ribbon_name(&t.panel) {
            return Err(error("a toolbar entry names a tab and a panel in upper case (1..24 letters and spaces)"));
        }
        if t.commands.is_empty() || t.commands.iter().any(|id| !ids.contains(id.as_str()) || !placed.insert(id.as_str())) {
            return Err(error("a toolbar entry lists the package's own commands, each once"));
        }
    }
    Ok(())
}

/// The icon files a manifest names (its own and its commands'), to read with the package.
pub(super) fn icon_files(m: &Manifest) -> BTreeSet<String> {
    let mut files: BTreeSet<String> = m.contributes.commands.iter().map(|c| c.icon.clone()).filter(|i| icon_file(i)).collect();
    files.extend(m.icon.clone().filter(|i| icon_file(i)));
    files
}

/// A contributed command, as the registry holds it.
#[derive(Debug)]
pub struct Contributed {
    /// The plugin's id, name and the approval hash of the content it came from.
    pub plugin: String,
    pub plugin_name: String,
    pub approval: String,
    pub tooltip: String,
    /// The SVG the package ships for it, when it ships one.
    pub icon_svg: Option<String>,
    pub params: Vec<ParamSpec>,
    /// A command plugin's entry; `None` for an importer's command.
    pub entry: Option<String>,
    /// The formats an importer's command opens.
    pub formats: Vec<FileFormat>,
}

impl Contributed {
    /// Whether it imports a file (and so takes `path` and `into`).
    pub fn imports(&self) -> bool {
        self.entry.is_none()
    }
}

/// The icon name a contributed command with a shipped SVG is drawn under.
pub fn icon_id(command: &str) -> String {
    format!("plugin:{command}")
}

/// The registry. Specs are leaked so they are `&'static` like the built-in ones; each package
/// content is leaked once (`made`), so syncing again reuses it.
#[derive(Default)]
struct Registry {
    active: Vec<&'static CommandSpec>,
    made: BTreeMap<String, Vec<&'static CommandSpec>>,
}

#[cfg(not(test))]
static REGISTRY: std::sync::RwLock<Option<Registry>> = std::sync::RwLock::new(None);
#[cfg(test)]
thread_local! {
    // Each test thread has its own, so a test that adds commands never shows them to another.
    static REGISTRY: std::cell::RefCell<Option<Registry>> = const { std::cell::RefCell::new(None) };
}

fn with_registry<R>(f: impl FnOnce(&mut Option<Registry>) -> R) -> Option<R> {
    #[cfg(not(test))]
    {
        REGISTRY.write().ok().map(|mut r| f(&mut r))
    }
    #[cfg(test)]
    {
        REGISTRY.with(|r| r.try_borrow_mut().ok().map(|mut r| f(&mut r)))
    }
}

/// The contributed commands now in the registry.
pub fn specs() -> Vec<&'static CommandSpec> {
    #[cfg(not(test))]
    {
        REGISTRY.read().ok().and_then(|r| r.as_ref().map(|r| r.active.clone())).unwrap_or_default()
    }
    #[cfg(test)]
    {
        REGISTRY.with(|r| r.try_borrow().ok().and_then(|r| r.as_ref().map(|r| r.active.clone()))).unwrap_or_default()
    }
}

/// Changes whenever the contributed commands do, so a cache of the ribbon knows to start over.
static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// See [`GENERATION`].
pub fn generation() -> u64 {
    GENERATION.load(std::sync::atomic::Ordering::Relaxed)
}

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

fn enabled_always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

/// Never called: a contributed command runs through [`run`] (see `CommandSpec::call`).
fn unreachable_run(_: &mut Session, _: &Value) -> Result<Value> {
    Err(error("a contributed command runs through its plugin"))
}

/// The parameter documentation `commands` lists for it.
fn params_doc(c: &Contributed) -> String {
    let mut parts: Vec<String> = Vec::new();
    if c.imports() {
        let exts = c.formats.iter().map(|f| format!(".{}", f.extension)).collect::<Vec<_>>().join(", ");
        parts.push(format!("path: a file to import ({exts})"));
        parts.push("into?: new|current (default new)".into());
    }
    for p in &c.params {
        let kind = match p.kind {
            ParamKind::Number => "number",
            ParamKind::Length => "length in mm",
            ParamKind::Angle => "angle in degrees",
            ParamKind::Bool => "bool",
            ParamKind::Enum => "enum",
            ParamKind::String => "text",
            ParamKind::Selection => "picks",
        };
        let choices = if p.options.is_empty() {
            String::new()
        } else {
            format!(" {}", p.options.iter().map(|o| o.value.as_str()).collect::<Vec<_>>().join("|"))
        };
        let default = p.default.as_ref().map(|d| format!(" (default {d})")).unwrap_or_default();
        parts.push(format!("{}{}: {kind}{choices}{default}", p.name, if p.required { "" } else { "?" }));
    }
    let what = if c.imports() { "imports with" } else { "runs" };
    format!("{}; {what} plugin `{}` → as file.import or plugin.run", parts.join("; "), c.plugin)
}

fn build(
    manifest: &Manifest,
    approval: &str,
    icons: &BTreeMap<String, String>,
    builtin: &dyn Fn(&str) -> bool,
    taken: &BTreeSet<String>,
) -> Vec<&'static CommandSpec> {
    let Some(runtime) = manifest.runtime.as_ref() else { return Vec::new() };
    let mut out = Vec::new();
    for cmd in &manifest.contributes.commands {
        let id = format!("plugin.{}.{}", manifest.id, cmd.id);
        let place = manifest.contributes.toolbar.iter().find(|t| t.commands.contains(&cmd.id));
        let icon_ref = Some(cmd.icon.clone()).filter(|i| !i.is_empty()).or_else(|| manifest.icon.clone()).unwrap_or_default();
        let icon_svg = icon_file(&icon_ref).then(|| icons.get(&icon_ref).cloned()).flatten();
        let icon = match (&icon_svg, icon_ref.as_str()) {
            (Some(_), _) => icon_id(&id),
            (None, name) if key(name) && builtin(name) => name.to_string(),
            _ => "play".to_string(),
        };
        let contributed = Contributed {
            plugin: manifest.id.clone(),
            plugin_name: manifest.name.clone(),
            approval: approval.to_string(),
            tooltip: cmd.tooltip.clone(),
            icon_svg,
            params: cmd.params.clone(),
            entry: cmd.entry.clone(),
            formats: if runtime.kind == Kind::Importer { runtime.formats.clone() } else { Vec::new() },
        };
        let mut spec = CommandSpec::new(leak(id), leak(cmd.label.clone()), unreachable_run)
            .icon(leak(icon))
            .params(leak(params_doc(&contributed)))
            .enabled(match cmd.when {
                When::Always => enabled_always,
                When::NoSketch => crate::cmd::not_in_sketch,
                When::Sketch => crate::cmd::in_sketch,
            });
        if let Some(place) = place {
            spec = spec.at(leak(place.tab.clone()), leak(place.panel.clone()));
        }
        // A shortcut a built-in command (or an earlier plugin) has stays theirs.
        if let Some(key) = cmd.shortcut.as_ref().filter(|k| !taken.contains(&k.to_ascii_lowercase())) {
            spec = spec.key(leak(key.clone()));
        }
        // An importer's command only reads its file; the import makes its own undo step.
        if runtime.kind == Kind::Importer {
            spec = spec.noundo();
        }
        spec.plugin = Some(Box::leak(Box::new(contributed)));
        out.push(&*Box::leak(Box::new(spec)));
    }
    out
}

/// Put the commands of every installed plugin in `dir` that is allowed for its exact content
/// and not turned off into the registry, and take out the rest. `builtin_icon` says whether an
/// icon name is one of the app's.
pub fn sync(dir: Option<&Path>, approvals: &BTreeMap<String, String>, disabled: &BTreeSet<String>, builtin_icon: &dyn Fn(&str) -> bool) {
    let installed = dir.map(super::installed).unwrap_or_default();
    let mut taken: BTreeSet<String> = crate::cmd::builtin_specs().iter().filter_map(|c| c.shortcut).map(str::to_ascii_lowercase).collect();
    let _ = with_registry(|registry| {
        let registry = registry.get_or_insert_with(Registry::default);
        let before: Vec<*const CommandSpec> = registry.active.iter().map(|s| *s as *const CommandSpec).collect();
        registry.active.clear();
        for p in installed {
            if disabled.contains(&p.manifest.id) || approvals.get(&p.manifest.id) != Some(&p.approval) || p.manifest.contributes.is_empty() {
                continue;
            }
            let specs = match registry.made.get(&p.approval) {
                Some(specs) => specs.clone(),
                None => {
                    let Ok(package) = super::read_package(&p.path.to_string_lossy()) else { continue };
                    if package.approval != p.approval {
                        continue;
                    }
                    let specs = build(&package.manifest, &package.approval, &package.icons, builtin_icon, &taken);
                    registry.made.insert(p.approval.clone(), specs.clone());
                    specs
                }
            };
            for spec in specs {
                if registry.active.iter().all(|s| s.id != spec.id) {
                    taken.extend(spec.shortcut.map(str::to_ascii_lowercase));
                    registry.active.push(spec);
                }
            }
        }
        if registry.active.iter().map(|s| *s as *const CommandSpec).ne(before) {
            GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    });
}

/// Run contributed command `spec` with `params`: check them against its declaration, then
/// import (an importer) or run its entry (a command plugin), as long as its plugin is still
/// allowed for the content the command came from.
pub(crate) fn run(s: &mut Session, spec: &CommandSpec, c: &Contributed, params: &Value) -> Result<Value> {
    let bad = |msg: String| EngineError::BadParams { cmd: spec.id.to_string(), msg };
    let installed = super::allowed_installed(s, &c.plugin).map_err(EngineError::Other)?;
    if installed.approval != c.approval {
        return Err(EngineError::Other(format!("plugin `{}` changed since it was allowed: allow it again in Preferences > Plugins", c.plugin)));
    }
    let given = match params {
        Value::Object(o) => o.clone(),
        Value::Null => Default::default(),
        _ => return Err(bad("parameters must be an object".into())),
    };
    let host_keys: &[&str] = if c.imports() { &["path", "into"] } else { &[] };
    if let Some(unknown) = given.keys().find(|k| !host_keys.contains(&k.as_str()) && !c.params.iter().any(|p| &p.name == *k)) {
        return Err(bad(format!("unknown parameter `{unknown}`")));
    }
    let mut checked = serde_json::Map::new();
    for p in &c.params {
        match given.get(&p.name).or(p.default.as_ref()) {
            Some(v) => {
                checked.insert(p.name.clone(), p.check(v).map_err(bad)?);
            }
            None if p.required => return Err(bad(format!("`{}` is required", p.name))),
            None => {}
        }
    }
    if c.imports() {
        let path = given.get("path").and_then(Value::as_str).filter(|p| !p.trim().is_empty()).ok_or_else(|| bad("`path` is required".into()))?;
        let ext = Path::new(path).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
        if !c.formats.iter().any(|f| f.extension == ext) {
            return Err(bad(format!(
                "this command imports {}",
                c.formats.iter().map(|f| format!(".{}", f.extension)).collect::<Vec<_>>().join(", ")
            )));
        }
        let into_current = match given.get("into").and_then(Value::as_str) {
            None | Some("new") => false,
            Some("current") => true,
            Some(_) => return Err(bad("`into` must be new or current".into())),
        };
        let how = crate::cmd::file::ImportOptions { options: checked, into_current };
        crate::cmd::file::import_with(s, path, &installed.path.to_string_lossy(), None, &how)
    } else {
        let entry = c.entry.as_deref().unwrap_or_default();
        let package = super::read_package(&installed.path.to_string_lossy())?;
        super::run(s, package, entry, &Value::Object(checked))
    }
}

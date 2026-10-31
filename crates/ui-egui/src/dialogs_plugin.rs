//! A plugin command's dialog, drawn from the parameters its package declares: numbers (with a
//! unit, a length or an angle), checkboxes, choices, text and picks of faces, edges or bodies,
//! in the same rows, fields and OK/Cancel as the app's own dialogs. An importer's command first
//! asks for the file (filtered to its formats), then shows its options and where the result
//! goes; OK imports it on the file worker, with progress and Cancel.

use serde_json::{Value, json};
use solidcraft_engine::plugin::contrib::{Contributed, ParamKind, ParamSpec, Pick};
use solidcraft_engine::{CommandSpec, Sel};

use crate::SolveApp;
use crate::dialogs::{Dialog, Kind, form_row};
use crate::selection::SelInput;

/// A value being edited in the dialog.
#[derive(Clone, Debug, PartialEq)]
pub enum Field {
    Number(f64),
    Bool(bool),
    /// An index into the parameter's options.
    Choice(usize),
    Text(String),
    /// Picked in the view: the dialog input with this index.
    Picks(usize),
}

/// The dialog's state: the command, the file (an importer's), the values and where an
/// import goes.
#[derive(Clone, Debug, PartialEq)]
pub struct Form {
    pub command: &'static str,
    pub path: Option<String>,
    pub fields: Vec<Field>,
    pub into_current: bool,
}

impl Form {
    fn spec(&self) -> Option<(&'static CommandSpec, &'static Contributed)> {
        let spec = solidcraft_engine::find_command(self.command)?;
        Some((spec, spec.plugin?))
    }
    /// Whether OK imports a file.
    pub fn imports(&self) -> bool {
        self.spec().is_some_and(|(_, c)| c.imports())
    }
}

fn start_value(p: &ParamSpec, picks: &mut usize) -> Field {
    let d = p.default.as_ref();
    match p.kind {
        ParamKind::Number | ParamKind::Length | ParamKind::Angle => {
            Field::Number(d.and_then(Value::as_f64).or(p.min.filter(|m| *m > 0.0)).unwrap_or(0.0))
        }
        ParamKind::Bool => Field::Bool(d.and_then(Value::as_bool).unwrap_or(false)),
        ParamKind::Enum => Field::Choice(d.and_then(Value::as_str).and_then(|v| p.options.iter().position(|o| o.value == v)).unwrap_or(0)),
        ParamKind::String => Field::Text(d.and_then(Value::as_str).unwrap_or_default().to_string()),
        ParamKind::Selection => {
            *picks += 1;
            Field::Picks(*picks - 1)
        }
    }
}

/// Whether `id` is a plugin's command the dialog draws.
pub fn handles(id: &str) -> bool {
    solidcraft_engine::find_command(id).is_some_and(|c| c.plugin.is_some())
}

/// Start plugin command `id` from the ribbon: an importer's asks for the file first.
pub fn start(app: &mut SolveApp, id: &str) {
    let Some(spec) = solidcraft_engine::find_command(id) else { return };
    let Some(c) = spec.plugin else { return };
    if c.imports() {
        let extensions: Vec<&'static str> = c.formats.iter().map(|f| f.extension.as_str()).collect();
        let command = spec.id;
        app.pick_open_for(&extensions, move |app, path| {
            if let Some(path) = path {
                open(app, command, Some(path));
            }
        });
    } else {
        open(app, spec.id, None);
    }
}

/// Open the dialog of plugin command `id` from the control channel (`ui.start`): an importer's
/// on `path`, which it must be given; with no path, as the ribbon starts it.
pub fn open_for(app: &mut SolveApp, id: &str, path: Option<String>) -> Result<(), String> {
    let spec = solidcraft_engine::find_command(id).ok_or("unknown command")?;
    let c = spec.plugin.ok_or("not a plugin's command")?;
    match path {
        Some(path) if c.imports() => {
            let ext = std::path::Path::new(&path).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
            if !c.formats.iter().any(|f| f.extension == ext) {
                return Err(format!(
                    "{} imports {}",
                    spec.label,
                    c.formats.iter().map(|f| format!(".{}", f.extension)).collect::<Vec<_>>().join(", ")
                ));
            }
            open(app, spec.id, Some(path));
        }
        Some(_) => return Err(format!("`{id}` takes no path")),
        None => start(app, spec.id),
    }
    Ok(())
}

/// Open the dialog of `command` (for `path`, an importer's file).
pub fn open(app: &mut SolveApp, command: &'static str, path: Option<String>) {
    let Some(c) = solidcraft_engine::find_command(command).and_then(|s| s.plugin) else { return };
    let mut picks = 0;
    let fields = c.params.iter().map(|p| start_value(p, &mut picks)).collect();
    let inputs = c
        .params
        .iter()
        .filter(|p| p.kind == ParamKind::Selection)
        .map(|p| {
            let accept = match p.accept {
                Some(Pick::Faces) => crate::selection::FACES,
                Some(Pick::Edges) => crate::selection::EDGES,
                _ => crate::selection::BODIES,
            };
            SelInput::new(p.label.as_str(), accept, p.many)
        })
        .collect();
    app.dialog = Some(Dialog::new(Kind::Plugin(Form { command, path, fields, into_current: false }), inputs));
}

/// The dialog's title: the command's label.
pub fn title(f: &Form) -> &'static str {
    f.spec().map_or("PLUGIN", |(s, _)| s.label)
}

/// The rows: the file, each declared parameter, and where an import goes. Selection inputs are
/// drawn by the dialog's own input rows.
pub fn rows(ui: &mut egui::Ui, f: &mut Form) -> bool {
    let Some((_, c)) = f.spec() else { return false };
    let mut enter = false;
    if let Some(path) = &f.path {
        let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.clone());
        form_row(ui, crate::i18n::tr("ui.file"), |ui| {
            ui.label(name).on_hover_text(path);
        });
    }
    for (i, (p, field)) in c.params.iter().zip(f.fields.iter_mut()).enumerate() {
        let handle = format!("dialog:plugin:{}", p.name);
        match field {
            Field::Number(v) => form_row(ui, p.label.as_str(), |ui| {
                let unit = match p.kind {
                    ParamKind::Length => " mm".to_string(),
                    ParamKind::Angle => "\u{b0}".to_string(),
                    _ if p.unit.is_empty() => String::new(),
                    _ => format!(" {}", p.unit),
                };
                let r = crate::ui_kit::number_field(ui, v, p.min.unwrap_or(-1e9)..=p.max.unwrap_or(1e9), 0.1, &unit);
                crate::scenario::publish_handle(&handle, r.rect.center());
            }),
            Field::Bool(on) => form_row(ui, p.label.as_str(), |ui| crate::dialogs::check(ui, on)),
            Field::Choice(sel) => form_row(ui, p.label.as_str(), |ui| {
                let labels: Vec<&str> = p.options.iter().map(|o| o.label.as_str()).collect();
                crate::dialogs::combo(ui, &format!("plugin_param_{i}"), &labels, sel);
            }),
            Field::Text(text) => form_row(ui, p.label.as_str(), |ui| {
                enter |= crate::ui_kit::text_field(ui, text, "").0;
            }),
            Field::Picks(_) => {}
        }
        if !p.help.is_empty() {
            crate::ui_kit::text(ui, &p.help, crate::ui_kit::Text::Note("plugin_param_help"));
        }
    }
    if c.imports() {
        form_row(ui, crate::i18n::tr("ui.plugin_open_as"), |ui| {
            let mut sel = usize::from(f.into_current);
            let choices = [crate::i18n::tr("ui.plugin_into_new"), crate::i18n::tr("ui.plugin_into_current")];
            crate::dialogs::combo(ui, "plugin_import_into", &choices, &mut sel);
            f.into_current = sel == 1;
        });
    }
    enter
}

fn pick_json(sel: &Sel) -> Option<Value> {
    Some(match sel {
        Sel::Body { name } => json!(name),
        Sel::Face { body, index, .. } => json!({"body": body, "face": index}),
        Sel::Edge { body, index, .. } => json!({"body": body, "edge": index}),
        _ => return None,
    })
}

/// The parameters OK runs the command with.
pub fn params(f: &Form, inputs: &[SelInput]) -> Result<Value, String> {
    let (_, c) = f.spec().ok_or("the plugin was turned off")?;
    let mut out = serde_json::Map::new();
    for (p, field) in c.params.iter().zip(&f.fields) {
        let value = match field {
            Field::Number(v) => json!(v),
            Field::Bool(on) => json!(on),
            Field::Choice(i) => json!(p.options.get(*i).map(|o| o.value.as_str()).unwrap_or_default()),
            Field::Text(t) if t.is_empty() && !p.required => continue,
            Field::Text(t) => json!(t),
            Field::Picks(i) => {
                let picks: Vec<Value> = inputs.get(*i).map(|inp| inp.items.iter().filter_map(pick_json).collect()).unwrap_or_default();
                match (picks.is_empty(), p.many) {
                    (true, _) if p.required => return Err(format!("pick {}", p.label.to_lowercase())),
                    (true, _) => continue,
                    (false, true) => Value::Array(picks),
                    (false, false) => picks.into_iter().next().unwrap_or_default(),
                }
            }
        };
        p.check(&value)?;
        out.insert(p.name.clone(), value);
    }
    if let Some(path) = &f.path {
        out.insert("path".into(), json!(path));
        out.insert("into".into(), json!(if f.into_current { "current" } else { "new" }));
    }
    Ok(Value::Object(out))
}

/// What OK runs, for a command plugin's command (the dialog applies it as any other).
pub fn commands(f: &Form, inputs: &[SelInput]) -> Result<Vec<(String, Value)>, String> {
    Ok(vec![(f.command.to_string(), params(f, inputs)?)])
}

/// OK on an importer's dialog: the import runs on the file worker, with its progress and
/// Cancel, as a new design or into this one.
pub fn start_import(app: &mut SolveApp, f: &Form, inputs: &[SelInput]) -> Result<(), String> {
    let params = params(f, inputs)?;
    let path = f.path.clone().ok_or("no file was picked")?;
    let mut options = params.as_object().cloned().unwrap_or_default();
    options.remove("path");
    let kind = if f.into_current { crate::file_load::Kind::Insert } else { crate::file_load::Kind::Open };
    let mut request = crate::file_load::Request::new(&path, kind);
    request.options = options;
    request.command = Some(f.command);
    request.import = !f.into_current;
    crate::file_load::begin_request(app, request)
}

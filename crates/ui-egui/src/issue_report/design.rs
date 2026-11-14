//! "Include this design" in the report: the open design and the script of its history, as one zip a
//! reporter drags into the issue (a new-issue link cannot carry a file).

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::SolveApp;

/// The history script inside the zip, as `solidcraft-cli script design.sc --out repro.json` writes
/// it, so the history reads without the app.
pub(crate) const REPRO: &str = "repro.json";

/// What was prepared: the zip's file name and, where it was written to a folder, its path.
pub(crate) struct Attached {
    pub name: String,
    pub path: Option<PathBuf>,
}

/// The folder the app keeps report designs in: beside the crash reports, or the one a test sets.
fn folder(app: &SolveApp) -> Option<PathBuf> {
    app.issue_report.folder.clone().or_else(|| solidcraft_engine::crash::default_dir().and_then(|d| d.parent().map(|p| p.join("reports"))))
}

/// Whether the zip goes out as a browser download (the web build) rather than into a folder.
fn downloads(app: &SolveApp) -> bool {
    app.services.download_design.is_some()
}

/// The message id saying why the design cannot be included now; `None` when it can.
pub(crate) fn unavailable(app: &SolveApp) -> Option<&'static str> {
    if app.home.open {
        Some("ui.report_design_none")
    } else if !downloads(app) && folder(app).is_none() {
        Some("ui.report_design_nowhere")
    } else {
        None
    }
}

/// A file name stem made of what is safe in any file system.
fn stem(name: &str) -> String {
    let stem: String = name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).take(60).collect();
    let stem = stem.trim_matches('-');
    if stem.is_empty() { "design".into() } else { stem.into() }
}

/// The zip's name and bytes: the design (`<name>.sc`) and `repro.json`. A history that cannot be
/// written as a script leaves the design alone in the zip and says so in `repro.json`.
pub(crate) fn package(app: &mut SolveApp) -> Result<(String, Vec<u8>), String> {
    let design = app.session.design_container(solidcraft_engine::io::DesignCompression::Fast, false, false).map_err(|e| e.to_string())?;
    let repro = match app.session.execute("timeline.export_script", &json!({})) {
        Ok(reply) => reply.get("script").cloned().unwrap_or(Value::Null),
        Err(e) => json!({"unavailable": e.to_string()}),
    };
    let repro = serde_json::to_vec_pretty(&repro).map_err(|e| e.to_string())?;
    let stem = stem(&app.session.doc.name);
    let design_name = format!("{stem}.{}", solidcraft_engine::io::DESIGN_EXT);
    let zip = solidcraft_engine::io::safe_zip::write(&[(&design_name, &design), (REPRO, &repro)]).map_err(|e| e.to_string())?;
    Ok((format!("{stem}-report.zip"), zip))
}

/// Write `bytes` to a new file in `dir`: `name`, or `name` with `-2`, `-3`, … before the extension
/// when that file exists, so an earlier report's zip is never replaced. A request's folders
/// (`allow_paths`) are honoured: a folder outside them is refused before anything is written.
#[cfg(not(target_arch = "wasm32"))]
fn write_to(dir: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    use std::io::Write;
    solidcraft_engine::io::vfs::check(&dir.to_string_lossy()).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let (stem, ext) = name.rsplit_once('.').unwrap_or((name, ""));
    for n in 1..=1000u32 {
        let file = match (n, ext.is_empty()) {
            (1, _) => name.to_string(),
            (_, true) => format!("{stem}-{n}"),
            (_, false) => format!("{stem}-{n}.{ext}"),
        };
        let path = dir.join(file);
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut f) => {
                f.write_all(bytes).map_err(|e| e.to_string())?;
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Err("too many report archives with the same name".into())
}

#[cfg(target_arch = "wasm32")]
fn write_to(_dir: &Path, _name: &str, _bytes: &[u8]) -> Result<PathBuf, String> {
    Err("the web app has no folder to save to".into())
}

/// Save the zip of the open design. With `show`, a person asked for it in the dialog: the folder
/// opens in the file browser (the web build offers a download). Without it, a programmatic call:
/// the zip is written and nothing opens.
pub(crate) fn attach(app: &mut SolveApp, show: bool) -> Result<Attached, String> {
    if let Some(reason) = unavailable(app) {
        return Err(solidcraft_ui_text::tr(reason).to_string());
    }
    let (name, bytes) = package(app)?;
    if downloads(app) {
        if !show {
            return Err(solidcraft_ui_text::tr("ui.report_design_web_only").to_string());
        }
        if let Some(download) = app.services.download_design.as_ref() {
            download(&name, &bytes)?;
        }
        return Ok(Attached { name, path: None });
    }
    let dir = folder(app).ok_or_else(|| solidcraft_ui_text::tr("ui.report_design_nowhere").to_string())?;
    let path = write_to(&dir, &name, &bytes)?;
    let name = path.file_name().map_or(name, |n| n.to_string_lossy().into_owned());
    if show && let Some(show_in_folder) = app.services.show_in_folder.as_ref() {
        show_in_folder(&path.to_string_lossy());
    }
    Ok(Attached { name, path: Some(path) })
}

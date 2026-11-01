//! Dutchman Convert as a SolidCraft importer plugin: Fusion, Inventor, SolidWorks and FreeCAD
//! files in, a SolidCraft design with the mapped feature history out.
//!
//! One crate, two builds of the same converter:
//! - `wasm32-wasip2`: the component the desktop app runs (the `importer-plugin` WIT world).
//! - `wasm32-wasip1`: the core module the browser runs (SolidCraft's web ABI, see
//!   `docs/plugin-runtime.md`), in a Web Worker. WASI preview 1 gives the standard library its
//!   clock (the decoders time their work); the browser host answers it with nothing real.
//!
//! Either is given one file's bytes and nothing else: no files, network, clock or processes.
//! Dutchman's checks that need the SolidCraft CLI (replaying the history to prove it, and
//! finding the edges of fillets and chamfers) are therefore off: blends are left out and said
//! so in the notes, the original geometry is the visible model, and the unchecked history is
//! kept in a hidden "Editable history (unverified)" part beside it.

use std::path::Path;

use dutchman_core::{Format, Report, Writer};
use dutchman_solidcraft::{SolidCraftWriter, Verification};

/// The formats this importer converts: what Dutchman reads from one file, with no other files
/// beside it.
const FORMATS: [Format; 5] = [
    Format::FusionArchive,
    Format::FusionMultiDocument,
    Format::InventorPart,
    Format::SolidWorksPart,
    Format::FreeCadDocument,
];

/// Most bytes of notes handed back.
const MAX_NOTES: usize = 64 * 1024;

/// The steps of an import, as the host shows them.
#[derive(Clone, Copy)]
enum Step {
    Reading,
    Decoding,
    Mapping,
    Writing,
}

/// What the user chose in the import dialog (the parameters `plugin.json` declares, checked by
/// SolidCraft), and where the result goes.
struct Options {
    /// The feature history (hidden and unverified) as well as the geometry.
    history: bool,
    /// Reference sketches too.
    sketches: bool,
}

impl Options {
    fn from_json(text: &str) -> Self {
        let options: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
        let text = |key: &str| options.get(key).and_then(serde_json::Value::as_str);
        let preset = text("preset").unwrap_or("standard");
        // Into an open design only the geometry comes along.
        let history = options.get("keep_history").and_then(serde_json::Value::as_bool) != Some(false)
            && preset != "fast"
            && text("into") != Some("current");
        Options { history, sketches: preset == "full" }
    }
}

/// Why an import failed: the file is not one this importer reads, or converting it failed.
enum Failure {
    Invalid(String),
    Failed(String),
}

/// Convert one file: the design's bytes and the notes (what did not come across).
fn convert(file_name: &str, bytes: Vec<u8>, options: &str, progress: &dyn Fn(Step, u32, u32)) -> Result<(Vec<u8>, String), Failure> {
    progress(Step::Reading, 0, 1);
    let name = Path::new(file_name);
    let format = Format::from_path(name)
        .filter(|f| FORMATS.contains(f))
        .ok_or_else(|| Failure::Invalid(format!("{file_name}: not a file this importer converts")))?;
    let options = Options::from_json(options);
    let mut report = Report::default();
    progress(Step::Decoding, 0, 1);
    let model = dutchman_core::read_bytes(format, name, &bytes, &mut report).map_err(|e| Failure::Failed(e.to_string()))?;
    drop(bytes);
    progress(Step::Mapping, 0, 1);
    // No SolidCraft CLI exists in the sandbox, so the history cannot be checked against the
    // original: keep the original as the visible model and the history hidden.
    let writer = SolidCraftWriter::new()
        .verification(Verification::Off)
        .hide_unverified_history(true)
        .import_only(!options.history)
        .sketches(options.sketches);
    let mut design = Vec::new();
    progress(Step::Writing, 0, 1);
    writer.write(&model, &mut design, &mut report).map_err(|e| Failure::Failed(e.to_string()))?;
    progress(Step::Writing, 1, 1);
    Ok((design, notes(&report)))
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

/// The desktop build: a component in the `importer-plugin` world.
#[cfg(all(target_os = "wasi", target_env = "p2"))]
mod component {
    mod bindings {
        wit_bindgen::generate!({ path: "../../../wit", world: "importer-plugin" });
    }
    use bindings::exports::solidcraft::plugin::importer::{Guest, Imported, Payload};
    use bindings::solidcraft::plugin::import_context::{self, Stage};
    use bindings::solidcraft::plugin::types::PluginError;

    struct DutchmanConvert;

    impl Guest for DutchmanConvert {
        fn import(file_name: String, bytes: Vec<u8>) -> Result<Imported, PluginError> {
            let progress = |step: super::Step, done: u32, total: u32| {
                let stage = match step {
                    super::Step::Reading => Stage::Reading,
                    super::Step::Decoding => Stage::Decoding,
                    super::Step::Mapping => Stage::Mapping,
                    super::Step::Writing => Stage::Writing,
                };
                import_context::progress(stage, done, total);
            };
            match super::convert(&file_name, bytes, &import_context::options(), &progress) {
                Ok((design, notes)) => Ok(Imported { payload: Payload::Design(design), notes }),
                Err(super::Failure::Invalid(e)) => Err(PluginError::Invalid(e)),
                Err(super::Failure::Failed(e)) => Err(PluginError::Failed(e)),
            }
        }
    }

    bindings::export!(DutchmanConvert with_types_in bindings);
}

/// The browser build: a core module with SolidCraft's web ABI. The host writes the file into
/// memory this module hands out (`sc_alloc`), calls `sc_import`, and reads the result back.
#[cfg(all(target_os = "wasi", target_env = "p1"))]
mod web {
    use std::cell::RefCell;

    #[link(wasm_import_module = "env")]
    unsafe extern "C" {
        fn sc_progress(stage: i32, done: i32, total: i32);
        fn sc_log(ptr: i32, len: i32);
    }

    /// A line for the host's notes (and its error, when the import stops).
    fn log(line: &str) {
        let line = &line.as_bytes()[..line.len().min(1024)];
        // SAFETY: an import the host gives every module (the web ABI's `env.sc_log`); it only
        // reads `len` bytes at `ptr`, which `line` holds.
        unsafe { sc_log(line.as_ptr() as i32, line.len() as i32) };
    }

    struct Done {
        kind: i32,
        payload: Vec<u8>,
        notes: Vec<u8>,
    }

    thread_local! {
        static RESULTS: RefCell<Vec<Option<Done>>> = const { RefCell::new(Vec::new()) };
    }

    /// Room for `len` bytes the host writes into; taken back by `sc_import`.
    #[unsafe(no_mangle)]
    pub extern "C" fn sc_alloc(len: i32) -> i32 {
        let room = vec![0u8; usize::try_from(len).unwrap_or(0)].into_boxed_slice();
        Box::into_raw(room).cast::<u8>() as i32
    }

    /// The bytes the host wrote at `ptr` (from `sc_alloc(len)`), owned again.
    fn take(ptr: i32, len: i32) -> Vec<u8> {
        let len = usize::try_from(len).unwrap_or(0);
        // SAFETY: `ptr` and `len` are those of a block `sc_alloc` handed out and nothing else
        // owns; the host only wrote into it.
        unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr as *mut u8, len)) }.into_vec()
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn sc_import(name_ptr: i32, name_len: i32, bytes_ptr: i32, bytes_len: i32, options_ptr: i32, options_len: i32) -> i32 {
        // A panic aborts the module (Wasm has no unwinding): say where first, for the host's error.
        std::panic::set_hook(Box::new(|info| log(&format!("the converter stopped: {info}"))));
        let name = String::from_utf8_lossy(&take(name_ptr, name_len)).into_owned();
        let bytes = take(bytes_ptr, bytes_len);
        let options = String::from_utf8_lossy(&take(options_ptr, options_len)).into_owned();
        let progress = |step: super::Step, done: u32, total: u32| {
            let code = step as i32;
            // SAFETY: an import the host gives every module (the web ABI's `env.sc_progress`).
            unsafe { sc_progress(code, done as i32, total as i32) };
        };
        let done = match super::convert(&name, bytes, &options, &progress) {
            Ok((design, notes)) => Done { kind: 0, payload: design, notes: notes.into_bytes() },
            Err(super::Failure::Invalid(e) | super::Failure::Failed(e)) => Done { kind: 2, payload: e.into_bytes(), notes: Vec::new() },
        };
        RESULTS.with(|r| {
            let mut r = r.borrow_mut();
            r.push(Some(done));
            r.len() as i32 - 1
        })
    }

    fn with<T>(handle: i32, f: impl FnOnce(&Done) -> T, none: T) -> T {
        RESULTS.with(|r| r.borrow().get(usize::try_from(handle).unwrap_or(usize::MAX)).and_then(Option::as_ref).map(f).unwrap_or(none))
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn sc_result_kind(handle: i32) -> i32 {
        with(handle, |d| d.kind, 2)
    }
    #[unsafe(no_mangle)]
    pub extern "C" fn sc_result_ptr(handle: i32) -> i32 {
        with(handle, |d| d.payload.as_ptr() as i32, 0)
    }
    #[unsafe(no_mangle)]
    pub extern "C" fn sc_result_len(handle: i32) -> i32 {
        with(handle, |d| d.payload.len() as i32, 0)
    }
    #[unsafe(no_mangle)]
    pub extern "C" fn sc_notes_ptr(handle: i32) -> i32 {
        with(handle, |d| d.notes.as_ptr() as i32, 0)
    }
    #[unsafe(no_mangle)]
    pub extern "C" fn sc_notes_len(handle: i32) -> i32 {
        with(handle, |d| d.notes.len() as i32, 0)
    }
    #[unsafe(no_mangle)]
    pub extern "C" fn sc_free(handle: i32) {
        RESULTS.with(|r| {
            if let Some(slot) = r.borrow_mut().get_mut(usize::try_from(handle).unwrap_or(usize::MAX)) {
                *slot = None;
            }
        });
    }
}

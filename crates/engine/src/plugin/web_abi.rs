//! The web ABI: how the browser build runs an importer. Browsers run core WebAssembly modules,
//! not components, so an importer that also works in the browser ships a core module built for
//! `wasm32-unknown-unknown` (`runtime.web.module`) with this small C-style interface:
//!
//! Imports (nothing else is given; a module asking for anything else is refused):
//! - `env.memory`: the module's only memory, made by the host with the package's memory limit as
//!   its maximum (link with `--import-memory`).
//! - `env.sc_progress(stage: i32, done: i32, total: i32)`: the step the import is on (0 reading,
//!   1 decoding, 2 mapping, 3 writing), as `import-context.progress`.
//! - `env.sc_log(ptr: i32, len: i32)`: a line for the notes (UTF-8, at most 1 KiB).
//! - For a module built for WASI preview 1 (`wasm32-wasip1`), the [`WASI`] functions Rust's
//!   standard library calls, answered with nothing real (see [`WASI`]). It may export
//!   `_initialize`, which the host calls first.
//!
//! Exports:
//! - `sc_alloc(len: i32) -> i32`: room for `len` bytes the host writes into.
//! - `sc_import(name_ptr, name_len, bytes_ptr, bytes_len, options_ptr, options_len: i32) -> i32`:
//!   convert the file (its name without folders, its bytes, the options as JSON) and return a
//!   handle to the result.
//! - `sc_result_kind(handle) -> i32`: 0 a SolidCraft design, 1 STEP text, 2 failed (the payload
//!   is then the error's text).
//! - `sc_result_ptr(handle) -> i32`, `sc_result_len(handle) -> i32`: the payload.
//! - `sc_notes_ptr(handle) -> i32`, `sc_notes_len(handle) -> i32`: what did not come across.
//! - `sc_free(handle)`: the host is done with the result.
//!
//! The host runs each import in a Web Worker of its own: the time limit and Cancel end the
//! worker, the memory limit is the memory's maximum, and the worker has no network, page or
//! storage access to give. The logic here is plain Rust, shared with native tests.

use super::{Imported, Limits, MAX_IMPORT_BYTES, MAX_NOTES_BYTES, Payload, error};
use crate::Result;

/// A WebAssembly page.
pub const PAGE_BYTES: usize = 64 * 1024;

/// What a module may import, and of which kind.
pub const IMPORTS: [(&str, &str, &str); 3] = [("env", "memory", "memory"), ("env", "sc_progress", "function"), ("env", "sc_log", "function")];

/// The WASI preview 1 functions a module may import (`wasi_snapshot_preview1`): what Rust's
/// standard library calls. The host answers them with nothing real: no arguments, environment
/// or files, a clock that counts its own readings, fixed "random" bytes, output to the log.
pub const WASI: &[&str] = &[
    "args_get",
    "args_sizes_get",
    "clock_res_get",
    "clock_time_get",
    "environ_get",
    "environ_sizes_get",
    "fd_close",
    "fd_fdstat_get",
    "fd_fdstat_set_flags",
    "fd_filestat_get",
    "fd_prestat_dir_name",
    "fd_prestat_get",
    "fd_read",
    "fd_readdir",
    "fd_seek",
    "fd_sync",
    "fd_tell",
    "fd_write",
    "path_create_directory",
    "path_filestat_get",
    "path_link",
    "path_open",
    "path_readlink",
    "path_remove_directory",
    "path_rename",
    "path_unlink_file",
    "poll_oneoff",
    "proc_exit",
    "random_get",
];

/// What a module must export.
pub const EXPORTS: [&str; 8] =
    ["sc_alloc", "sc_import", "sc_result_kind", "sc_result_ptr", "sc_result_len", "sc_notes_ptr", "sc_notes_len", "sc_free"];

/// The maximum pages of the memory the host gives a module with these limits.
pub fn memory_pages(limits: &Limits) -> u32 {
    let pages = limits.memory_mib.saturating_mul(1024 * 1024) / PAGE_BYTES;
    u32::try_from(pages).unwrap_or(u32::MAX).min(65536)
}

/// Refuse a module that imports anything but the ABI's imports, of their kinds, or lacks an
/// export. `imports`: (module, name, kind) as `WebAssembly.Module.imports` lists them.
pub fn check_interface(imports: &[(String, String, String)], exports: &[String]) -> Result<()> {
    for (module, name, kind) in imports {
        let wasi = module == "wasi_snapshot_preview1" && kind == "function" && WASI.contains(&name.as_str());
        if !wasi && !IMPORTS.iter().any(|(m, n, k)| m == module && n == name && k == kind) {
            return Err(error(format!("the web module asks for `{module}.{name}` ({kind}), which the browser host does not give")));
        }
    }
    if !imports.iter().any(|(m, n, _)| m == "env" && n == "memory") {
        return Err(error("the web module must import its memory (`env.memory`; link with --import-memory)"));
    }
    if let Some(missing) = EXPORTS.iter().find(|e| !exports.iter().any(|x| x == *e)) {
        return Err(error(format!("the web module does not export `{missing}`")));
    }
    Ok(())
}

/// What a core module imports and exports, read from its bytes: the browser host checks it
/// against the ABI before it compiles anything, and sizes the memory it gives from it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Interface {
    /// (module, name, kind) of each import; kind is `function`, `table`, `memory`, `global` or
    /// `tag`.
    pub imports: Vec<(String, String, String)>,
    /// The minimum pages of the memory it imports, when it imports one.
    pub memory_min: Option<u64>,
    pub exports: Vec<String>,
}

/// Read a core module's imports and exports. Only the import and export sections are read;
/// a component, or bytes that are not a module, are refused.
pub fn interface(module: &[u8]) -> Result<Interface> {
    struct Reader<'a> {
        b: &'a [u8],
        at: usize,
    }
    impl<'a> Reader<'a> {
        fn byte(&mut self) -> Result<u8> {
            let b = *self.b.get(self.at).ok_or_else(|| error("the web module ends early"))?;
            self.at += 1;
            Ok(b)
        }
        fn uleb(&mut self) -> Result<u64> {
            let mut v = 0u64;
            for shift in (0..64).step_by(7) {
                let b = self.byte()?;
                v |= u64::from(b & 0x7f) << shift;
                if b & 0x80 == 0 {
                    return Ok(v);
                }
            }
            Err(error("the web module has a malformed number"))
        }
        fn bytes(&mut self, n: u64) -> Result<&'a [u8]> {
            let n = usize::try_from(n).map_err(|_| error("the web module is malformed"))?;
            let end = self.at.checked_add(n).filter(|e| *e <= self.b.len()).ok_or_else(|| error("the web module ends early"))?;
            let s = self.b.get(self.at..end).ok_or_else(|| error("the web module ends early"))?;
            self.at = end;
            Ok(s)
        }
        fn name(&mut self) -> Result<String> {
            let n = self.uleb()?;
            Ok(String::from_utf8_lossy(self.bytes(n)?).into_owned())
        }
        fn limits(&mut self) -> Result<u64> {
            let flags = self.byte()?;
            let min = self.uleb()?;
            if flags & 1 != 0 {
                self.uleb()?;
            }
            Ok(min)
        }
    }
    if module.get(..8) != Some(&[0, b'a', b's', b'm', 1, 0, 0, 0][..]) {
        return Err(error("the web module is not a core WebAssembly module (a component cannot run in the browser)"));
    }
    let mut r = Reader { b: module, at: 8 };
    let mut out = Interface::default();
    while r.at < module.len() {
        let id = r.byte()?;
        let size = r.uleb()?;
        let body = r.bytes(size)?;
        let mut s = Reader { b: body, at: 0 };
        match id {
            2 => {
                for _ in 0..s.uleb()?.min(100_000) {
                    let (m, n) = (s.name()?, s.name()?);
                    let kind = match s.byte()? {
                        0 => {
                            s.uleb()?;
                            "function"
                        }
                        1 => {
                            s.byte()?;
                            s.limits()?;
                            "table"
                        }
                        2 => {
                            let min = s.limits()?;
                            if m == "env" && n == "memory" {
                                out.memory_min = Some(min);
                            }
                            "memory"
                        }
                        3 => {
                            s.byte()?;
                            s.byte()?;
                            "global"
                        }
                        4 => {
                            s.byte()?;
                            s.uleb()?;
                            "tag"
                        }
                        _ => return Err(error("the web module has an import of an unknown kind")),
                    };
                    out.imports.push((m, n, kind.to_string()));
                }
            }
            7 => {
                for _ in 0..s.uleb()?.min(100_000) {
                    let n = s.name()?;
                    s.byte()?;
                    s.uleb()?;
                    out.exports.push(n);
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

/// The progress stage the app shows for an ABI stage code.
pub fn stage(code: i32) -> &'static str {
    match code {
        0 => "plugin.reading",
        1 => "plugin.decoding",
        2 => "plugin.mapping",
        _ => "plugin.writing",
    }
}

/// A byte range a module returned, checked against its memory's size.
pub fn range(ptr: i32, len: i32, memory_len: usize) -> Result<std::ops::Range<usize>> {
    let (Ok(ptr), Ok(len)) = (usize::try_from(ptr), usize::try_from(len)) else {
        return Err(error("the web module returned a negative pointer or length"));
    };
    let end = ptr.checked_add(len).filter(|end| *end <= memory_len).ok_or_else(|| error("the web module returned bytes outside its memory"))?;
    Ok(ptr..end)
}

/// What an import returned, as the engine takes it: `kind` from `sc_result_kind`, the payload
/// and the notes. A failed import is its error; sizes are checked as for a component.
pub fn imported(kind: i32, payload: Vec<u8>, notes: &[u8], log: &[String]) -> Result<Imported> {
    let text = |b: &[u8]| String::from_utf8_lossy(b).chars().filter(|c| *c == '\n' || !c.is_control()).collect::<String>();
    let payload = match kind {
        0 => Payload::Design(payload),
        1 => Payload::Step(payload),
        2 => return Err(error(format!("the importer failed: {}", text(&payload).chars().take(2048).collect::<String>()))),
        other => return Err(error(format!("the web module returned an unknown result kind {other}"))),
    };
    let (Payload::Design(made) | Payload::Step(made)) = &payload;
    if made.len() > MAX_IMPORT_BYTES {
        return Err(error("the imported design exceeds 512 MiB"));
    }
    if notes.len() > MAX_NOTES_BYTES {
        return Err(error("the importer's notes exceed 64 KiB"));
    }
    let mut notes = text(notes);
    for line in log.iter().take(64) {
        notes.push_str(&text(line.as_bytes()).chars().take(1024).collect::<String>());
        notes.push('\n');
    }
    Ok(Imported { payload, notes })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn the_memory_maximum_follows_the_package_limit() {
        let limits = |memory_mib| Limits { memory_mib, ..Limits::default() };
        assert_eq!(memory_pages(&limits(64)), 1024);
        assert_eq!(memory_pages(&limits(1024)), 16384);
        assert_eq!(memory_pages(&limits(4096)), 65536, "Wasm32's whole address space at most");
    }

    #[test]
    fn a_module_gets_only_the_abi_imports_and_must_export_it() {
        let own = |v: &[(&str, &str, &str)]| v.iter().map(|(a, b, c)| (a.to_string(), b.to_string(), c.to_string())).collect::<Vec<_>>();
        let exports: Vec<String> = EXPORTS.iter().map(|e| e.to_string()).collect();
        let abi = own(&IMPORTS);
        assert!(check_interface(&abi, &exports).is_ok());
        assert!(check_interface(&own(&[("env", "memory", "memory")]), &exports).is_ok(), "progress and log are optional");
        // The empty WASI's functions are allowed; nothing else.
        let mut with_wasi = abi.clone();
        with_wasi.extend(own(&[("wasi_snapshot_preview1", "fd_write", "function"), ("wasi_snapshot_preview1", "clock_time_get", "function")]));
        assert!(check_interface(&with_wasi, &exports).is_ok());
        for extra in [
            ("wasi_snapshot_preview1", "sock_accept", "function"),
            ("__wbindgen_placeholder__", "__wbindgen_describe", "function"),
            ("env", "fetch", "function"),
        ] {
            let mut imports = abi.clone();
            imports.extend(own(&[extra]));
            assert!(check_interface(&imports, &exports).unwrap_err().to_string().contains(extra.1), "{extra:?}");
        }
        // Its own memory instead of the host's (whose maximum is the limit): refused.
        assert!(check_interface(&own(&[("env", "sc_progress", "function")]), &exports).is_err());
        assert!(check_interface(&abi, &exports[1..]).unwrap_err().to_string().contains("sc_alloc"));
    }

    /// A hand-assembled module: imports env.memory (min 2) and env.sc_log, exports sc_alloc.
    fn tiny_module() -> Vec<u8> {
        let mut m = vec![0, b'a', b's', b'm', 1, 0, 0, 0];
        // Type section: one type () -> ().
        m.extend([1, 4, 1, 0x60, 0, 0]);
        let imports: Vec<u8> =
            [vec![2], vec![3, b'e', b'n', b'v', 6], b"memory".to_vec(), vec![2, 0, 2], vec![3, b'e', b'n', b'v', 6], b"sc_log".to_vec(), vec![0, 0]]
                .concat();
        m.push(2);
        m.push(imports.len() as u8);
        m.extend(imports);
        let exports: Vec<u8> = [vec![1, 8], b"sc_alloc".to_vec(), vec![0, 0]].concat();
        m.push(7);
        m.push(exports.len() as u8);
        m.extend(exports);
        m
    }

    #[test]
    fn a_modules_interface_is_read_from_its_bytes() {
        let i = interface(&tiny_module()).unwrap();
        assert_eq!(i.imports, vec![("env".into(), "memory".into(), "memory".into()), ("env".into(), "sc_log".into(), "function".into())]);
        assert_eq!(i.memory_min, Some(2));
        assert_eq!(i.exports, vec!["sc_alloc".to_string()]);
        // A component, garbage and a cut-short module are refused, never read past their end.
        assert!(interface(&[0, b'a', b's', b'm', 0x0d, 0, 1, 0]).is_err());
        assert!(interface(b"not wasm at all").is_err());
        let module = tiny_module();
        for cut in 9..module.len() {
            let _ = interface(&module[..cut]);
        }
    }

    #[test]
    fn results_are_bounded_and_failures_are_errors() {
        assert_eq!(range(8, 4, 16).unwrap(), 8..12);
        for (ptr, len) in [(-1, 4), (8, -1), (14, 4), (i32::MAX, i32::MAX)] {
            assert!(range(ptr, len, 16).is_err(), "{ptr} {len}");
        }
        let done = imported(0, b"design".to_vec(), b"lost a fillet\n", &["from the log".into()]).unwrap();
        assert!(matches!(done.payload, Payload::Design(ref b) if b == b"design"));
        assert_eq!(done.notes, "lost a fillet\nfrom the log\n");
        assert!(matches!(imported(1, b"ISO-10303-21;".to_vec(), b"", &[]).unwrap().payload, Payload::Step(_)));
        assert!(imported(2, b"no such feature".to_vec(), b"", &[]).unwrap_err().to_string().contains("no such feature"));
        assert!(imported(7, Vec::new(), b"", &[]).is_err());
        assert!(imported(0, Vec::new(), &vec![b'a'; MAX_NOTES_BYTES + 1], &[]).is_err());
        assert_eq!(stage(1), "plugin.decoding");
    }
}

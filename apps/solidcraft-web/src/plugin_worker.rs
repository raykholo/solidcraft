//! The browser's plugin worker: runs one importer plugin's core module (the web ABI, see
//! `docs/plugin-runtime.md` and `solidcraft_engine::plugin::web_abi`) in a Web Worker of its
//! own, started by the app for each import and ended when the import ends, its time runs out or
//! the user cancels.
//!
//! The module is given only what the ABI defines: its memory (made here, with the package's
//! memory limit as its maximum), a progress call, a log call, and for a module built for WASI
//! preview 1 an empty WASI answered here: no arguments, environment or files, a clock that
//! counts its own readings, fixed "random" bytes, and output that goes to the log. A worker has
//! no page to reach, and the module gets none of the worker's own functions: no network,
//! storage or real clock.
//!
//! It posts `{ready}` once it can take the import, receives `{module, name, bytes, options,
//! initial, maximum}`, and posts `{progress: [stage, done, total]}` and `{log: text}` while it
//! runs, then `{kind, payload, notes}` or `{error}`.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(target_arch = "wasm32")]
mod worker {
    use js_sys::{Array, Function, Object, Reflect, Uint8Array, WebAssembly};
    use wasm_bindgen::{JsCast as _, JsValue, closure::Closure};
    use wasm_bindgen_futures::{JsFuture, spawn_local};
    use web_sys::{DedicatedWorkerGlobalScope, MessageEvent};

    fn scope() -> DedicatedWorkerGlobalScope {
        js_sys::global().unchecked_into()
    }

    fn text(e: JsValue) -> String {
        e.as_string().or_else(|| Reflect::get(&e, &"message".into()).ok().and_then(|m| m.as_string())).unwrap_or_else(|| "the plugin stopped".into())
    }

    fn post(message: &Object, transfer: Option<&JsValue>) {
        let r = match transfer {
            Some(buffer) => scope().post_message_with_transfer(message, &Array::of1(buffer)),
            None => scope().post_message(message),
        };
        if let Err(e) = r {
            log::warn!("plugin worker: {}", text(e));
        }
    }

    fn set(o: &Object, key: &str, value: &JsValue) -> Result<(), String> {
        Reflect::set(o, &key.into(), value).map(|_| ()).map_err(text)
    }

    fn get(o: &JsValue, key: &str) -> Result<JsValue, String> {
        Reflect::get(o, &key.into()).map_err(text)
    }

    fn number(o: &JsValue, key: &str) -> Result<f64, String> {
        get(o, key)?.as_f64().ok_or_else(|| format!("`{key}` is not a number"))
    }

    /// Call an export with i32 arguments; its i32 result.
    fn call(exports: &JsValue, name: &str, args: &[i32]) -> Result<i32, String> {
        let f: Function = get(exports, name)?.dyn_into().map_err(|_| format!("the module does not export `{name}`"))?;
        let list = Array::new();
        for a in args {
            list.push(&JsValue::from(*a));
        }
        let r = f.apply(&JsValue::UNDEFINED, &list).map_err(text)?;
        Ok(r.as_f64().map(|v| v as i32).unwrap_or(0))
    }

    /// The bytes `ptr..ptr + len` of the memory, checked against its size.
    fn read(memory: &WebAssembly::Memory, ptr: i32, len: i32) -> Result<Uint8Array, String> {
        let buffer = Uint8Array::new(&memory.buffer());
        let (start, end) = (u32::try_from(ptr).ok(), u32::try_from(len).ok().and_then(|l| u32::try_from(ptr).ok()?.checked_add(l)));
        match (start, end) {
            (Some(start), Some(end)) if end <= buffer.length() => Ok(buffer.slice(start, end)),
            _ => Err("the module returned bytes outside its memory".into()),
        }
    }

    /// Copy `bytes` into the module's memory; their address.
    fn write(memory: &WebAssembly::Memory, exports: &JsValue, bytes: &Uint8Array) -> Result<i32, String> {
        let len = i32::try_from(bytes.length()).map_err(|_| "the file is too large".to_string())?;
        let ptr = call(exports, "sc_alloc", &[len])?;
        let buffer = Uint8Array::new(&memory.buffer());
        let at = u32::try_from(ptr).map_err(|_| "the module gave no room for the file".to_string())?;
        if at.checked_add(bytes.length()).is_none_or(|end| end > buffer.length()) {
            return Err("the module gave no room for the file".into());
        }
        buffer.set(bytes, at);
        Ok(ptr)
    }

    /// The empty WASI preview 1 a module built for it gets: the calls Rust's standard library
    /// makes, answered without anything real behind them.
    struct Wasi {
        object: Object,
        _closures: Vec<Box<dyn std::any::Any>>,
    }

    /// WASI error codes.
    const EBADF: i32 = 8;
    const ENOSYS: i32 = 52;

    impl Wasi {
        fn new(memory: &WebAssembly::Memory) -> Self {
            let object = Object::new();
            let mut closures: Vec<Box<dyn std::any::Any>> = Vec::new();
            let view = {
                let memory = memory.clone();
                move || js_sys::DataView::new(&memory.buffer().unchecked_into(), 0, Uint8Array::new(&memory.buffer()).length() as usize)
            };
            macro_rules! add {
                ($name:literal, $closure:expr) => {{
                    let c = $closure;
                    let _ = Reflect::set(&object, &$name.into(), c.as_ref());
                    closures.push(Box::new(c));
                }};
            }
            // No arguments and no environment: counts of zero.
            for name in ["args_sizes_get", "environ_sizes_get"] {
                let v = view.clone();
                let c = Closure::<dyn Fn(i32, i32) -> i32>::new(move |count: i32, size: i32| {
                    let d = v();
                    d.set_uint32_endian(count as usize, 0, true);
                    d.set_uint32_endian(size as usize, 0, true);
                    0
                });
                let _ = Reflect::set(&object, &name.into(), c.as_ref());
                closures.push(Box::new(c));
            }
            add!("args_get", Closure::<dyn Fn(i32, i32) -> i32>::new(|_: i32, _: i32| 0));
            add!("environ_get", Closure::<dyn Fn(i32, i32) -> i32>::new(|_: i32, _: i32| 0));
            // A clock that counts its own readings, as the desktop host's does.
            let ticks = std::rc::Rc::new(std::cell::Cell::new(0u32));
            {
                let (v, ticks) = (view.clone(), ticks.clone());
                add!(
                    "clock_time_get",
                    Closure::<dyn Fn(i32, JsValue, i32) -> i32>::new(move |_: i32, _: JsValue, out: i32| {
                        let d = v();
                        ticks.set(ticks.get().wrapping_add(1));
                        d.set_uint32_endian(out as usize, ticks.get(), true);
                        d.set_uint32_endian(out as usize + 4, 0, true);
                        0
                    })
                );
            }
            {
                let v = view.clone();
                add!(
                    "clock_res_get",
                    Closure::<dyn Fn(i32, i32) -> i32>::new(move |_: i32, out: i32| {
                        let d = v();
                        d.set_uint32_endian(out as usize, 1, true);
                        d.set_uint32_endian(out as usize + 4, 0, true);
                        0
                    })
                );
            }
            // Fixed "random" bytes, so one file converts the same way every time.
            {
                let memory = memory.clone();
                add!(
                    "random_get",
                    Closure::<dyn Fn(i32, i32) -> i32>::new(move |ptr: i32, len: i32| {
                        let bytes: Vec<u8> = (0..len.max(0) as usize).map(|i| (i as u8).wrapping_mul(31).wrapping_add(7)).collect();
                        Uint8Array::new(&memory.buffer()).set(&Uint8Array::from(&bytes[..]), ptr as u32);
                        0
                    })
                );
            }
            // Output goes nowhere but the log (a panic's message goes to stderr).
            {
                let (v, memory) = (view.clone(), memory.clone());
                add!(
                    "fd_write",
                    Closure::<dyn Fn(i32, i32, i32, i32) -> i32>::new(move |fd: i32, iovs: i32, count: i32, written: i32| {
                        let d = v();
                        let mut total = 0u32;
                        let mut text = Vec::new();
                        for i in 0..count.clamp(0, 1024) as usize {
                            let base = iovs as usize + i * 8;
                            let (ptr, len) = (d.get_uint32_endian(base, true), d.get_uint32_endian(base + 4, true));
                            total = total.saturating_add(len);
                            if fd == 2 && text.len() < 1024 {
                                text.extend(Uint8Array::new(&memory.buffer()).slice(ptr, ptr.saturating_add(len.min(1024))).to_vec());
                            }
                        }
                        d.set_uint32_endian(written as usize, total, true);
                        if !text.is_empty() {
                            let m = Object::new();
                            let _ = Reflect::set(&m, &"log".into(), &String::from_utf8_lossy(&text).trim_end().to_string().into());
                            post(&m, None);
                        }
                        0
                    })
                );
            }
            // No files: every descriptor is bad (this also ends the standard library's search
            // for preopened folders).
            for name in [
                "fd_close",
                "fd_prestat_get",
                "fd_prestat_dir_name",
                "fd_fdstat_get",
                "fd_fdstat_set_flags",
                "fd_filestat_get",
                "fd_read",
                "fd_seek",
                "fd_sync",
                "fd_tell",
                "fd_readdir",
                "path_open",
                "path_filestat_get",
                "path_create_directory",
                "path_remove_directory",
                "path_unlink_file",
                "path_rename",
                "path_readlink",
                "path_link",
            ] {
                let c = Closure::<dyn Fn() -> i32>::new(|| EBADF);
                let _ = Reflect::set(&object, &name.into(), c.as_ref());
                closures.push(Box::new(c));
            }
            add!("sched_yield", Closure::<dyn Fn() -> i32>::new(|| 0));
            add!("poll_oneoff", Closure::<dyn Fn() -> i32>::new(|| ENOSYS));
            add!("proc_exit", Closure::<dyn Fn(i32)>::new(|code: i32| wasm_bindgen::throw_str(&format!("the module exited with code {code}"))));
            Wasi { object, _closures: closures }
        }
    }

    async fn convert(data: JsValue) -> Result<(Object, JsValue), String> {
        let module: WebAssembly::Module = get(&data, "module")?.dyn_into().map_err(|_| "no module".to_string())?;
        let descriptor = Object::new();
        set(&descriptor, "initial", &number(&data, "initial")?.into())?;
        set(&descriptor, "maximum", &number(&data, "maximum")?.into())?;
        let memory = WebAssembly::Memory::new(&descriptor).map_err(text)?;
        let env = Object::new();
        set(&env, "memory", &memory)?;
        let progress = Closure::<dyn Fn(i32, i32, i32)>::new(|stage: i32, done: i32, total: i32| {
            let m = Object::new();
            let _ = Reflect::set(&m, &"progress".into(), &Array::of3(&stage.into(), &done.into(), &total.into()));
            post(&m, None);
        });
        set(&env, "sc_progress", progress.as_ref())?;
        let log_memory = memory.clone();
        let log = Closure::<dyn Fn(i32, i32)>::new(move |ptr: i32, len: i32| {
            if let Ok(bytes) = read(&log_memory, ptr, len.min(1024)) {
                let m = Object::new();
                let _ = Reflect::set(&m, &"log".into(), &String::from_utf8_lossy(&bytes.to_vec()).into_owned().into());
                post(&m, None);
            }
        });
        set(&env, "sc_log", log.as_ref())?;
        let imports = Object::new();
        set(&imports, "env", &env)?;
        let wasi = Wasi::new(&memory);
        set(&imports, "wasi_snapshot_preview1", &wasi.object)?;
        let instance: WebAssembly::Instance = JsFuture::from(WebAssembly::instantiate_module(&module, &imports))
            .await
            .map_err(text)?
            .dyn_into()
            .map_err(|_| "no instance".to_string())?;
        let exports: JsValue = instance.exports().into();
        // A WASI reactor runs its start-up code (the standard library's) once, first.
        if get(&exports, "_initialize")?.is_function() {
            call(&exports, "_initialize", &[])?;
        }
        let name = Uint8Array::from(get(&data, "name")?.as_string().unwrap_or_default().as_bytes());
        let options = Uint8Array::from(get(&data, "options")?.as_string().unwrap_or_default().as_bytes());
        let bytes: Uint8Array = get(&data, "bytes")?.dyn_into().map_err(|_| "no file".to_string())?;
        let (name_ptr, bytes_ptr, options_ptr) =
            (write(&memory, &exports, &name)?, write(&memory, &exports, &bytes)?, write(&memory, &exports, &options)?);
        let len = |a: &Uint8Array| a.length() as i32;
        let handle = call(&exports, "sc_import", &[name_ptr, len(&name), bytes_ptr, len(&bytes), options_ptr, len(&options)])?;
        let kind = call(&exports, "sc_result_kind", &[handle])?;
        let payload = read(&memory, call(&exports, "sc_result_ptr", &[handle])?, call(&exports, "sc_result_len", &[handle])?)?;
        let notes = read(&memory, call(&exports, "sc_notes_ptr", &[handle])?, call(&exports, "sc_notes_len", &[handle])?)?;
        let _ = call(&exports, "sc_free", &[handle]);
        drop((progress, log, wasi));
        let reply = Object::new();
        set(&reply, "kind", &kind.into())?;
        set(&reply, "payload", &payload)?;
        set(&reply, "notes", &notes)?;
        Ok((reply, payload.buffer().into()))
    }

    pub fn start() {
        let on = Closure::<dyn FnMut(MessageEvent)>::new(|e: MessageEvent| {
            let data = e.data();
            spawn_local(async move {
                match convert(data).await {
                    Ok((reply, buffer)) => post(&reply, Some(&buffer)),
                    Err(e) => {
                        let m = Object::new();
                        let _ = Reflect::set(&m, &"error".into(), &e.into());
                        post(&m, None);
                    }
                }
            });
        });
        scope().set_onmessage(Some(on.as_ref().unchecked_ref()));
        // The worker lives for one import and is then ended, handler and all.
        on.forget();
        // Messages sent before this point were dropped: say it is ready for the import.
        let ready = Object::new();
        let _ = Reflect::set(&ready, &"ready".into(), &JsValue::TRUE);
        post(&ready, None);
    }
}

fn main() {
    #[cfg(target_arch = "wasm32")]
    worker::start();
}

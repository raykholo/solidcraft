//! The browser's plugin host: runs an importer plugin's core module (the web ABI) in a Web
//! Worker of its own for each import (`plugin_worker.rs`), for the app's file loads
//! (`Services::run_plugin_import`).
//!
//! - The module is checked against the ABI before anything runs (only its memory, progress and
//!   log imports), and compiled once: its bytes are kept in the browser's Cache Storage and
//!   compiled with `WebAssembly.compileStreaming`, so the browser keeps its compiled code too.
//! - Its memory's maximum is the package's memory limit; the worker is ended when the package's
//!   time limit passes or the user cancels, whatever the module is doing.
//! - Progress the module reports goes to the file load's progress card.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use js_sys::{Array, Object, Reflect, Uint8Array, WebAssembly};
use solidcraft_engine::doc::progress::Progress;
use solidcraft_engine::plugin::{HostRequest, Imported, web_abi};
use wasm_bindgen::{JsCast as _, JsValue, closure::Closure};
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{MessageEvent, Worker, WorkerOptions, WorkerType};

/// The worker's script, written by the build (trunk's loader for the worker binary).
const WORKER: &str = "./solidcraft-plugin-worker_loader.js";
/// The Cache Storage the modules are kept in, and the key prefix.
const CACHE: &str = "solidcraft-plugins";
/// How often a running import checks for Cancel and its time.
const TICK_MS: i32 = 100;

thread_local! {
    /// Compiled modules, by approval hash.
    static MODULES: RefCell<HashMap<String, WebAssembly::Module>> = RefCell::new(HashMap::new());
}

fn text(e: JsValue) -> String {
    e.as_string().or_else(|| Reflect::get(&e, &"message".into()).ok().and_then(|m| m.as_string())).unwrap_or_else(|| format!("{e:?}"))
}

/// The module compiled (from memory, from the browser's cache, or from its bytes).
async fn compiled(approval: &str, bytes: &[u8]) -> Result<WebAssembly::Module, String> {
    if let Some(m) = MODULES.with(|m| m.borrow().get(approval).cloned()) {
        return Ok(m);
    }
    let began = js_sys::Date::now();
    let window = web_sys::window().ok_or("no window")?;
    let key = format!("/{CACHE}/{approval}.wasm");
    // Keep the bytes in Cache Storage and compile from there: the browser caches the compiled
    // code of a module streamed from it, so a later session compiles it at once.
    let module = match window.caches() {
        Ok(caches) => {
            let cache: web_sys::Cache = JsFuture::from(caches.open(CACHE)).await.map_err(text)?.dyn_into().map_err(|_| "no cache".to_string())?;
            let mut found = JsFuture::from(cache.match_with_str(&key)).await.map_err(text)?;
            if found.is_undefined() {
                let headers = web_sys::Headers::new().map_err(text)?;
                headers.set("content-type", "application/wasm").map_err(text)?;
                let init = web_sys::ResponseInit::new();
                init.set_headers(&headers);
                let mut owned = bytes.to_vec();
                let response = web_sys::Response::new_with_opt_u8_array_and_init(Some(&mut owned), &init).map_err(text)?;
                JsFuture::from(cache.put_with_str(&key, &response)).await.map_err(text)?;
                found = JsFuture::from(cache.match_with_str(&key)).await.map_err(text)?;
            }
            let promise = js_sys::Promise::resolve(&found);
            JsFuture::from(WebAssembly::compile_streaming(&promise)).await.map_err(text)?
        }
        // No Cache Storage (an insecure page): compile from the bytes.
        Err(_) => JsFuture::from(WebAssembly::compile(&Uint8Array::from(bytes).into())).await.map_err(text)?,
    };
    let module: WebAssembly::Module = module.dyn_into().map_err(|_| "not a module".to_string())?;
    log::info!("plugin module ready in {:.0} ms", js_sys::Date::now() - began);
    MODULES.with(|m| m.borrow_mut().insert(approval.to_string(), module.clone()));
    Ok(module)
}

/// What one import holds while it runs; dropped (worker ended, timers cleared) when it ends.
struct Run {
    worker: Worker,
    tick: Option<i32>,
    done: Option<Box<dyn FnOnce(Result<Imported, String>)>>,
    log: Vec<String>,
    _on_message: Option<Closure<dyn FnMut(MessageEvent)>>,
    _on_tick: Option<Closure<dyn FnMut()>>,
}

fn finish(run: &Rc<RefCell<Option<Run>>>, result: Result<Imported, String>, ctx: &egui::Context) {
    let Some(mut r) = run.borrow_mut().take() else { return };
    r.worker.terminate();
    if let (Some(id), Some(w)) = (r.tick, web_sys::window()) {
        w.clear_interval_with_handle(id);
    }
    if let Some(done) = r.done.take() {
        done(result);
    }
    // The handlers may be the very code running now: let them go once it has returned.
    spawn_local(async move { drop(r) });
    ctx.request_repaint();
}

async fn start(request: HostRequest, progress: Arc<Progress>, done: Box<dyn FnOnce(Result<Imported, String>)>, ctx: egui::Context) {
    let Some(bytes) = request.web_module.as_deref() else {
        return done(Err(format!("plugin `{}` has no browser build: it runs in the desktop app", request.plugin)));
    };
    progress.report_work("plugin.preparing", 0, 1);
    ctx.request_repaint();
    let interface = match web_abi::interface(bytes).and_then(|i| web_abi::check_interface(&i.imports, &i.exports).map(|()| i)) {
        Ok(i) => i,
        Err(e) => return done(Err(e.to_string())),
    };
    let maximum = web_abi::memory_pages(&request.limits);
    let initial = interface.memory_min.unwrap_or(1);
    if initial > u64::from(maximum) {
        return done(Err(format!("plugin `{}` needs more memory to start than its limit", request.plugin)));
    }
    let module = match compiled(&request.approval, bytes).await {
        Ok(m) => m,
        Err(e) => return done(Err(format!("plugin `{}`: {e}", request.plugin))),
    };
    if progress.is_cancelled() {
        return done(Err("import cancelled".into()));
    }
    let options = WorkerOptions::new();
    options.set_type(WorkerType::Module);
    let worker = match Worker::new_with_options(WORKER, &options) {
        Ok(w) => w,
        Err(e) => return done(Err(format!("could not start the plugin worker: {}", text(e)))),
    };
    let run =
        Rc::new(RefCell::new(Some(Run { worker: worker.clone(), tick: None, done: Some(done), log: Vec::new(), _on_message: None, _on_tick: None })));
    let message = Object::new();
    let set = |k: &str, v: &JsValue| {
        let _ = Reflect::set(&message, &k.into(), v);
    };
    set("module", &module);
    set("name", &request.file_name.clone().into());
    set("options", &request.options.clone().into());
    set("bytes", &Uint8Array::from(&request.bytes[..]));
    set("initial", &JsValue::from(initial as f64));
    set("maximum", &JsValue::from(maximum));
    // Messages: ready (then the import goes over), progress, log lines, then the result.
    let on_message = {
        let (run, progress, ctx, worker, message) = (run.clone(), progress.clone(), ctx.clone(), worker.clone(), message.clone());
        Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
            let data = e.data();
            let field = |k: &str| Reflect::get(&data, &k.into()).ok().filter(|v| !v.is_undefined());
            if field("ready").is_some() {
                if let Err(e) = worker.post_message(&message) {
                    finish(&run, Err(format!("could not start the plugin worker: {}", text(e))), &ctx);
                }
            } else if let Some(p) = field("progress") {
                let p: Array = p.unchecked_into();
                let n = |i: u32| p.get(i).as_f64().unwrap_or(0.0) as i32;
                let (done, total) = (usize::try_from(n(1)).unwrap_or(0), usize::try_from(n(2)).unwrap_or(0));
                let (done, total) = if total == 0 { (0, 1) } else { (done.min(total), total) };
                progress.report_work(web_abi::stage(n(0)), done, total);
                ctx.request_repaint();
            } else if let Some(line) = field("log").and_then(|l| l.as_string()) {
                if let Some(r) = run.borrow_mut().as_mut().filter(|r| r.log.len() < 64) {
                    r.log.push(line);
                }
            } else if let Some(error) = field("error") {
                // What the module logged last says why (a panic's message, for one).
                let said = run.borrow().as_ref().and_then(|r| r.log.last().cloned()).map(|l| format!(" ({l})")).unwrap_or_default();
                finish(&run, Err(format!("the importer failed: {}{said}", text(error))), &ctx);
            } else {
                let bytes = |k: &str| field(k).map(|v| Uint8Array::new(&v).to_vec()).unwrap_or_default();
                let kind = field("kind").and_then(|k| k.as_f64()).unwrap_or(-1.0) as i32;
                let log = run.borrow().as_ref().map(|r| r.log.clone()).unwrap_or_default();
                let result = web_abi::imported(kind, bytes("payload"), &bytes("notes"), &log).map_err(|e| e.to_string());
                finish(&run, result, &ctx);
            }
        })
    };
    worker.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    // The time limit and Cancel end the worker, whatever the module is doing.
    let deadline = js_sys::Date::now() + request.limits.deadline_ms as f64;
    let seconds = request.limits.deadline_ms as f64 / 1000.0;
    let on_tick = {
        let (run, progress, ctx) = (run.clone(), progress.clone(), ctx.clone());
        Closure::<dyn FnMut()>::new(move || {
            if progress.is_cancelled() {
                finish(&run, Err("import cancelled".into()), &ctx);
            } else if js_sys::Date::now() >= deadline {
                finish(&run, Err(format!("the importer ran past its time limit ({seconds} s) and was stopped")), &ctx);
            }
        })
    };
    let tick =
        web_sys::window().and_then(|w| w.set_interval_with_callback_and_timeout_and_arguments_0(on_tick.as_ref().unchecked_ref(), TICK_MS).ok());
    if let Some(r) = run.borrow_mut().as_mut() {
        r.tick = tick;
        r._on_message = Some(on_message);
        r._on_tick = Some(on_tick);
    }
}

/// The service the app's file loads call.
pub fn service(ctx: &egui::Context) -> solidcraft_ui_egui::PluginImportHost {
    let ctx = ctx.clone();
    Box::new(move |request, progress, done| {
        spawn_local(start(request, progress, done, ctx.clone()));
    })
}

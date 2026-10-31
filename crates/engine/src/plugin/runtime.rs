//! Native command-component sandbox. No WASI is linked, even for a trusted package.
use super::{MAX_JSON_BYTES, Package, allowed_command, error};
use crate::{Result, Session};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use wasmtime::component::{Component, Linker};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder};

mod bindings {
    wasmtime::component::bindgen!({ path: "../../wit", world: "command-plugin" });
}
use bindings::solidcraft::plugin::{engine, log, store, types::PluginError};

struct State {
    session: Session,
    commands: BTreeSet<String>,
    limits: StoreLimits,
    failed: Option<String>,
    calls: usize,
    log_bytes: usize,
    deadline: Instant,
}
impl State {
    fn refuse(&mut self, message: &str) -> PluginError {
        self.failed.get_or_insert_with(|| message.to_string());
        PluginError::Denied(message.to_string())
    }
}
impl bindings::solidcraft::plugin::types::Host for State {}
impl engine::Host for State {
    fn run(&mut self, command: String, args: String) -> std::result::Result<String, PluginError> {
        if self.failed.is_some() {
            return Err(self.refuse("transaction already failed"));
        }
        self.calls += 1;
        if self.calls > 256 || Instant::now() >= self.deadline || args.len() > MAX_JSON_BYTES {
            return Err(self.refuse("command count, JSON or deadline limit"));
        }
        if !self.commands.contains(&command) || !allowed_command(&command) {
            return Err(self.refuse("command is not granted"));
        }
        let params: Value = serde_json::from_str(&args).map_err(|_| self.refuse("invalid command JSON"))?;
        let before = self.session.doc.clone();
        let previous: BTreeMap<_, _> = self.session.model.results.iter().map(|r| (r.id, r.error.clone())).collect();
        // This also contains implicit reads during rebuild (fonts, linked designs, etc.).
        let reply = solidcraft_io::vfs::scoped(Some(Default::default()), || self.session.execute(&command, &params)).map_err(|e| {
            let e = e.to_string();
            self.failed = Some(e.clone());
            PluginError::Failed(e)
        })?;
        if let Some(failed) = self
            .session
            .model
            .results
            .iter()
            .find(|r| r.error.is_some() && (previous.get(&r.id) != Some(&r.error) || before.feature(r.id) != self.session.doc.feature(r.id)))
        {
            let message = failed.error.clone().unwrap_or_default();
            self.failed = Some(message.clone());
            return Err(PluginError::Failed(message));
        }
        let text = serde_json::to_string(&reply).map_err(|_| self.refuse("invalid command result"))?;
        if text.len() > MAX_JSON_BYTES {
            return Err(self.refuse("command result exceeds 1 MiB"));
        }
        Ok(text)
    }
}
impl log::Host for State {
    fn log(&mut self, _level: log::Level, message: String) {
        if message.len() > 1024 || self.log_bytes.saturating_add(message.len()) > 64 * 1024 {
            self.refuse("log limit exceeded");
            return;
        }
        self.log_bytes += message.len();
        self.session.echo(format!("plugin: {}", message.chars().filter(|c| !c.is_control()).collect::<String>()));
    }
}
impl store::Host for State {
    fn get(&mut self, _: String) -> std::result::Result<Option<Vec<u8>>, PluginError> {
        Err(self.refuse("persistent store is not supported"))
    }
    fn set(&mut self, _: String, _: Vec<u8>) -> std::result::Result<(), PluginError> {
        Err(self.refuse("persistent store is not supported"))
    }
    fn remove(&mut self, _: String) -> std::result::Result<(), PluginError> {
        Err(self.refuse("persistent store is not supported"))
    }
    fn keys(&mut self) -> std::result::Result<Vec<String>, PluginError> {
        Err(self.refuse("persistent store is not supported"))
    }
    fn previous_version(&mut self) -> Option<String> {
        None
    }
}

// The outer Display can contain only a Wasm backtrace. Keep the root cause first,
// plus the typed trap and exact fuel counters, without relaxing any execution limit.
pub(super) fn execution_error(failure: &wasmtime::Error, remaining: Option<u64>, limits: &super::Limits) -> crate::EngineError {
    let trap = failure.downcast_ref::<wasmtime::Trap>().map(|trap| format!("{trap:?}")).unwrap_or_else(|| "none".into());
    let fuel = remaining
        .map(|remaining| format!("fuel_consumed={}, fuel_remaining={remaining}", limits.fuel.saturating_sub(remaining)))
        .unwrap_or_else(|| "fuel counters unavailable".into());
    error(format!(
        "component execution failed: {} (trap={trap}, {fuel}, fuel_limit={}, memory_limit_mib={}, deadline_ms={}): {failure:#}",
        failure.root_cause(),
        limits.fuel,
        limits.memory_mib,
        limits.deadline_ms
    ))
}

pub(super) fn run(session: &mut Session, package: &Package, entry: &str, params: &Value) -> Result<Value> {
    let runtime = package.manifest.runtime.as_ref().ok_or_else(|| error("no runtime"))?;
    let params = serde_json::to_string(params).map_err(|e| error(e.to_string()))?;
    if params.len() > MAX_JSON_BYTES {
        return Err(error("parameters exceed 1 MiB"));
    }
    let mut config = Config::new();
    config.consume_fuel(true).epoch_interruption(true).wasm_memory64(false).wasm_multi_memory(false).max_wasm_stack(512 * 1024);
    let engine = Engine::new(&config).map_err(|e| error(e.to_string()))?;
    // Compilation is native work, outside guest fuel and deadline. Size is capped by read_package.
    // Never deserialize executable cache bytes from a package or a design.
    let component = Component::from_binary(&engine, &package.component).map_err(|e| error(format!("component compilation failed: {e}")))?;
    let limit = Duration::from_millis(runtime.limits.deadline_ms);
    let state = State {
        session: session.scratch(),
        commands: runtime.commands.clone(),
        failed: None,
        calls: 0,
        log_bytes: 0,
        limits: StoreLimitsBuilder::new()
            .memory_size(runtime.limits.memory_mib.checked_mul(1024 * 1024).ok_or_else(|| error("memory limit cannot be represented on this host"))?)
            .instances(8)
            .memories(1)
            .tables(2)
            .table_elements(10_000)
            .trap_on_grow_failure(true)
            .build(),
        deadline: Instant::now() + limit,
    };
    let mut store = Store::new(&engine, state);
    store.limiter(|s| &mut s.limits);
    store.set_fuel(runtime.limits.fuel).map_err(|e| error(e.to_string()))?;
    store.set_epoch_deadline(1);
    let mut linker = Linker::new(&engine);
    bindings::CommandPlugin::add_to_linker::<_, wasmtime::component::HasSelf<_>>(&mut linker, |s| s).map_err(|e| error(e.to_string()))?;
    let (stop, stopped) = mpsc::channel::<()>();
    let timer_engine = engine.clone();
    let clock = std::thread::Builder::new()
        .name("plugin-deadline".into())
        .spawn(move || {
            if stopped.recv_timeout(limit).is_err() {
                timer_engine.increment_epoch();
            }
        })
        .map_err(|e| error(format!("could not start the deadline worker: {e}")))?;
    let response = (|| {
        let guest = bindings::CommandPlugin::instantiate(&mut store, &component, &linker)?;
        guest.solidcraft_plugin_command().call_execute(&mut store, entry, &params)
    })();
    let _ = stop.send(());
    if clock.join().is_err() {
        return Err(error("deadline worker failed"));
    }
    let response = response.map_err(|e| execution_error(&e, store.get_fuel().ok(), &runtime.limits))?;
    let text = response.map_err(|e| error(format!("entry failed: {e:?}")))?;
    if text.len() > MAX_JSON_BYTES {
        return Err(error("entry result exceeds 1 MiB"));
    }
    let value = serde_json::from_str(&text).map_err(|e| error(format!("entry result is not JSON: {e}")))?;
    let remaining = store.get_fuel().ok();
    let mut state = store.into_data();
    if let Some(message) = state.failed {
        return Err(error(message));
    }
    if Instant::now() >= state.deadline {
        return Err(error("entry deadline exceeded"));
    }
    if let Some(remaining) = remaining {
        ::log::debug!(
            "plugin execution completed: fuel_consumed={}, fuel_remaining={remaining}, fuel_limit={}",
            runtime.limits.fuel.saturating_sub(remaining),
            runtime.limits.fuel
        );
    }
    // Nested commands ran only on the scratch session. The outer plugin.run records one step.
    state.session.undo.clear();
    state.session.redo.clear();
    if !std::sync::Arc::ptr_eq(&session.doc, &state.session.doc)
        || session.active_sketch != state.session.active_sketch
        || session.pending_moves != state.session.pending_moves
    {
        session.accept_computed(state.session);
    } else {
        for message in state.session.log {
            session.echo(message);
        }
    }
    Ok(value)
}

mod importer_bindings {
    wasmtime::component::bindgen!({ path: "../../wit", world: "importer-plugin" });
}

/// What an importer may do: write to the log, within a budget, and use an empty WASI.
struct ImportState {
    limits: StoreLimits,
    log_bytes: usize,
    messages: Vec<String>,
    wasi: wasmtime_wasi::WasiCtx,
    table: wasmtime::component::ResourceTable,
}
impl wasmtime_wasi::WasiView for ImportState {
    fn ctx(&mut self) -> wasmtime_wasi::WasiCtxView<'_> {
        wasmtime_wasi::WasiCtxView { ctx: &mut self.wasi, table: &mut self.table }
    }
}

/// The wall clock an importer sees: always the Unix epoch.
struct NoWallClock;
impl wasmtime_wasi::HostWallClock for NoWallClock {
    fn resolution(&self) -> Duration {
        Duration::from_secs(1)
    }
    fn now(&self) -> Duration {
        Duration::ZERO
    }
}
/// The monotonic clock an importer sees: a counter that moves one nanosecond per reading, so
/// the time it measures tells it nothing about the machine.
struct CountingClock(std::sync::atomic::AtomicU64);
impl wasmtime_wasi::HostMonotonicClock for CountingClock {
    fn resolution(&self) -> u64 {
        1
    }
    fn now(&self) -> u64 {
        self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }
}

/// The WASI an importer gets so a standard `wasm32-wasip2` build links: no preopened folders
/// (so no file can be opened), no arguments or environment, closed standard input, discarded
/// output, no network (every address and name lookup refused) and clocks that tell nothing.
fn empty_wasi() -> wasmtime_wasi::WasiCtx {
    wasmtime_wasi::WasiCtxBuilder::new()
        .wall_clock(NoWallClock)
        .monotonic_clock(CountingClock(std::sync::atomic::AtomicU64::new(0)))
        .allow_ip_name_lookup(false)
        .allow_tcp(false)
        .allow_udp(false)
        // Reproducible conversions: an importer has no use for real randomness.
        .secure_random(wasmtime_wasi::Deterministic::new((0..=255).collect()))
        .insecure_random(wasmtime_wasi::Deterministic::new((0..=255).rev().collect()))
        .insecure_random_seed(0)
        .build()
}

/// Whether an importer's WASI offers any folder at all (what the guest's `get-directories`
/// returns). Tests: it must not.
#[cfg(test)]
pub(super) fn empty_wasi_has_no_folders() -> bool {
    use wasmtime_wasi::filesystem::WasiFilesystemView;
    use wasmtime_wasi::p2::bindings::filesystem::preopens::Host;
    let mut state = ImportState {
        limits: StoreLimitsBuilder::new().build(),
        log_bytes: 0,
        messages: Vec::new(),
        wasi: empty_wasi(),
        table: wasmtime::component::ResourceTable::new(),
    };
    state.filesystem().get_directories().is_ok_and(|dirs| dirs.is_empty())
}
impl importer_bindings::solidcraft::plugin::types::Host for ImportState {}
impl importer_bindings::solidcraft::plugin::log::Host for ImportState {
    fn log(&mut self, _level: importer_bindings::solidcraft::plugin::log::Level, message: String) {
        if message.len() > 1024 || self.log_bytes.saturating_add(message.len()) > 64 * 1024 {
            return;
        }
        self.log_bytes += message.len();
        self.messages.push(message.chars().filter(|c| !c.is_control()).collect());
    }
}

/// The importers' engine and the components it compiled, by approval hash: compiling a large
/// importer takes longer than running it, and its code never changes under one hash.
struct Compiled {
    engine: Engine,
    components: Vec<(String, Component)>,
}
static IMPORTERS: std::sync::Mutex<Option<Compiled>> = std::sync::Mutex::new(None);
/// How many compiled importers are kept.
const KEEP_COMPILED: usize = 4;
/// The native stack an importer runs on, and how much of it Wasm frames may use.
const IMPORT_THREAD_STACK: usize = 64 * 1024 * 1024;
const IMPORT_WASM_STACK: usize = 16 * 1024 * 1024;

fn compiled(package: &Package) -> Result<(Engine, Component)> {
    let mut guard = IMPORTERS.lock().map_err(|_| error("the importer cache failed"))?;
    if guard.is_none() {
        let mut config = Config::new();
        config
            .consume_fuel(true)
            .epoch_interruption(true)
            .wasm_memory64(false)
            .wasm_multi_memory(false)
            .max_wasm_stack(IMPORT_WASM_STACK)
            // WASI turns on Wasmtime's async support, whose stack must hold the Wasm stack, though
            // importers run synchronously.
            .async_stack_size(IMPORT_WASM_STACK + 1024 * 1024);
        *guard = Some(Compiled { engine: Engine::new(&config).map_err(|e| error(e.to_string()))?, components: Vec::new() });
    }
    let cache = guard.as_mut().ok_or_else(|| error("the importer cache failed"))?;
    if let Some((_, component)) = cache.components.iter().find(|(hash, _)| *hash == package.approval) {
        return Ok((cache.engine.clone(), component.clone()));
    }
    // Compilation is native work, outside guest fuel and deadline. Size is capped by read_package.
    let component = Component::from_binary(&cache.engine, &package.component).map_err(|e| error(format!("component compilation failed: {e:#}")))?;
    if cache.components.len() >= KEEP_COMPILED {
        cache.components.remove(0);
    }
    cache.components.push((package.approval.clone(), component.clone()));
    Ok((cache.engine.clone(), component))
}

/// Run an importer component on its own thread (a decoder may recurse deeply), under the
/// package's fuel, memory and deadline. Nothing but `file_name` and `bytes` goes in.
pub(super) fn import(package: &Package, file_name: &str, bytes: Vec<u8>) -> Result<super::Imported> {
    let runtime = package.manifest.runtime.as_ref().ok_or_else(|| error("no runtime"))?;
    let limits = runtime.limits.clone();
    let (engine, component) = compiled(package)?;
    let name = file_name.to_string();
    let worker = std::thread::Builder::new()
        .name("plugin-import".into())
        .stack_size(IMPORT_THREAD_STACK)
        .spawn(move || import_on_this_thread(&engine, &component, &limits, &name, &bytes))
        .map_err(|e| error(format!("could not start the importer: {e}")))?;
    worker.join().map_err(|_| error("the importer failed"))?
}

fn import_on_this_thread(engine: &Engine, component: &Component, limits: &super::Limits, file_name: &str, bytes: &[u8]) -> Result<super::Imported> {
    use importer_bindings::exports::solidcraft::plugin::importer::Payload;
    let deadline = Instant::now() + Duration::from_millis(limits.deadline_ms);
    let state = ImportState {
        limits: StoreLimitsBuilder::new()
            .memory_size(limits.memory_mib.checked_mul(1024 * 1024).ok_or_else(|| error("memory limit cannot be represented on this host"))?)
            .instances(16)
            .memories(1)
            .tables(4)
            .table_elements(100_000)
            .trap_on_grow_failure(true)
            .build(),
        log_bytes: 0,
        messages: Vec::new(),
        wasi: empty_wasi(),
        table: wasmtime::component::ResourceTable::new(),
    };
    let mut store = Store::new(engine, state);
    store.limiter(|s| &mut s.limits);
    store.set_fuel(limits.fuel).map_err(|e| error(e.to_string()))?;
    // The engine is shared: every run bumps the epoch at its own deadline, and each store
    // only stops once its own deadline has passed.
    store.set_epoch_deadline(1);
    store.epoch_deadline_callback(move |_| {
        if Instant::now() >= deadline { Err(wasmtime::Error::msg("entry deadline exceeded")) } else { Ok(wasmtime::UpdateDeadline::Continue(1)) }
    });
    let mut linker = Linker::new(engine);
    importer_bindings::ImporterPlugin::add_to_linker::<_, wasmtime::component::HasSelf<_>>(&mut linker, |s| s).map_err(|e| error(e.to_string()))?;
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(|e| error(e.to_string()))?;
    let (stop, stopped) = mpsc::channel::<()>();
    let timer_engine = engine.clone();
    let wait = Duration::from_millis(limits.deadline_ms);
    let clock = std::thread::Builder::new()
        .name("plugin-deadline".into())
        .spawn(move || {
            if stopped.recv_timeout(wait).is_err() {
                timer_engine.increment_epoch();
            }
        })
        .map_err(|e| error(format!("could not start the deadline worker: {e}")))?;
    let response = (|| {
        let guest = importer_bindings::ImporterPlugin::instantiate(&mut store, component, &linker)?;
        guest.solidcraft_plugin_importer().call_import(&mut store, file_name, bytes)
    })();
    let _ = stop.send(());
    if clock.join().is_err() {
        return Err(error("deadline worker failed"));
    }
    let response = response.map_err(|e| execution_error(&e, store.get_fuel().ok(), limits))?;
    let imported = response.map_err(|e| error(format!("the importer failed: {}", plugin_error_text(&e))))?;
    if Instant::now() >= deadline {
        return Err(error("entry deadline exceeded"));
    }
    let payload = match imported.payload {
        Payload::Design(bytes) => super::Payload::Design(bytes),
        Payload::Step(bytes) => super::Payload::Step(bytes),
    };
    let (super::Payload::Design(made) | super::Payload::Step(made)) = &payload;
    if made.len() > super::MAX_IMPORT_BYTES {
        return Err(error("the imported design exceeds 512 MiB"));
    }
    let mut notes: String = imported.notes.chars().filter(|c| *c == '\n' || !c.is_control()).collect();
    if notes.len() > super::MAX_NOTES_BYTES {
        return Err(error("the importer's notes exceed 64 KiB"));
    }
    for message in store.into_data().messages {
        notes.push_str(&message);
        notes.push('\n');
    }
    Ok(super::Imported { payload, notes })
}

fn plugin_error_text(e: &importer_bindings::solidcraft::plugin::types::PluginError) -> String {
    use importer_bindings::solidcraft::plugin::types::PluginError as E;
    let (kind, text) = match e {
        E::Failed(t) => ("failed", t),
        E::Denied(t) => ("denied", t),
        E::Limit(t) => ("limit", t),
        E::Invalid(t) => ("invalid", t),
    };
    format!("{kind}: {}", text.chars().filter(|c| !c.is_control()).take(2048).collect::<String>())
}

//! SolidCraft policy and transactions for the shared Craft plugin host.

use crate::{EngineError, Result, Session};
pub use craft_plugin::{
    API, Effect, Entry, EntryKind, FileGrant, HostLimits, Limits, MAX_COMPONENT_BYTES, MAX_JSON_BYTES, MAX_MANIFEST_BYTES, MAX_MEMORY_MIB, Manifest,
    RuntimeManifest, valid_id,
};
use craft_plugin::{CommandSchema, HostError, PluginHost};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;

static PROCESS_HOST_LIMITS: std::sync::OnceLock<HostLimits> = std::sync::OnceLock::new();
pub fn configure_process_host_limits(limits: HostLimits) -> Result<()> {
    limits.validate()?;
    PROCESS_HOST_LIMITS.set(limits).map_err(|_| error("process plugin host limits are already configured"))
}
pub(crate) fn default_host_limits() -> HostLimits {
    PROCESS_HOST_LIMITS.get().copied().unwrap_or_default()
}
fn error(s: impl Into<String>) -> EngineError {
    EngineError::Other(format!("plugin: {}", s.into()))
}

/// Trusted host state, never loaded from a design or supplied by guest code.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AppState {
    pub packages: BTreeMap<String, String>,
    pub file_grants: Vec<FileGrant>,
}

pub fn parse_manifest(text: &[u8], folder: &str) -> Result<Manifest> {
    let manifest = craft_plugin::parse_manifest(text, folder)?;
    if manifest.runtime.as_ref().is_some_and(|r| r.commands.iter().any(|c| !allowed_command(c))) {
        return Err(HostError::InvalidManifest {
            field: "runtime.commands".into(),
            reason: "commands include an unavailable or forbidden built-in command".into(),
        }
        .into());
    }
    Ok(manifest)
}
/// Positive host policy, intersected with the manifest. New command families require review.
pub fn allowed_command(id: &str) -> bool {
    let family = id.starts_with("solid.") || id.starts_with("sketch.");
    let explicit = matches!(
        id,
        "component.activate"
            | "component.create"
            | "occurrence.move"
            | "parameters.add"
            | "parameters.change"
            | "parameters.delete"
            | "timeline.add"
            | "timeline.inspect"
            | "timeline.redefine"
            | "timeline.delete"
            | "document.inspect"
            | "model.faces"
            | "model.edges"
            | "model.mesh"
            | "inspect.measure"
    );
    (family || explicit) && !id.starts_with("sketch.insert_") && crate::find_command(id).is_some()
}

pub struct Package {
    pub manifest: Manifest,
    pub approval: String,
    pub component: Vec<u8>,
}

/// Reads through the caller's existing file scope; no permissions are added here.
pub fn read_package(path: &str) -> Result<Package> {
    let dir = Path::new(path);
    let folder = dir.file_name().and_then(|s| s.to_str()).ok_or_else(|| error("package needs an id folder"))?;
    let manifest_path = dir.join("plugin.json");
    let bytes = solidcraft_io::vfs::read(&manifest_path.to_string_lossy(), MAX_MANIFEST_BYTES).map_err(|e| error(e.to_string()))?;
    let manifest = parse_manifest(&bytes, folder)?;
    let runtime = manifest.runtime.as_ref().ok_or_else(|| error("metadata-only package has no executable runtime"))?;
    let module_path = dir.join(&runtime.module);
    let component = solidcraft_io::vfs::read(&module_path.to_string_lossy(), MAX_COMPONENT_BYTES).map_err(|e| error(e.to_string()))?;
    let mut digest = Sha256::new();
    digest.update(b"solidcraft-plugin-approval-v1\0");
    digest.update((bytes.len() as u64).to_le_bytes());
    digest.update(&bytes);
    digest.update(&component);
    Ok(Package { manifest, approval: format!("{:x}", digest.finalize()), component })
}

pub fn run(session: &mut Session, package: Package, entry: &str, params: &serde_json::Value) -> Result<serde_json::Value> {
    if session.disabled_plugins.contains(&package.manifest.id) {
        return Err(error("plugin is disabled"));
    }
    let rt = package.manifest.runtime.as_ref().ok_or_else(|| error("no executable runtime"))?;
    session.plugin_limits.check(&rt.limits)?;
    if !rt.entries.iter().any(|e| e.name == entry && e.kind == EntryKind::Command) {
        return Err(error("unknown entry"));
    }
    #[cfg(all(feature = "plugins", any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        let (host, result) = craft_plugin_wasm::run(
            SolidCraftHost { session: session.scratch(), transaction: None },
            &package.manifest,
            &package.component,
            entry,
            params,
        );
        let value = result?;
        if !std::sync::Arc::ptr_eq(&session.doc, &host.session.doc)
            || session.active_sketch != host.session.active_sketch
            || session.pending_moves != host.session.pending_moves
        {
            session.accept_computed(host.session);
        } else {
            for message in host.session.log {
                session.echo(message);
            }
        }
        Ok(value)
    }
    #[cfg(not(all(feature = "plugins", any(target_arch = "x86_64", target_arch = "aarch64"))))]
    {
        let _ = params;
        Err(error("plugin execution requires an x86-64 or AArch64 native build with the plugins feature"))
    }
}

/// An inert file-format contribution. Approval still requires reviewing the permissions.
#[derive(Clone, Debug)]
pub struct PluginFormat {
    pub id: String,
    pub plugin_id: String,
    pub entry: String,
    pub name: String,
    pub extensions: Vec<String>,
    pub approval: String,
    pub input_schema: Value,
    pub permissions: craft_plugin::Permissions,
}

/// Discover installed formats without running plugin code or granting any permissions.
pub fn formats(session: &Session, kind: EntryKind) -> Vec<PluginFormat> {
    let paths = session.plugin_state.packages.clone();
    #[cfg(not(target_arch = "wasm32"))]
    let paths = {
        let mut paths = paths;
        if let Some(root) = crate::recovery::data_dir().map(|p| p.join("plugins"))
            && let Ok(folders) = std::fs::read_dir(root)
        {
            for folder in folders.flatten().take(1024) {
                let path = folder.path().to_string_lossy().into_owned();
                if let Ok(package) = read_package(&path) {
                    paths.entry(package.manifest.id).or_insert(path);
                }
            }
        }
        paths
    };
    let mut result = Vec::new();
    for path in paths.values() {
        let Ok(package) = read_package(path) else { continue };
        if session.disabled_plugins.contains(&package.manifest.id) {
            continue;
        }
        let Some(runtime) = &package.manifest.runtime else { continue };
        for entry in runtime.entries.iter().filter(|e| e.kind == kind) {
            result.push(PluginFormat {
                id: format!("{}.{}", package.manifest.id, entry.name),
                plugin_id: package.manifest.id.clone(),
                entry: entry.name.clone(),
                name: format!("{} — {}", package.manifest.name, entry.name),
                extensions: entry.extensions.clone(),
                approval: package.approval.clone(),
                input_schema: entry.input_schema.clone(),
                permissions: runtime.permissions.clone(),
            });
        }
    }
    result.sort_by(|a, b| a.id.cmp(&b.id));
    result
}

/// Resolve an explicit entry id, or an unambiguous extension; built-in precedence is decided by the caller.
pub fn resolve_format(session: &Session, kind: EntryKind, format: Option<&str>, path: &str) -> Result<Option<PluginFormat>> {
    let extension = Path::new(path).extension().and_then(|x| x.to_str()).unwrap_or_default();
    let matches: Vec<_> = formats(session, kind)
        .into_iter()
        .filter(|entry| match format {
            Some(id) => entry.id == id,
            None => entry.extensions.iter().any(|x| x.eq_ignore_ascii_case(extension)),
        })
        .collect();
    if matches.len() > 1 {
        return Err(
            HostError::InvalidInput { field: "format".into(), reason: "multiple plugins read this extension; specify a format id".into() }.into()
        );
    }
    Ok(matches.into_iter().next())
}

fn approved_format(session: &Session, kind: EntryKind, path: &str, params: &Value, command: &str) -> Result<(Package, PluginFormat)> {
    if let Some(fields) = params.as_object()
        && let Some(field) = fields.keys().find(|key| !matches!(key.as_str(), "path" | "format" | "approval" | "options"))
    {
        return Err(crate::params::bad(command, format!("`{field}` does not apply to plugin formats; use the declared options")));
    }
    let format = resolve_format(session, kind, params.get("format").and_then(Value::as_str), path)?
        .ok_or_else(|| crate::params::bad(command, "unknown plugin format id or file extension"))?;
    let package_path = session
        .plugin_state
        .packages
        .get(&format.plugin_id)
        .cloned()
        .or_else(|| crate::recovery::data_dir().map(|root| root.join("plugins").join(&format.plugin_id).to_string_lossy().into_owned()))
        .ok_or_else(|| crate::params::bad(command, "plugin package is unavailable"))?;
    let package = read_package(&package_path)?;
    if params.get("approval").and_then(Value::as_str) != Some(package.approval.as_str()) {
        return Err(crate::params::bad(command, "approval does not match this manifest and component; inspect and review again"));
    }
    session.plugin_limits.check(&package.manifest.runtime.as_ref().ok_or_else(|| error("no executable runtime"))?.limits)?;
    Ok((package, format))
}

/// Import on a fresh session and replace the open design only after the complete call succeeds.
pub fn import_file(session: &mut Session, path: &str, params: &Value) -> Result<Value> {
    let (package, format) = approved_format(session, EntryKind::Importer, path, params, "file.import")?;
    #[cfg(all(feature = "plugins", any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        if solidcraft_io::vfs::len(path).map_err(|e| error(e.to_string()))? > craft_plugin::MAX_STREAM_BYTES {
            return Err(HostError::LimitExceeded { limit: "input stream bytes".into() }.into());
        }
        let input = solidcraft_io::vfs::read(path, craft_plugin::MAX_STREAM_BYTES as usize).map_err(|e| error(e.to_string()))?;
        let name = Path::new(path).file_name().and_then(|x| x.to_str()).unwrap_or(path).to_owned();
        let stem = Path::new(path).file_stem().and_then(|x| x.to_str()).unwrap_or("Untitled");
        let mut fresh = Session::new(solidcraft_doc::Document::new(stem));
        fresh.set_validation(session.validation());
        fresh.set_command_check(session.command_check());
        fresh.disabled_plugins.clone_from(&session.disabled_plugins);
        fresh.plugin_limits = session.plugin_limits;
        fresh.plugin_state.clone_from(&session.plugin_state);
        let component = fresh.execute("component.create", &json!({"name":stem}))?["component"]
            .as_u64()
            .ok_or_else(|| error("component.create returned no component"))?;
        let streams = craft_plugin_wasm::IoStreams {
            input: Some(craft_plugin_wasm::InputStream { name, reader: Box::new(std::io::Cursor::new(input)) }),
            output: None,
        };
        let (mut host, result) = craft_plugin_wasm::run_io(
            SolidCraftHost { session: fresh, transaction: None },
            &package.manifest,
            &package.component,
            &format.entry,
            &params.get("options").cloned().unwrap_or(json!({})),
            streams,
        );
        let loss_report: craft_plugin::LossReport = serde_json::from_value(result?).map_err(|e| error(e.to_string()))?;
        if solidcraft_doc::progress::cancelled() {
            return Err(HostError::Cancelled.into());
        }
        if host.session.model.state().bodies.iter().any(|body| body.component == 0)
            || host.session.doc.components.iter().any(|item| item.id != 0 && item.parent == 0 && item.id != component)
            || host.session.doc.occurrences.iter().any(|item| item.parent == 0 && item.component != component)
        {
            return Err(HostError::InvalidInput {
                field: "design".into(),
                reason: "imported bodies and assemblies must stay inside the file component".into(),
            }
            .into());
        }
        host.session.lock_plugin_import_contents(component)?;
        if solidcraft_doc::progress::cancelled() {
            return Err(HostError::Cancelled.into());
        }
        host.session.undo.clear();
        host.session.redo.clear();
        host.session.mark_unsaved();
        let reply = serde_json::to_value(crate::schema::plugin::ImportResult {
            path: path.into(),
            format: format.id,
            name: host.session.doc.name.clone(),
            bodies: host.session.model.state().bodies.len(),
            loss_report,
            param_warnings: vec![],
        })
        .map_err(|e| error(e.to_string()))?;
        if solidcraft_doc::progress::cancelled() {
            return Err(HostError::Cancelled.into());
        }
        session.replace_design(host.session);
        Ok(reply)
    }
    #[cfg(not(all(feature = "plugins", any(target_arch = "x86_64", target_arch = "aarch64"))))]
    {
        let _ = (package, format);
        Err(error("plugin execution requires a native build with the plugins feature"))
    }
}

/// Export from a scratch read transaction, publishing bytes atomically only after success.
pub fn export_file(session: &mut Session, path: &str, params: &Value) -> Result<Value> {
    let (package, format) = approved_format(session, EntryKind::Exporter, path, params, "file.export")?;
    solidcraft_io::vfs::check(path).map_err(|e| error(e.to_string()))?;
    #[cfg(all(feature = "plugins", any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        #[derive(Clone)]
        struct Buffer(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
        impl std::io::Write for Buffer {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                let mut buffer = self.0.lock().map_err(|_| std::io::Error::other("export buffer unavailable"))?;
                buffer.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let buffer = Buffer(Default::default());
        let streams = craft_plugin_wasm::IoStreams { input: None, output: Some(Box::new(buffer.clone())) };
        let (_, result) = craft_plugin_wasm::run_io(
            SolidCraftHost { session: session.scratch(), transaction: None },
            &package.manifest,
            &package.component,
            &format.entry,
            &params.get("options").cloned().unwrap_or(json!({})),
            streams,
        );
        result?;
        if solidcraft_doc::progress::cancelled() {
            return Err(HostError::Cancelled.into());
        }
        let bytes = buffer.0.lock().map_err(|_| error("export buffer unavailable"))?;
        solidcraft_io::safe_write::write_atomic(Path::new(path), &bytes, false).map_err(|e| error(e.to_string()))?;
        serde_json::to_value(crate::schema::plugin::ExportResult {
            path: path.into(),
            format: format.id,
            bytes: bytes.len() as u64,
            param_warnings: vec![],
        })
        .map_err(|e| error(e.to_string()))
    }
    #[cfg(not(all(feature = "plugins", any(target_arch = "x86_64", target_arch = "aarch64"))))]
    {
        let _ = (package, format);
        Err(error("plugin execution requires a native build with the plugins feature"))
    }
}

#[cfg(test)]
mod tests;

/// The application adapter owns a scratch transaction. Wasmtime sees only PluginHost.
pub struct SolidCraftHost {
    pub session: Session,
    transaction: Option<Session>,
}
impl PluginHost for SolidCraftHost {
    fn command_catalog(&self) -> Vec<CommandSchema> {
        crate::command_specs()
            .iter()
            .filter(|c| allowed_command(c.id))
            .map(|c| CommandSchema {
                name: c.id.into(),
                input_schema: c.contract.map(|c| c.input_schema().to_value()).unwrap_or_else(|| json!({"type":"object"})),
                output_schema: c.output.map(|f| f().to_value()).unwrap_or(json!({})),
                effect: if c.undoable || c.id == "component.activate" { Effect::Edit } else { Effect::Read },
            })
            .collect()
    }
    fn run_command(&mut self, name: &str, params: &Value) -> craft_plugin::Result<Value> {
        if !allowed_command(name) {
            return Err(HostError::PermissionDenied { permission: format!("command {name} is not granted") });
        }
        let s = self.transaction.as_mut().ok_or_else(|| HostError::CommandFailed { command: name.into(), reason: "no active transaction".into() })?;
        let before = s.doc.clone();
        let previous: BTreeMap<_, _> = s.model.results.iter().map(|r| (r.id, r.error.clone())).collect();
        // Even implicit font and linked-design reads have no authority from a command grant.
        let reply = solidcraft_io::vfs::scoped(Some(Default::default()), || s.execute(name, params))
            .map_err(|e| HostError::CommandFailed { command: name.into(), reason: e.to_string() })?;
        if let Some(failed) = s
            .model
            .results
            .iter()
            .find(|r| r.error.is_some() && (previous.get(&r.id) != Some(&r.error) || before.feature(r.id) != s.doc.feature(r.id)))
        {
            return Err(HostError::CommandFailed { command: name.into(), reason: failed.error.clone().unwrap_or_default() });
        }
        Ok(reply)
    }
    fn begin_transaction(&mut self) -> craft_plugin::Result<()> {
        if self.transaction.is_some() {
            return Err(HostError::InvalidInput { field: "transaction".into(), reason: "already active".into() });
        }
        self.transaction = Some(self.session.scratch());
        Ok(())
    }
    fn commit_transaction(&mut self) -> craft_plugin::Result<()> {
        let mut computed =
            self.transaction.take().ok_or_else(|| HostError::InvalidInput { field: "transaction".into(), reason: "not active".into() })?;
        computed.undo.clear();
        computed.redo.clear();
        // Session::execute(plugin.run) owns the one outer undo step.
        if !std::sync::Arc::ptr_eq(&self.session.doc, &computed.doc)
            || self.session.active_sketch != computed.active_sketch
            || self.session.pending_moves != computed.pending_moves
        {
            self.session.accept_computed(computed);
        } else {
            for message in computed.log {
                self.session.echo(message);
            }
        }
        Ok(())
    }
    fn rollback_transaction(&mut self) -> craft_plugin::Result<()> {
        self.transaction = None;
        Ok(())
    }
    fn granted_folders(&self) -> Vec<FileGrant> {
        let scope = solidcraft_io::vfs::current_scope();
        self.session
            .plugin_state
            .file_grants
            .iter()
            .filter(|g| scope.as_ref().is_none_or(|s| s.check(&g.folder.to_string_lossy()).is_ok()))
            .cloned()
            .collect()
    }
    fn log(&mut self, message: &str) {
        if let Some(s) = self.transaction.as_mut() {
            s.echo(format!("plugin: {}", message.chars().filter(|c| !c.is_control()).collect::<String>()));
        }
    }
    fn progress(&mut self, fraction: f32, message: &str) {
        solidcraft_doc::progress::item(Some((message, (fraction.clamp(0., 1.) * 100.) as usize, 100)));
    }
    fn is_cancelled(&self) -> bool {
        solidcraft_doc::progress::cancelled()
    }
}

/// Resolve an inspected entry through the same approved plugin.run transaction.
/// Unknown packages remain unknown commands; inspecting a package never approves it.
pub fn resolve_call(s: &Session, id: &str, params: &Value) -> Result<Option<Value>> {
    let Some((package, entry)) = id.split_once('.') else { return Ok(None) };
    let Some(path) = s.plugin_state.packages.get(package) else { return Ok(None) };
    if crate::find_command(id).is_some() {
        return Ok(None);
    }
    if !valid_id(entry) {
        return Err(EngineError::UnknownCommand(id.into()));
    }
    let approval =
        params.get("approval").and_then(Value::as_str).ok_or_else(|| crate::params::bad(id, "approval from plugin.inspect is required"))?;
    Ok(Some(json!({"path":path,"approval":approval,"entry":entry,"params":params.get("params").cloned().unwrap_or(json!({}))})))
}

pub fn describe_entry(s: &Session, id: &str) -> Result<Option<Value>> {
    if crate::find_command(id).is_some() {
        return Ok(None);
    }
    let Some((package, name)) = id.split_once('.') else { return Ok(None) };
    let Some(path) = s.plugin_state.packages.get(package) else { return Ok(None) };
    let package = read_package(path)?;
    let Some(runtime) = package.manifest.runtime else { return Ok(None) };
    let Some(entry) = runtime.entries.iter().find(|e| e.name == name && e.kind == EntryKind::Command) else { return Ok(None) };
    Ok(Some(json!({"id":id,"label":name,"params":"approval, params: declared entry inputs",
        "input":{"type":"object","required":["approval","params"],"additionalProperties":false,"properties":{
            "approval":{"type":"string"},"params":entry.input_schema}},
        "output":{},"effect":entry.effect,"slots":entry.slots})))
}

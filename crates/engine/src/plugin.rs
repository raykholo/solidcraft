//! Explicit, content-pinned one-shot plugins. The design keeps ordinary native features.

use std::collections::BTreeSet;
use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{EngineError, Result, Session};

#[cfg(all(feature = "plugins", any(target_arch = "x86_64", target_arch = "aarch64")))]
mod runtime;

pub const MAX_MANIFEST_BYTES: usize = 16 * 1024;
/// The component size a package may ship without asking for more (`limits.component_mib`).
pub const MAX_COMPONENT_BYTES: usize = 16 * 1024 * 1024;
/// The most `limits.component_mib` may ask for: an importer that links several file-format
/// decoders and a design writer is larger than a command plugin.
pub const MAX_COMPONENT_MIB: usize = 128;
pub const MAX_JSON_BYTES: usize = 1024 * 1024;
pub const API: &str = "solidcraft:plugin@0.1.0";
/// Memory32 can address at most 65536 pages of 64 KiB; the runtime allows one memory.
pub const MAX_MEMORY_MIB: usize = 4096;

/// User-owned ceilings, separate from a package's exact, content-approved request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct HostLimits {
    pub max_fuel: u64,
    pub max_memory_mib: usize,
}
impl Default for HostLimits {
    fn default() -> Self {
        Self { max_fuel: 100_000_000_000, max_memory_mib: 128 }
    }
}
impl HostLimits {
    pub fn validate(&self) -> Result<()> {
        if self.max_fuel == 0 || !(1..=MAX_MEMORY_MIB).contains(&self.max_memory_mib) {
            return Err(error("host limits require positive integer max_fuel and max_memory_mib 1..4096 (Wasm32)"));
        }
        Ok(())
    }
    pub fn check(&self, requested: &Limits) -> Result<()> {
        self.validate()?;
        requested.validate()?;
        if requested.fuel > self.max_fuel || requested.memory_mib > self.max_memory_mib {
            return Err(error(format!(
                "requested fuel {} / memory {} MiB exceeds user host limits {} / {} MiB; explicitly change the host settings and review the package before running",
                requested.fuel, requested.memory_mib, self.max_fuel, self.max_memory_mib
            )));
        }
        Ok(())
    }
}
static PROCESS_HOST_LIMITS: std::sync::OnceLock<HostLimits> = std::sync::OnceLock::new();
/// Configure trusted host startup once, before creating CLI/headless sessions. Not a guest command.
pub fn configure_process_host_limits(limits: HostLimits) -> Result<()> {
    limits.validate()?;
    PROCESS_HOST_LIMITS.set(limits).map_err(|_| error("process plugin host limits are already configured"))
}
pub(crate) fn default_host_limits() -> HostLimits {
    PROCESS_HOST_LIMITS.get().copied().unwrap_or_default()
}

/// Metadata is inert. A runtime declaration still needs explicit approval to execute.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<RuntimeManifest>,
}

/// What a package's component does, and so which WIT world it is run in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// `command-plugin`: entries run through `plugin.run`, editing the design through granted
    /// engine commands.
    #[default]
    Command,
    /// `importer-plugin`: converts one file's bytes into a design (`file.import`). It is given
    /// those bytes and nothing else: no engine commands, files, network or clock.
    Importer,
}

/// A file type an importer converts.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileFormat {
    /// The extension without its dot, matched without regard to case (`f3d`).
    pub extension: String,
    /// The format's name, as File > Import lists it (`Fusion design`).
    pub name: String,
}

/// The plugin worlds: no WASI, files, network or process imports.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RuntimeManifest {
    pub api: String,
    /// A single file name within the package, never an absolute or relative directory.
    pub module: String,
    #[serde(default)]
    pub kind: Kind,
    /// A command plugin's entries (1..32); an importer has none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<String>,
    /// Exact built-in command ids. No patterns, aliases or plugin-to-plugin calls. An importer
    /// is granted none.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub commands: BTreeSet<String>,
    /// The file types an importer converts (1..32); a command plugin has none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub formats: Vec<FileFormat>,
    #[serde(default)]
    pub limits: Limits,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub fuel: u64,
    pub memory_mib: usize,
    pub deadline_ms: u64,
    /// The largest component file the package may have, in MiB (default 16, at most 128). It is
    /// part of what the user approves: a larger component takes longer and more memory to compile.
    #[serde(default = "default_component_mib", skip_serializing_if = "is_default_component_mib")]
    pub component_mib: usize,
}
fn default_component_mib() -> usize {
    MAX_COMPONENT_BYTES / (1024 * 1024)
}
fn is_default_component_mib(mib: &usize) -> bool {
    *mib == default_component_mib()
}
impl Default for Limits {
    fn default() -> Self {
        Self { fuel: 10_000_000, memory_mib: 64, deadline_ms: 5000, component_mib: default_component_mib() }
    }
}
impl Limits {
    fn validate(&self) -> Result<()> {
        if self.fuel == 0 || !(1..=MAX_MEMORY_MIB).contains(&self.memory_mib) || !(1..=30_000).contains(&self.deadline_ms) {
            return Err(error("limits require positive fuel, memory 1..4096 MiB (Wasm32) and deadline 1..30000 ms"));
        }
        if !(1..=MAX_COMPONENT_MIB).contains(&self.component_mib) {
            return Err(error("component_mib must be 1..128"));
        }
        Ok(())
    }
}

pub fn valid_id(id: &str) -> bool {
    let b = id.as_bytes();
    (1..=64).contains(&b.len())
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
        && b.first() != Some(&b'-')
        && b.last() != Some(&b'-')
}
fn plain(s: &str, max: usize) -> bool {
    !s.trim().is_empty() && s.chars().count() <= max && !s.chars().any(char::is_control)
}
fn error(s: impl Into<String>) -> EngineError {
    EngineError::Other(format!("plugin: {}", s.into()))
}

pub fn parse_manifest(text: &[u8], folder: &str) -> Result<Manifest> {
    if text.len() > MAX_MANIFEST_BYTES {
        return Err(error("manifest exceeds 16 KiB"));
    }
    let mut m: Manifest = serde_json::from_slice(text).map_err(|e| error(format!("invalid manifest: {e}")))?;
    // Keep the metadata page's existing whitespace normalization.
    m.name = m.name.trim().to_string();
    m.version = m.version.trim().to_string();
    m.description = m.description.trim().to_string();
    if !valid_id(&m.id) || m.id != folder {
        return Err(error("manifest id does not match its lowercase package folder"));
    }
    if !plain(&m.name, 80)
        || !plain(&m.version, 32)
        || !m.version.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
        || m.description.chars().count() > 240
        || m.description.chars().any(char::is_control)
    {
        return Err(error("invalid name, version or description"));
    }
    if let Some(runtime) = m.runtime.as_mut() {
        if runtime.api != API {
            return Err(error("unsupported plugin API"));
        }
        if !valid_id(runtime.module.strip_suffix(".wasm").unwrap_or("")) {
            return Err(error("module must be one lowercase .wasm file name"));
        }
        match runtime.kind {
            Kind::Command => {
                if runtime.entries.is_empty()
                    || runtime.entries.len() > 32
                    || runtime.entries.iter().any(|e| !valid_id(e))
                    || runtime.entries.iter().collect::<BTreeSet<_>>().len() != runtime.entries.len()
                {
                    return Err(error("entries must be 1..32 distinct ids"));
                }
                if runtime.commands.len() > 128 || runtime.commands.iter().any(|id| !allowed_command(id)) {
                    return Err(error("commands include an unavailable or forbidden built-in command"));
                }
                if !runtime.formats.is_empty() {
                    return Err(error("only an importer declares file formats"));
                }
            }
            Kind::Importer => {
                if !runtime.entries.is_empty() || !runtime.commands.is_empty() {
                    return Err(error("an importer has no entries and is granted no engine commands"));
                }
                if runtime.formats.is_empty() || runtime.formats.len() > 32 {
                    return Err(error("an importer declares 1..32 file formats"));
                }
                let mut seen = BTreeSet::new();
                for format in &mut runtime.formats {
                    format.extension = format.extension.trim().trim_start_matches('.').to_ascii_lowercase();
                    format.name = format.name.trim().to_string();
                    if !valid_extension(&format.extension) || !plain(&format.name, 80) || !seen.insert(format.extension.clone()) {
                        return Err(error("each format needs a distinct extension (1..16 letters and digits) and a name"));
                    }
                }
            }
        }
        runtime.limits.validate()?;
    }
    Ok(m)
}

/// An extension an importer may claim: 1..16 lowercase ASCII letters and digits.
fn valid_extension(extension: &str) -> bool {
    (1..=16).contains(&extension.len()) && extension.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

/// Positive host policy, intersected with the manifest. New command families require review.
pub fn allowed_command(id: &str) -> bool {
    let family = id.starts_with("solid.") || id.starts_with("sketch.");
    let explicit = matches!(
        id,
        "parameters.add"
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
    let cap = runtime.limits.component_mib.saturating_mul(1024 * 1024);
    let component = solidcraft_io::vfs::read(&module_path.to_string_lossy(), cap)
        .map_err(|e| error(format!("{e} (the component may be at most {} MiB: limits.component_mib)", runtime.limits.component_mib)))?;
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
    if rt.kind != Kind::Command {
        return Err(error("an importer runs through file.import, not plugin.run"));
    }
    session.plugin_limits.check(&rt.limits)?;
    if !rt.entries.iter().any(|e| e == entry) {
        return Err(error("unknown entry"));
    }
    #[cfg(all(feature = "plugins", any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        runtime::run(session, &package, entry, params)
    }
    #[cfg(not(all(feature = "plugins", any(target_arch = "x86_64", target_arch = "aarch64"))))]
    {
        let _ = params;
        Err(error("plugin execution requires an x86-64 or AArch64 native build with the plugins feature"))
    }
}

/// What an importer made of a file.
#[derive(Debug)]
pub enum Payload {
    /// A SolidCraft design container, opened as it is: its history stays editable.
    Design(Vec<u8>),
    /// STEP text, imported as STEP files are.
    Step(Vec<u8>),
}

/// An importer's result: the design, and what did not come across.
#[derive(Debug)]
pub struct Imported {
    pub payload: Payload,
    pub notes: String,
}

/// The largest file handed to an importer, and the largest design it may hand back.
pub const MAX_IMPORT_BYTES: usize = 512 * 1024 * 1024;
/// The most text of notes an importer may hand back.
pub const MAX_NOTES_BYTES: usize = 64 * 1024;

/// Run an importer on one file's contents. The component gets `file_name` (no folders) and
/// `bytes`, nothing else, under the package's approved limits; it cannot touch the session.
pub fn import(session: &Session, package: &Package, file_name: &str, bytes: Vec<u8>) -> Result<Imported> {
    if session.disabled_plugins.contains(&package.manifest.id) {
        return Err(error("plugin is disabled"));
    }
    let rt = package.manifest.runtime.as_ref().ok_or_else(|| error("no executable runtime"))?;
    if rt.kind != Kind::Importer {
        return Err(error("this plugin is not an importer"));
    }
    session.plugin_limits.check(&rt.limits)?;
    if bytes.len() > MAX_IMPORT_BYTES {
        return Err(error("the file is larger than an importer may be given (512 MiB)"));
    }
    let file_name = Path::new(file_name).file_name().and_then(|n| n.to_str()).unwrap_or_default();
    #[cfg(all(feature = "plugins", any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        runtime::import(package, file_name, bytes)
    }
    #[cfg(not(all(feature = "plugins", any(target_arch = "x86_64", target_arch = "aarch64"))))]
    {
        let _ = (file_name, bytes);
        Err(error("plugin execution requires an x86-64 or AArch64 native build with the plugins feature"))
    }
}

/// The folder installed plugins live in: `plugins` in the per-user data folder.
pub fn default_plugins_dir() -> Option<std::path::PathBuf> {
    // Tests never find the user's own plugins.
    if cfg!(test) {
        return None;
    }
    crate::recovery::data_dir().map(|d| d.join("plugins"))
}

/// An installed package: its folder, its manifest and the approval hash of its exact content.
#[derive(Clone, Debug)]
pub struct Installed {
    pub path: std::path::PathBuf,
    pub manifest: Manifest,
    pub approval: String,
}

/// The installed packages in `dir` that parse, by id. Unreadable folders are left out.
pub fn installed(dir: &Path) -> Vec<Installed> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return out };
    for entry in entries.flatten().take(256) {
        if !entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let path = entry.path();
        if let Ok(package) = read_package(&path.to_string_lossy()) {
            out.push(Installed { path, manifest: package.manifest, approval: package.approval });
        }
    }
    out.sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
    out
}

/// The installed importer that converts files with `path`'s extension: enabled, approved for
/// its exact content, and the first by id when more than one claims it. `None` when none does.
pub fn importer_for(session: &Session, path: &str) -> Option<Installed> {
    let extension = Path::new(path).extension()?.to_str()?.to_ascii_lowercase();
    let dir = session.plugins_dir.as_deref()?;
    installed(dir).into_iter().find(|p| {
        p.manifest.runtime.as_ref().is_some_and(|r| r.kind == Kind::Importer && r.formats.iter().any(|f| f.extension == extension))
            && !session.disabled_plugins.contains(&p.manifest.id)
            && session.plugin_approvals.get(&p.manifest.id) == Some(&p.approval)
    })
}

/// Copy the package in folder `from` into the plugins folder `dir`, replacing an installed
/// copy only when `replace`. Installing never approves it: the user allows it in Preferences.
pub fn install(from: &str, dir: &Path, replace: bool) -> Result<Installed> {
    let package = read_package(from)?;
    let runtime = package.manifest.runtime.as_ref().ok_or_else(|| error("metadata-only package has no executable runtime"))?;
    let to = dir.join(&package.manifest.id);
    if to.exists() && !replace {
        return Err(error(format!("`{}` is already installed; pass replace: true to replace it", package.manifest.id)));
    }
    let manifest =
        solidcraft_io::vfs::read(&Path::new(from).join("plugin.json").to_string_lossy(), MAX_MANIFEST_BYTES).map_err(|e| error(e.to_string()))?;
    let write = |path: std::path::PathBuf, bytes: &[u8]| std::fs::write(&path, bytes).map_err(|e| error(format!("{}: {e}", path.display())));
    std::fs::create_dir_all(&to).map_err(|e| error(format!("{}: {e}", to.display())))?;
    write(to.join(&runtime.module), &package.component)?;
    write(to.join("plugin.json"), &manifest)?;
    let installed = read_package(&to.to_string_lossy())?;
    if installed.approval != package.approval {
        return Err(error("the installed copy differs from the package; install it again"));
    }
    Ok(Installed { path: to, manifest: installed.manifest, approval: installed.approval })
}

#[cfg(test)]
mod tests;

//! Explicit, content-pinned one-shot plugins. The design keeps ordinary native features.

use std::collections::BTreeSet;
use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{EngineError, Result, Session};

pub mod contrib;
#[cfg(all(feature = "plugins", any(target_arch = "x86_64", target_arch = "aarch64")))]
mod runtime;
pub mod web_abi;

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
    /// The most work (Wasm instructions, counted as fuel) one run may do.
    pub max_fuel: u64,
    pub max_memory_mib: usize,
    /// The longest one run may take, in milliseconds (1..30000).
    pub max_deadline_ms: u64,
}
impl Default for HostLimits {
    fn default() -> Self {
        LimitPreset::Normal.limits()
    }
}

/// The plain choices for the host ceilings: how much a plugin may do before it is stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LimitPreset {
    /// Enough for small command plugins: the original ceilings.
    Normal,
    /// For importers and other large plugins: 2 GiB and ten times the work.
    Large,
    /// For trusted plugins: everything the host allows.
    Largest,
}

/// The longest any run may take, whatever the ceilings: 30 seconds.
pub const MAX_DEADLINE_MS: u64 = 30_000;

impl LimitPreset {
    pub const ALL: [LimitPreset; 3] = [LimitPreset::Normal, LimitPreset::Large, LimitPreset::Largest];
    pub fn limits(self) -> HostLimits {
        match self {
            LimitPreset::Normal => HostLimits { max_fuel: 100_000_000_000, max_memory_mib: 128, max_deadline_ms: MAX_DEADLINE_MS },
            LimitPreset::Large => HostLimits { max_fuel: 1_000_000_000_000, max_memory_mib: 2048, max_deadline_ms: MAX_DEADLINE_MS },
            LimitPreset::Largest => HostLimits { max_fuel: u64::MAX, max_memory_mib: MAX_MEMORY_MIB, max_deadline_ms: MAX_DEADLINE_MS },
        }
    }
    /// The preset these ceilings are exactly, if any (`None`: set by hand).
    pub fn of(limits: &HostLimits) -> Option<LimitPreset> {
        Self::ALL.into_iter().find(|p| p.limits() == *limits)
    }
}

impl HostLimits {
    pub fn validate(&self) -> Result<()> {
        if self.max_fuel == 0 || !(1..=MAX_MEMORY_MIB).contains(&self.max_memory_mib) || !(1..=MAX_DEADLINE_MS).contains(&self.max_deadline_ms) {
            return Err(error("host limits require positive integer max_fuel, max_memory_mib 1..4096 (Wasm32) and max_deadline_ms 1..30000"));
        }
        Ok(())
    }
    pub fn check(&self, requested: &Limits) -> Result<()> {
        self.validate()?;
        requested.validate()?;
        if requested.fuel > self.max_fuel || requested.memory_mib > self.max_memory_mib || requested.deadline_ms > self.max_deadline_ms {
            return Err(error(format!(
                "this plugin needs up to {} MiB of memory, {} s and {} instructions, more than the plugin settings allow ({} MiB, {} s, {} instructions); choose a larger setting in Preferences > Plugins (or --plugin-limits) if you trust it",
                requested.memory_mib,
                requested.deadline_ms as f64 / 1000.0,
                requested.fuel,
                self.max_memory_mib,
                self.max_deadline_ms as f64 / 1000.0,
                self.max_fuel
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
static PROCESS_APPROVALS: std::sync::OnceLock<std::collections::BTreeMap<String, String>> = std::sync::OnceLock::new();
/// Allow installed plugins (id → approval hash) for every local session this process makes,
/// and put their contributed commands in the registry. Trusted host startup only (the CLI's
/// `--allow-plugin`), never a guest command.
pub fn configure_process_approvals(approvals: std::collections::BTreeMap<String, String>) -> Result<()> {
    if approvals.iter().any(|(id, hash)| !valid_id(id) || hash.len() != 64 || !hash.bytes().all(|c| c.is_ascii_hexdigit())) {
        return Err(error("an approval is an installed plugin's id and the 64-digit hash plugin.inspect gives"));
    }
    contrib::sync(default_plugins_dir().as_deref(), &approvals, &Default::default(), &|_| true);
    PROCESS_APPROVALS.set(approvals).map_err(|_| error("process plugin approvals are already configured"))
}
pub(crate) fn default_approvals() -> std::collections::BTreeMap<String, String> {
    PROCESS_APPROVALS.get().cloned().unwrap_or_default()
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
    /// Who publishes it, as the Plugins page shows it (at most 80 characters).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub publisher: String,
    /// The package's icon: an SVG file in the package (`icon.svg`), drawn as the app's own icons
    /// are (only their plain shapes and palette colours).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<RuntimeManifest>,
    /// Commands it adds to the app, and where on the ribbon they go.
    #[serde(default, skip_serializing_if = "contrib::Contributes::is_empty")]
    pub contributes: contrib::Contributes,
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
    /// An importer's core WebAssembly module for the browser (the web ABI, see [`web_abi`]),
    /// beside its component. Part of what the user approves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web: Option<WebModule>,
}

/// The browser's build of an importer: a core module file in the package.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WebModule {
    /// A single lowercase `.wasm` file name, not the component's.
    pub module: String,
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
    m.publisher = m.publisher.trim().to_string();
    if m.publisher.chars().count() > 80 || m.publisher.chars().any(char::is_control) {
        return Err(error("the publisher is at most 80 characters"));
    }
    if m.icon.as_deref().is_some_and(|i| !contrib::icon_file(i)) {
        return Err(error("the icon is a lowercase .svg file in the package"));
    }
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
        if let Some(web) = &runtime.web {
            if runtime.kind != Kind::Importer {
                return Err(error("only an importer has a web module"));
            }
            if !valid_id(web.module.strip_suffix(".wasm").unwrap_or("")) || web.module == runtime.module {
                return Err(error("the web module is one lowercase .wasm file name, not the component's"));
            }
        }
    }
    contrib::validate(&m)?;
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
    /// The SVG icons the manifest names, by file name (each at most 32 KiB of UTF-8).
    pub icons: std::collections::BTreeMap<String, String>,
    /// The browser's core module (`runtime.web`), when the package has one.
    pub web_module: Option<Vec<u8>>,
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
    // The browser's module is part of what the user approves; a package without one hashes as
    // before.
    let web_module = match runtime.web.as_ref() {
        Some(web) => {
            let bytes = solidcraft_io::vfs::read(&dir.join(&web.module).to_string_lossy(), cap)
                .map_err(|e| error(format!("{}: {e} (the web module may be at most {} MiB)", web.module, runtime.limits.component_mib)))?;
            digest.update(b"\0web\0");
            digest.update((bytes.len() as u64).to_le_bytes());
            digest.update(&bytes);
            Some(bytes)
        }
        None => None,
    };
    // Icons are part of what the user approves; a package without any hashes as before.
    let mut icons = std::collections::BTreeMap::new();
    for name in contrib::icon_files(&manifest) {
        let bytes = solidcraft_io::vfs::read(&dir.join(&name).to_string_lossy(), contrib::MAX_ICON_BYTES)
            .map_err(|e| error(format!("{name}: {e} (an icon is at most 32 KiB)")))?;
        let text = String::from_utf8(bytes).map_err(|_| error(format!("{name}: an icon is UTF-8 SVG text")))?;
        if icons.is_empty() {
            digest.update(b"\0icons\0");
        }
        digest.update((name.len() as u64).to_le_bytes());
        digest.update(name.as_bytes());
        digest.update((text.len() as u64).to_le_bytes());
        digest.update(text.as_bytes());
        icons.insert(name, text);
    }
    Ok(Package { manifest, approval: format!("{:x}", digest.finalize()), component, icons, web_module })
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

/// An import a host runs itself (the browser's plugin host): everything the importer is given.
#[derive(Clone, Debug)]
pub struct HostRequest {
    pub plugin: String,
    pub approval: String,
    /// The package's browser module (`runtime.web`); `None` when it has none.
    pub web_module: Option<Vec<u8>>,
    pub file_name: String,
    pub bytes: Vec<u8>,
    /// The checked options, as JSON.
    pub options: String,
    pub limits: Limits,
    /// Which import this is, to match its result to it.
    pub key: String,
}

enum Hosted {
    /// Record the next import instead of running it.
    Ask(Option<HostRequest>),
    /// The result of the import with this key.
    Answer(String, Option<std::result::Result<Imported, String>>),
}

thread_local! {
    static HOSTED: std::cell::RefCell<Option<Hosted>> = const { std::cell::RefCell::new(None) };
}

/// Run `f` (a command) without running any importer: the first import it reaches is recorded
/// and refused. Returns `f`'s result and that import, for the host to run.
pub fn ask_host<R>(f: impl FnOnce() -> R) -> (R, Option<HostRequest>) {
    HOSTED.with(|h| *h.borrow_mut() = Some(Hosted::Ask(None)));
    let r = f();
    let asked = HOSTED.with(|h| match h.borrow_mut().take() {
        Some(Hosted::Ask(asked)) => asked,
        _ => None,
    });
    (r, asked)
}

/// Run `f` (the same command again) with the host's result for the import `key`.
pub fn answer_host<R>(key: &str, result: std::result::Result<Imported, String>, f: impl FnOnce() -> R) -> R {
    HOSTED.with(|h| *h.borrow_mut() = Some(Hosted::Answer(key.to_string(), Some(result))));
    let r = f();
    HOSTED.with(|h| *h.borrow_mut() = None);
    r
}

fn hosted(
    package: &Package,
    file_name: &str,
    bytes: &[u8],
    options: &serde_json::Map<String, serde_json::Value>,
) -> Result<Option<Result<Imported>>> {
    let options = serde_json::to_string(options).map_err(|e| error(e.to_string()))?;
    let mut digest = Sha256::new();
    for part in [package.approval.as_bytes(), file_name.as_bytes(), options.as_bytes(), bytes] {
        digest.update((part.len() as u64).to_le_bytes());
        digest.update(part);
    }
    let key = format!("{:x}", digest.finalize());
    HOSTED.with(|h| {
        let mut h = h.borrow_mut();
        Ok(match h.as_mut() {
            None => None,
            Some(Hosted::Ask(asked)) => {
                let runtime = package.manifest.runtime.as_ref().ok_or_else(|| error("no executable runtime"))?;
                *asked = Some(HostRequest {
                    plugin: package.manifest.id.clone(),
                    approval: package.approval.clone(),
                    web_module: package.web_module.clone(),
                    file_name: file_name.to_string(),
                    bytes: bytes.to_vec(),
                    options,
                    limits: runtime.limits.clone(),
                    key,
                });
                Some(Err(error("this import runs in the plugin host")))
            }
            Some(Hosted::Answer(answered, result)) if *answered == key => {
                Some(result.take().unwrap_or_else(|| Err("the plugin host's result was already used".into())).map_err(error))
            }
            Some(Hosted::Answer(..)) => Some(Err(error("the file or options changed while the plugin host ran the import"))),
        })
    })
}

/// The largest file handed to an importer, and the largest design it may hand back.
pub const MAX_IMPORT_BYTES: usize = 512 * 1024 * 1024;
/// The most text of notes an importer may hand back.
pub const MAX_NOTES_BYTES: usize = 64 * 1024;

/// Run an importer on one file's contents. The component gets `file_name` (no folders) and
/// `bytes`, nothing else, under the package's approved limits; it cannot touch the session.
/// `options`: the checked values of the package's import command parameters, which the
/// importer reads through `import-context.options`.
pub fn import(
    session: &Session,
    package: &Package,
    file_name: &str,
    bytes: Vec<u8>,
    options: &serde_json::Map<String, serde_json::Value>,
) -> Result<Imported> {
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
    // A host that runs importers itself (the browser's, which runs them in a Web Worker) asks
    // for the import first, then hands back its result.
    if let Some(r) = hosted(package, file_name, &bytes, options)? {
        return r;
    }
    #[cfg(all(feature = "plugins", any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        let options = serde_json::to_string(options).map_err(|e| error(e.to_string()))?;
        runtime::import(package, file_name, bytes, options, session.plugins_dir.as_deref().and_then(compiled_cache_dir).as_deref())
    }
    #[cfg(not(all(feature = "plugins", any(target_arch = "x86_64", target_arch = "aarch64"))))]
    {
        let _ = (file_name, bytes, options);
        Err(error("plugin execution requires an x86-64 or AArch64 native build with the plugins feature"))
    }
}

/// Where compiled importers are kept between sessions: `plugin-cache` beside the plugins
/// folder `plugins_dir`. Only SolidCraft writes it; a package or a design never supplies
/// compiled code.
pub fn compiled_cache_dir(plugins_dir: &Path) -> Option<std::path::PathBuf> {
    plugins_dir.parent().map(|d| d.join("plugin-cache"))
}

/// Compile an importer now (or load its earlier compilation from the cache beside
/// `plugins_dir`), so its first import does not wait for it. Runs no plugin code. A command
/// plugin, or a build without an execution host, has nothing to prepare.
pub fn prepare_importer(package: &Package, plugins_dir: &Path) -> Result<()> {
    let rt = package.manifest.runtime.as_ref().ok_or_else(|| error("no executable runtime"))?;
    if rt.kind != Kind::Importer {
        return Ok(());
    }
    #[cfg(all(feature = "plugins", any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        runtime::prepare(package, compiled_cache_dir(plugins_dir).as_deref())
    }
    #[cfg(not(all(feature = "plugins", any(target_arch = "x86_64", target_arch = "aarch64"))))]
    {
        let _ = plugins_dir;
        Ok(())
    }
}

/// The folder installed plugins live in: `plugins` in the per-user data folder.
pub fn default_plugins_dir() -> Option<std::path::PathBuf> {
    // Tests never find the user's own plugins.
    if cfg!(test) {
        return None;
    }
    // The browser keeps them in its private storage, as it keeps designs.
    if cfg!(target_arch = "wasm32") {
        return Some(std::path::PathBuf::from(WEB_PLUGINS_DIR));
    }
    crate::recovery::data_dir().map(|d| d.join("plugins"))
}

/// Where the browser build keeps installed plugins (its private storage, OPFS).
pub const WEB_PLUGINS_DIR: &str = "/opfs/plugins";

/// The package folders in `dir`.
fn package_folders(dir: &Path) -> Vec<std::path::PathBuf> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
        entries.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).map(|e| e.path()).take(256).collect()
    }
    #[cfg(target_arch = "wasm32")]
    {
        // Browser storage keys files by their whole path: a package is a `<dir>/<id>/plugin.json`.
        let prefix = format!("{}/", dir.to_string_lossy().trim_end_matches('/'));
        let mut out: Vec<std::path::PathBuf> = solidcraft_io::vfs::list(&prefix)
            .into_iter()
            .filter_map(|(p, _)| p.strip_prefix(&prefix)?.strip_suffix("/plugin.json").filter(|id| !id.contains('/')).map(|id| dir.join(id)))
            .collect();
        out.truncate(256);
        out
    }
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
    for path in package_folders(dir) {
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

/// The installed plugin `id`, when the user allowed it for its exact content now and has not
/// turned it off; otherwise why not, in words for the user.
pub fn allowed_installed(session: &Session, id: &str) -> std::result::Result<Installed, String> {
    let dir = session.plugins_dir.as_deref().ok_or("this build has no plugins folder")?;
    let installed = installed(dir).into_iter().find(|p| p.manifest.id == id).ok_or_else(|| format!("no installed plugin `{id}`"))?;
    if session.disabled_plugins.contains(id) {
        return Err(format!("plugin `{id}` is turned off in Preferences > Plugins"));
    }
    if session.plugin_approvals.get(id) != Some(&installed.approval) {
        return Err(format!("plugin `{id}` is not allowed to run: allow it in Preferences > Plugins"));
    }
    Ok(installed)
}

/// Delete installed package `id` from the plugins folder `dir`: only a folder `installed`
/// lists, by its exact path inside `dir`.
pub fn remove(id: &str, dir: &Path) -> Result<()> {
    let package = installed(dir).into_iter().find(|p| p.manifest.id == id).ok_or_else(|| error(format!("no installed plugin `{id}`")))?;
    if package.path != dir.join(id) {
        return Err(error("the package is not a folder of the plugins folder"));
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        if package.path.symlink_metadata().map(|m| m.file_type().is_symlink()).unwrap_or(true) {
            return Err(error("the package is not a folder of the plugins folder"));
        }
        std::fs::remove_dir_all(&package.path).map_err(|e| error(format!("{}: {e}", package.path.display())))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let prefix = format!("{}/", package.path.to_string_lossy());
        for (file, _) in solidcraft_io::vfs::list(&prefix) {
            solidcraft_io::vfs::remove(&file).map_err(|e| error(format!("{file}: {e}")))?;
        }
        Ok(())
    }
}

/// The most files and bytes a package file (zip) may hold.
const MAX_PACKAGE_FILES: usize = 64;
const MAX_PACKAGE_BYTES: u64 = 2 * MAX_COMPONENT_MIB as u64 * 1024 * 1024 + 4 * 1024 * 1024;

/// A package file's contents: its id (from its plugin.json) and its files by name. The files
/// may sit at the top of the zip or in one folder; each is a plain file name, and the total is
/// bounded before anything is decompressed past the limit.
pub fn unpack(zip_bytes: &[u8]) -> Result<(String, std::collections::BTreeMap<String, Vec<u8>>)> {
    use std::io::Read;
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).map_err(|e| error(format!("not a plugin package file: {e}")))?;
    if archive.len() > MAX_PACKAGE_FILES * 2 {
        return Err(error("a plugin package holds at most 64 files"));
    }
    let mut files = std::collections::BTreeMap::new();
    let mut total = 0u64;
    let mut top: Option<Option<String>> = None;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| error(format!("plugin package: {e}")))?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().map_err(|e| error(format!("plugin package: {e}")))?.replace('\\', "/");
        let (folder, file) = match name.rsplit_once('/') {
            Some((folder, file)) => (Some(folder.to_string()), file.to_string()),
            None => (None, name.clone()),
        };
        // One folder level at most, the same for every file.
        if folder.as_deref().is_some_and(|f| f.contains('/') || f.is_empty() || f == "..") || top.as_ref().is_some_and(|t| *t != folder) {
            return Err(error("a plugin package's files are at its top, or in one folder"));
        }
        top = Some(folder);
        if file.is_empty() || file.starts_with('.') || file.contains("..") || file.len() > 64 {
            return Err(error(format!("plugin package: `{name}` is not a plain file name")));
        }
        total = total.saturating_add(entry.size());
        if total > MAX_PACKAGE_BYTES || files.len() >= MAX_PACKAGE_FILES {
            return Err(error("the plugin package is too large"));
        }
        let mut bytes = Vec::new();
        entry.by_ref().take(MAX_PACKAGE_BYTES).read_to_end(&mut bytes).map_err(|e| error(format!("plugin package: {e}")))?;
        files.insert(file, bytes);
    }
    let manifest = files.get("plugin.json").ok_or_else(|| error("the plugin package has no plugin.json"))?;
    #[derive(Deserialize)]
    struct Id {
        id: String,
    }
    let id: Id = serde_json::from_slice(manifest).map_err(|e| error(format!("plugin.json: {e}")))?;
    if !valid_id(&id.id) {
        return Err(error("plugin.json: the id must be 1..64 lowercase letters, digits and hyphens"));
    }
    Ok((id.id, files))
}

/// Install the package file (zip) at `path`: its files are unpacked into a folder of their own,
/// checked as a package folder is, and installed from there.
pub fn install_file(path: &str, dir: &Path, replace: bool) -> Result<Installed> {
    let bytes = solidcraft_io::vfs::read(path, MAX_PACKAGE_BYTES as usize).map_err(|e| error(format!("{path}: {e}")))?;
    let (id, files) = unpack(&bytes)?;
    #[cfg(not(target_arch = "wasm32"))]
    let staging = {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        solidcraft_io::vfs::temp_dir().join(format!("solidcraft-plugin-package-{}-{n}", std::process::id())).join(&id)
    };
    #[cfg(target_arch = "wasm32")]
    let staging = Path::new("/uploads/plugin-package").join(&id);
    for (name, bytes) in &files {
        let to = staging.join(name);
        #[cfg(not(target_arch = "wasm32"))]
        {
            std::fs::create_dir_all(&staging).map_err(|e| error(format!("{}: {e}", staging.display())))?;
            std::fs::write(&to, bytes).map_err(|e| error(format!("{}: {e}", to.display())))?;
        }
        #[cfg(target_arch = "wasm32")]
        solidcraft_io::vfs::write(&to.to_string_lossy(), bytes).map_err(|e| error(format!("{}: {e}", to.display())))?;
    }
    let installed = install(&staging.to_string_lossy(), dir, replace);
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(parent) = staging.parent() {
        let _ = std::fs::remove_dir_all(parent);
    }
    #[cfg(target_arch = "wasm32")]
    for name in files.keys() {
        let _ = solidcraft_io::vfs::remove(&staging.join(name).to_string_lossy());
    }
    installed
}

/// Copy the package in folder `from` into the plugins folder `dir`, replacing an installed
/// copy only when `replace`. Installing never approves it: the user allows it in Preferences.
pub fn install(from: &str, dir: &Path, replace: bool) -> Result<Installed> {
    let package = read_package(from)?;
    let runtime = package.manifest.runtime.as_ref().ok_or_else(|| error("metadata-only package has no executable runtime"))?;
    let to = dir.join(&package.manifest.id);
    #[cfg(not(target_arch = "wasm32"))]
    let present = to.exists();
    #[cfg(target_arch = "wasm32")]
    let present = solidcraft_io::vfs::exists(&to.join("plugin.json").to_string_lossy());
    if present && !replace {
        return Err(error(format!("`{}` is already installed; pass replace: true to replace it", package.manifest.id)));
    }
    let manifest =
        solidcraft_io::vfs::read(&Path::new(from).join("plugin.json").to_string_lossy(), MAX_MANIFEST_BYTES).map_err(|e| error(e.to_string()))?;
    #[cfg(not(target_arch = "wasm32"))]
    let write = |path: std::path::PathBuf, bytes: &[u8]| std::fs::write(&path, bytes).map_err(|e| error(format!("{}: {e}", path.display())));
    #[cfg(target_arch = "wasm32")]
    let write = |path: std::path::PathBuf, bytes: &[u8]| {
        solidcraft_io::vfs::write(&path.to_string_lossy(), bytes).map_err(|e| error(format!("{}: {e}", path.display())))
    };
    #[cfg(not(target_arch = "wasm32"))]
    std::fs::create_dir_all(&to).map_err(|e| error(format!("{}: {e}", to.display())))?;
    write(to.join(&runtime.module), &package.component)?;
    for (name, text) in &package.icons {
        write(to.join(name), text.as_bytes())?;
    }
    if let (Some(web), Some(bytes)) = (runtime.web.as_ref(), package.web_module.as_ref()) {
        write(to.join(&web.module), bytes)?;
    }
    write(to.join("plugin.json"), &manifest)?;
    let installed = read_package(&to.to_string_lossy())?;
    if installed.approval != package.approval {
        return Err(error("the installed copy differs from the package; install it again"));
    }
    Ok(Installed { path: to, manifest: installed.manifest, approval: installed.approval })
}

#[cfg(test)]
mod tests;

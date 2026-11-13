//! Bounded inert package review, approved grants and native removal.
use super::*;
use crate::schema::plugin::{RemoveResult, ReviewResult};
use std::io::{Cursor, Read};

const MAX_FILES: usize = 1024;
const MAX_PACKAGE_BYTES: usize = 64 << 20;

fn invalid(reason: impl Into<String>) -> EngineError {
    HostError::InvalidInput { field: "package".into(), reason: reason.into() }.into()
}
fn io(e: impl std::fmt::Display) -> EngineError {
    invalid(e.to_string())
}
fn check(path: &Path) -> Result<()> {
    solidcraft_io::vfs::check(&path.to_string_lossy()).map_err(|e| HostError::PermissionDenied { permission: e.to_string() }.into())
}
fn relative(name: &str) -> Result<PathBuf> {
    if name.len() > 512 || name.contains(['\\', ':', '<', '>', '\"', '|', '?', '*', '~']) || name.chars().any(char::is_control) {
        return Err(invalid("package entry has an invalid path"));
    }
    for part in name.split('/') {
        let stem = part.split('.').next().unwrap_or_default().trim_end_matches(' ').to_ascii_lowercase();
        let device = matches!(stem.as_str(), "con" | "prn" | "aux" | "nul" | "conin$" | "conout$")
            || ["com", "lpt"].iter().any(|prefix| {
                stem.strip_prefix(*prefix).is_some_and(|n| matches!(n, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"))
            });
        if part.ends_with(['.', ' ']) || device {
            return Err(invalid("package entry names a device or a platform path alias"));
        }
    }
    let path = Path::new(name);
    if name.is_empty() || path.components().count() > 16 || path.components().any(|c| !matches!(c, std::path::Component::Normal(_))) {
        return Err(invalid("package entry escapes its root or is too deeply nested"));
    }
    // Reject dot segments explicitly: Path::components normalizes interior `.` segments.
    if name.split('/').any(|s| s == "." || s == "..") {
        return Err(invalid("package entry escapes its root"));
    }
    Ok(path.to_path_buf())
}
fn add(files: &mut BTreeMap<PathBuf, Vec<u8>>, name: PathBuf, bytes: Vec<u8>, total: &mut usize) -> Result<()> {
    *total = total.checked_add(bytes.len()).ok_or_else(|| invalid("package is too large"))?;
    if files.len() >= MAX_FILES || *total > MAX_PACKAGE_BYTES || files.insert(name, bytes).is_some() {
        return Err(invalid("package has too many files, duplicate entries, or exceeds 64 MiB"));
    }
    Ok(())
}
/// Validated inert package bytes for an installer to publish without reading the source again.
pub struct PackageContents {
    pub files: BTreeMap<PathBuf, Vec<u8>>,
    pub manifest: Manifest,
    pub approval: String,
}
/// Read a bounded folder or zip once. Every file key is a validated relative package path.
pub fn read_package_contents(path: &str) -> Result<PackageContents> {
    let root = Path::new(path);
    check(root)?;
    let meta = std::fs::symlink_metadata(root).map_err(io)?;
    if meta.file_type().is_symlink() {
        return Err(invalid("package symlinks are refused"));
    }
    let mut files = BTreeMap::new();
    let mut total = 0;
    if meta.is_dir() {
        let mut pending = vec![PathBuf::new()];
        let mut entries = 0;
        while let Some(dir) = pending.pop() {
            let full = root.join(&dir);
            check(&full)?;
            for entry in std::fs::read_dir(&full).map_err(io)? {
                entries += 1;
                if entries > MAX_FILES {
                    return Err(invalid("package has too many entries"));
                }
                let entry = entry.map_err(io)?;
                let leaf = entry.file_name();
                let leaf = leaf.to_str().ok_or_else(|| invalid("package entry names must be UTF-8"))?;
                let name = dir.join(relative(leaf)?);
                let components = name.components().count();
                let path_bytes = name.components().map(|c| c.as_os_str().as_encoded_bytes().len()).sum::<usize>() + components.saturating_sub(1);
                if components > 16 || path_bytes > 512 {
                    return Err(invalid("package entry is too deeply nested or its path exceeds 512 bytes"));
                }
                let full = root.join(&name);
                check(&full)?;
                let meta = std::fs::symlink_metadata(&full).map_err(io)?;
                if meta.file_type().is_symlink() || (!meta.is_dir() && !meta.is_file()) {
                    return Err(invalid("package symlinks and special files are refused"));
                }
                if meta.is_dir() {
                    pending.push(name);
                } else {
                    let bytes = solidcraft_io::vfs::read(&full.to_string_lossy(), MAX_PACKAGE_BYTES.saturating_sub(total)).map_err(io)?;
                    add(&mut files, name, bytes, &mut total)?;
                }
            }
        }
    } else if meta.is_file() {
        if !root.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip")) {
            return Err(invalid("choose a .zip package or a folder"));
        }
        let bytes = solidcraft_io::vfs::read(path, MAX_PACKAGE_BYTES).map_err(io)?;
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(io)?;
        if archive.len() > MAX_FILES {
            return Err(invalid("package has too many entries"));
        }
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i).map_err(io)?;
            let name = relative(entry.name().map_err(io)?.trim_end_matches('/'))?;
            if let Some(mode) = entry.unix_mode() {
                let kind = mode & 0o170000;
                if kind != 0 && kind != 0o100000 && kind != 0o040000 {
                    return Err(invalid("package symlinks and special files are refused"));
                }
            }
            if entry.is_dir() {
                continue;
            }
            let remaining = MAX_PACKAGE_BYTES.saturating_sub(total);
            if entry.size() > remaining as u64 {
                return Err(invalid("package exceeds 64 MiB"));
            }
            let mut bytes = Vec::new();
            entry.by_ref().take(remaining as u64 + 1).read_to_end(&mut bytes).map_err(io)?;
            add(&mut files, name, bytes, &mut total)?;
        }
        // Accept either plugin.json at the zip root or one enclosing package folder.
        if !files.contains_key(Path::new("plugin.json")) {
            let first = files.keys().next().and_then(|p| p.components().next()).ok_or_else(|| invalid("empty package"))?.as_os_str().to_owned();
            let prefix = PathBuf::from(first);
            if !files.keys().all(|p| p.starts_with(&prefix)) {
                return Err(invalid("package needs plugin.json at its root"));
            }
            files = files.into_iter().map(|(p, b)| p.strip_prefix(&prefix).map(|r| (r.to_path_buf(), b)).map_err(io)).collect::<Result<_>>()?;
        }
    } else {
        return Err(invalid("package is not a regular file or folder"));
    }
    // Zip files need not declare their parent directories; those still count as package entries.
    let mut nodes = BTreeSet::new();
    let mut portable_names = BTreeSet::new();
    for path in files.keys() {
        if path.as_os_str().is_empty() {
            return Err(invalid("package contains an empty relative file path"));
        }
        if !portable_names.insert(path.to_string_lossy().to_lowercase()) {
            return Err(invalid("package entries conflict on a case-insensitive filesystem"));
        }
    }
    for path in files.keys() {
        for ancestor in path.ancestors().filter(|p| !p.as_os_str().is_empty()) {
            if ancestor != path.as_path() && portable_names.contains(&ancestor.to_string_lossy().to_lowercase()) {
                return Err(invalid("package file conflicts with an ancestor directory"));
            }
            nodes.insert(ancestor.to_path_buf());
            if nodes.len() > MAX_FILES {
                return Err(invalid("package has too many entries, including implicit directories"));
            }
        }
    }
    let bytes = files.get(Path::new("plugin.json")).ok_or_else(|| invalid("package needs plugin.json"))?;
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err(invalid("manifest exceeds 16 KiB"));
    }
    let value: Value = serde_json::from_slice(bytes).map_err(io)?;
    let id = value.get("id").and_then(Value::as_str).ok_or_else(|| invalid("manifest needs an id"))?;
    let manifest = parse_manifest(bytes, id)?;
    if let Some(rt) = &manifest.runtime {
        let module = files.get(Path::new(&rt.module)).ok_or_else(|| invalid("package is missing its component"))?;
        if module.len() > MAX_COMPONENT_BYTES {
            return Err(invalid("component exceeds 16 MiB"));
        }
    }
    let mut hash = Sha256::new();
    hash.update(b"solidcraft-plugin-package-v1\0");
    for (name, bytes) in &files {
        let name = name.to_string_lossy();
        hash.update((name.len() as u64).to_le_bytes());
        hash.update(name.as_bytes());
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    Ok(PackageContents { files, manifest, approval: format!("{:x}", hash.finalize()) })
}
fn requested(manifest: &Manifest) -> Grants {
    manifest.runtime.as_ref().map_or_else(Grants::default, |rt| Grants {
        command_groups: rt.permissions.command_groups.clone(),
        folders: rt
            .permissions
            .files
            .iter()
            .map(|g| FileGrant { name: g.name.clone(), folder: PathBuf::new(), read: g.read, write: g.write })
            .collect(),
    })
}
pub fn review(path: &str) -> Result<ReviewResult> {
    let source = read_package_contents(path)?;
    Ok(ReviewResult { requested: requested(&source.manifest), manifest: source.manifest, approval: source.approval, param_warnings: vec![] })
}
pub fn validate_grants(manifest: &Manifest, grants: &Grants) -> Result<()> {
    let asked = requested(manifest);
    let deny = || HostError::PermissionDenied { permission: "approval contains an unrequested, duplicate, invalid or excessive grant".into() };
    if grants.command_groups.len() > 4 || grants.folders.len() > 32 || !grants.command_groups.is_subset(&asked.command_groups) {
        return Err(deny().into());
    }
    let mut names = BTreeSet::new();
    for grant in &grants.folders {
        if !names.insert(&grant.name)
            || !grant.folder.is_absolute()
            || grant.folder.to_string_lossy().len() > 4096
            || grant.folder.to_string_lossy().chars().any(char::is_control)
            || (!grant.read && !grant.write)
            || !asked.folders.iter().any(|r| r.name == grant.name && (!grant.read || r.read) && (!grant.write || r.write))
        {
            return Err(deny().into());
        }
        check(&grant.folder)?;
        if !std::fs::metadata(&grant.folder).map_err(io)?.is_dir() {
            return Err(deny().into());
        }
    }
    Ok(())
}
fn root(s: &Session) -> Result<PathBuf> {
    s.plugin_state.install_root.clone().or_else(plugins_dir).ok_or_else(|| invalid("plugin directory is unavailable"))
}
/// Store grants after an installer has successfully published the reviewed package.
/// This does not install files or grant permission to execute plugin code.
pub fn store_grants(s: &mut Session, manifest: &Manifest, mut grants: Grants) -> Result<()> {
    if !valid_id(&manifest.id) {
        return Err(invalid("invalid plugin id"));
    }
    validate_grants(manifest, &grants)?;
    if !s.plugin_state.grants.contains_key(&manifest.id) && s.plugin_state.grants.len() >= 256 {
        return Err(invalid("at most 256 stored plugin grants"));
    }
    grants.folders.sort_by(|a, b| a.name.cmp(&b.name));
    s.plugin_state.grants.insert(manifest.id.clone(), grants);
    Ok(())
}

/// Deleting a whole folder must not bypass a caller's hidden-path restrictions.
fn check_remove_tree(root: &Path) -> Result<()> {
    if solidcraft_io::vfs::current_scope().is_none() {
        return Ok(());
    }
    let mut pending = vec![root.to_path_buf()];
    let mut entries = 0;
    while let Some(dir) = pending.pop() {
        check(&dir)?;
        for entry in std::fs::read_dir(dir).map_err(io)? {
            entries += 1;
            if entries > MAX_FILES {
                return Err(invalid("installed package has too many entries"));
            }
            let path = entry.map_err(io)?.path();
            check(&path)?;
            if std::fs::symlink_metadata(&path).map_err(io)?.is_dir() {
                pending.push(path);
            }
        }
    }
    Ok(())
}

pub fn remove(s: &mut Session, id: &str) -> Result<RemoveResult> {
    if !valid_id(id) {
        return Err(invalid("invalid plugin id"));
    }
    let path = root(s)?.join(id);
    check(&path)?;
    let meta = std::fs::symlink_metadata(&path).map_err(io)?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(invalid("installed package must be a directory, not a symlink"));
    }
    check_remove_tree(&path)?;
    std::fs::remove_dir_all(path).map_err(io)?;
    s.plugin_state.grants.remove(id);
    s.plugin_state.packages.remove(id);
    s.disabled_plugins.remove(id);
    Ok(RemoveResult { id: id.into(), removed: true, param_warnings: vec![] })
}

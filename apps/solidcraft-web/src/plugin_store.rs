//! Installed plugins in the browser: kept in the origin private file system (OPFS) under
//! `plugins/<id>/`, apart from the designs, and seen by the engine as files under
//! `/opfs/plugins/<id>/` (`solidcraft_engine::plugin::WEB_PLUGINS_DIR`). They are read into the
//! engine's files at start; what `plugin.install` writes there is stored, and a package
//! `plugin.remove` deleted is deleted from storage too.
//!
//! The path logic is plain Rust and tested natively; the browser calls are wasm-only.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

/// Where installed plugins appear to the engine.
pub const PREFIX: &str = "/opfs/plugins/";

/// A plugin file's place in storage: (package id, file name), for an engine path under
/// [`PREFIX`] naming one plain file in one package folder.
pub fn place(path: &str) -> Option<(&str, &str)> {
    let (id, file) = path.strip_prefix(PREFIX)?.split_once('/')?;
    let plain = |s: &str| !s.is_empty() && !s.contains('/') && !s.starts_with('.') && s != "..";
    (plain(id) && plain(file)).then_some((id, file))
}

/// The package folders that had files and have none now: their storage is deleted.
pub fn emptied<'a>(before: impl IntoIterator<Item = &'a str>, now: &[(String, u64)]) -> Vec<String> {
    let mut gone: Vec<String> = before
        .into_iter()
        .filter_map(|p| place(p).map(|(id, _)| id.to_string()))
        .filter(|id| !now.iter().any(|(p, _)| place(p).is_some_and(|(other, _)| other == id)))
        .collect();
    gone.sort();
    gone.dedup();
    gone
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use std::cell::RefCell;
    use std::collections::BTreeSet;

    use js_sys::{Function, Reflect};
    use solidcraft_engine::io::vfs;
    use wasm_bindgen::{JsCast as _, JsValue};
    use wasm_bindgen_futures::{JsFuture, spawn_local};
    use web_sys::{
        FileSystemDirectoryHandle, FileSystemFileHandle, FileSystemGetDirectoryOptions, FileSystemGetFileOptions, FileSystemWritableFileStream,
    };

    use super::{PREFIX, emptied, place};
    use crate::files::err;

    thread_local! {
        /// The plugin files in storage, as engine paths.
        static KNOWN: RefCell<BTreeSet<String>> = const { RefCell::new(BTreeSet::new()) };
    }

    async fn folder(parent: &FileSystemDirectoryHandle, name: &str, create: bool) -> Result<FileSystemDirectoryHandle, String> {
        let opts = FileSystemGetDirectoryOptions::new();
        opts.set_create(create);
        JsFuture::from(parent.get_directory_handle_with_options(name, &opts)).await.map_err(err)?.dyn_into().map_err(|_| "not a folder".to_string())
    }

    /// The entries of a folder, as (name, handle).
    async fn entries(dir: &FileSystemDirectoryHandle) -> Result<Vec<(String, JsValue)>, String> {
        let iter: JsValue = dir.entries().into();
        let next: Function = Reflect::get(&iter, &"next".into()).map_err(err)?.dyn_into().map_err(|_| "no iterator".to_string())?;
        let mut out = Vec::new();
        for _ in 0..1000 {
            let step = JsFuture::from(js_sys::Promise::resolve(&next.call0(&iter).map_err(err)?)).await.map_err(err)?;
            if Reflect::get(&step, &"done".into()).ok().and_then(|d| d.as_bool()).unwrap_or(true) {
                break;
            }
            let pair: js_sys::Array = Reflect::get(&step, &"value".into()).map_err(err)?.unchecked_into();
            if let Some(name) = pair.get(0).as_string() {
                out.push((name, pair.get(1)));
            }
        }
        Ok(out)
    }

    /// Read every stored plugin file into the engine's files; how many there were.
    pub async fn load() -> Result<usize, String> {
        let root = crate::files::opfs_root().await?;
        let Ok(plugins) = folder(&root, "plugins", false).await else { return Ok(0) };
        let mut count = 0;
        for (id, handle) in entries(&plugins).await? {
            let Ok(dir) = handle.dyn_into::<FileSystemDirectoryHandle>() else { continue };
            for (name, handle) in entries(&dir).await? {
                let Ok(file) = handle.dyn_into::<FileSystemFileHandle>() else { continue };
                let path = format!("{PREFIX}{id}/{name}");
                if place(&path).is_none() {
                    continue;
                }
                match crate::files::read_handle(&file).await {
                    Ok(bytes) => {
                        vfs::insert(&path, bytes);
                        KNOWN.with(|k| k.borrow_mut().insert(path));
                        count += 1;
                    }
                    Err(e) => log::warn!("{path}: {e}"),
                }
            }
        }
        Ok(count)
    }

    async fn write(id: String, name: String, bytes: Vec<u8>) -> Result<(), String> {
        let root = crate::files::opfs_root().await?;
        let dir = folder(&folder(&root, "plugins", true).await?, &id, true).await?;
        let opts = FileSystemGetFileOptions::new();
        opts.set_create(true);
        let h: FileSystemFileHandle =
            JsFuture::from(dir.get_file_handle_with_options(&name, &opts)).await.map_err(err)?.dyn_into().map_err(|_| "not a file".to_string())?;
        let w: FileSystemWritableFileStream =
            JsFuture::from(h.create_writable()).await.map_err(err)?.dyn_into().map_err(|_| "no writable stream".to_string())?;
        JsFuture::from(w.write_with_u8_array(&bytes).map_err(err)?).await.map_err(err)?;
        JsFuture::from(w.close()).await.map_err(err)?;
        Ok(())
    }

    async fn remove(id: String) -> Result<(), String> {
        let root = crate::files::opfs_root().await?;
        let plugins = folder(&root, "plugins", false).await?;
        let opts = web_sys::FileSystemRemoveOptions::new();
        opts.set_recursive(true);
        JsFuture::from(plugins.remove_entry_with_options(&id, &opts)).await.map_err(err)?;
        Ok(())
    }

    /// Store a plugin file the engine wrote. False: not a plugin file.
    pub fn store(path: &str, bytes: Vec<u8>, say: impl Fn(String) + 'static) -> bool {
        let Some((id, name)) = place(path) else { return false };
        KNOWN.with(|k| k.borrow_mut().insert(path.to_string()));
        let (id, name) = (id.to_string(), name.to_string());
        spawn_local(async move {
            if let Err(e) = write(id, name.clone(), bytes).await {
                say(format!("plugin {name}: {e}"));
            }
        });
        true
    }

    /// Delete from storage the packages the engine no longer has (`plugin.remove`).
    pub fn forget_removed(say: impl Fn(String) + Clone + 'static) {
        let now = vfs::list(PREFIX);
        let gone = KNOWN.with(|k| emptied(k.borrow().iter().map(String::as_str), &now));
        if gone.is_empty() {
            return;
        }
        KNOWN.with(|k| k.borrow_mut().retain(|p| place(p).is_some_and(|(id, _)| !gone.iter().any(|g| g == id))));
        for id in gone {
            let say = say.clone();
            spawn_local(async move {
                if let Err(e) = remove(id.clone()).await {
                    say(format!("plugin {id}: {e}"));
                }
            });
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use browser::{forget_removed, load, store};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_files_in_one_package_folder_are_plugin_files() {
        assert_eq!(place("/opfs/plugins/dutchman-convert/plugin.json"), Some(("dutchman-convert", "plugin.json")));
        for path in
            ["/opfs/plugins/x", "/opfs/plugins/x/a/b", "/opfs/plugins/../x/y", "/opfs/plugins/x/.hidden", "/opfs/design.sc", "/opfs/plugins//y"]
        {
            assert_eq!(place(path), None, "{path}");
        }
    }

    #[test]
    fn a_package_whose_files_are_all_gone_is_deleted_from_storage() {
        let before = ["/opfs/plugins/a/plugin.json", "/opfs/plugins/a/plugin.wasm", "/opfs/plugins/b/plugin.json"];
        let now = vec![("/opfs/plugins/b/plugin.json".to_string(), 10)];
        assert_eq!(emptied(before, &now), vec!["a".to_string()]);
        assert!(emptied(before, &[("/opfs/plugins/a/plugin.json".into(), 1), ("/opfs/plugins/b/plugin.json".into(), 1)]).is_empty());
    }
}

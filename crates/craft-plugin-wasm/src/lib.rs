//! An app-independent component sandbox. Only explicitly bound Craft imports exist.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use craft_plugin::{Manifest, PluginHost, Result};
use serde_json::Value;
use std::io::{Read, Write};

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
mod native;

/// Invocation-local input. Opening the input grants no access to its parent folder.
pub struct InputStream {
    pub name: String,
    pub reader: Box<dyn Read + Send>,
}

/// The host owns storage and publication; exporters write to a staging sink.
#[derive(Default)]
pub struct IoStreams {
    pub input: Option<InputStream>,
    pub output: Option<Box<dyn Write + Send>>,
}

/// Trusted app configuration, separate from the untrusted package's budgets.
#[derive(Clone, Copy, Debug)]
pub struct RuntimeOptions {
    pub stack_bytes: usize,
}
impl Default for RuntimeOptions {
    fn default() -> Self {
        Self { stack_bytes: 512 * 1024 }
    }
}

/// Runs with an owned host because Wasmtime store data must be `'static`.
/// Returns the host on success and failure, after committing or rolling back.
pub fn run<H: PluginHost + 'static>(host: H, manifest: &Manifest, component: &[u8], entry: &str, params: &Value) -> (H, Result<Value>) {
    run_with_options(host, manifest, component, entry, params, RuntimeOptions::default())
}

pub fn run_with_options<H: PluginHost + 'static>(
    host: H,
    manifest: &Manifest,
    component: &[u8],
    entry: &str,
    params: &Value,
    options: RuntimeOptions,
) -> (H, Result<Value>) {
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    {
        native::run(host, manifest, component, entry, params, options, None)
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        let _ = (manifest, component, entry, params, options);
        (host, Err(craft_plugin::HostError::Trap { reason: "component execution requires an x86-64 or AArch64 native build".into() }))
    }
}

/// Runs a format entry with bounded streams under the ordinary sandbox transaction.
pub fn run_io<H: PluginHost + 'static>(
    host: H,
    manifest: &Manifest,
    component: &[u8],
    entry: &str,
    params: &Value,
    streams: IoStreams,
) -> (H, Result<Value>) {
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    {
        native::run(host, manifest, component, entry, params, RuntimeOptions::default(), Some(streams))
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        let _ = (manifest, component, entry, params, streams);
        (host, Err(craft_plugin::HostError::Trap { reason: "component execution requires an x86-64 or AArch64 native build".into() }))
    }
}

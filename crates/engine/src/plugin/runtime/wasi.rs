//! In-memory WASI compatibility for Rust importers. No operating-system WASI host is used.
//! Only these interfaces may link; unknown interfaces (including sockets and HTTP) stay
//! undefined. Files cannot be opened, streams are empty/discarded, clocks count calls and
//! random bytes are fixed. Unsupported operations trap rather than reaching the machine.
use super::ImportState;
use wasmtime::component::{Component, Linker, Resource, ResourceAny, ResourceType, Val, types::ComponentItem};
use wasmtime::{Engine, StoreContextMut};

const INTERFACES: &[&str] = &[
    "wasi:cli/environment",
    "wasi:cli/exit",
    "wasi:cli/stderr",
    "wasi:cli/stdin",
    "wasi:cli/stdout",
    "wasi:cli/terminal-input",
    "wasi:cli/terminal-output",
    "wasi:cli/terminal-stderr",
    "wasi:cli/terminal-stdin",
    "wasi:cli/terminal-stdout",
    "wasi:clocks/monotonic-clock",
    "wasi:clocks/wall-clock",
    "wasi:filesystem/preopens",
    "wasi:filesystem/types",
    "wasi:io/error",
    "wasi:io/poll",
    "wasi:io/streams",
    "wasi:random/insecure",
    "wasi:random/insecure-seed",
    "wasi:random/random",
];

// Distinct host types keep unrelated guest resources distinct. They have no OS handles.
macro_rules! resources {
    ($($ty:ident => $name:literal),* $(,)?) => {
        $(struct $ty;)*
        fn resource_type(name: &str) -> wasmtime::Result<ResourceType> {
            match name {
                $($name => Ok(ResourceType::host::<$ty>()),)*
                _ => Err(wasmtime::Error::msg("unsupported WASI resource")),
            }
        }
        fn resource(name: &str, store: StoreContextMut<'_, ImportState>) -> wasmtime::Result<Val> {
            match name {
                $($name => Ok(Val::Resource(ResourceAny::try_from_resource(Resource::<$ty>::new_own(0), store)?)),)*
                _ => Err(wasmtime::Error::msg("unsupported WASI resource")),
            }
        }
    };
}
resources! {
    IoError => "error", Pollable => "pollable", Input => "input-stream", Output => "output-stream",
    Descriptor => "descriptor", Directory => "directory-entry-stream",
    TerminalInput => "terminal-input", TerminalOutput => "terminal-output",
}

pub(super) fn add_to_linker(linker: &mut Linker<ImportState>, engine: &Engine, component: &Component) -> wasmtime::Result<()> {
    for (name, import) in component.component_type().imports(engine) {
        let Some((interface, version)) = name.split_once('@') else { continue };
        // These are the stable preview-2 revisions supported by the former WASI host.
        let revision = version.strip_prefix("0.2.").and_then(|v| v.parse::<u16>().ok());
        if !revision.is_some_and(|v| v <= 12) || !INTERFACES.contains(&interface) {
            continue;
        }
        let ComponentItem::ComponentInstance(instance) = import.ty else { continue };
        let mut host = linker.instance(name)?;
        for (function, export) in instance.exports(engine) {
            match export.ty {
                ComponentItem::Resource(_) => host.resource(function, resource_type(function)?, |_, _| Ok(()))?,
                ComponentItem::ComponentFunc(_) => {
                    let interface = interface.to_string();
                    let function_name = function.to_string();
                    host.func_new(function, move |store, _, args, results| {
                        let value = call(store, &interface, &function_name, args)?;
                        match (results, value) {
                            ([], None) => Ok(()),
                            ([out], Some(value)) => {
                                *out = value;
                                Ok(())
                            }
                            _ => Err(wasmtime::Error::msg("unsupported WASI function signature")),
                        }
                    })?;
                }
                ComponentItem::Type(_) => {}
                _ => return Err(wasmtime::Error::msg("unsupported WASI import")),
            }
        }
    }
    Ok(())
}

fn call(mut store: StoreContextMut<'_, ImportState>, interface: &str, function: &str, args: &[Val]) -> wasmtime::Result<Option<Val>> {
    let ok = || Some(Val::Result(Ok(None)));
    let value = match (interface, function) {
        ("wasi:cli/environment", "get-environment" | "get-arguments") | ("wasi:filesystem/preopens", "get-directories") => Val::List(vec![]),
        ("wasi:cli/environment", "initial-cwd")
        | ("wasi:cli/terminal-stdin", "get-terminal-stdin")
        | ("wasi:cli/terminal-stdout", "get-terminal-stdout")
        | ("wasi:cli/terminal-stderr", "get-terminal-stderr") => Val::Option(None),
        ("wasi:cli/stdin", "get-stdin") => resource("input-stream", store)?,
        ("wasi:cli/stdout", "get-stdout") | ("wasi:cli/stderr", "get-stderr") => resource("output-stream", store)?,
        ("wasi:clocks/wall-clock", "now" | "resolution") => Val::Record(vec![("seconds".into(), Val::U64(0)), ("nanoseconds".into(), Val::U32(0))]),
        ("wasi:clocks/monotonic-clock", "now") => {
            store.data_mut().ticks = store.data().ticks.saturating_add(1);
            Val::U64(store.data().ticks)
        }
        ("wasi:clocks/monotonic-clock", "resolution") => Val::U64(1),
        ("wasi:clocks/monotonic-clock", "subscribe-instant" | "subscribe-duration")
        | ("wasi:io/streams", "[method]input-stream.subscribe" | "[method]output-stream.subscribe") => resource("pollable", store)?,
        ("wasi:random/random", "get-random-u64") | ("wasi:random/insecure", "get-insecure-random-u64") => Val::U64(0),
        ("wasi:random/insecure-seed", "insecure-seed") => Val::Tuple(vec![Val::U64(0), Val::U64(0)]),
        ("wasi:random/random", "get-random-bytes") | ("wasi:random/insecure", "get-insecure-random-bytes") => {
            let Some(Val::U64(len)) = args.first() else { return Err(wasmtime::Error::msg("invalid random length")) };
            if *len > super::MAX_JSON_BYTES as u64 {
                return Err(wasmtime::Error::msg("random byte limit exceeded"));
            }
            Val::List(vec![Val::U8(0); *len as usize])
        }
        ("wasi:io/poll", "[method]pollable.ready") => Val::Bool(true),
        ("wasi:io/poll", "[method]pollable.block") => return Ok(None),
        ("wasi:io/poll", "poll") => {
            let Some(Val::List(items)) = args.first() else { return Err(wasmtime::Error::msg("invalid poll list")) };
            Val::List((0..items.len()).map(|i| Val::U32(i as u32)).collect())
        }
        ("wasi:io/streams", "[method]output-stream.check-write") => Val::Result(Ok(Some(Box::new(Val::U64(1024 * 1024))))),
        (
            "wasi:io/streams",
            "[method]output-stream.write"
            | "[method]output-stream.flush"
            | "[method]output-stream.blocking-flush"
            | "[method]output-stream.blocking-write-and-flush"
            | "[method]output-stream.write-zeroes"
            | "[method]output-stream.blocking-write-zeroes-and-flush",
        ) => return Ok(ok()),
        (
            "wasi:io/streams",
            "[method]input-stream.read" | "[method]input-stream.blocking-read" | "[method]input-stream.skip" | "[method]input-stream.blocking-skip",
        ) => Val::Result(Err(Some(Box::new(Val::Variant("closed".into(), None))))),
        ("wasi:filesystem/types", "filesystem-error-code") => Val::Option(None),
        // No descriptor can be obtained: there are no preopens. These imports are present
        // for standard-library compatibility but cannot grant access to any file.
        _ => return Err(wasmtime::Error::msg(format!("WASI operation is unavailable: {interface}#{function}"))),
    };
    Ok(Some(value))
}

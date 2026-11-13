# Write a SolidCraft plugin

A command plugin is a WebAssembly component plus `plugin.json` in a folder named
after its id. Plugins use the same engine commands and schemas as MCP. The host
checks declared permissions, limits resource use and rolls back failed edits.

Start with the [plugin author skill](../skills/solidcraft-plugins/SKILL.md) and its
[Rust and TypeScript quickstarts](../skills/solidcraft-plugins/references/quickstarts.md).
The [generated API reference](plugin-api/README.md) contains the authoritative WIT
and command schemas. Keep the plugin's source and release package in a separate
repository. Rust uses `wit-bindgen`; TypeScript uses `jco types` and
`jco componentize` with its JavaScript engine inside the component.

Declare each entry's input schema and read/edit effect in the manifest. List only
the exact engine commands it needs. Folder access requires named host grants;
network requests and network imports are refused. Plugins supply no UI code.
See [the runtime reference](plugin-runtime.md) for the manifest, permitted command
families and execution limits.

**Build for `wasm32-unknown-unknown`, not `wasm32-wasip2`.** The host refuses WASI
imports, and the `wasip2` target's reactor shim links the whole WASI CLI suite
(`wasi:io/poll`, `wasi:cli/*`) even from a `#![no_std]` crate. On
`wasm32-unknown-unknown` the build is a core module with component metadata; lift it
with `wasm-tools component new` (or `wit_component::ComponentEncoder`) and the result
imports only the plugin world. A `no_std` crate with `alloc` and an alloc-only serde
keeps it that way; a small command plugin is around 150 KiB lifted.

**Failures need no wrapping.** A failed `run` reaches you as the host's own typed
`command_failed` error naming the command, and the edit is rolled back whatever the
plugin returns after it. An entry the manifest doesn't declare is refused before any
plugin code runs, with no transaction opened. List exactly the commands the plugin
calls: an undeclared command then fails loudly instead of widening what it may do.

Validate the package without running it, then run it explicitly on a fixture:

```sh
solidcraft-cli plugin check /path/to/my-plugin
solidcraft-cli plugin test /path/to/my-plugin --design fixture.sc --entry make --input '{}'
```

Omit `--entry` to test every entry, or `--design` to start with an empty design.
Each entry starts from the same initial design. Failed calls save the rolled-back
design; journals record attempted host commands, including commands later rolled back.
Both commands print JSON. Check diagnostics identify manifest fields or imports.
Test reports include per-entry pass/fail, typed errors and saved design and
command journal paths. Exit zero means every selected entry passed; inspect the
artifacts to assert the intended result. The test command executes plugin code.
Normal desktop execution retains its separate package approval flow.

The reference is generated with `cargo xtask plugin-api`; maintainers use
`cargo xtask plugin-api --check` to detect stale output.

---
name: solidcraft-plugins
description: "Use when authoring, packaging, validating or testing a SolidCraft WebAssembly command plugin in Rust or TypeScript."
---

# Author a command plugin

Read the [generated API reference](../../docs/plugin-api/README.md), its
[WIT](../../docs/plugin-api/plugin.wit) and [command schemas](../../docs/plugin-api/commands.json).
These come from the shared WIT and engine schemas MCP serves.
Use exact command ids and schemas; the catalog is broader than
the permitted plugin commands. See [host policy](../../docs/plugin-runtime.md#command-and-data-surface).
Regenerate with `cargo xtask plugin-api`; `cargo xtask plugin-api --check` detects drift.
Never maintain a second API catalog by hand.

## Package and permissions

Keep plugin source and examples in a separate repository. Ship a folder whose
name equals the manifest `id`, containing `plugin.json` and the component named by
`runtime.module`. Ship a WebAssembly component, not a core Wasm module or a script.
The [quickstarts](references/quickstarts.md) include a manifest.

Declare the exact WIT package identifier in `runtime.api`, exact built-in command ids in
`runtime.commands`, and 1..32 named entries in `runtime.entries`. Declare each entry's
`effect` (`read` or `edit`), object `input_schema`, and optional menu/ribbon `slots`.
Selection and target fields are arrays. The app owns slots and draws forms; plugins ship
no UI. A read entry can call only read commands. Model edits use engine commands, so
outputs remain native features and failures roll back the whole invocation.

Permissions use `runtime.permissions`: `files` lists named read/write folder requests;
only matching host-approved grants allow access. `clock: true` enables wall time; without
it, time is fixed. `net: true` is refused. No WASI, socket, HTTP or fetch imports are linked.
Log, progress and cancellation are available through WIT. Command requests are intersected
with host policy. Component bytes, JSON, memory, fuel and execution time have limits;
see [runtime limits](../../docs/plugin-runtime.md#budgets-and-limits).

## Rust quickstart

Use `wit-bindgen` with world `command-plugin` from the published WIT. Implement the generated
command `Guest` trait, call `export!`, compile a `cdylib` for `wasm32-unknown-unknown`, then
wrap it with `wasm-tools component new`. Follow the complete
[Rust steps](references/quickstarts.md#rust-wit-bindgen), including the manifest and check.

## TypeScript quickstart

Generate declarations with `jco types` from the same WIT. Compile TypeScript to JavaScript,
then use `jco componentize` with world `command-plugin` and ambient features disabled.
The JS engine lives inside the component; SolidCraft embeds no interpreter. Follow the
[TypeScript steps](references/quickstarts.md#typescript-jco) and inspect remaining imports.

## Check, run, assert

```sh
solidcraft-cli plugin check /path/to/my-plugin
solidcraft-cli plugin test /path/to/my-plugin
solidcraft-cli plugin test /path/to/my-plugin --design fixture.sc --entry echo --input '{}'
```

Check validates without executing. Read JSON diagnostics by manifest field or import name.
Test executes external code on the headless host. It reports each entry's pass/fail,
typed host error, and design/journal artifact paths. Exit zero means every selected entry
passed. Inspect artifacts for semantic assertions; a successful call alone does not prove
the intended geometry. Fix the named cause and rerun; preserve failing fixtures outside
this repository. Desktop package approval is separate from this explicit author test.

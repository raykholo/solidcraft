# Plugins

[RFC 0002: Plugins](rfcs/0002-plugins.md) defines the plugin API and phases.
The host has two app-agnostic crates:

- `crates/craft-plugin` owns the generic WIT contract, manifest, permissions,
  entry input schemas and typed host errors. It has no wasmtime dependency.
- `crates/craft-plugin-wasm` runs WebAssembly components against that contract
  with fuel, memory and stack limits. Its linker exposes no network imports.

An app implements `PluginHost`: it supplies command schemas, each command's
`read` or `edit` effect, command execution, transaction begin/commit/rollback,
and the folders that file grants may resolve to. A successful command entry is
one undo step; a failed call rolls back the transaction. Importers publish a new
design and exporters publish a file after success. App commands remain
JSON data rather than app-specific WIT types. Both crates are Shared crates;
`cargo xtask layers` forbids direct and indirect SolidCraft dependencies.

SolidCraft's adapter lives in `crates/engine` and uses the same command surface
as MCP. Entries appear as `plugin_id.function`; `plugin.list` and
`engine.describe` expose their declared inputs. Selection and target inputs
are lists. Entries may declare named menu or ribbon slots; each app decides
which slots exist and draws forms from the input schemas. Plugins ship no UI
code. SolidCraft retains approval, cancellation and plugin preferences.

File access requires approved folder grants. The reserved `net` permission is
refused with a typed error. Traps, missing permissions, failed commands, limits
and invalid manifest fields return structured errors. Installation and browser execution remain later phases.


## Importers and exporters

A runtime entry declares `kind: "importer"` or `kind: "exporter"` and the
lowercase `extensions` it reads or writes, without dots. Its `input_schema`
describes format options. Command entries keep `kind: "command"` by default.
Importers have edit effects; exporters have read effects and cannot modify the
design even when their manifest lists an edit command.

File > Import and File > Export show installed formats beside the built-in
formats, with their extensions. The host draws options and approval controls;
plugins supply no UI code. Conversion runs through the existing file worker,
with progress and cancellation. A cancelled or failed import leaves the open
design untouched. A successful import opens a new unsaved design and shows its
structured loss report. Importers emit ordinary app commands; there is no
plugin feature or plugin timeline row. The file-named component owns the imported
bodies and assembly contents. Contents are rigid until Unlock Contents, as for
built-in assembly imports.

Agents use `file.import` and `file.export` with a namespaced format id such as
`converter.read-part`. Both commands publish input and output schemas and
require the inspected content approval hash. Over MCP, use `execute` with the
engine command id and parameters; `describe_command` exposes its schemas.
Programmatic calls return the loss report as JSON and never open a file picker,
approval dialog or report window. An extension can identify a unique installed
importer; use its explicit id when more than one importer reads that extension.
An extension never grants approval to execute code.

Input and output use invocation-local streams. Each host call transfers at most
64 KiB, and each stream is limited to 64 MiB. An importer receives the input's
base file name and reads bytes through the stream interface. Reading assembly
siblings requires a separately approved named folder grant and the manifest's
matching read permission. Opening the main file does not grant its folder.
An exporter reads the design through read-effect commands and writes bytes to
the output stream. The host publishes the output only after success.

See [the runtime author guide](plugin-runtime.md#importer-and-exporter-entries)
for the manifest, stream protocol and loss report format. The WIT package name
has one source: the package declaration in
[`plugin.wit`](../crates/craft-plugin/wit/plugin.wit).

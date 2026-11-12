# Experimental plugin runtime

This is the implemented native runtime for commands, importers and exporters. It does not implement every part of
[issue #1221](https://github.com/tangency-labs/solidcraft/issues/1221), the broader
[interface proposal #4980](https://github.com/tangency-labs/solidcraft/pull/4980), or
[Utilities placement proposal #4933](https://github.com/tangency-labs/solidcraft/pull/4933).
The [current plugin RFC](rfcs/0002-plugins.md), especially its Decisions section, takes
precedence over these earlier proposals. The shared command world is defined in
[`craft-plugin`'s WIT](../crates/craft-plugin/wit/plugin.wit). The package
manifest uses an optional `runtime` block so old metadata-only packages still work. Its manifest
layout and explicit per-invocation approval flow are deliberately documented as experimental,
not as a claim that the outstanding design proposals have been accepted.

## Boundary and lifecycle

`craft-plugin` owns the manifest, typed errors, permissions and `PluginHost` trait.
`craft-plugin-wasm` runs components against that trait without app dependencies.
SolidCraft's engine implements the trait with its command schemas and transaction surface.
The runtime takes ownership of a host and returns `(host, result)` on success and failure.
This satisfies Wasmtime's requirement for store data without borrowed references. SolidCraft
supplies an owned scratch session and adopts its edits only after a successful call.

1. Put an independently built component beside a static `plugin.json` in its id folder.
2. `plugin.inspect` reads bounded bytes through the caller's existing file scope and returns
   the manifest and content approval hash. It neither instantiates the component nor invokes
   a manifest function. Metadata can therefore be reviewed before code runs.
3. The user reviews the package and explicitly calls `plugin.run` with that exact hash, an
   exported entry and JSON parameters. The package is read once for this call; the bytes whose
   hash was checked are the bytes compiled and executed. Changing the files afterward cannot
   substitute different code for that invocation.
4. The host creates a fresh Wasmtime engine/store, with fuel, epoch interruption and a resource
   limiter, and links only the published component interfaces. No WASI is linked. Reserved
   sockets, HTTP and fetch imports are refused even if unused. Instantiation and the entry
   share the same execution budget, including component start functions.
5. `engine.run` reaches `PluginHost::run_command`, which SolidCraft implements with
   `Session::execute` on a scratch session, under an empty file scope.
   Explicit paths and implicit reads during feature evaluation cannot inherit the caller's broader
   file access. Results are bounded JSON. Refused calls poison the transaction, even if guest code
   catches the error and returns success.
6. The result must be valid JSON, every newly changed feature must evaluate, and the call must
   remain within its budget. Only then are the native results accepted as one undo step.
7. The desktop control channel performs the whole computation on a worker, warms the resulting
   display and adopts the existing computed-commands transaction exactly once. A changed design,
   authoring context, selection, disabled-plugin policy, or Stop prevents adoption. It does not
   re-run the plugin on the window thread. Headless CLI execution is synchronous.

Discovery, preference toggles and opening a saved native design never execute a component.
File-format conversion requires explicit content approval. Every invocation gets a fresh
instance, so guest memory is not persistent state. A design is data, never
an instruction to install, approve, download or execute a package.

## Manifest

The metadata fields remain `id`, `name`, `version`, and optional `description`. IDs are 1..64
lowercase letters, digits and hyphens, without a leading/trailing hyphen, and must equal the folder
name. Names are at most 80 characters; versions at most 32; descriptions at most 240. Control
characters and unknown fields are refused. The manifest has a 16 KiB byte cap.

`runtime` is optional for metadata-only packages. Executable packages declare:

| Field | Meaning |
|---|---|
| `api` | Exactly the package identifier declared in the [WIT](../crates/craft-plugin/wit/plugin.wit) |
| `module` | One lowercase `.wasm` file name within the package; no directories |
| `entries` | 1..32 distinct entries with `name`, `kind`, `effect: read \| edit`, `input_schema` and optional `slots`; file formats require `extensions`; legacy string ids default to edit commands |
| `commands` | At most 128 exact existing built-in command ids, intersected with host policy |
| `limits` | Optional object with all of `fuel`, `memory_mib`, `deadline_ms`; omitted object uses defaults |
| `permissions` | Optional object: `files` requests named read/write folder grants, `clock` requests wall clock; `net: true` is refused |

Entry inputs are object schemas; selection and target fields are arrays. Named slots use
`{"kind":"menu","name":"…"}` or `{"kind":"ribbon","name":"…"}`; the app decides which
slots exist. Declaring a slot never supplies UI code. `plugin.list` and `engine.describe`
expose the entry declarations. A read entry can call only commands marked read in the app catalog.

`files.read-file` and `files.write-file` resolve paths only within matching app-approved folder
grants with the requested access. Declaring a folder name does not itself grant access. `clock.now`
returns a fixed epoch without the clock permission. Log, progress and cancellation checks are
always available. There are no socket, HTTP, fetch or WASI imports.

The component must be a WebAssembly **component**, not an ordinary core `.wasm` module, DLL,
executable or script. Unsupported imports fail instantiation; the host never falls back to starting
another process. A dependency is not installed when a component fails to load.

## Command and data surface

The host admits existing `solid.*` and `sketch.*` commands (except file-based `sketch.insert_*`),
plus `component.create/activate`, `occurrence.move`, `parameters.add/change/delete`,
`timeline.add/inspect/redefine/delete`, `document.inspect`,
`model.faces`, `model.edges`, `model.mesh` and `inspect.measure`. Admission still requires the exact
id in the package manifest. `component.activate` is an edit even though it does
not add an undo step by itself. New families require a host policy review. Wildcards, command aliases,
plugin-to-plugin calls, batches/scripts and `file.*`/`doc.*` commands are not guest capabilities.
The empty file scope also applies to a feature that names an external font or linked design.

`model.mesh {body, tolerance?}` is a general read-only input boundary. Tolerance is a finite positive
number in millimetres (default 0.05). It returns `body`, its owning `component`,
`frame: "component-local-mm"`, `positions: [[x,y,z], ...]` and `triangles: [[i,j,k], ...]`. Indices are
zero-based. The mesh is of the evaluated body, rather than only the original import's vertices.
The command refuses more than 8192 vertices or 16384 triangles. A plugin's host-call result must
also fit the JSON byte limit; a smaller mesh can still exceed that limit and be refused.
No reconstruction, fitting, sewing, mesh replacement or conversion algorithm is part of this API.

A plugin computes its specialised behaviour externally, then emits ordinary native commands or
validated native features through `timeline.add`. New algorithms do not acquire private kernel
entry points. This initial boundary does not promise arbitrary B-rep construction, a stable binary
kernel serialization, persistent custom feature types, or automatically replaying plugin code on
parameter changes. A later specialised plugin must prove its own geometry, failure, lineage,
attachment, undo and reopen contracts in its own change.

## Consent and identity

`plugin.inspect` returns SHA-256 of a domain tag, the manifest byte length, exact manifest bytes,
and exact component bytes. `plugin.run` compares it before instantiation. Whitespace changes to a
manifest also require reinspection. A matching digest is an identity check, not a signature;
reviewing permissions does not establish whether a publisher is trustworthy.

There is no persistent automatic approval list in this version. A caller supplies the reviewed
content identity on every invocation. Agent chat separately asks before running external code;
its model cannot turn an inspected hash into user consent. The same gate covers
`file.import`, plugin `file.export`, and `doc.open` when a plugin format handles
the file. Desktop batch and script requests refuse these external calls; send
them individually so they run on the cancellable worker. A host or MCP caller is responsible
for its existing user-consent policy and package-folder scope. Desktop disable preferences apply
to the running desktop session and all plugin worker copies; a separate CLI process has its own
policy, not the GUI's preferences.

## Budgets and limits

| Resource | Default / limit |
|---|---|
| Component input | 16 MiB |
| Guest fuel per invocation | 10,000,000; user host ceiling defaults to 100,000,000,000 |
| Guest memory | 64 MiB; user host ceiling defaults to 128 MiB; one memory total |
| Entry deadline | 5 s / 30 s |
| Core instances / tables | 8 / 2 (guest plus canonical ABI shim) |
| Table elements / guest stack | 10,000 / 512 KiB |
| Non-stream host calls | 256 |
| Stream calls / chunk / total input or output | 4096 / 64 KiB / 64 MiB |
| Params, one host result, final result | 1 MiB each, bounded JSON parser depth |
| Log messages | 1 KiB each, 64 KiB per call; control characters stripped |

Fuel counts metered Wasm operations rather than milliseconds. A substantial separately compiled
CAD routine can use billions of fuel units. The default invocation budgets and default host
ceilings above are unchanged. The initial 128 MiB host ceiling is a conservative policy choice,
not a measured minimum or a guarantee that every plugin fits.

### Change the host ceilings without rebuilding

The native GUI persists `plugin_max_fuel` and `plugin_max_memory_mib` in user preferences.
**Preferences > Plugins** presents exact decimal text fields (**Max plugin fuel** and
**Max plugin memory (MiB)**). Enter, Apply or OK validates and accepts both integers together;
an incomplete or invalid field cannot silently save a numeric prefix. Cancel restores the last
Apply/open snapshot. Restore defaults resets this page to the original host ceilings. The
`ui.prefs` control method uses the same strict validation. Agent chat cannot change preferences;
guests have no preference or host-policy command.

A standalone CLI or `mcp --headless` process can read the same two ceiling concepts from an
explicit startup file:

```json
{"max_fuel": 200000000000, "max_memory_mib": 512}
```

```text
solidcraft-cli --plugin-limits plugin-limits.json run inspect-plugin.json --results
solidcraft-cli --plugin-limits plugin-limits.json mcp --headless --allow-path /path/to/plugins
```

This UTF-8 JSON file is at most 16 KiB; a UTF-8 BOM is accepted for Windows editors. Its keys are
`max_fuel` and `max_memory_mib`; missing keys use the original host defaults, while unknown keys,
zero, fractional and out-of-range values are refused. The CLI reads it once before creating
local sessions. Omit the flag to use the original ceilings. `--connect` and non-headless `mcp`
refuse this flag because the connected desktop owns its preferences. GUI preferences and CLI
startup files are separate user settings; a package or saved design cannot supply either.

Both host and manifest accept positive integer fuel through 18446744073709551615 (`u64::MAX`),
without conversion through floating-point controls. Memory accepts 1..4096 MiB: the remaining
4096 MiB maximum is the technical Wasm32 address-space boundary (65536 pages of 64 KiB), not
the old policy ceiling. Memory64 remains disabled and only one linear memory is allowed. A
higher ceiling does not reserve memory or guarantee allocation; select a value your machine
can sustain. The 30-second maximum deadline and the other fixed limits in the table remain.

### Approve the package's actual request

`runtime.limits` still determines the invocation's fuel, memory and deadline. Omitting that
object still requests 10 million fuel, 64 MiB and 5000 ms, even under much larger host ceilings.
`plugin.inspect` accepts a structurally valid larger request without instantiating it. Its reply
contains the exact requested manifest limits, current `host_limits`, `within_host_limits`, and
an optional `limits_error`. Inspect and review the code, grants and budgets first. If needed,
explicitly raise the host ceilings, then pass the inspected content hash to `plugin.run`.
The content hash covers the declared budgets: editing them requires fresh inspection and approval.
Raising a host ceiling alone never approves content, increases a request, executes or retries it.
The host rechecks the current ceilings before compilation/execution. Lowering them can refuse a
previously approved package. Desktop results prepared under changed ceilings or disable settings
are discarded rather than adopted into the current design. Settings survive new/open design
operations and scratch copies, but are never serialized as design data.

Fuel and epoch checks contain guest execution, not arbitrary native work. Component compilation
happens before the guest deadline; its input size is bounded, but compiler memory/time are not
covered by guest fuel. Each approved native command runs under its existing engine/kernel limits;
the guest deadline prevents more commands rather than forcibly interrupting a native call.
The desktop worker keeps this work off the window thread. Stop drops adoption; an already running
worker finishes unseen. It does not synchronously kill a native compiler or kernel call.

Host failures are typed data: missing permission, failed command and reason, exhausted limit,
invalid manifest or input field and reason, trap, or cancellation. The WIT and Rust `HostError`
describe the same cases. A failure never retries the component, raises its limits or commits
a partial transaction.

There is no compiled-code cache, unsafe deserialization, WASI filesystem/clock/socket/process API,
network integration, persistent store, plugin installer, signed index, custom panels or icons.
The WIT has no persistent store interface. `manifest()` is not used.

## Author an original plugin

Plugin packages, examples and converters live outside this repository, as specified in
[the plugin RFC](rfcs/0002-plugins.md). This repository contains the host, WIT and synthetic test
fixtures. The host compiles no plugin algorithm into SolidCraft. Rust first builds a core module with WIT metadata; `wasm-tools component new`
wraps it as a component without a WASI adapter. Other languages may use the same published
[WIT](../crates/craft-plugin/wit/plugin.wit) if they produce compatible components without unsupported imports.

## Platforms and validation

Native execution on x86-64 or AArch64 requires the engine's `plugins` Cargo feature.
Other native architectures retain the commands but report that no execution host is available. The browser build publishes the
same command descriptions but refuses execution without a native host. Turning off this feature
keeps metadata, inspection and the general mesh query available while excluding Wasmtime.
No new design format or custom `FeatureKind::Plugin` is introduced.

The runtime tests use original generated components, not mocks of a successful plugin: host calls,
two edits grouped into one undo step, measured volume, undo/redo and cold reopen after removing
the package; guest traps, fuel/memory exhaustion, forbidden calls, stale approval, disabled plugins,
and caller path scopes. The desktop tests cover worker scheduling and rejection of stale results.
The PR records actual validation results; this guide is not a claim that a particular build passed.


## Importer and exporter entries

All entry kinds use the same `command.execute(entry, params)` export. App command
names and arguments remain JSON data; the WIT never defines CAD-specific types.
The new `io` import provides invocation-local file streams. A component that
only implements commands does not need to import `io`.

Declare each file format in the manifest's `runtime.entries` list:

```json
[
  {
    "name": "read-part",
    "kind": "importer",
    "extensions": ["part", "assembly"],
    "input_schema": {
      "type": "object",
      "properties": {"units": {"type": "string", "enum": ["mm", "inch"]}}
    }
  },
  {
    "name": "write-part",
    "kind": "exporter",
    "extensions": ["part"],
    "input_schema": {"type": "object", "properties": {}}
  }
]
```

Each format requires 1..32 distinct extensions of 1..16 lowercase ASCII letters
or digits. Dots, path separators and duplicates are refused. Importers default
to `effect: "edit"`; exporters default to `effect: "read"`. An explicit effect
that disagrees with its kind is refused. Existing string entries remain edit
commands, and command declarations must not include extensions.

Format options use a bounded offline JSON Schema subset: `type` (including a
list of types), `properties`, `required`, `additionalProperties`, `items`,
`enum`, `const`, `allOf`, `anyOf`, `oneOf`, local `$ref`/`$defs`, numeric minimum
and maximum bounds (including exclusive bounds), string length and array length
bounds. An options object is validated before any plugin code runs. Common
annotation fields (`title`, `description`, `default`, `examples` and schema
metadata) are accepted. Unsupported validation keywords and remote references
are refused in format manifests. Validation caps schema depth and total work;
recursive references cannot hang the host.
Integer options use signed or unsigned 64-bit JSON integers without floating-point
conversion, matching the native command field types.

An importer calls `io.input-name()` for the input's base file name, then
`io.read-input(max-bytes)` repeatedly. Request 1..65536 bytes per call. A short
read is valid; an empty chunk means EOF. The host rejects input larger than
64 MiB. It exposes no implicit filesystem access. To read an assembly sibling,
use `files.read-file` with a matching manifest permission and an app-approved
folder grant. Absolute paths and traversal outside that grant are refused by
the existing capability-based file host. Sibling reads retain the existing
1 MiB per-file bound.

The importer creates the design through approved `engine.run` commands and
returns a JSON loss report as the successful `execute` result:

```json
{
  "counts": {"bodies_read": 2, "materials_skipped": 1},
  "items": [
    {"item": "Body 2", "code": "material-unsupported", "message": "Material was omitted"}
  ]
}
```

`counts` has at most 128 printable keys of at most 64 characters and unsigned
integer values. `items` has at most 4096 entries. Each item identifies the source
object (1..256 printable characters), a stable lowercase code using the manifest
id syntax, and a message (1..1024 printable characters). Empty reports use
`{"counts":{},"items":[]}`. The final JSON still has the existing 1 MiB limit.
The host validates the report before committing; malformed reports roll back
all commands. Plugin messages are displayed as text, without markup execution.

An exporter queries the design through `engine.run` and calls
`io.write-output(bytes)` repeatedly with chunks no larger than 64 KiB. Its total
output must fit 64 MiB. It returns a small JSON result, for example `{}`.
Read-effect enforcement applies to every command, independent of manifest
permissions. Format entries cannot write through the filesystem interface,
even with a write grant. The app stages the output and writes the destination only after
a successful call. A failed or cancelled exporter never publishes partial bytes.

File-format ids do not bypass the caller's file scope. Scoped MCP/control callers
must allow the package folder (manifest and component) as well as the input or
output folder, as for `plugin.inspect` and `plugin.run`. Granting those caller
paths does not grant the guest access to sibling files: guest folder access
still requires a separate named `FileGrant` and manifest permission.

An explicit dotted export format with non-null `approval` or `options` marks a
plugin request. If that exporter is missing or disabled, the command refuses it
instead of falling back to a native writer with the same extension. Legacy
filename-shaped native `format` values remain supported without plugin fields.

The app-independent Rust runtime exposes `run_io` with an owned `IoStreams`.
`InputStream` contains `name` and an owned `Read + Send`; output is an owned
`Write + Send`. This lets hosts use files, memory or their own bounded streams.
Streams are not guest filesystem grants. The existing `run` API remains for
commands. Hosts must stage external writes until success, since a generic sink
cannot roll back bytes already written.

The same fuel, memory, stack, deadline, progress, cancellation and no-network
policy covers all kinds. A stream error poisons the call even when guest code
ignores its result. Cancellation is checked during streaming and before commit.
SolidCraft builds an import in a fresh session, then publishes it as a new unsaved
design only after the whole call succeeds. No saved plugin dependency is added.
The host draws the loss list and options from data for menu actions; a component
supplies no UI. Programmatic imports return the report in JSON without opening
a report window, picker or approval dialog.

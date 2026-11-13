# Plugins

[RFC 0002: Plugins](rfcs/0002-plugins.md) defines the plugin API and phases.
Phase 1 splits the host into two app-agnostic crates:

- `crates/craft-plugin` owns the generic WIT contract, manifest, permissions,
  entry input schemas and typed host errors. It has no wasmtime dependency.
- `crates/craft-plugin-wasm` runs WebAssembly components against that contract
  with fuel, memory and stack limits. Its linker exposes no network imports.

An app implements `PluginHost`: it supplies command schemas, each command's
`read` or `edit` effect, command execution, transaction begin/commit/rollback,
and the folders that file grants may resolve to. One successful plugin call is
one undo step; a failed guest call rolls back the transaction. Granted file writes
are staged and published only after the guest succeeds and the app commits. A rollback
deletes staged files without replacing their targets. A filesystem failure during publication
after app commit is reported and may leave some files changed. App commands remain
JSON data rather than app-specific WIT types. Both crates are Shared crates;
`cargo xtask layers` forbids direct and indirect SolidCraft dependencies.

SolidCraft's adapter lives in `crates/engine` and uses the same command surface
as MCP. Entries appear as `plugin_id.function`; `plugin.list` and
`engine.describe` expose their declared inputs. Selection and target inputs
are lists. Entries may declare named menu or ribbon slots; each app decides
which slots exist and draws forms from the input schemas. Plugins ship no UI
code. SolidCraft retains approval, cancellation and plugin preferences.

## Permission review, grants and removal

The desktop permission screen shows a package's name, version, description, capabilities,
command groups, named folders and entries with their declared slots. Each command group explains
what access it enables: files, opening or switching designs, application settings, or other
plugins. Enter an absolute local path for each requested folder and review its read/write access.
**Approve** returns the reviewed package identity and selected grants to the installer.
**Cancel** returns nothing. Reviewing or approving never installs or executes a package.

The installer owns file selection, zip publication and the `plugin.install` command; those
surfaces connect through the integration APIs below. This change supplies the permission screen
and review data, rather than a second installer.

Approved command groups and folders are stored separately for each plugin id in desktop
preferences after installation succeeds. A package copied into the plugin directory by hand
starts with no grants. A plugin cannot choose an unrelated folder during execution.

In **Preferences > Plugins**, each installed plugin has an enable switch and **Remove** action.
Remove asks for confirmation, then deletes the package and its stored grants immediately.
Cancelling Preferences does not restore a removed package. Removing a plugin preserves its data
in saved designs. The browser Plugins page remains desktop-only.

The command surface exposes review, listing and removal without opening dialogs:

| Command | Parameters | Result |
|---|---|---|
| `plugin.review` | `path`: zip or package folder | `{manifest, requested: {command_groups, folders}, approval}`; the manifest includes capabilities and entry slots |
| `plugin.list` | `{}` | Installed and inspected manifests, namespaced commands and disabled state |
| `plugin.remove` | `id`: validated plugin id | `{id, removed: true}` |

These commands have input and output schemas exposed by `engine.describe`. Review returns empty
`folder` paths; the host selects absolute local paths before approval. Folder grants contain
`name`, `folder`, `read` and `write`. Unrequested groups, folder names or access modes are refused
with typed errors; omitted grants grant no access. `plugin.run` intersects each plugin's stored
grants with its manifest's requests.

Desktop clients send `plugin.review` and `plugin.remove` as separate `engine.execute` requests.
`engine.batch` and `engine.script` refuse either command before running any earlier steps.
Direct requests perform package work on a worker, keeping the window responsive.

Review validates bounded zip or folder packages without running code. Zip packages have
`plugin.json` at their root or inside a single enclosing directory. Escaping paths, symbolic links,
special files, duplicate paths, device names and portable path aliases are refused. Packages allow
at most 1,024 entries and 64 MiB of uncompressed content; zip input is also limited to 64 MiB.
Paths allow at most 16 levels and 512 bytes. The manifest remains limited to 16 KiB and the
component to 16 MiB. Preferences retain at most 256 plugin grant records, each with at most four
command groups and 32 folders.

Review's `approval` hash covers every package file. The installer must compare the reviewed
identity with the bytes it publishes. This identity does not approve execution: `plugin.inspect`
and `plugin.run` retain their separate manifest-and-component approval flow.

## Menu and ribbon entries

Discovery examines at most 1,024 package folders and 8,192 host slot items. Only enabled plugins
contribute items. The host draws every item with the plugin's name,
a plugin badge and the entry name. Plugins declare placement in their manifests and supply no
widget, menu or ribbon code.

| Declared slot | Placement (names ignore case) |
|---|---|
| `menu` named `File` or `Help` | The existing File or Help menu; each item includes the plugin's name |
| Any other `menu` name | Rows supplied to the shared Plugins menu through the integration API below |
| `ribbon` named for an existing Design ribbon tab | A group named for the plugin in that tab |
| Any other `ribbon` name | One Plugins ribbon tab, with a group per plugin |

The Design tab names are Model, Surface, Assemble, Insert, Inspect, Manage, View, Sketch,
Sheet Metal and Weldments. Existing tab visibility and workspace rules still apply.
For example, `{"kind":"menu","name":"File"}` places an entry in File;
`{"kind":"ribbon","name":"Model"}` places it in Model.

Clicking a File, Help or ribbon item queues its namespaced `<plugin-id>.<entry>` command for
the shared plugin entry dialog. The slot itself executes no code. The shared dialog owns input
forms, consent and submission through the existing `plugin.run` transaction.

## Integration APIs

The installer and generic Plugins menu/input-schema dialog are separate integration work.
Their connections are explicit; this change alone does not make a slot click execute an entry.

| Host API | Integration responsibility |
|---|---|
| `plugin::read_package_contents(path)` | Return validated manifest, relative file bytes and all-file approval identity; the installer publishes these exact bytes |
| `plugin::validate_grants(manifest, grants)` | Refuse groups, folder names or access modes that the manifest did not request |
| `plugin::store_grants(session, manifest, grants)` | Validate and store approved grants after successful publication; never use this as execution approval |
| `SolveApp::review_plugin_package(path)` | Start asynchronous review and open the host permission screen in Preferences |
| `SolveApp::take_approved_plugin_review()` | Consume `ApprovedReview {path, manifest, approval, grants}` after Approve; hand the result to the installer |
| `SolveApp::persist_plugin_grants()` | Persist session grants into desktop preferences after successful publication and `store_grants` |
| `SolveApp::take_plugin_entry_request()` | Consume the namespaced command queued by a File, Help or ribbon click; open the shared input-schema/consent dialog |
| `SolveApp::plugin_fallback_menu_items()` | Return host menu rows for unrecognised declared menus, already labelled with plugin name and entry and carrying the plugin badge |

Engine APIs above live in `solidcraft_engine::plugin`; desktop methods live on `SolveApp`.
`ApprovedReview` is exposed by the UI crate. Installer order is: validate the reviewed identity
and grants, publish the package atomically, call `store_grants`, then call `persist_plugin_grants`.
Preflight the 256-record grant-store limit before publication. The installer owns rollback if
publication or subsequent grant storage fails; the helpers neither copy files nor undo a copy.
`plugin.remove` persists its grant deletion automatically. The shared entry dialog must inspect content,
draw the entry's declared inputs and supply the reviewed execution hash through the normal
namespaced resolution and worker path. An entry without fields receives `{}`. The generic
Plugins menu must feed its chosen namespaced row into that same shared entry dialog.
Installing, reviewing, enabling or discovering a plugin never runs an entry.

The installer and shared-dialog changes meet this implementation at these files:

| File | Shared code or hook |
|---|---|
| `crates/engine/Cargo.toml`, `Cargo.lock` | Reuse the workspace's existing zip 9 dependency on native targets; the installer shares this package reader |
| `crates/craft-plugin/src/lib.rs` | `FileGrant` serialization, deserialization and JSON schema used by approval data and preferences |
| `crates/engine/src/cmd/plugin.rs`, `crates/engine/src/schema/plugin.rs` | Plugin command registration and input/output schemas |
| `crates/engine/src/plugin.rs` | Package API reexports, `AppState::grants`, `read_package` and grant enforcement in `run` |
| `crates/engine/src/plugin/package.rs` | Bounded package reader, review, grant validation/storage and removal |
| `crates/ui-egui/src/plugins.rs` | Permission review, `ApprovedReview`, installer handoff and grant persistence hooks |
| `crates/ui-egui/src/prefs.rs` | `plugin_grants` storage, bounds and preservation across Preferences Cancel/defaults |
| `crates/ui-kit/src/preferences_cards.rs` | Footer action hook for Remove; the original `settings_plugin_card` wrapper and signature remain available |
| `crates/ui-egui/src/lib.rs` | `command_completed` persists install/remove effects; `run_control` synchronizes changed grants |
| `crates/ui-egui/src/control_job.rs` | Direct review/remove worker routing and rejection of running-plugin results after grant changes |
| `crates/ui-egui/src/control.rs` | Batch/script preflight refuses review/remove before any earlier edits; installer file operations must use the same guard |
| `crates/ui-egui/src/toolbar.rs`, `crates/ui-egui/src/help.rs`, `crates/ui-egui/src/plugin_slots.rs` | Named File/Help/ribbon slots, fallback menu rows and `take_plugin_entry_request` |
| `crates/ui-egui/src/workspace.rs` | Fallback Plugins ribbon-tab visibility; the generic Plugins menu remains separate integration work |

## Versions

The WIT package is `craft:plugin@0.1.0`. Today's surface is one capability, `commands@0.1`:
running app commands, logging, progress and cancellation, named files, a clock, and the exported
entry. A plugin's `plugin.json` lists what it needs, for example
`"capabilities": ["commands@0.1"]`; the host refuses an unknown or newer capability when it reads
the manifest, with a plain message such as "needs commands 0.2; update SolidCraft or the plugin".
`ui`, `viewport`, `formats` and `storage` arrive later as their own versioned interfaces. While a
version is 0.x, a minor version may break the one before it.

## Commands and permissions

A plugin can call every command in the MCP catalog that its manifest lists. Commands that reach
past the open design are grouped (`file`, `doc`, `app`, `plugin`) and refused unless the
manifest asks for the group in `permissions.command_groups` and the user grants it.
File access requires approved folder grants. The reserved `net` permission is
refused with a typed error. Traps, missing permissions, failed commands, limits
and invalid manifest fields return structured errors. Later phases add
import/export, author tooling and browser execution after the
phase 1 design review.

## Plugin data in a design

A design container reserves `plugins/<plugin-id>/` for a plugin's own files. They are kept and
saved back untouched when the plugin is missing, and opening a design never fails because a plugin
is missing. The `storage` capability will let plugins read and write them.

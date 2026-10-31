# Plugin architecture notes

One entry per user-visible plugin feature: what the architecture needed for it, why, and what
is still missing. The runtime reference is [plugin-runtime.md](plugin-runtime.md).

## Fast imports after the first: compiled importers kept on disk

**Feature.** The first import with an importer compiles its component (about 2.5 minutes for
the 68 MB Dutchman Convert component); every later import, in this run or the next, loads the
compiled code instead. Allowing an importer in Preferences › Plugins compiles it in the
background, so even the first import rarely waits.

**Architecture needed.**
- Host API: Wasmtime's own compilation cache (`cache` feature, `Config::cache`), in
  `plugin-cache` beside the plugins folder. Its key is a hash of the component's exact bytes,
  the engine's settings and the Wasmtime version, so a changed component, setting or release
  compiles again. An entry that does not decompress or load is compiled again and replaced.
  The engine crate forbids `unsafe`, so loading a serialized component ourselves
  (`Component::deserialize_file`) was not an option; Wasmtime's cache does the same work
  inside Wasmtime.
- Host API: `plugin::prepare_importer` (compile without running anything), called by the
  Plugins page's Allow and `ui.prefs {"plugin_allow"}`.

**Why.** Compilation is native work outside the guest's fuel and deadline; it dominated the
first import. Measured with the CLI (a new process each time, so nothing in memory), Fusion
`M3 Parametric FHCS.f3d` with the 68.7 MB component: 155.8 s before; with the cache 152.5 s the
first time and 1.4 s every time after (the cache holds 61 MB).

**Still missing.** The cache has no integrity check of its own beyond decompression and
Wasmtime's object checks; it trusts the user's own data folder, as every Wasmtime embedder's
cache does. Cancel cannot stop a compilation that is under way (it is native code); the import
stops at its first check after it.

## A smaller importer component

**Feature.** Dutchman Convert's component went from 68.7 MB to 39.7 MB, so it downloads,
installs, hashes and compiles faster.

| Step | Size |
|---|---|
| Every cadmpeg codec, `opt-level = "z"`, 16 code units, no LTO | 68.7 MB |
| Only the codecs of the formats it claims (Fusion, Inventor, SolidWorks, FreeCAD) | 43.9 MB |
| One code unit | 40.8 MB |
| Thin LTO | 39.7 MB |

**Architecture needed.** Dutchman: a Cargo feature per input format in `dutchman-core` (all on by
default), so a build names only the formats it reads; `dutchman-solidcraft` turns on only the
STEP codec it writes with. The component already strips symbols, and Wasm panics abort.

**Still missing.** The Fusion codec and the STEP writer are most of what is left; a smaller
component would need cadmpeg features inside those codecs.

## Ribbon presence: a plugin's commands are commands

**Feature.** An allowed plugin adds commands that look and behave like built-in ones: a button
on an existing tab (in a panel named by the plugin) or on a tab of its own, with an icon,
label, tooltip, optional shortcut and an enabled state. The palette, search, shortcuts, the
control channel, MCP and the CLI reach them by id (`plugin.<package id>.<command id>`). Turning
the plugin off, revoking it or removing it takes them away. Dutchman Convert adds
**Insert › Dutchman › Import CAD design**.

**Architecture needed.**
- Manifest field `contributes.commands` (id, label, tooltip, icon, shortcut, `when`, `entry`
  for a command plugin, `params`) and `contributes.toolbar` (tab, panel, command ids).
  Checked strictly by `parse_manifest`; part of the approval hash like the rest.
- Engine: a contributed-command registry (`plugin::contrib`). `command_specs()` lists the
  built-in commands, then the contributed ones, so every consumer of the registry sees them.
  Each contributed command is an ordinary `CommandSpec` with `plugin: Some(..)`;
  `CommandSpec::call` runs it through its plugin instead of a `run` function. Specs are leaked
  once per approved package content so they are `&'static` like the built-in ones.
- Engine: running a contributed command checks that its plugin is still allowed for the
  exact content the command came from, checks the parameters against the declaration, then
  runs `file.import` (an importer) or `plugin.run` (a command plugin's entry).
- UI hook: `plugins::sync_contributions` refreshes the registry when the plugin policy changes
  (Allow, Revoke, Turn off, Remove, Install); the ribbon's cached panel lists start over when
  the registry's generation changes. Panels a plugin adds go after the tab's own; tabs of its
  own go after the built-in tabs and can be hidden like them.
- UI hook: the agent chat asks before running any contributed command.

**Why.** "Everything is a command": registering through `CommandSpec` is what makes every
surface reach the plugin with no special cases.

**Still missing.** Labels and tooltips come from the manifest in one language; a manifest
catalog of translations is not designed yet. A shortcut a built-in command already has is not
bound (the plugin's command keeps working from the palette). A plugin's own tab has no
user-chosen position yet.

## Native dialogs from a declared schema

**Feature.** A plugin command's parameters are declared in its manifest (number with a unit,
length, angle, checkbox, choice, text, picks of faces, edges or bodies) and drawn by the app's
own dialog: the same rows, fields, validation, OK/Cancel and Enter/Esc. Dutchman Convert's
dialog asks for the file (filtered to its formats), then Preset (Standard, Fast, Full), Keep
history, and whether the result opens as a new design or goes into this one.

**Architecture needed.**
- Manifest: `params` on a contributed command (`name`, `label`, `type`, `unit`, `min`, `max`,
  `options`, `default`, `accept`, `many`, `required`, `help`).
- UI: `dialogs::Kind::Plugin` (`dialogs_plugin.rs`) builds its rows and selection inputs from
  the declaration. The engine checks the values again when the command runs.
- WIT: `import-context.options()` hands an importer the checked values as JSON. It is a host
  import, so components built before it still run.
- Engine: `file.import` takes `options` (handed to the importer) and `into: new|current`.
  Into the current design, the converted geometry joins it as a component named after the
  file, in one undo step; the importer is told (`into` in its options) so it can leave the
  history out.

**Why.** A plugin never draws UI; the host draws it from data, so it looks native and cannot
do anything but describe values.

**Still missing.** Live preview for plugin commands (a command plugin's preview would run its
code on every change). Conditional rows (show a field only when another has a value).
Importing into the current design brings geometry only, not the source's history.

## Plugin icons

**Feature.** A package may ship SVG icons for itself and its commands; they draw exactly as the
app's own icons do (both themes, the colour-vision palettes), or fall back to the generic plugin
icon. Dutchman Convert uses SolidCraft's own `import` icon.

**Architecture needed.**
- Manifest: `icon` (package) and a command's `icon`: a built-in icon's name or a `.svg` file in
  the package. Icon files are read with the package, at most 32 KiB each, and are part of the
  approval hash (a package without icons hashes as before).
- UI: `icon_svg::register_plugin_icon` parses them with the same whitelist parser as the
  built-in masters: only `svg`, `g`, `defs`, `mask`, `clipPath`, `path` and `circle`, a 64-unit
  view box and the palette's colours. Scripts, images, text, styles and external references
  are refused, and so is the whole icon with them.

**Why.** One rendering path keeps plugin icons consistent with the style guide's colours and
weights, and the whitelist is the sanitizer.

**Still missing.** No review step checks a plugin's drawing against the style guide.

## Status and results like built-ins

**Feature.** An import shows the app's file progress card with the importer's own steps
(preparing the importer, reading, decoding, mapping the history, writing), and Cancel stops
the importer and leaves the design as it was. Afterwards an Import report lists what the
plugin brought over and its notes (what did not come across, what was kept hidden and why).
Plugin errors are command errors, shown the same way.

**Architecture needed.**
- WIT: `import-context.progress(stage, done, total)`, with a fixed stage list so the app can
  name each step in every language.
- Host: the importer reports through the file worker's watch (`solidcraft_kernel::watch`);
  Cancel interrupts it through Wasmtime's epoch interruption. The deadline thread now ticks
  every 100 ms and each running importer checks its own deadline and Cancel.
- UI: the progress card names `plugin.*` stages; `plugin_report` shows the report window
  (`ui_kit::Window`) after a plugin import.

**Still missing.** A stage is reported at its start only; finer progress inside Dutchman's
decoders would need a callback through Dutchman and cadmpeg.

## Preferences › Plugins cards

**Feature.** Each plugin is a card: icon, name, version and publisher; its description; what it
adds (each command and where it is on the ribbon); the formats or commands it is granted and
the fuel, memory and time it asks for; Allow/Revoke; Turn off/on; Show folder; Remove (asks
once more).

**Architecture needed.**
- Manifest: `publisher`, `icon`.
- Engine: `plugin.remove {id}` deletes an installed package's folder (only a folder the
  plugins folder lists) and its approval.

**Still missing.** Updating a plugin in place (install over it) needs a new Allow, by design;
the card does not yet say what changed between the allowed and the installed content.

## Assemblies keep unchecked history hidden too

**Feature.** A Fusion `.f3z` assembly imported through Dutchman Convert keeps each part's
original geometry visible and its mapped, unchecked history in a hidden "Editable history
(unverified)" component inside the part, as a single part does.

**Architecture needed.** Dutchman only: its assembly writer honours `hide_unverified_history`
per part (on Dutchman's `assembly-hidden-unverified` branch, in the plugin's pin). Nothing
changed in SolidCraft.

**Still missing.** `.iam` and `.sldasm` assemblies are still not claimed: their parts are other
files, which the sandboxed importer cannot read. That needs a host API that hands an importer
the files an assembly names (with the user's consent), not designed yet.

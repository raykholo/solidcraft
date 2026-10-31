# Dutchman Convert importer

An importer plugin built from [Dutchman](https://github.com/dutchman-cad/dutchman) (Apache-2.0)
and the [cadmpeg](https://github.com/cadmpeg/cadmpeg) decoders it uses (Apache-2.0). It opens
Fusion (`.f3d`, `.f3z`), Inventor (`.ipt`), SolidWorks (`.sldprt`) and FreeCAD (`.FCStd`) files
as SolidCraft designs **with their feature history**: Dutchman maps the source's sketches,
extrusions, revolutions, holes and parameters onto SolidCraft features, and keeps the original
geometry beside them as a suppressed import.

It runs in the `importer-plugin` world: SolidCraft reads the file the user chose and hands the
component its bytes and its name, nothing else. It has no files, network, clock, processes or
engine commands, so:

- Dutchman's checks that run the SolidCraft CLI (replaying the history to prove it matches the
  source, and finding the edges each fillet or chamfer rounds) are off. Fillets and chamfers
  are left out of the history and listed in the notes; the suppressed import keeps the exact
  original geometry.
- Assemblies (`.iam`, `.sldasm`) are not claimed: their parts are other files, which the
  component cannot read. A Fusion `.f3z` archive carries its parts inside and converts.
- SolidCraft opens `.FCStd` itself. Choose this importer for a FreeCAD file with
  `file.import {"path": "…", "plugin": "dutchman-convert"}` to get its history instead.

## Build

With Rust's `wasm32-wasip2` target installed (`rustup target add wasm32-wasip2`), from this
folder:

```sh
cargo build --release --target wasm32-wasip2
cp target/wasm32-wasip2/release/solidcraft_dutchman_convert.wasm plugin.wasm
```

The target's linker writes a component directly; no adapter or `wasm-tools` step is needed.
The component is about 38 MiB (39.7 MB). It links only the cadmpeg codecs of the formats it
claims (Dutchman's per-format Cargo features) and is built for size: `opt-level = "z"`, one
code unit and thin LTO, which links within 12 GiB of memory. `plugin.json` asks for
`limits.component_mib: 48` (the default cap is 16 MiB); the Plugins page shows it, and it is
part of what you allow.
The component imports only the plugin log and the WASI interfaces Rust's standard library
links; SolidCraft gives an importer an empty WASI (see
[the runtime reference](../../../docs/plugin-runtime.md#importer-plugins)).

Dutchman reads files from disk; the plugin calls `dutchman_core::read_bytes`, Dutchman's
entry point for a file's contents alone, then `SolidCraftWriter` with verification off.
`Cargo.toml` pins Dutchman's `solidcraft-plugin-pin` branch: main with a Cargo feature per
input format and assembly parts that keep an unchecked history hidden.

## Known limits

- **Fusion history comes across, unverified and hidden.** Dutchman's cadmpeg pin carries the
  wasm32 fix for Fusion parameter scopes (cadmpeg #645; before it, a 32-bit build decoded no
  feature timeline at all) and the centimetre sketch-frame fix (cadmpeg #646). With no
  SolidCraft CLI in the sandbox the history cannot be checked against the original, so the
  plugin does what Dutchman does when a check fails: the original geometry is the visible
  model and the mapped history sits in a hidden "Editable history (unverified)" part. Show it
  in the browser to edit it. Each part of a Fusion `.f3z` assembly is written the same way.
  - `M3 Parametric FHCS.f3d`: Sketch 1, Revolve 1, Sketch 2 and Extrude 1 are in the hidden
    part and rebuild the original exactly, the hex socket included (49.86 mm³).
  - `M3 Parametric BHCS.f3d`: the same four features; Thread 1 (a cosmetic thread with no
    resolved standard and pitch) stops the history there.
  - Fillets and chamfers are left out, since finding their edges needs the SolidCraft CLI.
- The first use compiles the component. SolidCraft does it in the background when you click
  **Allow**, and keeps the compiled importer on disk (`plugin-cache` beside the plugins
  folder), so imports after that, in this run or the next, start at once.

## Install

1. In SolidCraft, open **Preferences › Plugins** and click **Install plugin…**.
2. Pick this folder's `plugin.json` (with `plugin.wasm` beside it).
3. Check what the page lists: the formats it imports and the fuel, memory and time it asks
   for. It asks for 1024 MiB: raise **Max plugin memory (MiB)** to at least that.
4. Click **Allow**. The approval is for this exact manifest and component; a rebuilt
   component must be allowed again.

Then **Insert › Dutchman › Import CAD design** asks for a Fusion, Inventor, SolidWorks or
FreeCAD file and shows the import options:

- **Preset**: Standard (the feature history and joints), Full (reference sketches too) or Fast
  (the geometry alone, as one import).
- **Keep history**: off brings the geometry alone.
- **Open as**: a new design, or into the design that is open (its geometry as a component).

The import shows its steps (reading, decoding, mapping the history, writing) with Cancel, and
then a report of what did not come across. **File › Import…**, File › Open and dropping a file
on the window also open these files through the plugin, with the Standard preset. From a
script or the control channel:

```json
{"command": "plugin.dutchman-convert.import", "params": {"path": "/path/to/M3 Parametric BHCS.f3d", "preset": "full"}}
{"command": "file.import", "params": {"path": "/path/to/M3 Parametric BHCS.f3d"}}
```

The reply names the plugin and carries its `notes`: what did not come across.

## Licences

The component links Dutchman (Apache-2.0), cadmpeg (Apache-2.0) and their permissively
licensed Rust dependencies; this example's own code is Apache-2.0. See `ATTRIBUTION.md`.

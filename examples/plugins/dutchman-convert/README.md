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
The component is about 65 MiB: it links every cadmpeg decoder Dutchman's format registry
names, and LTO is off because a fat or thin LTO link of all of them needs more than 12 GiB of
memory. `plugin.json` therefore asks for `limits.component_mib: 80` (the default cap is 16 MiB);
the Plugins page shows it, and it is part of what you allow.
The component imports only the plugin log and the WASI interfaces Rust's standard library
links; SolidCraft gives an importer an empty WASI (see
[the runtime reference](../../../docs/plugin-runtime.md#importer-plugins)).

Dutchman reads files from disk; the plugin calls `dutchman_core::read_bytes`, Dutchman's
entry point for a file's contents alone (a small addition on Dutchman's `wasm-read-bytes`
branch, which `Cargo.toml` pins), then `SolidCraftWriter` with verification off.

## Known limits

- **Fusion history comes across, unverified.** `Cargo.toml` patches Dutchman's cadmpeg pin to
  cadmpeg main, which carries the wasm32 fix for Fusion parameter scopes (cadmpeg #645); before
  it, a 32-bit build decoded no feature timeline at all. Dutchman's own pin is unchanged.
  - `M3 Parametric FHCS.f3d`: Sketch 1, Revolve 1, Sketch 2 and Extrude 1 come across as
    editable features, with the original geometry kept as a suppressed import. The history is
    not checked against the original (no SolidCraft CLI in the sandbox), and here it differs:
    Extrude 1 cuts the hex socket 1 mm in the negative direction, away from the head, and the
    body is 51.59 mm³ against the original's 49.86 mm³.
  - `M3 Parametric BHCS.f3d`: the same four features are kept as a hidden "Editable history"
    prefix; Thread 1 (a cosmetic thread with no resolved standard and pitch) stops the history,
    so the visible body is the imported final shape.
  - Fillets and chamfers are left out, since finding their edges needs the SolidCraft CLI.
- The first import compiles the component (about three minutes for 65 MiB); SolidCraft keeps
  the compiled importer in memory, so later imports in the same session take about a second.

## Install

1. In SolidCraft, open **Preferences › Plugins** and click **Install plugin…**.
2. Pick this folder's `plugin.json` (with `plugin.wasm` beside it).
3. Check what the page lists: the formats it imports and the fuel, memory and time it asks
   for. It asks for 1024 MiB: raise **Max plugin memory (MiB)** to at least that.
4. Click **Allow**. The approval is for this exact manifest and component; a rebuilt
   component must be allowed again.

Then **File › Import…** (or File › Open, or dropping a file on the window) opens a Fusion,
Inventor or SolidWorks file through the plugin. From a script or the control channel:

```json
{"command": "file.import", "params": {"path": "/path/to/M3 Parametric BHCS.f3d"}}
```

The reply names the plugin and carries its `notes`: what did not come across.

## Licences

The component links Dutchman (Apache-2.0), cadmpeg (Apache-2.0) and their permissively
licensed Rust dependencies; this example's own code is Apache-2.0. See `ATTRIBUTION.md`.

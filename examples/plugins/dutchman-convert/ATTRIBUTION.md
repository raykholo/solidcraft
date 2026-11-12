# Attribution

The `plugin.wasm` this example builds links these projects as Cargo dependencies. Nothing from
them is copied into this folder; SolidCraft neither ships nor bundles the component.

| Project | Source | Licence |
|---|---|---|
| Dutchman (`dutchman-core`, `dutchman-solidcraft`) | https://github.com/dutchman-cad/dutchman | Apache-2.0 |
| cadmpeg (`cadmpeg-core`, `cadmpeg-ir`, `cadmpeg-registry` and its codecs) | https://github.com/cadmpeg/cadmpeg | Apache-2.0 |
| wit-bindgen | https://github.com/bytecodealliance/wit-bindgen | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |

Their transitive Rust dependencies are permissively licensed; `cargo tree` in this folder lists
them. A redistributed `plugin.wasm` carries these licences' notice requirements.

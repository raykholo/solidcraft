# Command plugin quickstarts

Create the plugin in its own repository, outside the SolidCraft source tree. These
small echo entries prove the component boundary before adding model operations.
Both languages use the same manifest and host checks. This is authoring guidance;
the [generated reference](../../../docs/plugin-api/README.md) defines the API.

## Shared package

Set `SOLIDCRAFT_SOURCE` to your SolidCraft checkout. In a fresh plugin repository:

```sh
mkdir -p wit package/echo
cp "$SOLIDCRAFT_SOURCE/crates/craft-plugin/wit/plugin.wit" wit/plugin.wit
```

Save this as `package/echo/plugin.json`. Its API identifier must match the package
declaration in the copied WIT; update from that source when targeting another API.

```json
{
  "id": "echo",
  "name": "Echo",
  "version": "0.1.0",
  "description": "Return object inputs to verify the component boundary.",
  "runtime": {
    "api": "solidcraft:plugin@1.0.0",
    "module": "plugin.wasm",
    "entries": [{
      "name": "echo",
      "effect": "read",
      "input_schema": {"type": "object", "properties": {}, "additionalProperties": false},
      "slots": []
    }],
    "commands": [],
    "permissions": {"files": [], "clock": false, "net": false}
  }
}
```

The omitted limits object uses host defaults. Add engine commands only as needed,
using exact permitted ids and schemas from the reference. Change `effect` to `edit`
when the entry edits the model. Folder permissions request named grants; they do
not grant arbitrary paths. See [the manifest contract](../../../docs/plugin-runtime.md#manifest).

## Rust: wit-bindgen

Install the guest tools using your normal Rust toolchain. Pin resolved versions
and commit the guest's lockfile for reproducible releases.

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-tools --locked
cargo init --lib --name echo-guest .
cargo add wit-bindgen
```

Add this section to the guest's `Cargo.toml`:

```toml
[lib]
crate-type = ["cdylib"]
```

Replace `src/lib.rs` with:

```rust
wit_bindgen::generate!({ path: "wit", world: "command-plugin" });

use exports::solidcraft::plugin::command::Guest;
use solidcraft::plugin::types::PluginError;

struct Echo;

impl Guest for Echo {
    fn execute(_entry: String, params: String) -> Result<String, PluginError> {
        Ok(params)
    }
}

export!(Echo);
```

Build the core module with WIT metadata, then wrap it as a component:

```sh
cargo build --release --target wasm32-unknown-unknown
wasm-tools component new target/wasm32-unknown-unknown/release/echo_guest.wasm -o package/echo/plugin.wasm
```

No WASI adapter is needed for this target. Guest code that calls the engine uses
the generated `solidcraft::plugin::engine::run`; pass JSON strings and propagate
the generated typed error. Validate parameters and final JSON before returning.

## TypeScript: jco

Install Node.js and initialize the separate guest package. Pin resolved package
versions with your lockfile.

```sh
npm init -y
npm install --save-dev @bytecodealliance/jco @bytecodealliance/componentize-js typescript
npx jco types wit --world-name command-plugin --out-dir types
```

The generated declarations describe WIT JSON as a string and failures as typed
variants. Use those declarations when implementing the exported interface. Save
this as `plugin.ts`:

```typescript
import type { Json } from './types/interfaces/solidcraft-plugin-types.js';

export const command = {
  execute(_entry: string, params: Json): Json {
    return params;
  }
};
```

Transpile TypeScript, then componentize the JavaScript:

```sh
npx tsc plugin.ts --target es2022 --module esnext --outDir dist --skipLibCheck
npx jco componentize dist/plugin.js --wit wit --world-name command-plugin --disable all --out package/echo/plugin.wasm
wasm-tools component wit package/echo/plugin.wasm
```

Disable ambient JavaScript features because the host links only its published
interfaces. Inspect the resulting imports: toolchain versions can retain WASI
imports even when a guest does not call them. Any remaining unsupported import
must be removed or resolved inside the component before packaging; the host
does not supply WASI, Node.js, network, filesystem or process APIs. `plugin check`
is the compatibility gate, and `plugin test` also checks runtime budgets. A JS
engine inside the component consumes the same memory and fuel as other guests.

## Validate either language

With a native SolidCraft CLI that includes plugin support:

```sh
solidcraft-cli plugin check package/echo
solidcraft-cli plugin test package/echo --entry echo --input '{}'
```

Expect a passing entry and inspect its design and command journal paths in the
JSON report. The echo entry performs no model edits. For a model plugin, add a
known design with `--design fixture.sc`, exercise every entry, and assert the
resulting native features and command journal. Entries run independently from
the same initial design. A failed call saves the unchanged design and a journal
of attempted host commands, including edits later rolled back. Artifacts remain
in the reported temporary folder until you remove them. Input schemas are checked
offline; bundle definitions with the schema rather than referencing external files
or URLs. Test refusal paths too: malformed
inputs, denied commands, missing grants and resource limits. Package the checked
`plugin.json` and component together in the id folder.

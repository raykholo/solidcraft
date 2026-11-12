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

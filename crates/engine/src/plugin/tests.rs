use super::*;
use serde_json::{Value, json};
use solidcraft_geom::Vec3Geom as _;

fn manifest() -> Value {
    json!({"id":"sample", "name":"Sample", "version":"0.1.0", "runtime":{
        "api":API,"module":"plugin.wasm","entries":["make"],"commands":["solid.box"],
        "limits":{"fuel":100000,"memory_mib":1,"deadline_ms":5000}}})
}
#[test]
fn manifest_is_inert_bounded_and_refuses_unknown_capabilities() {
    assert!(parse_manifest(br#"{"id":"sample","name":"Sample","version":"1"}"#, "sample").unwrap().runtime.is_none());
    assert!(parse_manifest(&serde_json::to_vec(&manifest()).unwrap(), "sample").is_ok());
    let mut explicit = manifest();
    explicit["runtime"]["limits"]["fuel"] = json!(100_000_000_000_u64);
    assert!(parse_manifest(&serde_json::to_vec(&explicit).unwrap(), "sample").is_ok());
    assert_eq!(Limits::default().fuel, 10_000_000);
    let old = parse_manifest(br#"{"id":"sample","name":" Sample ","version":" 1 ","description":" Old metadata "}"#, "sample").unwrap();
    assert_eq!((old.name.as_str(), old.version.as_str(), old.description.as_str()), ("Sample", "1", "Old metadata"));
    for mutate in [
        |m: &mut Value| m["runtime"]["module"] = json!("../other.wasm"),
        |m: &mut Value| m["runtime"]["commands"] = json!(["file.export"]),
        |m: &mut Value| m["runtime"]["commands"] = json!(["plugin.run"]),
        |m: &mut Value| m["runtime"]["entries"] = json!(["make", "make"]),
        |m: &mut Value| m["runtime"]["limits"]["memory_mib"] = json!(4097),
        |m: &mut Value| m["runtime"]["limits"]["fuel"] = json!(0),
        |m: &mut Value| m["runtime"]["limits"]["deadline_ms"] = json!(30_001),
        |m: &mut Value| m["runtime"]["api"] = json!("solidcraft:plugin@99.0.0"),
        |m: &mut Value| m["network"] = json!(true),
        |m: &mut Value| m["permissions"] = json!(["net"]),
        |m: &mut Value| m["runtime"]["permissions"] = json!(["net"]),
        |m: &mut Value| m["runtime"]["permissions"] = json!({"net":true}),
    ] {
        let mut m = manifest();
        mutate(&mut m);
        assert!(parse_manifest(&serde_json::to_vec(&m).unwrap(), "sample").is_err(), "{m}");
    }
    assert!(parse_manifest(&vec![b' '; MAX_MANIFEST_BYTES + 1], "sample").is_err());
}
#[test]
fn disabled_ids_survive_scratch_and_new_design() {
    let mut s = Session::default();
    s.disabled_plugins.insert("sample".into());
    assert!(s.scratch().disabled_plugins.contains("sample"));
    s.execute("file.new", &json!({})).unwrap();
    assert!(s.disabled_plugins.contains("sample"));
}

#[cfg(all(feature = "plugins", any(target_arch = "x86_64", target_arch = "aarch64")))]
mod native {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new(code: &str) -> Self {
            let root =
                solidcraft_io::vfs::temp_dir().join(format!("solidcraft-plugin-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            let dir = root.join("sample");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("plugin.json"), serde_json::to_vec(&manifest()).unwrap()).unwrap();
            std::fs::write(dir.join("plugin.wasm"), component(code)).unwrap();
            Self(root)
        }
        fn path(&self) -> String {
            self.0.join("sample").to_string_lossy().into_owned()
        }
        fn run(&self, s: &mut Session) -> crate::Result<Value> {
            let approval = s.execute("plugin.inspect", &json!({"path":self.path()}))?["approval"].clone();
            s.execute("plugin.run", &json!({"path":self.path(),"approval":approval,"entry":"make","params":{}}))
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    // An original component fixture. A real canonical lift/lower crosses the host boundary.
    // $box asks for a 2 x 3 x 4 mm solid; memory at 0 is the canonical result area.
    fn component_template() -> String {
        let (package, version) = API.split_once('@').unwrap();
        include_str!("fixture.wat").replace("\r\n", "\n").replace("__PACKAGE__", package).replace("__VERSION__", version)
    }
    fn component(code: &str) -> Vec<u8> {
        let template = component_template();
        let original = "      call $box\n      call $box\n";
        assert!(template.contains(original));
        wat::parse_str(template.replacen(original, &format!("      {code}\n"), 1)).unwrap()
    }
    fn file_component(write: bool) -> Vec<u8> {
        let (package, version) = API.split_once('@').unwrap();
        let template = component_template();
        // Keep exactly the shared WIT types, then exercise the real files interface.
        let (types, _) = template.split_once(&format!("  (type $ty-{package}/engine@{version}")).unwrap();
        let (operation, result, parameters, arguments) = if write {
            ("write-file", "(result (error $error))", "(param \"data\" (list u8))", "i32.const 128 i32.const 7")
        } else {
            ("read-file", "(result (list u8) (error $error))", "", "")
        };
        let core_parameters = if write { "i32 i32 i32 i32 i32 i32 i32" } else { "i32 i32 i32 i32 i32" };
        let source = format!(
            r#"{types}
  (type $files-type (instance
    (alias outer 1 $plugin-error (type $error))
    (export "plugin-error" (type (eq $error)))
    (type $file-result {result})
    (type $operation (func (param "grant" string) (param "path" string) {parameters} (result $file-result)))
    (export "{operation}" (func (type $operation)))
  ))
  (import "{package}/files@{version}" (instance $files (type $files-type)))
  (alias export $files "{operation}" (func $file))
  (core module $allocation
    (memory (export "memory") 1)
    (global $heap (mut i32) i32.const 4096)
    (func (export "realloc") (param i32 i32 i32 i32) (result i32)
      (local $at i32)
      global.get $heap local.tee $at local.get 3 i32.add global.set $heap local.get $at)
  )
  (core instance $allocation (instantiate $allocation))
  (alias core export $allocation "memory" (core memory $memory))
  (alias core export $allocation "realloc" (core func $realloc))
  (core func $file (canon lower (func $file) (memory $memory) (realloc $realloc)))
  (core instance $imports (export "file" (func $file)) (export "memory" (memory $memory)))
  (core module $main
    (import "host" "file" (func $file (param {core_parameters})))
    (import "host" "memory" (memory 1))
    (data (i32.const 64) "project")
    (data (i32.const 96) "secret.txt")
    (data (i32.const 128) "changed")
    (func (export "execute") (param i32 i32 i32 i32) (result i32)
      i32.const 64 i32.const 7 i32.const 96 i32.const 10 {arguments} i32.const 0 call $file
      i32.const 0)
  )
  (core instance $main (instantiate $main (with "host" (instance $imports))))
  (alias core export $main "execute" (core func $execute))
  (func $execute (param "entry" string) (param "params" $json) (result (result $json (error $plugin-error)))
    (canon lift (core func $execute) (memory $memory) (realloc $realloc)))
  (instance $command
    (export "json" (type $json))
    (export "plugin-error" (type $plugin-error))
    (export "execute" (func $execute)))
  (export "{package}/command@{version}" (instance $command))
)"#
        );
        wat::parse_str(source).unwrap()
    }
    #[test]
    fn larger_declared_budgets_need_user_ceilings_and_exact_content_approval() {
        let f = Fixture::new("call $box i32.const 2048 memory.grow i32.const -1 i32.eq if unreachable end");
        let path = f.0.join("sample/plugin.json");
        let mut m = manifest();
        m["runtime"]["limits"] = json!({"fuel":200_000_000_000_u64,"memory_mib":256,"deadline_ms":5000});
        std::fs::write(&path, m.to_string()).unwrap();
        let mut s = Session::default();
        let inspection = s.execute("plugin.inspect", &json!({"path":f.path()})).unwrap();
        assert_eq!(inspection["within_host_limits"], false);
        assert_eq!(inspection["host_limits"]["max_memory_mib"], 128);
        assert!(inspection["limits_error"].as_str().unwrap().contains("user host limits"));
        let before = s.doc.to_json();
        assert!(f.run(&mut s).unwrap_err().to_string().contains("user host limits"));
        assert_eq!(s.doc.to_json(), before);
        assert!(s.undo.is_empty());
        s.plugin_limits = HostLimits { max_fuel: 200_000_000_000, max_memory_mib: 256 };
        let approved = s.execute("plugin.inspect", &json!({"path":f.path()})).unwrap();
        assert_eq!(approved["within_host_limits"], true);
        assert_eq!(approved["approval"], inspection["approval"], "approval pins the exact request; a ceiling change never changes its budget");
        f.run(&mut s).unwrap();
        assert_eq!(s.execute("inspect.measure", &json!({})).unwrap()["total"]["volume_mm3"], 24.);
        assert_eq!(s.undo.len(), 1);
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(s.doc.to_json(), before);
        // Even the largest u64 budget reaches Wasmtime exactly; a new request needs new approval.
        m["runtime"]["limits"]["fuel"] = json!(u64::MAX);
        std::fs::write(&path, m.to_string()).unwrap();
        s.plugin_limits.max_fuel = u64::MAX;
        assert!(s.execute("plugin.run", &json!({"path":f.path(),"approval":approved["approval"],"entry":"make"})).is_err());
        assert_eq!(s.doc.to_json(), before);
        f.run(&mut s).unwrap();
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(s.doc.to_json(), before);
        // Reducing the host policy blocks a previously reviewed package.
        s.plugin_limits = HostLimits::default();
        assert!(f.run(&mut s).is_err());
        assert_eq!(s.doc.to_json(), before);
    }
    #[test]
    fn a_plugin_prepared_under_an_old_host_policy_is_not_adopted() {
        for command in ["plugin.run", "PLUGIN.RUN"] {
            let f = Fixture::new("call $box");
            let mut s = Session::default();
            let i = s.execute("plugin.inspect", &json!({"path":f.path()})).unwrap();
            let params = json!({"path":f.path(),"approval":i["approval"],"entry":"make"});
            let prepared = s.scratch().compute_commands(&[(command.into(), params)], None).unwrap();
            assert!(prepared.is_current(&s));
            s.plugin_limits.max_memory_mib = 512;
            assert!(!prepared.is_current(&s));
            let before = s.doc.to_json();
            assert!(s.accept_commands(prepared).is_err());
            assert_eq!(s.doc.to_json(), before);
            assert!(s.undo.is_empty());
        }
    }
    #[test]
    fn component_edits_are_one_undo_step_and_reopen_without_the_plugin() {
        let f = Fixture::new("call $box call $box");
        let mut s = Session::default();
        s.execute("solid.box", &json!({"length":2,"width":3,"height":4})).unwrap();
        let before = s.doc.to_json();
        let undo = s.undo.len();
        f.run(&mut s).unwrap();
        assert_eq!(s.doc.features.len(), 3);
        assert_eq!(s.undo.len(), undo + 1);
        assert_eq!(s.execute("inspect.measure", &json!({})).unwrap()["total"]["volume_mm3"], 72.);
        let saved = s.doc.to_json();
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(s.doc.to_json(), before);
        assert_eq!(s.undo.len(), undo, "one undo removes the entire plugin edit");
        s.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(s.doc.to_json(), saved);
        assert_eq!(s.undo.len(), undo + 1);
        drop(f);
        let mut reopened = Session::new(solidcraft_doc::Document::from_json(&saved).unwrap());
        assert_eq!(reopened.execute("inspect.measure", &json!({})).unwrap()["total"]["volume_mm3"], 72.);
    }
    #[test]
    fn component_file_reads_and_writes_need_a_host_grant_even_when_declared() {
        for write in [false, true] {
            for declared in [false, true] {
                let f = Fixture::new("call $box");
                let secret = f.0.join("sample/secret.txt");
                std::fs::write(&secret, b"private contents").unwrap();
                std::fs::write(f.0.join("sample/plugin.wasm"), file_component(write)).unwrap();
                if declared {
                    let mut m = manifest();
                    m["runtime"]["permissions"] = json!({"files":[{"name":"project","read":true,"write":true}]});
                    std::fs::write(f.0.join("sample/plugin.json"), m.to_string()).unwrap();
                }
                let mut s = Session::default();
                s.execute("solid.box", &json!({"length":1,"width":2,"height":3})).unwrap();
                let before = s.doc.to_json();
                let history = (s.undo.len(), s.redo.len());
                assert!(s.plugin_state.file_grants.is_empty());
                let error = f.run(&mut s).unwrap_err();
                let expected = if write { "files.write:project" } else { "files.read:project" };
                assert!(
                    matches!(&error, EngineError::Plugin(craft_plugin::HostError::PermissionDenied { permission }) if permission == expected),
                    "write={write}, declared={declared}: {error}"
                );
                assert_eq!(std::fs::read(&secret).unwrap(), b"private contents");
                assert_eq!(s.doc.to_json(), before);
                assert_eq!((s.undo.len(), s.redo.len()), history);
                assert_eq!(solidcraft_io::vfs::current_scope(), None);
            }
        }
    }
    #[test]
    fn traps_fuel_memory_and_denied_calls_leave_design_and_history_untouched() {
        for (code, diagnostic) in [
            ("call $box unreachable", "trap=UnreachableCodeReached"),
            ("call $box i32.const 8 i32.const 1 i32.store", "entry result is not JSON"),
            ("call $box (loop $forever br $forever)", "trap=OutOfFuel"),
            ("call $box i32.const 10000 memory.grow drop", "forcing trap when growing memory"),
            ("call $box i32.const -1 i32.load drop", "trap=MemoryOutOfBounds"),
            ("call $box i32.const 1 i32.const 0 i32.div_u drop", "trap=IntegerDivisionByZero"),
            ("call $box i32.const 2048 i32.const 11 i32.const 1024 i32.const 33 i32.const 0 call $run", "not granted"),
            (
                "call $box i32.const 2048 i32.const 11 i32.const 1024 i32.const 33 i32.const 0 call $run i32.const 0 i32.const 0 i32.store i32.const 4 i32.const 1024 i32.store i32.const 8 i32.const 33 i32.store",
                "not granted",
            ),
        ] {
            let f = Fixture::new(code);
            let mut s = Session::default();
            s.execute("solid.box", &json!({"length":1,"width":1,"height":1})).unwrap();
            s.execute("edit.undo", &json!({})).unwrap();
            let before = s.doc.to_json();
            let history = (s.undo.len(), s.redo.len());
            let message = f.run(&mut s).unwrap_err().to_string();
            assert!(message.contains(diagnostic), "{code}: {message}");
            if diagnostic == "trap=OutOfFuel" {
                assert!(message.contains("fuel_consumed=100000"), "{message}");
                assert!(message.contains("fuel_remaining=0"), "{message}");
            }
            assert_eq!(s.doc.to_json(), before, "{code}");
            assert_eq!((s.undo.len(), s.redo.len()), history, "{code}");
            assert_eq!(s.active_sketch, None);
            assert_eq!(solidcraft_io::vfs::current_scope(), None);
        }
    }
    #[test]
    fn a_granted_component_reads_only_the_evaluated_mesh_and_preserves_history() {
        let f = Fixture::new("call $box");
        let template = component_template();
        let old_args = r#"{\22length\22:2,\22width\22:3,\22height\22:4}"#;
        assert!(template.contains(old_args));
        assert_eq!(template.matches("i32.const 9\n").count(), 1);
        assert_eq!(template.matches("i32.const 33\n").count(), 1);
        let wat = template
            .replace("      call $box\n      call $box\n", "      call $box\n")
            .replace("solid.box", "model.mesh")
            .replace(old_args, r#"{\22body\22:\22Box\22}"#)
            .replace("i32.const 9\n", "i32.const 10\n")
            .replace("i32.const 33\n", "i32.const 14\n");
        std::fs::write(f.0.join("sample/plugin.wasm"), wat::parse_str(wat).unwrap()).unwrap();
        let mut m = manifest();
        m["runtime"]["commands"] = json!(["model.mesh"]);
        std::fs::write(f.0.join("sample/plugin.json"), m.to_string()).unwrap();
        let mut s = Session::default();
        s.execute("solid.box", &json!({"length":2,"width":3,"height":4,"body_name":"Box"})).unwrap();
        let before = s.doc.to_json();
        let history = (s.undo.len(), s.redo.len());
        let result = f.run(&mut s).unwrap();
        assert_eq!(result["result"]["frame"], "component-local-mm");
        assert!(!result["result"]["triangles"].as_array().unwrap().is_empty());
        assert_eq!(s.doc.to_json(), before);
        assert_eq!((s.undo.len(), s.redo.len()), history);
    }

    #[test]
    fn approval_binds_exact_bytes_and_disabled_plugins_never_execute() {
        let f = Fixture::new("call $box");
        let mut s = Session::default();
        let inspected = s.execute("plugin.inspect", &json!({"path":f.path()})).unwrap();
        let args = json!({"path":f.path(),"approval":inspected["approval"],"entry":"make"});
        let path = f.0.join("sample/plugin.json");
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.push(b' ');
        std::fs::write(&path, bytes).unwrap();
        assert!(s.execute("plugin.run", &args).unwrap_err().to_string().contains("approval"));
        s.disabled_plugins.insert("sample".into());
        assert!(f.run(&mut s).unwrap_err().to_string().contains("disabled"));
        assert!(s.doc.features.is_empty());
        solidcraft_io::vfs::scoped(Some(Default::default()), || {
            assert!(s.execute("plugin.inspect", &json!({"path":f.path()})).is_err());
        });
    }

    #[test]
    fn net_manifest_permission_is_refused_before_execution() {
        let f = Fixture::new("call $box");
        let mut m = manifest();
        m["runtime"]["permissions"] = json!({"net":true});
        std::fs::write(f.0.join("sample/plugin.json"), m.to_string()).unwrap();
        let mut s = Session::default();
        let before = s.doc.to_json();
        let error = f.run(&mut s).unwrap_err();
        assert!(matches!(error, EngineError::Plugin(craft_plugin::HostError::PermissionDenied { ref permission }) if permission == "net"));
        assert_eq!(s.doc.to_json(), before);
        assert!(s.undo.is_empty());
    }

    #[test]
    fn sockets_and_http_imports_are_absent_from_the_linker() {
        let (package, version) = API.split_once('@').unwrap();
        for import in [
            "wasi:sockets/network@0.2.0".to_string(),
            "wasi:http/outgoing-handler@0.2.0".into(),
            format!("{package}/fetch@{version}"),
            "fetch".into(),
        ] {
            let f = Fixture::new("call $box");
            let template = component_template();
            let (prefix, _) = template.rsplit_once(')').unwrap();
            let wat = format!("{prefix}\n(type $network-type (instance))\n(import \"{import}\" (instance $network (type $network-type)))\n)");
            std::fs::write(f.0.join("sample/plugin.wasm"), wat::parse_str(wat).unwrap()).unwrap();
            let mut s = Session::default();
            let before = s.doc.to_json();
            let error = f.run(&mut s).unwrap_err();
            assert!(matches!(error, EngineError::Plugin(craft_plugin::HostError::PermissionDenied { .. })), "{import}: {error}");
            assert!(error.to_string().contains(&import), "{error}");
            assert_eq!(s.doc.to_json(), before);
            assert!(s.undo.is_empty());
        }
    }

    #[test]
    fn declared_entry_inputs_are_discoverable_and_namespaced_calls_share_approval() {
        let f = Fixture::new("call $box");
        let mut m = manifest();
        let schema = json!({"type":"object","properties":{"targets":{"type":"array","items":{"type":"string"}}}});
        m["runtime"]["entries"] = json!([{"name":"make","effect":"edit","input_schema":schema,"slots":[{"kind":"menu","name":"tools"}]}]);
        std::fs::write(f.0.join("sample/plugin.json"), m.to_string()).unwrap();
        let mut s = Session::default();
        let inspection = s.execute("plugin.inspect", &json!({"path":f.path()})).unwrap();
        let listed = s.execute("plugin.list", &json!({})).unwrap();
        let entry = listed["plugins"].as_array().unwrap().iter().find(|p| p["manifest"]["id"] == "sample").unwrap();
        assert_eq!(entry["commands"], json!(["sample.make"]));
        assert_eq!(entry["manifest"]["runtime"]["entries"][0]["input_schema"], schema);
        let description = s.execute("engine.describe", &json!({"command":"sample.make"})).unwrap();
        assert_eq!(description["input"]["properties"]["params"], schema);
        assert_eq!(description["effect"], "edit");
        assert!(s.doc.features.is_empty());
        assert!(s.execute("sample.make", &json!({"params":{}})).is_err());
        s.execute("sample.make", &json!({"approval":inspection["approval"],"params":{"targets":[]}})).unwrap();
        assert_eq!(s.undo.len(), 1);
        assert_eq!(s.execute("inspect.measure", &json!({})).unwrap()["total"]["volume_mm3"], 24.);
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(s.doc.features.is_empty());
    }

    #[test]
    fn read_entries_cannot_edit_even_with_an_edit_command_grant() {
        let f = Fixture::new("call $box");
        let mut m = manifest();
        m["runtime"]["entries"] = json!([{"name":"make","effect":"read","input_schema":{"type":"object"}}]);
        std::fs::write(f.0.join("sample/plugin.json"), m.to_string()).unwrap();
        let mut s = Session::default();
        let before = s.doc.to_json();
        let error = f.run(&mut s).unwrap_err();
        assert!(
            matches!(error, EngineError::Plugin(craft_plugin::HostError::PermissionDenied { ref permission }) if permission.contains("read entry"))
        );
        assert_eq!(s.doc.to_json(), before);
        assert!(s.undo.is_empty());
    }
    fn format_component(code: &str, report: &Value) -> Vec<u8> {
        let (package, version) = API.split_once('@').unwrap();
        let template = include_str!("fixture.wat").replace("__PACKAGE__", package).replace("__VERSION__", version);
        let (types, _) = template.split_once(&format!("  (type $ty-{package}/engine@{version}")).unwrap();
        let text = report.to_string();
        let data: String = text.as_bytes().iter().map(|byte| format!("\\{byte:02x}")).collect();
        let length = text.len();
        let source = format!(
            r#"{types}
(type $engine-type (instance
  (alias outer 1 $json (type $json)) (export "json" (type (eq $json)))
  (alias outer 1 $plugin-error (type $error)) (export "plugin-error" (type (eq $error)))
  (type $result (result $json (error $error)))
  (export "run" (func (param "command" string) (param "args" $json) (result $result)))
))
(import "{package}/engine@{version}" (instance $engine (type $engine-type)))
(type $io-type (instance
  (alias outer 1 $plugin-error (type $error)) (export "plugin-error" (type (eq $error)))
  (export "input-name" (func (result string)))
  (export "read-input" (func (param "max-bytes" u32) (result (result (list u8) (error $error)))))
  (export "write-output" (func (param "data" (list u8)) (result (result (error $error)))))
))
(import "{package}/io@{version}" (instance $io (type $io-type)))
(alias export $engine "run" (func $run))
(alias export $io "input-name" (func $name))
(alias export $io "read-input" (func $read))
(alias export $io "write-output" (func $write))
(core module $allocation
  (memory (export "memory") 3)
  (func (export "realloc") (param i32 i32 i32 i32) (result i32) i32.const 65536)
)
(core instance $allocation (instantiate $allocation))
(alias core export $allocation "memory" (core memory $memory))
(alias core export $allocation "realloc" (core func $realloc))
(core func $run (canon lower (func $run) (memory $memory) (realloc $realloc)))
(core func $name (canon lower (func $name) (memory $memory) (realloc $realloc)))
(core func $read (canon lower (func $read) (memory $memory) (realloc $realloc)))
(core func $write (canon lower (func $write) (memory $memory) (realloc $realloc)))
(core instance $imports
  (export "run" (func $run)) (export "name" (func $name))
  (export "read" (func $read)) (export "write" (func $write)) (export "memory" (memory $memory))
)
(core module $main
  (import "host" "run" (func $run (param i32 i32 i32 i32 i32)))
  (import "host" "name" (func $name (param i32)))
  (import "host" "read" (func $read (param i32 i32)))
  (import "host" "write" (func $write (param i32 i32 i32)))
  (import "host" "memory" (memory 3))
  (data (i32.const 512) "solid.box")
  (data (i32.const 544) "model.mesh")
  (data (i32.const 1024) "{{\22length\22:2,\22width\22:3,\22height\22:4}}")
  (data (i32.const 2048) "{data}")
  (data (i32.const 4096) "exported bytes")
  (func $body i32.const 512 i32.const 9 i32.const 1024 i32.const 33 i32.const 256 call $run)
  (func (export "execute") (param i32 i32 i32 i32) (result i32)
    (local $count i32)
    {code}
    i32.const 0 i32.const 0 i32.store
    i32.const 4 i32.const 2048 i32.store
    i32.const 8 i32.const {length} i32.store
    i32.const 0
  )
)
(core instance $main (instantiate $main (with "host" (instance $imports))))
(alias core export $main "execute" (core func $execute))
(func $execute (param "entry" string) (param "params" $json) (result (result $json (error $plugin-error)))
  (canon lift (core func $execute) (memory $memory) (realloc $realloc)))
(instance $command (export "json" (type $json)) (export "plugin-error" (type $plugin-error)) (export "execute" (func $execute)))
(export "{package}/command@{version}" (instance $command))
)"#
        );
        wat::parse_str(source).unwrap()
    }

    fn importer_fixture(code: &str) -> Fixture {
        let f = Fixture::new("call $box");
        let mut m = manifest();
        m["runtime"]["entries"] = json!([{"name":"make","kind":"importer","extensions":["sample"],"input_schema":{"type":"object"}}]);
        m["runtime"]["commands"] = json!(["solid.box", "component.activate", "component.create"]);
        std::fs::write(f.0.join("sample/plugin.json"), m.to_string()).unwrap();
        let report = r#"{"counts":{"bodies":1,"losses":1},"items":[{"item":"Body1","code":"colour-omitted","message":"Colour was omitted"}]}"#;
        let bytes: String = report.bytes().map(|b| format!("\\{b:02x}")).collect();
        let code =
            format!("{code} i32.const 0 i32.const 0 i32.store i32.const 4 i32.const 3000 i32.store i32.const 8 i32.const {} i32.store", report.len());
        let template = component_template().replacen("      call $box\n      call $box\n", &format!("      {code}\n"), 1).replacen(
            "    (data (;2;)",
            &format!("    (data (i32.const 3000) \"{bytes}\")\n    (data (;2;)"),
            1,
        );
        let template = template
            .replace("\"file.export\"", "\"component.activate\"")
            .replace(
                "    (func $execute",
                "    (func $root (type 2) i32.const 2048 i32.const 18 i32.const 3500 i32.const 15 i32.const 0 call $run)\n    (func $execute",
            )
            .replace(
                "    (data (;2;)",
                r#"    (data (i32.const 3500) "{\22component\22:0}")
    (data (;2;)"#,
            );
        let template = template
            .replace(
                "    (func $root",
                "    (func $child (type 2) i32.const 3600 i32.const 16 i32.const 3650 i32.const 16 i32.const 0 call $run)\n    (func $root",
            )
            .replace(
                "    (data (;2;)",
                r#"    (data (i32.const 3600) "component.create")
    (data (i32.const 3650) "{\22name\22:\22Child\22}")
    (data (;2;)"#,
            );
        std::fs::write(f.0.join("sample/plugin.wasm"), wat::parse_str(template).unwrap()).unwrap();
        std::fs::write(f.0.join("part.sample"), b"untrusted input").unwrap();
        f
    }

    fn import_args(f: &Fixture, s: &mut Session) -> Value {
        let inspection = s.execute("plugin.inspect", &json!({"path":f.path()})).unwrap();
        json!({"path":f.0.join("part.sample").to_string_lossy(),"format":"sample.make","approval":inspection["approval"]})
    }

    #[test]
    fn plugin_import_opens_an_unsaved_component_and_returns_structured_losses() {
        let f = importer_fixture("call $box");
        let mut s = Session::default();
        s.execute("solid.box", &json!({"length":10,"width":10,"height":10})).unwrap();
        let args = import_args(&f, &mut s);
        s.param_check = crate::schema::ParamCheck::Strict;
        let result = s.execute("file.import", &args).unwrap();
        assert_eq!(result["format"], "sample.make");
        assert_eq!(result["loss_report"]["counts"]["losses"], 1);
        assert_eq!(result["loss_report"]["items"][0]["code"], "colour-omitted");
        assert_eq!(s.doc.name, "part");
        assert!(s.path.is_none());
        assert!(s.is_dirty());
        assert!(s.undo.is_empty());
        let state = s.model.state();
        assert_eq!(state.bodies.len(), 1);
        assert_ne!(state.bodies[0].component, 0);
        assert_eq!(s.execute("inspect.measure", &json!({})).unwrap()["total"]["volume_mm3"], 24.);
        assert_eq!(s.doc.features.len(), 1, "the host adds no plugin feature row");
    }

    #[test]
    fn plugin_imported_assembly_contents_are_rigid_until_unlocked() {
        let f = importer_fixture("call $child call $box");
        let mut s = Session::default();
        let args = import_args(&f, &mut s);
        s.execute("file.import", &args).unwrap();
        let component = s.doc.components.iter().find(|c| c.name == "part").unwrap().id;
        let child = s.doc.components.iter().find(|c| c.name == "Child").unwrap().id;
        let occurrence = s.doc.occurrence_of(child).unwrap().id;
        assert_eq!(s.contents_joints(component).len(), 1);
        assert!(
            s.execute("occurrence.move", &json!({"occurrence":occurrence,"translate":[1,0,0]})).unwrap_err().to_string().contains("Unlock Contents")
        );
        s.execute("component.unlock", &json!({"component":component})).unwrap();
        s.execute("occurrence.move", &json!({"occurrence":occurrence,"translate":[1,0,0]})).unwrap();
    }

    #[test]
    fn plugin_import_refuses_bodies_outside_the_file_component() {
        let f = importer_fixture("call $root call $box");
        let mut s = Session::default();
        let args = import_args(&f, &mut s);
        let before = s.doc.to_json();
        let error = s.execute("file.import", &args).unwrap_err();
        assert!(matches!(error, EngineError::Plugin(HostError::InvalidInput { ref field, .. }) if field == "design"));
        assert_eq!(s.doc.to_json(), before);
        assert!(s.undo.is_empty());
    }

    #[test]
    fn plugin_import_cancellation_preserves_the_open_design_and_undo_history() {
        let f = importer_fixture("call $box");
        let mut s = Session::default();
        s.execute("solid.box", &json!({"length":7,"width":8,"height":9})).unwrap();
        s.path = Some("original.sc".into());
        let args = import_args(&f, &mut s);
        let before = s.doc.to_json();
        let undo = s.undo.len();
        let progress = std::sync::Arc::new(solidcraft_doc::progress::Progress::default());
        let cancel = progress.clone();
        let entered = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let seen = entered.clone();
        s.set_command_check(Some(std::sync::Arc::new(move |id, _, _| {
            if id == "solid.box" {
                seen.store(true, Ordering::Relaxed);
                cancel.cancel();
            }
            Ok(())
        })));
        let result = solidcraft_doc::progress::watch(progress, || s.execute("file.import", &args));
        assert!(entered.load(Ordering::Relaxed), "cancellation happened inside the importer");
        assert!(result.is_err());
        assert_eq!(s.doc.to_json(), before);
        assert_eq!(s.undo.len(), undo);
        assert_eq!(s.path.as_deref(), Some("original.sc"));
    }

    #[test]
    fn plugin_extension_routing_is_case_insensitive_explicit_and_ignores_disabled_packages() {
        let f = importer_fixture("call $box");
        let mut s = Session::default();
        let args = import_args(&f, &mut s);
        let commands = s.execute("engine.commands", &json!({})).unwrap();
        assert!(commands.as_array().unwrap().iter().all(|entry| entry["id"] != "sample.make"));
        assert!(describe_entry(&s, "sample.make").unwrap().is_none());
        assert_eq!(resolve_format(&s, EntryKind::Importer, None, "PART.SAMPLE").unwrap().unwrap().id, "sample.make");
        assert!(resolve_format(&s, EntryKind::Exporter, None, "part.sample").unwrap().is_none());
        let mut args = args;
        args.as_object_mut().unwrap().remove("format");
        s.execute("doc.open", &args).unwrap();
        assert_eq!(s.model.state().bodies.len(), 1);
        let mut m = manifest();
        m["runtime"]["entries"] = json!([
            {"name":"make","kind":"importer","extensions":["sample"]},
            {"name":"other","kind":"importer","extensions":["sample"]}
        ]);
        std::fs::write(f.0.join("sample/plugin.json"), m.to_string()).unwrap();
        assert!(resolve_format(&s, EntryKind::Importer, None, "part.sample").unwrap_err().to_string().contains("multiple"));
        assert_eq!(resolve_format(&s, EntryKind::Importer, Some("sample.other"), "part.sample").unwrap().unwrap().entry, "other");
        s.disabled_plugins.insert("sample".into());
        assert!(formats(&s, EntryKind::Importer).iter().all(|f| f.plugin_id != "sample"));
    }

    #[test]
    fn plugin_format_ids_override_extensions_while_native_formats_keep_precedence() {
        let f = importer_fixture("call $box");
        let manifest_path = f.0.join("sample/plugin.json");
        let mut m: Value = serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
        m["runtime"]["entries"][0]["extensions"] = json!(["stl"]);
        std::fs::write(manifest_path, m.to_string()).unwrap();
        let path = f.0.join("part.stl");
        std::fs::write(&path, b"invalid STL input").unwrap();
        let mut s = Session::default();
        let approval = s.execute("plugin.inspect", &json!({"path":f.path()})).unwrap()["approval"].clone();
        let before = s.doc.to_json();
        assert!(s.execute("doc.open", &json!({"path":path.to_string_lossy(),"approval":approval})).is_err());
        assert_eq!(s.doc.to_json(), before);
        let result = s.execute("doc.open", &json!({"path":path.to_string_lossy(),"format":"sample.make","approval":approval})).unwrap();
        assert_eq!(result["format"], "sample.make");
        assert_eq!(s.model.state().bodies.len(), 1);
    }

    #[test]
    fn plugin_import_refuses_an_oversized_source_without_opening_a_design() {
        let f = importer_fixture("call $box");
        let mut s = Session::default();
        let args = import_args(&f, &mut s);
        std::fs::File::options().write(true).open(f.0.join("part.sample")).unwrap().set_len(craft_plugin::MAX_STREAM_BYTES + 1).unwrap();
        let before = s.doc.to_json();
        let error = s.execute("file.import", &args).unwrap_err();
        assert!(matches!(error, EngineError::Plugin(HostError::LimitExceeded { .. })));
        assert_eq!(s.doc.to_json(), before);
    }

    #[test]
    fn plugin_export_stream_is_published_without_editing_the_design() {
        let f = Fixture::new("");
        let mut m = manifest();
        m["runtime"]["entries"] = json!([{"name":"stl","kind":"exporter","extensions":["sample"]}]);
        std::fs::write(f.0.join("sample/plugin.json"), m.to_string()).unwrap();
        std::fs::write(f.0.join("sample/plugin.wasm"), format_component("i32.const 4096 i32.const 14 i32.const 128 call $write", &json!({})))
            .unwrap();
        let path = f.0.join("output.sample");
        let mut s = Session::default();
        s.execute("solid.box", &json!({"length":2,"width":3,"height":4})).unwrap();
        let approval = s.execute("plugin.inspect", &json!({"path":f.path()})).unwrap()["approval"].clone();
        let before = s.doc.to_json();
        let undo = s.undo.len();
        s.param_check = crate::schema::ParamCheck::Strict;
        let result = s.execute("file.export", &json!({"path":path.to_string_lossy(),"format":"sample.stl","approval":approval})).unwrap();
        assert_eq!(result["bytes"], 14);
        assert_eq!(result["format"], "sample.stl");
        assert_eq!(std::fs::read(path).unwrap(), b"exported bytes");
        let routed = s.execute("file.export", &json!({"path":f.0.join("routed.sample").to_string_lossy(),"approval":approval})).unwrap();
        assert_eq!(routed["format"], "sample.stl");
        assert_eq!(s.doc.to_json(), before);
        assert_eq!(s.undo.len(), undo);
    }

    #[test]
    fn missing_plugin_export_ids_do_not_fall_back_to_native_filename_formats() {
        let f = Fixture::new("");
        let path = f.0.join("output.stl");
        std::fs::write(&path, b"previous complete file").unwrap();
        let mut s = Session::default();
        s.execute("solid.box", &json!({"length":2,"width":3,"height":4})).unwrap();
        let before = s.doc.to_json();
        for extra in [json!({"approval":"missing"}), json!({"options":{}})] {
            let mut params = json!({"path":path.to_string_lossy(),"format":"missing-plugin.stl"});
            params.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            let error = s.execute("file.export", &params).unwrap_err();
            assert!(error.to_string().contains("unknown plugin format"));
            assert_eq!(std::fs::read(&path).unwrap(), b"previous complete file");
        }
        let reply = s.execute("file.export", &json!({"path":path.to_string_lossy(),"format":"part.stl"})).unwrap();
        assert_eq!(reply["format"], "StlBinary");
        assert_eq!(std::fs::read(path).unwrap().len(), 684, "a box has12 triangles:84+12*50 bytes");
        assert_eq!(s.doc.to_json(), before);
    }

    #[test]
    fn plugin_export_trap_preserves_the_previous_destination_and_design() {
        let f = Fixture::new("unreachable");
        let mut m = manifest();
        m["runtime"]["entries"] = json!([{"name":"make","kind":"exporter","extensions":["sample"]}]);
        std::fs::write(f.0.join("sample/plugin.json"), m.to_string()).unwrap();
        let path = f.0.join("output.sample");
        std::fs::write(&path, b"previous complete file").unwrap();
        let mut s = Session::default();
        let approval = s.execute("plugin.inspect", &json!({"path":f.path()})).unwrap()["approval"].clone();
        let before = s.doc.to_json();
        assert!(s.execute("file.export", &json!({"path":path.to_string_lossy(),"format":"sample.make","approval":approval})).is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"previous complete file");
        assert_eq!(s.doc.to_json(), before);
        assert!(s.undo.is_empty());
    }
}

#[test]
fn evaluated_mesh_transfer_is_bounded_and_does_not_edit_the_design() {
    let mut s = Session::default();
    s.execute("solid.box", &json!({"length":2,"width":3,"height":4,"body_name":"Box"})).unwrap();
    let before = s.doc.to_json();
    let undo = s.undo.len();
    let result = s.execute("model.mesh", &json!({"body":"Box"})).unwrap();
    assert_eq!(result["frame"], "component-local-mm");
    let positions: Vec<[f64; 3]> = serde_json::from_value(result["positions"].clone()).unwrap();
    let triangles: Vec<[u32; 3]> = serde_json::from_value(result["triangles"].clone()).unwrap();
    let volume: f64 = triangles
        .iter()
        .map(|t| {
            let a = solidcore_record::Vec3::from(positions[t[0] as usize]);
            let b = solidcore_record::Vec3::from(positions[t[1] as usize]);
            let c = solidcore_record::Vec3::from(positions[t[2] as usize]);
            a.dot(b.cross(c)) / 6.
        })
        .sum();
    assert!((volume.abs() - 24.).abs() < 1e-8);
    assert_eq!(s.doc.to_json(), before);
    assert_eq!(s.undo.len(), undo);
    assert!(s.execute("model.mesh", &json!({"body":"Box","tolerance":0})).is_err());
    assert!(s.execute("model.mesh", &json!({"body":"Missing"})).is_err());
}

#[test]
fn evaluated_mesh_transfer_refuses_an_oversized_input_without_truncation() {
    let mut doc = solidcraft_doc::Document::default();
    doc.add_feature(
        solidcraft_doc::FeatureKind::MeshImport {
            file: "budget.stl".into(),
            meshes: vec![solidcraft_doc::MeshData {
                name: "Many".into(),
                positions: (0..8193).flat_map(|i| [i as f64, (i % 2) as f64, 0.]).collect(),
                triangles: (0..2731).flat_map(|i| [3 * i, 3 * i + 1, 3 * i + 2]).collect(),
                color: None,
                corner_normals: Default::default(),
            }],
        },
        Some("Budget mesh"),
    )
    .unwrap();
    doc.features.last_mut().unwrap().body_names = vec!["Many".into()];
    let mut s = Session::new(doc);
    let before = s.doc.to_json();
    assert!(s.execute("model.mesh", &json!({"body":"Many"})).unwrap_err().to_string().contains("transfer budget"));
    assert_eq!(s.doc.to_json(), before);
}

#[test]
fn host_resource_policy_is_exact_user_state_and_never_design_data() {
    let defaults = HostLimits::default();
    assert_eq!((defaults.max_fuel, defaults.max_memory_mib), (100_000_000_000, 128));
    let request = Limits::default();
    assert_eq!((request.fuel, request.memory_mib, request.deadline_ms), (10_000_000, 64, 5000));
    assert!(defaults.check(&request).is_ok());
    for limits in
        [HostLimits { max_fuel: 0, ..defaults }, HostLimits { max_memory_mib: 0, ..defaults }, HostLimits { max_memory_mib: 4097, ..defaults }]
    {
        assert!(limits.validate().is_err());
    }
    let mut m = manifest();
    m["runtime"]["limits"] = json!({"fuel":u64::MAX,"memory_mib":4096,"deadline_ms":30000});
    assert!(parse_manifest(&serde_json::to_vec(&m).unwrap(), "sample").is_ok());
    let policy = HostLimits { max_fuel: u64::MAX, max_memory_mib: 4096 };
    assert_eq!(serde_json::from_str::<HostLimits>(&serde_json::to_string(&policy).unwrap()).unwrap(), policy);
    let mut s = Session { plugin_limits: policy, ..Default::default() };
    assert_eq!(s.scratch().plugin_limits, policy);
    s.execute("file.new", &json!({})).unwrap();
    assert_eq!(s.plugin_limits, policy);
    assert!(!s.doc.to_json().contains("max_fuel"));
    assert!(!allowed_command("plugin.configure"));
}

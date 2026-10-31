use super::*;
use serde_json::{Value, json};

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
    fn component(code: &str) -> Vec<u8> {
        let template = include_str!("fixture.wat").replace("\r\n", "\n");
        let original = "      call $box\n      call $box\n";
        assert!(template.contains(original));
        wat::parse_str(template.replacen(original, &format!("      {code}\n"), 1)).unwrap()
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
        let f = Fixture::new("call $box");
        let mut s = Session::default();
        let i = s.execute("plugin.inspect", &json!({"path":f.path()})).unwrap();
        let params = json!({"path":f.path(),"approval":i["approval"],"entry":"make"});
        let prepared = s.scratch().compute_commands(&[("plugin.run".into(), params)], None).unwrap();
        assert!(prepared.is_current(&s));
        s.plugin_limits.max_memory_mib = 512;
        assert!(!prepared.is_current(&s));
        let before = s.doc.to_json();
        assert!(s.accept_commands(prepared).is_err());
        assert_eq!(s.doc.to_json(), before);
        assert!(s.undo.is_empty());
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
        s.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(s.doc.to_json(), saved);
        drop(f);
        let mut reopened = Session::new(solidcraft_doc::Document::from_json(&saved).unwrap());
        assert_eq!(reopened.execute("inspect.measure", &json!({})).unwrap()["total"]["volume_mm3"], 72.);
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
    fn epoch_interrupt_is_distinct_from_fuel_exhaustion_in_diagnostics() {
        let mut config = wasmtime::Config::new();
        config.consume_fuel(true).epoch_interruption(true);
        let engine = wasmtime::Engine::new(&config).unwrap();
        let module = wasmtime::Module::from_binary(&engine, &wat::parse_str(r#"(module (func (export "run") (loop br 0)))"#).unwrap()).unwrap();
        let mut store = wasmtime::Store::new(&engine, ());
        let limits = Limits { fuel: 100_000, memory_mib: 1, deadline_ms: 5000 };
        store.set_fuel(limits.fuel).unwrap();
        store.set_epoch_deadline(1);
        let instance = wasmtime::Instance::new(&mut store, &module, &[]).unwrap();
        let run = instance.get_typed_func::<(), ()>(&mut store, "run").unwrap();
        // Advance the engine explicitly: no sleeps or wall-clock race in this test.
        engine.increment_epoch();
        let failure = run.call(&mut store, ()).unwrap_err();
        let remaining = store.get_fuel().unwrap();
        assert!(remaining > 0);
        let message = crate::plugin::runtime::execution_error(&failure, Some(remaining), &limits).to_string();
        assert!(message.contains("trap=Interrupt"), "{message}");
        assert!(!message.contains("OutOfFuel"), "{message}");
        assert!(message.contains(&format!("fuel_remaining={remaining}")), "{message}");
    }
    #[test]
    fn a_granted_component_reads_only_the_evaluated_mesh_and_preserves_history() {
        let f = Fixture::new("call $box");
        let template = include_str!("fixture.wat").replace("\r\n", "\n");
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
            let a = solidcraft_geom::Vec3::from(positions[t[0] as usize]);
            let b = solidcraft_geom::Vec3::from(positions[t[1] as usize]);
            let c = solidcraft_geom::Vec3::from(positions[t[2] as usize]);
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

fn importer_manifest(extensions: &[&str]) -> Value {
    let formats: Vec<Value> = extensions.iter().map(|e| json!({"extension": e, "name": format!("Test {e}")})).collect();
    json!({"id":"fixture-importer", "name":"Fixture importer", "version":"0.1.0", "runtime":{
        "api":API,"module":"plugin.wasm","kind":"importer","formats":formats,
        "limits":{"fuel":10_000_000,"memory_mib":4,"deadline_ms":5000}}})
}

#[test]
fn an_importer_declares_formats_and_is_granted_no_commands() {
    let m = parse_manifest(&serde_json::to_vec(&importer_manifest(&[".FIXD", "fixs"])).unwrap(), "fixture-importer").unwrap();
    let runtime = m.runtime.unwrap();
    assert_eq!(runtime.kind, Kind::Importer);
    assert_eq!(runtime.formats.iter().map(|f| f.extension.as_str()).collect::<Vec<_>>(), ["fixd", "fixs"]);
    for mutate in [
        |m: &mut Value| m["runtime"]["commands"] = json!(["solid.box"]),
        |m: &mut Value| m["runtime"]["entries"] = json!(["make"]),
        |m: &mut Value| m["runtime"]["formats"] = json!([]),
        |m: &mut Value| m["runtime"]["formats"] = json!([{"extension":"a/b","name":"Slash"}]),
        |m: &mut Value| m["runtime"]["formats"] = json!([{"extension":"fixd","name":"A"},{"extension":"FIXD","name":"B"}]),
        |m: &mut Value| m["runtime"]["formats"][0]["reads_folder"] = json!(true),
    ] {
        let mut m = importer_manifest(&["fixd"]);
        mutate(&mut m);
        assert!(parse_manifest(&serde_json::to_vec(&m).unwrap(), "fixture-importer").is_err(), "{m}");
    }
    // A command plugin cannot claim file formats.
    let mut command = manifest();
    command["runtime"]["formats"] = json!([{"extension":"fixd","name":"Test"}]);
    assert!(parse_manifest(&serde_json::to_vec(&command).unwrap(), "sample").is_err());
}

#[cfg(all(feature = "plugins", any(target_arch = "x86_64", target_arch = "aarch64")))]
mod importer {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);

    /// Hands the file's bytes back as the payload `case` (0 design, 1 STEP), with the file
    /// name it was given as its notes.
    fn echo(case: u8) -> String {
        format!(
            "i32.const 0 i32.const 0 i32.store8 i32.const 4 i32.const {case} i32.store8 \
             i32.const 8 local.get $bytes i32.store i32.const 12 local.get $bytes_len i32.store \
             i32.const 16 local.get $name i32.store i32.const 20 local.get $name_len i32.store i32.const 0"
        )
    }
    fn component(body: &str, extra_import: &str) -> Vec<u8> {
        let template = include_str!("importer_fixture.wat").replace("\r\n", "\n");
        assert!(template.contains(";; BODY") && template.contains(";; EXTRA-IMPORT"));
        wat::parse_str(template.replacen(";; BODY", body, 1).replacen(";; EXTRA-IMPORT", extra_import, 1)).unwrap()
    }

    /// A plugins folder, a package folder to install from, and files to import.
    struct Setup {
        root: std::path::PathBuf,
    }
    impl Setup {
        fn new(body: &str, extensions: &[&str]) -> Self {
            Self::with_import(body, extensions, "")
        }
        fn with_import(body: &str, extensions: &[&str], extra_import: &str) -> Self {
            let root =
                solidcraft_io::vfs::temp_dir().join(format!("solidcraft-importer-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            let package = root.join("package/fixture-importer");
            std::fs::create_dir_all(&package).unwrap();
            std::fs::create_dir_all(root.join("plugins")).unwrap();
            std::fs::write(package.join("plugin.json"), serde_json::to_vec(&importer_manifest(extensions)).unwrap()).unwrap();
            std::fs::write(package.join("plugin.wasm"), component(body, extra_import)).unwrap();
            Self { root }
        }
        fn file(&self, name: &str) -> String {
            self.root.join(name).to_string_lossy().into_owned()
        }
        /// A session with the importer installed, and allowed when `allow`.
        fn session(&self, allow: bool) -> Session {
            let mut s = Session { plugins_dir: Some(self.root.join("plugins")), ..Default::default() };
            let installed = s.execute("plugin.install", &json!({"path": self.file("package/fixture-importer"), "replace": true})).unwrap();
            assert_eq!(installed["allowed"], false, "installing allows nothing");
            if allow {
                s.plugin_approvals.insert("fixture-importer".into(), installed["approval"].as_str().unwrap().to_string());
            }
            s
        }
    }
    impl Drop for Setup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn boxed() -> Session {
        let mut s = Session::default();
        s.execute("solid.box", &json!({"length":20,"width":10,"height":3})).unwrap();
        s
    }

    #[test]
    fn an_allowed_importer_is_picked_by_extension_and_its_design_keeps_its_history() {
        let f = Setup::new(&echo(0), &["fixd"]);
        let mut source = boxed();
        source.execute("file.save", &json!({"path": f.file("source.sc")})).unwrap();
        std::fs::copy(f.file("source.sc"), f.file("part.fixd")).unwrap();

        // Installed but not allowed: nothing runs, and the design on screen stays.
        let mut s = f.session(false);
        s.execute("solid.box", &json!({"length":1,"width":1,"height":1})).unwrap();
        let before = s.doc.to_json();
        assert!(s.execute("file.import", &json!({"path": f.file("part.fixd")})).unwrap_err().to_string().contains("Preferences > Plugins"));
        assert_eq!(s.doc.to_json(), before);

        let mut s = f.session(true);
        for command in ["doc.open", "file.import"] {
            let out = s.execute(command, &json!({"path": f.file("part.fixd")})).unwrap();
            assert_eq!(out["plugin"], "fixture-importer", "{command}: {out}");
            assert_eq!(out["payload"], "design");
            // The importer is given the file's name, never its folders.
            assert_eq!(out["notes"], "part.fixd");
            assert_eq!(s.doc.features.len(), source.doc.features.len());
            assert_eq!(s.execute("inspect.measure", &json!({})).unwrap()["total"]["volume_mm3"], 600.);
            assert!(s.path.is_none(), "an import is a new, unsaved design");
        }
        // Turned off in Preferences: no longer picked.
        s.disabled_plugins.insert("fixture-importer".into());
        assert!(s.execute("file.import", &json!({"path": f.file("part.fixd")})).is_err());
    }

    #[test]
    fn a_step_payload_imports_as_geometry_and_native_formats_keep_their_own_path() {
        let f = Setup::new(&echo(1), &["fixs", "step"]);
        let mut source = boxed();
        source.execute("file.export", &json!({"path": f.file("source.step")})).unwrap();
        std::fs::copy(f.file("source.step"), f.file("part.fixs")).unwrap();
        let mut s = f.session(true);
        let out = s.execute("file.import", &json!({"path": f.file("part.fixs")})).unwrap();
        assert_eq!(out["payload"], "step");
        assert_eq!(s.execute("inspect.measure", &json!({})).unwrap()["total"]["volume_mm3"].as_f64().map(|v| (v - 600.).abs() < 1e-6), Some(true));
        // SolidCraft reads STEP itself; the plugin's claim does not take it over.
        let native = s.execute("file.import", &json!({"path": f.file("source.step")})).unwrap();
        assert!(native.get("plugin").is_none(), "{native}");
        // Unless the caller names the importer.
        let named = s.execute("file.import", &json!({"path": f.file("source.step"), "plugin": "fixture-importer"})).unwrap();
        assert_eq!(named["plugin"], "fixture-importer");
    }

    #[test]
    fn a_trapping_or_over_budget_importer_leaves_the_design_untouched() {
        for body in [
            "unreachable",
            "(loop $spin (br $spin)) i32.const 0",
            "i32.const 1024 memory.grow drop i32.const 0",
            // A result pointing outside the importer's memory.
            "i32.const 0 i32.const 0 i32.store8 i32.const 8 i32.const -16 i32.store i32.const 12 i32.const 64 i32.store i32.const 0",
        ] {
            let f = Setup::new(body, &["fixd"]);
            std::fs::write(f.file("part.fixd"), b"not a design").unwrap();
            let mut s = f.session(true);
            s.execute("solid.box", &json!({"length":2,"width":3,"height":4})).unwrap();
            let (before, undo) = (s.doc.to_json(), s.undo.len());
            assert!(s.execute("doc.open", &json!({"path": f.file("part.fixd")})).is_err(), "{body}");
            assert_eq!(s.doc.to_json(), before, "{body}");
            assert_eq!(s.undo.len(), undo);
            // The session still works.
            s.execute("solid.box", &json!({"length":1,"width":1,"height":1})).unwrap();
        }
        // An importer whose design does not read is refused the same way.
        let f = Setup::new(&echo(0), &["fixd"]);
        std::fs::write(f.file("part.fixd"), b"not a design").unwrap();
        let mut s = f.session(true);
        let before = s.doc.to_json();
        assert!(s.execute("doc.open", &json!({"path": f.file("part.fixd")})).is_err());
        assert_eq!(s.doc.to_json(), before);
    }

    #[test]
    fn an_importer_cannot_reach_files_or_anything_beyond_its_world() {
        // A component asking for engine commands (or anything else outside the importer world
        // and its empty WASI) never starts.
        for import in [
            r#"(import "solidcraft:plugin/engine@0.1.0" (instance (export "run" (func (param "command" string) (result string)))))"#,
            r#"(import "wasi:http/outgoing-handler@0.2.0" (instance (export "handle" (func (result u32)))))"#,
        ] {
            let f = Setup::with_import(&echo(0), &["fixd"], import);
            std::fs::write(f.file("part.fixd"), b"x").unwrap();
            let mut s = f.session(true);
            let before = s.doc.to_json();
            let err = s.execute("file.import", &json!({"path": f.file("part.fixd")})).unwrap_err().to_string();
            assert!(err.contains("solidcraft:plugin/engine") || err.contains("wasi:http"), "{err}");
            assert_eq!(s.doc.to_json(), before);
        }
        // Its WASI has no folders, so no path opens, whatever it asks for.
        assert!(super::runtime::empty_wasi_has_no_folders());
        // Only the file's name crosses, never the folders it is in.
        let f = Setup::new(&echo(0), &["fixd"]);
        let mut source = boxed();
        std::fs::create_dir_all(f.root.join("deep")).unwrap();
        source.execute("file.save", &json!({"path": f.file("deep/part.fixd"), "format": "container"})).unwrap();
        let mut s = f.session(true);
        let out = s.execute("file.import", &json!({"path": f.file("deep/part.fixd")})).unwrap();
        assert_eq!(out["notes"], "part.fixd");
    }
}

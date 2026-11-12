//! The registry remains valid JSON and a closed pipeline reader must not cause a panic.

use std::process::{Command, Stdio};

#[test]
fn registry_output_is_complete_json() {
    let output = Command::new(env!("CARGO_BIN_EXE_solidcraft-cli")).arg("commands").output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let registry: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let commands = registry.as_array().unwrap();
    assert!(commands.iter().any(|command| command["id"] == "solid.box"));
    assert!(commands.iter().any(|command| command["id"] == "engine.commands"));
    assert!(output.stdout.ends_with(b"\n"));
    assert!(output.stderr.is_empty());
}

#[test]
fn closed_registry_pipe_exits_without_panicking() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_solidcraft-cli")).arg("commands").stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    // Close our read end before waiting. No sleeps or timing threshold are needed:
    // the registry is larger than the pipe buffer, so it cannot all be written first.
    drop(child.stdout.take());
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "status: {}; stderr: {}", output.status, String::from_utf8_lossy(&output.stderr));
    assert!(output.stderr.is_empty(), "{}", String::from_utf8_lossy(&output.stderr));
}

struct PluginFixture(std::path::PathBuf);
impl PluginFixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "solidcraft-cli-plugin-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let dir = root.join("sample");
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = serde_json::json!({"id":"sample","name":"Sample","version":"1","runtime":{
            "api":craft_plugin::API,"module":"plugin.wasm","entries":["make","other"],"commands":["solid.box"]}});
        std::fs::write(dir.join("plugin.json"), manifest.to_string()).unwrap();
        let (package, version) = craft_plugin::API.split_once('@').unwrap();
        let component = include_str!("../../../crates/engine/src/plugin/fixture.wat").replace("__PACKAGE__", package).replace("__VERSION__", version);
        std::fs::write(dir.join("plugin.wasm"), wat::parse_str(component).unwrap()).unwrap();
        Self(root)
    }
    fn dir(&self) -> std::path::PathBuf {
        self.0.join("sample")
    }
    fn manifest(&self, change: impl FnOnce(&mut serde_json::Value)) {
        let path = self.dir().join("plugin.json");
        let mut value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        change(&mut value);
        std::fs::write(path, value.to_string()).unwrap();
    }
    fn run(&self, action: &str, args: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_solidcraft-cli")).args(["plugin", action]).arg(self.dir()).args(args).output().unwrap()
    }
}
impl Drop for PluginFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn plugin_test_reports_all_entries_and_saves_designs_and_command_journals() {
    let fixture = PluginFixture::new();
    let checked = fixture.run("check", &[]);
    assert!(checked.status.success(), "{} {}", String::from_utf8_lossy(&checked.stdout), String::from_utf8_lossy(&checked.stderr));
    let relative = Command::new(env!("CARGO_BIN_EXE_solidcraft-cli")).current_dir(fixture.dir()).args(["plugin", "check", "."]).output().unwrap();
    assert!(relative.status.success(), "{} {}", String::from_utf8_lossy(&relative.stdout), String::from_utf8_lossy(&relative.stderr));
    let output = fixture.run("test", &[]);
    assert!(output.status.success(), "{} {}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["passed"], true);
    assert_eq!(report["entries"].as_array().unwrap().len(), 2);
    for entry in report["entries"].as_array().unwrap() {
        assert_eq!(entry["passed"], true);
        assert!(entry["error"].is_null());
        let design = std::path::Path::new(entry["design"].as_str().unwrap());
        assert!(design.is_file());
        let (saved, _) = solidcraft_engine::io::read_design_file(design.to_str().unwrap(), None::<fn(_)>).unwrap();
        assert_eq!(saved.features.len(), 2, "each entry starts from the same empty design");
        let journal: serde_json::Value = serde_json::from_slice(&std::fs::read(entry["journal"].as_str().unwrap()).unwrap()).unwrap();
        assert_eq!(journal.as_array().unwrap().len(), 2);
        assert_eq!(journal[0]["command"], "solid.box");
        assert!(journal[0]["error"].is_null());
        let inspect = Command::new(env!("CARGO_BIN_EXE_solidcraft-cli")).arg("eval").arg(design).output().unwrap();
        assert!(inspect.status.success(), "{}", String::from_utf8_lossy(&inspect.stderr));
    }
    let first = &report["entries"][0];
    let selected = fixture.run("test", &["--design", first["design"].as_str().unwrap(), "--entry", "other", "--input", "{}"]);
    assert!(selected.status.success(), "{}", String::from_utf8_lossy(&selected.stderr));
    let selected: serde_json::Value = serde_json::from_slice(&selected.stdout).unwrap();
    assert_eq!(selected["entries"].as_array().unwrap().len(), 1);
    assert_eq!(selected["entries"][0]["entry"], "other");
    let (saved, _) = solidcraft_engine::io::read_design_file(selected["entries"][0]["design"].as_str().unwrap(), None::<fn(_)>).unwrap();
    assert_eq!(saved.features.len(), 4, "--design supplies the initial model");
    for result in [&report, &selected] {
        for entry in result["entries"].as_array().unwrap() {
            let _ = std::fs::remove_dir_all(std::path::Path::new(entry["design"].as_str().unwrap()).parent().unwrap());
        }
    }
}

#[test]
fn plugin_test_returns_failure_with_typed_error_and_rolled_back_artifacts() {
    let fixture = PluginFixture::new();
    let (package, version) = craft_plugin::API.split_once('@').unwrap();
    let wat = include_str!("../../../crates/engine/src/plugin/fixture.wat")
        .replace("__PACKAGE__", package)
        .replace("__VERSION__", version)
        .replace("      call $box\n      call $box", "      call $box\n      unreachable");
    std::fs::write(fixture.dir().join("plugin.wasm"), wat::parse_str(wat).unwrap()).unwrap();
    let output = fixture.run("test", &["--entry", "make"]);
    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["passed"], false);
    let entry = &report["entries"][0];
    assert_eq!(entry["error"]["kind"], "trap");
    let journal: serde_json::Value = serde_json::from_slice(&std::fs::read(entry["journal"].as_str().unwrap()).unwrap()).unwrap();
    assert_eq!(journal.as_array().unwrap().len(), 1);
    let (design, _) = solidcraft_engine::io::read_design_file(entry["design"].as_str().unwrap(), None::<fn(_)>).unwrap();
    assert!(design.features.is_empty());
    let _ = std::fs::remove_dir_all(std::path::Path::new(entry["design"].as_str().unwrap()).parent().unwrap());
}

#[test]
fn plugin_check_names_net_permission_and_socket_import_without_executing() {
    let fixture = PluginFixture::new();
    fixture.manifest(|m| m["runtime"]["permissions"] = serde_json::json!({"net":true}));
    let output = fixture.run("check", &[]);
    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["problems"][0]["field"], "runtime.permissions.net");
    assert_eq!(report["problems"][0]["error"]["kind"], "permission_denied");
    fixture.manifest(|m| m["runtime"]["permissions"] = serde_json::json!({}));
    let component = wat::parse_str(r#"(component (import "wasi:sockets/tcp@0.2.0" (instance)))"#).unwrap();
    std::fs::write(fixture.dir().join("plugin.wasm"), component).unwrap();
    let output = fixture.run("check", &[]);
    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        report["problems"]
            .as_array()
            .unwrap()
            .iter()
            .any(|problem| problem["field"] == "wasi:sockets/tcp@0.2.0" && problem["error"]["kind"] == "permission_denied")
    );
}

#[test]
fn plugin_api_schema_output_matches_engine_descriptions() {
    let output = Command::new(env!("CARGO_BIN_EXE_solidcraft-cli")).args(["commands", "--schemas"]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let schemas: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();
    assert!(schemas.windows(2).all(|pair| pair[0]["id"].as_str().unwrap() < pair[1]["id"].as_str().unwrap()));
    let mut session = solidcraft_engine::Session::default();
    let description = session.execute("engine.describe", &serde_json::json!({"command":"solid.box"})).unwrap();
    assert!(schemas.contains(&description));
}

#[cfg(unix)]
#[test]
fn plugin_test_non_utf8_artifact_directory_returns_typed_json_instead_of_panicking() {
    use std::os::unix::ffi::OsStringExt;
    let fixture = PluginFixture::new();
    let directory = fixture.0.join(std::ffi::OsString::from_vec(vec![b't', 0xff]));
    std::fs::create_dir(&directory).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_solidcraft-cli"))
        .args(["plugin", "test"])
        .arg(fixture.dir())
        .args(["--entry", "make"])
        .env("TMPDIR", directory)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["entries"][0]["error"]["kind"], "invalid_input");
    assert_eq!(report["entries"][0]["error"]["field"], "artifacts");
    assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked"));
}

use super::*;
use serde_json::json;

fn manifest() -> Value {
    json!({"id":"sample","name":" Sample ","version":" 1 ","runtime":{
        "api":API,"module":"plugin.wasm","entries":["make"],"commands":["app.make"]
    }})
}
fn parse(value: &Value) -> Result<Manifest> {
    parse_manifest(&serde_json::to_vec(value).unwrap(), "sample")
}

#[test]
fn metadata_normalization_and_legacy_entries() {
    let parsed = parse(&manifest()).unwrap();
    assert_eq!(parsed.name, "Sample");
    assert_eq!(parsed.version, "1");
    let runtime = parsed.runtime.unwrap();
    assert_eq!(runtime.entries[0].name, "make");
    assert_eq!(runtime.entries[0].effect, Effect::Edit);
    let serialized = serde_json::to_value(&runtime.entries[0]).unwrap();
    assert_eq!(serialized["input_schema"]["type"], "object");
    assert_eq!(serde_json::from_value::<Entry>(serialized).unwrap().name, "make");
    assert!(parse_manifest(br#"{"id":"sample","name":"Sample","version":"1"}"#, "sample").unwrap().runtime.is_none());
}

#[test]
fn declared_inputs_effects_and_slots_are_readable() {
    let mut value = manifest();
    value["runtime"]["entries"] = json!([{"name":"inspect","effect":"read",
        "input_schema":{"type":"object","properties":{"selection":{"type":"array","items":{"type":"string"}}}},
        "slots":[{"kind":"menu","name":"tools"},{"kind":"ribbon","name":"utilities"}]}]);
    let entry = parse(&value).unwrap().runtime.unwrap().entries.remove(0);
    assert_eq!(entry.effect, Effect::Read);
    assert_eq!(entry.slots.len(), 2);
    assert_eq!(entry.input_schema["properties"]["selection"]["type"], "array");
    value["runtime"]["entries"][0]["input_schema"]["properties"]["selection"]["type"] = json!("string");
    assert!(matches!(parse(&value), Err(HostError::InvalidManifest { field, .. }) if field.ends_with("selection")));
}

#[test]
fn network_permission_is_a_typed_refusal() {
    let mut value = manifest();
    value["runtime"]["permissions"] = json!({"net":true});
    assert_eq!(parse(&value).unwrap_err(), HostError::PermissionDenied { permission: "net".into() });
}

#[test]
fn named_files_permissions_require_access_and_unique_names() {
    let mut value = manifest();
    value["runtime"]["permissions"] = json!({"files":[{"name":"inputs","read":true}],"clock":true});
    let permissions = parse(&value).unwrap().runtime.unwrap().permissions;
    assert!(permissions.clock);
    assert!(permissions.files[0].read);
    value["runtime"]["permissions"]["files"] = json!([{"name":"inputs"}]);
    assert!(matches!(parse(&value), Err(HostError::InvalidManifest { .. })));
    value["runtime"]["permissions"]["files"] = json!([{"name":"inputs","read":true},{"name":"inputs","write":true}]);
    assert!(matches!(parse(&value), Err(HostError::InvalidManifest { .. })));
}

#[test]
fn hostile_manifest_fields_return_errors() {
    for change in [
        |value: &mut Value| value["runtime"]["module"] = json!("../plugin.wasm"),
        |value: &mut Value| value["runtime"]["entries"] = json!(["make", "make"]),
        |value: &mut Value| value["runtime"]["limits"] = json!({"fuel":0,"memory_mib":64,"deadline_ms":1}),
        |value: &mut Value| value["runtime"]["limits"] = json!({"fuel":1,"memory_mib":4097,"deadline_ms":1}),
        |value: &mut Value| value["runtime"]["limits"] = json!({"fuel":1,"memory_mib":1,"deadline_ms":30001}),
        |value: &mut Value| value["runtime"]["entries"] = json!([{"name":"make","input_schema":true}]),
    ] {
        let mut value = manifest();
        change(&mut value);
        assert!(matches!(parse(&value), Err(HostError::InvalidManifest { .. })));
    }
    assert!(matches!(parse_manifest(&vec![b' '; MAX_MANIFEST_BYTES + 1], "sample"), Err(HostError::LimitExceeded { .. })));
    assert!(parse_manifest(br#"{}"#, "sample").is_err());
}

#[test]
fn host_limits_have_typed_failures() {
    let limits = HostLimits::default();
    assert!(limits.check(&Limits::default()).is_ok());
    assert!(
        matches!(limits.check(&Limits { fuel: limits.max_fuel + 1, ..Limits::default() }), Err(HostError::LimitExceeded { limit }) if limit.contains("fuel"))
    );
    assert!(
        matches!(limits.check(&Limits { memory_mib: limits.max_memory_mib + 1, ..Limits::default() }), Err(HostError::LimitExceeded { limit }) if limit.contains("memory"))
    );
    assert!(HostLimits { max_fuel: 0, ..limits }.validate().is_err());
}

#[test]
fn host_errors_are_machine_readable() {
    let error = HostError::CommandFailed { command: "app.make".into(), reason: "missing part".into() };
    let value = serde_json::to_value(&error).unwrap();
    assert_eq!(value["kind"], "command_failed");
    assert_eq!(value["command"], "app.make");
    assert_eq!(serde_json::from_value::<HostError>(value).unwrap(), error);
    assert!(WIT.contains(&format!("package {API};")));
}

#[test]
fn plugin_check_validates_schemas_and_names_manifest_fields() {
    for schema in [
        json!({"type":"object","properties":{"x":{"type":"banana"}}}),
        json!({"type":"object","required":"x"}),
        json!({"type":"object","properties":{"x":{"$ref":"https://example.invalid/schema"}}}),
        json!({"type":"object","properties":{"x":{"$ref":"file:///tmp/schema"}}}),
    ] {
        let mut value = manifest();
        value["runtime"]["entries"] = json!([{"name":"make","input_schema":schema}]);
        let problem = check_manifest(&serde_json::to_vec(&value).unwrap(), "sample").unwrap_err();
        assert!(problem.field.contains("input_schema"), "{problem:?}");
    }
    let mut value = manifest();
    value["runtime"]["permissions"] = json!({"net":true});
    let problem = check_manifest(&serde_json::to_vec(&value).unwrap(), "sample").unwrap_err();
    assert_eq!(problem.field, "runtime.permissions.net");
    value["runtime"]["permissions"] = json!({"clock":"yes"});
    let problem = check_manifest(&serde_json::to_vec(&value).unwrap(), "sample").unwrap_err();
    assert_eq!(problem.field, "runtime.permissions.clock");
    assert!(parse_manifest(format!("{} false", manifest()).as_bytes(), "sample").is_err());
}

#[derive(Default)]
struct HarnessHost {
    value: u32,
    checkpoint: u32,
}
impl PluginHost for HarnessHost {
    fn command_catalog(&self) -> Vec<CommandSchema> {
        Vec::new()
    }
    fn run_command(&mut self, name: &str, _: &Value) -> Result<Value> {
        self.value += 1;
        if name == "fail" { Err(HostError::CommandFailed { command: name.into(), reason: "fixture failure".into() }) } else { Ok(json!(self.value)) }
    }
    fn begin_transaction(&mut self) -> Result<()> {
        self.checkpoint = self.value;
        Ok(())
    }
    fn commit_transaction(&mut self) -> Result<()> {
        Ok(())
    }
    fn rollback_transaction(&mut self) -> Result<()> {
        self.value = self.checkpoint;
        Ok(())
    }
    fn granted_folders(&self) -> Vec<FileGrant> {
        Vec::new()
    }
    fn log(&mut self, _: &str) {}
    fn progress(&mut self, _: f32, _: &str) {}
}

#[test]
fn plugin_test_isolates_entries_and_preserves_typed_errors_and_journals() {
    let mut value = manifest();
    value["runtime"]["entries"] = json!(["pass", "fail", "last"]);
    let manifest = parse(&value).unwrap();
    let mut saved = Vec::new();
    let report = test_entries(
        &manifest,
        None,
        &json!({}),
        || Ok(JournalHost::new(HarnessHost::default())),
        |mut host, entry, input| {
            host.begin_transaction().unwrap();
            let result = host.run_command(entry, input);
            if result.is_ok() {
                host.commit_transaction().unwrap();
            } else {
                host.rollback_transaction().unwrap();
            }
            (host, result)
        },
        |host, entry| {
            saved.push((host.host.value, host.journal.clone()));
            Ok(TestArtifacts { design: format!("{entry}.design").into(), journal: format!("{entry}.json").into() })
        },
    )
    .unwrap();
    let json = serde_json::to_value(report).unwrap();
    assert_eq!(json["passed"], false);
    assert_eq!(json["entries"].as_array().unwrap().len(), 3);
    assert_eq!(json["entries"][0]["output"], 1);
    assert_eq!(json["entries"][1]["error"]["kind"], "command_failed");
    assert_eq!(json["entries"][1]["design"], "fail.design");
    assert_eq!(json["entries"][2]["output"], 1);
    assert_eq!(saved[1].0, 0);
    assert!(saved[1].1[0].error.is_some());
    assert!(
        test_entries(
            &manifest,
            Some("missing"),
            &json!({}),
            || Ok(HarnessHost::default()),
            |host, _, _| (host, Ok(json!({}))),
            |_, _| panic!("unknown entry must not save artifacts")
        )
        .is_err()
    );
}

#[test]
fn plugin_test_validates_inputs_before_running_and_saves_failed_design() {
    let mut value = manifest();
    value["runtime"]["entries"] = json!([{"name":"make","input_schema":{
        "type":"object","properties":{"count":{"type":"integer","minimum":1}},"required":["count"],"additionalProperties":false
    }}]);
    let manifest = parse(&value).unwrap();
    let report = test_entries(
        &manifest,
        Some("make"),
        &json!({"count":0}),
        || Ok(HarnessHost::default()),
        |_, _, _| panic!("invalid input must never run"),
        |host, _| {
            assert_eq!(host.value, 0);
            Ok(TestArtifacts { design: "unchanged.design".into(), journal: "empty.json".into() })
        },
    )
    .unwrap();
    assert!(!report.passed);
    assert!(matches!(&report.entries[0].error, Some(HostError::InvalidInput { field, .. }) if field == "/count"));
    assert!(report.entries[0].artifacts.is_some());
}

#[test]
fn plugin_journal_limit_refuses_more_calls_before_running_the_command() {
    let mut host = JournalHost::new(HarnessHost::default());
    for _ in 0..256 {
        host.run_command("pass", &json!({})).unwrap();
    }
    assert!(matches!(host.run_command("pass", &json!({})), Err(HostError::LimitExceeded { .. })));
    assert_eq!(host.host.value, 256);
    assert_eq!(host.journal.len(), 256);
}

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
fn importer_and_exporter_declarations_roundtrip_and_enforce_rights() {
    let mut value = manifest();
    value["runtime"]["entries"] = json!([
        {"name":"read-part","kind":"importer","extensions":["part","assembly"],
         "input_schema":{"type":"object","properties":{"units":{"type":"string"}}}},
        {"name":"write-part","kind":"exporter","extensions":["part"]}
    ]);
    let runtime = parse(&value).unwrap().runtime.unwrap();
    assert_eq!(runtime.entries[0].kind, EntryKind::Importer);
    assert_eq!(runtime.entries[0].effect, Effect::Edit);
    assert_eq!(runtime.entries[1].kind, EntryKind::Exporter);
    assert_eq!(runtime.entries[1].effect, Effect::Read);
    let serialized = serde_json::to_value(&runtime.entries[1]).unwrap();
    assert_eq!(serde_json::from_value::<Entry>(serialized).unwrap().extensions, ["part"]);
    for extensions in [json!([]), json!([".part"]), json!(["PART"]), json!(["part", "part"]), json!(["../part"])] {
        value["runtime"]["entries"][0]["extensions"] = extensions;
        assert!(matches!(parse(&value), Err(HostError::InvalidManifest { field, .. }) if field.ends_with("extensions")));
    }
    value["runtime"]["entries"][0]["extensions"] = json!(["part"]);
    value["runtime"]["entries"][1]["effect"] = json!("edit");
    assert!(matches!(parse(&value), Err(HostError::InvalidManifest { field, .. }) if field.ends_with("effect")));
    value["runtime"]["entries"][1]["effect"] = json!("read");
    value["runtime"]["entries"][0]["effect"] = json!("read");
    assert!(parse(&value).is_err());
    value["runtime"]["entries"] = json!([{"name":"make","extensions":["part"]}]);
    assert!(parse(&value).is_err());
}

#[test]
fn loss_reports_are_structured_bounded_data() {
    let mut report = LossReport {
        counts: [("bodies_read".into(), 2), ("materials_skipped".into(), 1)].into(),
        items: vec![ItemLoss { item: "Body 2".into(), code: "material-unsupported".into(), message: "Material was omitted".into() }],
    };
    report.validate().unwrap();
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["counts"]["bodies_read"], 2);
    assert_eq!(serde_json::from_value::<LossReport>(json).unwrap(), report);
    report.items[0].code = "".into();
    assert!(report.validate().is_err());
    report.items[0].code = "unsupported".into();
    report.items[0].message = "bad\nmessage".into();
    assert!(report.validate().is_err());
    report.items[0].message = "Unsupported material".into();
    report.items = vec![report.items[0].clone(); MAX_LOSS_ITEMS + 1];
    assert!(report.validate().is_err());
}

#[test]
fn format_options_are_validated_offline_before_execution() {
    let mut value = manifest();
    value["runtime"]["entries"] = json!([{"name":"read","kind":"importer","extensions":["part"],
        "input_schema":{"type":"object","additionalProperties":false,"required":["scale","units"],
            "properties":{"scale":{"type":"number","exclusiveMinimum":0,"maximum":10},
                "units":{"enum":["mm","inch"]},
                "names":{"type":"array","maxItems":2,"items":{"type":"string","minLength":1}}}}}]);
    let entry = parse(&value).unwrap().runtime.unwrap().entries.remove(0);
    entry.validate_options(&json!({"scale":1,"units":"mm","names":["part"]})).unwrap();
    for invalid in [
        json!({}),
        json!({"scale":0,"units":"mm"}),
        json!({"scale":1,"units":"m"}),
        json!({"scale":1,"units":"mm","unknown":true}),
        json!({"scale":1,"units":"mm","names":[""]}),
    ] {
        assert!(matches!(entry.validate_options(&invalid), Err(HostError::InvalidInput { .. })));
    }
    value["runtime"]["entries"][0]["input_schema"]["properties"]["scale"] = json!({"$ref":"https://example.invalid/schema"});
    assert!(parse(&value).is_err());
    value["runtime"]["entries"][0]["input_schema"]["properties"]["scale"] = json!({"type":"string","pattern":".*"});
    assert!(parse(&value).is_err(), "unsupported keywords cannot silently bypass validation");
    let mut entry = entry;
    entry.input_schema = json!({"type":"object","$defs":{"positive":{"type":"number","minimum":1}},"properties":{"n":{"$ref":"#/$defs/positive"}}});
    assert!(entry.validate_options(&json!({"n":0})).is_err());
    entry.input_schema = json!({"type":"object","default":42,"properties":{"n":{"$ref":"#/default"}}});
    assert!(entry.validate_options(&json!({"n":1})).is_err(), "references must resolve to schemas");
    entry.input_schema = json!({"type":"object","properties":{"seed":{"type":"integer","maximum":u64::MAX - 1}}});
    assert!(entry.validate_options(&json!({"seed":u64::MAX - 1})).is_ok());
    assert!(entry.validate_options(&json!({"seed":u64::MAX})).is_err(), "integer bounds retain all 64 bits");
    entry.input_schema = json!({"type":"object","properties":{"values":{"oneOf":[
        {"type":"array","items":{"type":"integer"}}, {"type":"array"}]}}});
    assert!(
        matches!(entry.validate_options(&json!({"values":vec![0;8189]})), Err(HostError::LimitExceeded { .. })),
        "alternatives must propagate exhausted validation budgets instead of counting them as mismatches"
    );
    entry.input_schema = json!({"type":"object","$ref":"#"});
    assert!(entry.validate_options(&json!({})).is_err(), "recursive refs are bounded");
}

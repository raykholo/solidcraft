//! Offline author tooling. Testing a folder is explicit development execution.
use craft_plugin::{CheckProblem, CheckReport, HostError, JournalHost, MAX_COMPONENT_BYTES, MAX_JSON_BYTES, MAX_MANIFEST_BYTES, TestArtifacts};
use serde_json::{Value, json};
use solidcraft_engine::{Session, io, plugin::SolidCraftHost};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

fn input_error(field: &str, error: impl std::fmt::Display) -> HostError {
    HostError::InvalidInput { field: field.into(), reason: error.to_string() }
}
fn read(path: &Path, limit: usize) -> Result<Vec<u8>, HostError> {
    let file = std::fs::File::open(path).map_err(|e| input_error(&path.to_string_lossy(), e))?;
    if !file.metadata().map_err(|e| input_error("file", e))?.is_file() {
        return Err(input_error("file", "requires a regular file"));
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes).map_err(|e| input_error("file", e))?;
    if bytes.len() > limit {
        return Err(HostError::LimitExceeded { limit: format!("{} size exceeds {limit} bytes", path.display()) });
    }
    Ok(bytes)
}
fn print(value: &Value, passed: bool) -> Result<(), String> {
    let mut stdout = std::io::stdout().lock();
    serde_json::to_writer(&mut stdout, value).map_err(|e| e.to_string())?;
    stdout.write_all(b"\n").map_err(|e| e.to_string())?;
    if passed { Ok(()) } else { Err("plugin validation or test failed (see JSON output)".into()) }
}

pub fn run(args: &[String]) -> Result<(), String> {
    let result = run_inner(args);
    match result {
        Ok((value, passed)) => print(&value, passed),
        Err(error) => print(&json!(CheckReport::new(vec![CheckProblem::manifest(error)])), false),
    }
}

fn run_inner(args: &[String]) -> Result<(Value, bool), HostError> {
    let action = args.first().map(String::as_str).ok_or_else(|| input_error("action", "requires check or test"))?;
    if !matches!(action, "check" | "test") {
        return Err(input_error("action", "requires check or test"));
    }
    let dir = Path::new(args.get(1).ok_or_else(|| input_error("dir", "requires package folder"))?);
    let dir = dir.canonicalize().map_err(|e| input_error("dir", e))?;
    let mut design = None;
    let mut entry = None;
    let mut input = json!({});
    let mut seen = std::collections::BTreeSet::new();
    let mut options = args.iter().skip(2);
    while let Some(option) = options.next() {
        if action != "test" || !matches!(option.as_str(), "--design" | "--entry" | "--input") || !seen.insert(option) {
            return Err(input_error("arguments", format!("unknown or duplicate option {option}")));
        }
        let value = options.next().ok_or_else(|| input_error(option, "requires a value"))?;
        match option.as_str() {
            "--design" => design = Some(value),
            "--entry" => entry = Some(value.as_str()),
            "--input" => {
                if value.len() > MAX_JSON_BYTES {
                    return Err(input_error("input", "exceeds 1 MiB"));
                }
                input = serde_json::from_str(value).map_err(|e| input_error("input", e))?;
            }
            _ => {}
        }
    }
    let folder = dir.file_name().and_then(|name| name.to_str()).ok_or_else(|| input_error("dir", "requires package id folder"))?;
    let bytes = read(&dir.join("plugin.json"), MAX_MANIFEST_BYTES)?;
    let manifest = match craft_plugin::check_manifest(&bytes, folder) {
        Ok(manifest) => manifest,
        Err(problem) => return Ok((json!(CheckReport::new(vec![problem])), false)),
    };
    let runtime =
        manifest.runtime.as_ref().ok_or_else(|| HostError::InvalidManifest { field: "runtime".into(), reason: "no executable runtime".into() })?;
    let component = read(&dir.join(&runtime.module), MAX_COMPONENT_BYTES)?;
    let mut problems = craft_plugin_wasm::check(&component);
    // The app's command allowlist is policy in addition to the shared contract.
    for command in &runtime.commands {
        if !solidcraft_engine::plugin::allowed_command(command) {
            problems.push(CheckProblem {
                field: "runtime.commands".into(),
                error: HostError::InvalidManifest { field: "runtime.commands".into(), reason: format!("unavailable or forbidden command {command}") },
            });
        }
    }
    if action == "check" || !problems.is_empty() {
        let report = CheckReport::new(problems);
        return Ok((json!(report), report.passed));
    }
    let mut initial = match design {
        Some(path) => crate::load_with(path, solidcraft_engine::schema::ParamCheck::Strict, solidcraft_engine::kernel::Validation::default())
            .map_err(|e| input_error("design", e))?,
        None => Session::default(),
    };
    initial.param_check = solidcraft_engine::schema::ParamCheck::Strict;
    initial.plugin_limits.check(&runtime.limits)?;
    let output = output_dir()?;
    let report = craft_plugin::test_entries(
        &manifest,
        entry,
        &input,
        || Ok(JournalHost::new(SolidCraftHost::new(initial.scratch()))),
        |host, entry, input| craft_plugin_wasm::run(host, &manifest, &component, entry, input),
        |host, entry| {
            let design = output.join(format!("{entry}.sc"));
            let journal = output.join(format!("{entry}.journal.json"));
            let design_path = design.to_str().ok_or_else(|| input_error("artifacts", "output directory must be UTF-8"))?;
            let mut session = host.host.session.scratch();
            session.execute("file.save_as", &json!({"path":design_path})).map_err(|e| input_error("design", e))?;
            let bytes = serde_json::to_vec(&host.journal).map_err(|e| input_error("journal", e))?;
            io::safe_write::write_atomic(&journal, &bytes, false).map_err(|e| input_error("journal", e))?;
            Ok(TestArtifacts { design, journal })
        },
    )?;
    Ok((serde_json::to_value(&report).map_err(|e| input_error("report", e))?, report.passed))
}

fn output_dir() -> Result<PathBuf, HostError> {
    // A new directory prevents one test invocation from replacing another's evidence.
    let root = std::env::temp_dir();
    for index in 0..1000 {
        let path = root.join(format!("solidcraft-plugin-test-{}-{index}", std::process::id()));
        match std::fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(input_error("artifacts", error)),
        }
    }
    Err(input_error("artifacts", "could not reserve an output folder"))
}

use std::{
    error::Error,
    fs,
    fs::File,
    io::{self, BufRead, Read},
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use serde_json::{Map, Value};

use crate::rate_limit;
use crate::task_metadata::validate_task_metadata;

type DynError = Box<dyn Error + Send + Sync + 'static>;

const MAX_RUN_ATTEMPTS: u64 = 3;
const RATE_LIMIT_FILE: &str = ".ralph/rate_limit.json";
const RATE_LIMIT_WINDOW_SECONDS: u64 = 60;
const SCHEMA_PATH: &str = ".ralph/task_result.schema.json";

pub struct TaskAgentConfig {
    pub tasks_path: PathBuf,
    pub prompt_path: PathBuf,
    pub workspace: PathBuf,
    pub reset_task: bool,
    pub explicit_task_id: Option<String>,
}

pub fn run_task_agent(
    config: &TaskAgentConfig,
    task_id_override: Option<&str>,
    allow_next: bool,
) -> Result<i32, DynError> {
    let requested_task_id = task_id_override.or(config.explicit_task_id.as_deref());
    if requested_task_id.is_none() && !allow_next {
        return Err("Task agent requires --task-id or --next".to_string().into());
    }

    ensure_command_available("codex")?;

    let selection = match select_task(&config.tasks_path, requested_task_id, allow_next) {
        Ok(task) => task,
        Err(exit_code) => return Ok(exit_code),
    };

    if let Err(err) = validate_task_metadata(&selection.task_id, &selection.raw) {
        eprintln!("{}", err);
        return Ok(err.exit_code());
    }

    if !model_supported(&selection.model) {
        eprintln!(
            "Unsupported model in task {}: {}",
            selection.task_id, selection.model
        );
        return Ok(2);
    }

    let run_id = run_id()?;

    if config.reset_task {
        reset_task_attempts(
            &config.tasks_path,
            &selection.task_id,
            &run_id,
            "Reset attempts via --reset-task",
        )?;
    }

    let current_attempts = current_attempt_count(&config.tasks_path, &selection.task_id)?;
    if current_attempts >= MAX_RUN_ATTEMPTS {
        update_task_status(
            &config.tasks_path,
            &selection.task_id,
            "blocked",
            &run_id,
            &format!(
                "Attempt limit reached ({}/{}). Use --reset-task after human intervention.",
                current_attempts, MAX_RUN_ATTEMPTS
            ),
        )?;
        eprintln!(
            "Blocked: {} reached attempt limit ({}/{}).",
            selection.task_id, current_attempts, MAX_RUN_ATTEMPTS
        );
        return Ok(11);
    }

    let run_attempt = increment_attempt_count(&config.tasks_path, &selection.task_id)?;

    let run_dir_rel = PathBuf::from(".ralph")
        .join("runs")
        .join(&selection.task_id)
        .join(&run_id);
    let run_dir_abs = config.workspace.join(&run_dir_rel);
    fs::create_dir_all(&run_dir_abs)?;

    let task_snapshot_path = run_dir_abs.join("task.json");
    fs::write(&task_snapshot_path, format!("{}\n", selection.raw_json))?;

    ensure_schema_file(&config.workspace)?;

    if selection.status == "unstarted" || selection.status == "blocked" {
        update_task_status(
            &config.tasks_path,
            &selection.task_id,
            "started",
            &run_id,
            &format!("Run {} started (attempt {})", run_id, run_attempt),
        )?;
    }

    let prompt_path = run_dir_abs.join("prompt.md");
    build_prompt(
        &config.prompt_path,
        &prompt_path,
        &selection.title,
        &selection.definition_of_done,
        &selection.recommended_approach,
        &task_snapshot_path,
    )?;

    let result_path_rel = run_dir_rel.join("result.json");
    let result_path_abs = config.workspace.join(&result_path_rel);
    let codex_log_rel = run_dir_rel.join("codex.jsonl");
    let codex_log_abs = config.workspace.join(&codex_log_rel);

    let estimated_tokens = rate_limit::estimate_prompt_tokens(&prompt_path);
    rate_limit_sleep(
        &config.workspace.join(RATE_LIMIT_FILE),
        &selection.model,
        estimated_tokens,
    )?;

    let mut codex_exit = 1;
    for _ in 0..3 {
        codex_exit = run_codex(
            &config.workspace,
            &selection.model,
            &prompt_path,
            Path::new(SCHEMA_PATH),
            &result_path_rel,
            &codex_log_rel,
        )?;

        if result_path_abs.is_file() && result_path_abs.metadata().map(|m| m.len()).unwrap_or(0) > 0
        {
            break;
        }

        if let Some(delay) = rate_limit_retry_delay(&codex_log_abs)? {
            if delay > 0 {
                eprintln!("Rate limit retry: sleeping {}s before retry.", delay);
                std::thread::sleep(Duration::from_secs(delay));
            }
            continue;
        }

        break;
    }

    let tokens_used = parse_usage_tokens(&codex_log_abs).unwrap_or(estimated_tokens);
    record_rate_usage(
        &config.workspace.join(RATE_LIMIT_FILE),
        &selection.model,
        tokens_used,
    )?;

    if !result_path_abs.is_file()
        || result_path_abs.metadata().map(|m| m.len()).unwrap_or(0) == 0
    {
        update_task_status(
            &config.tasks_path,
            &selection.task_id,
            "blocked",
            &run_id,
            &format!(
                "Codex produced no result.json (exit={}). See {}",
                codex_exit,
                codex_log_rel.display()
            ),
        )?;
        eprintln!(
            "Blocked: missing result.json. See {}",
            codex_log_rel.display()
        );
        return Ok(10);
    }

    let result: Value = serde_json::from_str(&fs::read_to_string(&result_path_abs)?)?;
    let outcome = result
        .get("outcome")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let dod_met = result.get("dod_met").and_then(Value::as_bool).unwrap_or(false);

    let verify = if outcome == "completed" && dod_met {
        run_verification(&config.workspace, &run_dir_abs)?
    } else {
        VerificationResult::skipped()
    };

    if outcome == "completed" && dod_met && verify.ok {
        update_task_status(
            &config.tasks_path,
            &selection.task_id,
            "completed",
            &run_id,
            &format!("Run {} completed", run_id),
        )?;
        return Ok(0);
    }

    if outcome == "blocked" {
        update_task_status(
            &config.tasks_path,
            &selection.task_id,
            "blocked",
            &run_id,
            &format!("Run {} blocked. See {}", run_id, result_path_rel.display()),
        )?;
        return Ok(11);
    }

    let note = format!(
        "Run {} progress. outcome={} dod_met={} verify_ok={}. See {}",
        run_id,
        outcome,
        dod_met,
        verify.ok,
        result_path_rel.display()
    );
    update_task_status(
        &config.tasks_path,
        &selection.task_id,
        "started",
        &run_id,
        &note,
    )?;
    Ok(12)
}

fn ensure_command_available(command: &str) -> Result<(), DynError> {
    match Command::new(command).arg("--version").output() {
        Ok(_) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            Err(format!("Missing dependency: {}", command).into())
        }
        Err(err) => Err(Box::new(err)),
    }
}

struct SelectedTask {
    task_id: String,
    status: String,
    model: String,
    title: String,
    definition_of_done: Vec<String>,
    recommended_approach: String,
    raw: Value,
    raw_json: String,
}

fn select_task(
    tasks_path: &Path,
    requested_task_id: Option<&str>,
    allow_next: bool,
) -> Result<SelectedTask, i32> {
    let root = load_tasks_root(tasks_path).map_err(|_| 2)?;
    let tasks = tasks_array(&root).ok_or(2)?;

    let first_index = tasks.iter().position(|task| {
        let status = task
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("unstarted");
        status != "completed"
    });

    let first_index = match first_index {
        Some(idx) => idx,
        None => {
            eprintln!("No runnable task found");
            return Err(3);
        }
    };

    let first_task = &tasks[first_index];
    let first_task_id = match first_task.get("task_id").and_then(Value::as_str) {
        Some(value) if !value.is_empty() => value.to_string(),
        _ => {
            eprintln!("No runnable task found");
            return Err(3);
        }
    };

    let model = first_task.get("model").and_then(Value::as_str).unwrap_or("");
    if model == "human" {
        eprintln!("Task requires human: {}", first_task_id);
        return Err(4);
    }

    if let Some(requested) = requested_task_id {
        if requested != first_task_id {
            eprintln!(
                "Task {} cannot start until {} is completed.",
                requested, first_task_id
            );
            return Err(6);
        }
    } else if !allow_next {
        eprintln!("Specify --task-id or --next");
        return Err(2);
    }

    let status = first_task
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("unstarted")
        .to_string();

    let title = first_task
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    let definition_of_done = first_task
        .get("definition_of_done")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let recommended_approach = first_task
        .get("recommended")
        .and_then(Value::as_object)
        .and_then(|map| map.get("approach"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    let raw = first_task.clone();
    let raw_json = serde_json::to_string(&raw).map_err(|_| 2)?;

    Ok(SelectedTask {
        task_id: first_task_id,
        status,
        model: model.to_string(),
        title,
        definition_of_done,
        recommended_approach,
        raw,
        raw_json,
    })
}

fn model_supported(model: &str) -> bool {
    matches!(model, "gpt-5.1-codex-mini" | "gpt-5.1-codex" | "gpt-5.2-codex")
}

fn run_id() -> Result<String, DynError> {
    let stamp = utc_timestamp("+%Y%m%dT%H%M%SZ")?;
    Ok(format!("{}-{}", stamp, std::process::id()))
}

fn utc_timestamp(format: &str) -> Result<String, DynError> {
    let output = Command::new("date").arg("-u").arg(format).output()?;
    if !output.status.success() {
        return Err(format!("date command failed for format {}", format).into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn ensure_schema_file(workspace: &Path) -> Result<(), DynError> {
    let schema_path = workspace.join(SCHEMA_PATH);
    if schema_path.is_file() {
        return Ok(());
    }

    if let Some(parent) = schema_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let schema = r#"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "additionalProperties": false,
  "required": ["task_id", "outcome", "dod_met", "summary", "tests", "notes", "blockers"],
  "properties": {
    "task_id": { "type": "string" },
    "outcome": { "type": "string", "enum": ["completed", "blocked", "started"] },
    "dod_met": { "type": "boolean" },
    "summary": { "type": "string" },
    "tests": {
      "type": "object",
      "additionalProperties": false,
      "required": ["ran", "commands", "passed"],
      "properties": {
        "ran": { "type": "boolean" },
        "commands": { "type": "array", "items": { "type": "string" } },
        "passed": { "type": "boolean" }
      }
    },
    "notes": { "type": "string" },
    "blockers": { "type": "array", "items": { "type": "string" } }
  }
}
"#;
    fs::write(schema_path, schema)?;
    Ok(())
}

fn build_prompt(
    base_prompt: &Path,
    prompt_path: &Path,
    title: &str,
    dod: &[String],
    recommended: &str,
    task_snapshot: &Path,
) -> Result<(), DynError> {
    let mut prompt = fs::read_to_string(base_prompt)?;
    prompt.push_str("\n\n");
    prompt.push_str(&format!("Task title: {}\n", title));
    prompt.push_str("\nDefinition of done:\n");
    for item in dod {
        prompt.push_str(&format!("  - {}\n", item));
    }
    prompt.push_str("\nRecommended approach:\n");
    prompt.push_str(recommended);
    prompt.push('\n');
    prompt.push_str("\nTask JSON (authoritative):\n");
    prompt.push_str(&fs::read_to_string(task_snapshot)?);
    if !prompt.ends_with('\n') {
        prompt.push('\n');
    }
    fs::write(prompt_path, prompt)?;
    Ok(())
}

fn rate_limit_sleep(rate_file: &Path, model: &str, estimated_tokens: u64) -> Result<(), DynError> {
    let (tpm, rpm) = rate_limit::rate_limit_settings(model);
    let sleep_seconds = rate_limit::rate_limit_sleep_seconds(
        rate_file,
        model,
        Duration::from_secs(RATE_LIMIT_WINDOW_SECONDS),
        tpm,
        rpm,
        estimated_tokens,
    )?;
    if sleep_seconds > 0 {
        eprintln!(
            "Rate limit throttle: sleeping {}s for {}.",
            sleep_seconds, model
        );
        std::thread::sleep(Duration::from_secs(sleep_seconds));
    }
    Ok(())
}

fn record_rate_usage(rate_file: &Path, model: &str, tokens: u64) -> Result<(), DynError> {
    rate_limit::record_rate_usage(
        rate_file,
        model,
        Duration::from_secs(RATE_LIMIT_WINDOW_SECONDS),
        tokens,
    )
}

fn run_codex(
    workspace: &Path,
    model: &str,
    prompt_path: &Path,
    schema_path: &Path,
    result_path: &Path,
    log_path: &Path,
) -> Result<i32, DynError> {
    let prompt_file = File::open(prompt_path)?;
    let log_file = File::create(workspace.join(log_path))?;
    let log_file_err = log_file.try_clone()?;

    let status = Command::new("codex")
        .current_dir(workspace)
        .arg("exec")
        .arg("--yolo")
        .arg("--model")
        .arg(model)
        .arg("--output-schema")
        .arg(schema_path)
        .arg("--output-last-message")
        .arg(result_path)
        .arg("--json")
        .arg("--skip-git-repo-check")
        .arg("-")
        .stdin(prompt_file)
        .stdout(log_file)
        .stderr(log_file_err)
        .status()?;
    Ok(status.code().unwrap_or(1))
}

fn parse_usage_tokens(log_path: &Path) -> Option<u64> {
    let mut usage_tokens = None;
    let file = File::open(log_path).ok()?;
    let reader = io::BufReader::new(file);
    for line in reader.lines().flatten() {
        if !line.trim_start().starts_with('{') {
            continue;
        }
        let payload: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if payload.get("type").and_then(Value::as_str) != Some("turn.completed") {
            continue;
        }
        let usage = payload.get("usage")?;
        let input_tokens = usage
            .get("input_tokens")
            .or_else(|| usage.get("prompt_tokens"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let output_tokens = usage
            .get("output_tokens")
            .or_else(|| usage.get("completion_tokens"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let total = usage
            .get("total_tokens")
            .and_then(Value::as_i64)
            .unwrap_or(input_tokens + output_tokens);
        if total > 0 {
            usage_tokens = Some(total as u64);
        }
    }
    usage_tokens
}

fn rate_limit_retry_delay(log_path: &Path) -> Result<Option<u64>, DynError> {
    let mut raw = String::new();
    if File::open(log_path)
        .and_then(|mut f| f.read_to_string(&mut raw))
        .is_err()
    {
        return Ok(None);
    }
    let lower = raw.to_lowercase();
    if !lower.contains("rate limit") && !lower.contains("rate-limit") {
        return Ok(None);
    }
    let needle = "please try again in ";
    if let Some(idx) = lower.find(needle) {
        let tail = &raw[idx + needle.len()..];
        let mut number = String::new();
        for ch in tail.chars() {
            if ch.is_ascii_digit() || ch == '.' {
                number.push(ch);
            } else {
                break;
            }
        }
        if !number.is_empty() {
            if let Ok(value) = number.parse::<f64>() {
                return Ok(Some(value.ceil() as u64));
            }
        }
    }
    Ok(None)
}

fn load_tasks_root(path: &Path) -> Result<Value, DynError> {
    let raw = fs::read_to_string(path).map_err(|err| {
        format!(
            "Failed to read tasks file {}: {}",
            path.display(),
            err
        )
    })?;
    serde_json::from_str(&raw).map_err(|err| err.into())
}

fn tasks_array(root: &Value) -> Option<&Vec<Value>> {
    match root {
        Value::Array(items) => Some(items),
        Value::Object(map) => map.get("tasks").and_then(Value::as_array),
        _ => None,
    }
}

fn tasks_array_mut(root: &mut Value) -> Option<&mut Vec<Value>> {
    match root {
        Value::Array(items) => Some(items),
        Value::Object(map) => map.get_mut("tasks").and_then(Value::as_array_mut),
        _ => None,
    }
}

fn current_attempt_count(tasks_path: &Path, task_id: &str) -> Result<u64, DynError> {
    let root = load_tasks_root(tasks_path)?;
    let tasks = tasks_array(&root).ok_or("Tasks file is not a list")?;
    let task = tasks
        .iter()
        .find(|task| task.get("task_id").and_then(Value::as_str) == Some(task_id))
        .ok_or_else(|| format!("Task {} not found in {}", task_id, tasks_path.display()))?;
    Ok(task
        .get("observability")
        .and_then(Value::as_object)
        .and_then(|map| map.get("run_attempts"))
        .and_then(Value::as_u64)
        .unwrap_or(0))
}

fn increment_attempt_count(tasks_path: &Path, task_id: &str) -> Result<u64, DynError> {
    let mut root = load_tasks_root(tasks_path)?;
    let tasks = tasks_array_mut(&mut root).ok_or("Tasks file is not a list")?;
    let task = tasks
        .iter_mut()
        .find(|task| task.get("task_id").and_then(Value::as_str) == Some(task_id))
        .ok_or_else(|| format!("Task {} not found in {}", task_id, tasks_path.display()))?;

    let task_obj = task_object_mut(task)?;
    let obs = ensure_observability(task_obj);
    let current = obs
        .get("run_attempts")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let updated = current + 1;
    obs.insert("run_attempts".to_string(), Value::from(updated));
    write_tasks_root(tasks_path, &root)?;
    Ok(updated)
}

fn reset_task_attempts(
    tasks_path: &Path,
    task_id: &str,
    run_id: &str,
    note: &str,
) -> Result<(), DynError> {
    let mut root = load_tasks_root(tasks_path)?;
    let tasks = tasks_array_mut(&mut root).ok_or("Tasks file is not a list")?;
    let task = tasks
        .iter_mut()
        .find(|task| task.get("task_id").and_then(Value::as_str) == Some(task_id))
        .ok_or_else(|| format!("Task {} not found in {}", task_id, tasks_path.display()))?;

    let task_obj = task_object_mut(task)?;
    task_obj.insert("status".to_string(), Value::from("unstarted"));
    let obs = ensure_observability(task_obj);
    obs.insert("run_attempts".to_string(), Value::from(0));
    obs.insert("last_run_id".to_string(), Value::from(run_id));
    obs.insert(
        "last_update_utc".to_string(),
        Value::from(utc_timestamp("%Y-%m-%dT%H:%M:%SZ")?),
    );
    if !note.is_empty() {
        obs.insert("last_note".to_string(), Value::from(note));
    }

    write_tasks_root(tasks_path, &root)
}

fn update_task_status(
    tasks_path: &Path,
    task_id: &str,
    new_status: &str,
    run_id: &str,
    note: &str,
) -> Result<(), DynError> {
    let mut root = load_tasks_root(tasks_path)?;
    let tasks = tasks_array_mut(&mut root).ok_or("Tasks file is not a list")?;
    let task = tasks
        .iter_mut()
        .find(|task| task.get("task_id").and_then(Value::as_str) == Some(task_id))
        .ok_or_else(|| format!("Task {} not found in {}", task_id, tasks_path.display()))?;

    let task_obj = task_object_mut(task)?;
    task_obj.insert("status".to_string(), Value::from(new_status));
    let obs = ensure_observability(task_obj);
    obs.insert("last_run_id".to_string(), Value::from(run_id));
    obs.insert(
        "last_update_utc".to_string(),
        Value::from(utc_timestamp("%Y-%m-%dT%H:%M:%SZ")?),
    );
    if !note.is_empty() {
        obs.insert("last_note".to_string(), Value::from(note));
    }

    write_tasks_root(tasks_path, &root)
}

fn task_object_mut(task: &mut Value) -> Result<&mut Map<String, Value>, DynError> {
    task.as_object_mut()
        .ok_or_else(|| "Task entry is not an object".to_string().into())
}

fn ensure_observability(task: &mut Map<String, Value>) -> &mut Map<String, Value> {
    if !task
        .get("observability")
        .map(|value| value.is_object())
        .unwrap_or(false)
    {
        task.insert("observability".to_string(), Value::Object(Map::new()));
    }
    task.get_mut("observability")
        .and_then(Value::as_object_mut)
        .expect("observability must be an object")
}

fn write_tasks_root(path: &Path, root: &Value) -> Result<(), DynError> {
    let serialized = serde_json::to_string_pretty(root)?;
    fs::write(path, serialized)?;
    Ok(())
}

struct VerificationResult {
    ok: bool,
}

impl VerificationResult {
    fn skipped() -> Self {
        Self { ok: true }
    }
}

fn run_verification(workspace: &Path, run_dir: &Path) -> Result<VerificationResult, DynError> {
    let verify_log = run_dir.join("verify.log");
    let log_file = File::create(&verify_log)?;
    let mut selected_cmd = None;

    if is_executable(&workspace.join("scripts/ci.sh")) {
        selected_cmd = Some(vec!["./scripts/ci.sh".to_string()]);
    } else if makefile_has_ci(&workspace.join("Makefile"))? {
        selected_cmd = Some(vec!["make".to_string(), "ci".to_string()]);
    } else if is_executable(&workspace.join("tests/run.sh")) {
        selected_cmd = Some(vec!["./tests/run.sh".to_string()]);
    } else if command_available("pytest") && has_python_tests(workspace)? {
        selected_cmd = Some(vec!["pytest".to_string(), "-q".to_string()]);
    }

    let Some(cmd) = selected_cmd else {
        return Ok(VerificationResult { ok: true });
    };

    let mut command = Command::new(&cmd[0]);
    if cmd.len() > 1 {
        command.args(&cmd[1..]);
    }
    let status = command
        .current_dir(workspace)
        .stdout(log_file.try_clone()?)
        .stderr(log_file)
        .status()?;

    let ok = status.success();
    Ok(VerificationResult { ok })
}

fn makefile_has_ci(path: &Path) -> Result<bool, DynError> {
    if !path.is_file() {
        return Ok(false);
    }
    let content = fs::read_to_string(path)?;
    Ok(content
        .lines()
        .any(|line| line.trim_start().starts_with("ci:")))
}

fn command_available(command: &str) -> bool {
    Command::new(command)
        .arg("--version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

fn has_python_tests(workspace: &Path) -> Result<bool, DynError> {
    let root_markers = ["pytest.ini", "pyproject.toml", "setup.cfg", "tox.ini"];
    if root_markers
        .iter()
        .any(|marker| workspace.join(marker).is_file())
    {
        return Ok(true);
    }

    let tests_dir = workspace.join("tests");
    if !tests_dir.is_dir() {
        return Ok(false);
    }
    Ok(dir_contains_py(&tests_dir)?)
}

fn dir_contains_py(path: &Path) -> Result<bool, DynError> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            if dir_contains_py(&path)? {
                return Ok(true);
            }
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("py") {
            return Ok(true);
        }
    }
    Ok(false)
}

fn is_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = fs::metadata(path) {
            return metadata.permissions().mode() & 0o111 != 0;
        }
    }
    true
}

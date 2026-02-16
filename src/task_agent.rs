use std::{
    error::Error,
    ffi::OsString,
    fs,
    fs::File,
    io::{self, BufRead, IsTerminal, Read, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

use serde_json::{json, Map, Value};

use lever::context_compile::{ContextCompileConfig, ContextFailurePolicy};

use crate::rate_limit;
use crate::run_paths::run_paths;
use crate::task_metadata::validate_task_metadata;

type DynError = Box<dyn Error + Send + Sync + 'static>;

const MAX_RUN_ATTEMPTS: u64 = 3;
const RATE_LIMIT_FILE: &str = ".ralph/rate_limit.json";
const RATE_LIMIT_WINDOW_SECONDS: u64 = 60;
const SCHEMA_PATH: &str = ".ralph/task_result.schema.json";
const ASSEMBLY_REQUIRED_FAILURE_EXIT_CODE: i32 = 13;

pub struct TaskAgentConfig {
    pub tasks_path: PathBuf,
    pub prompt_path: PathBuf,
    pub workspace: PathBuf,
    pub reset_task: bool,
    pub explicit_task_id: Option<String>,
    pub context_compile: ContextCompileConfig,
}

pub fn run_task_agent(
    config: &TaskAgentConfig,
    task_id_override: Option<&str>,
    allow_next: bool,
    shutdown_flag: Option<&AtomicBool>,
) -> Result<i32, DynError> {
    let requested_task_id = task_id_override.or(config.explicit_task_id.as_deref());
    let _ = config.context_compile.enabled;
    if requested_task_id.is_none() && !allow_next {
        return Err("Task agent requires --task-id or --next".to_string().into());
    }

    ensure_command_available("codex")?;

    let selection = match select_task(&config.tasks_path, requested_task_id, allow_next) {
        Ok(task) => task,
        Err(exit_code) => return Ok(exit_code),
    };

    log_line(
        "INFO",
        "Task selected",
        &[
            format!("task_id={}", selection.task_id),
            format!("title={}", selection.title),
            format!("model={}", selection.model),
            format!("status={}", selection.status),
            format!("dod_count={}", selection.definition_of_done.len()),
        ],
    );

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
        git_commit_progress(&config.workspace, &selection.title, &selection.task_id)?;
        log_line(
            "WARN",
            "Attempt limit reached",
            &[
                format!("task_id={}", selection.task_id),
                format!("run_id={}", run_id),
                format!("attempts={}", current_attempts),
            ],
        );
        eprintln!(
            "Blocked: {} reached attempt limit ({}/{}).",
            selection.task_id, current_attempts, MAX_RUN_ATTEMPTS
        );
        return Ok(11);
    }

    let run_attempt = current_attempts + 1;

    let paths = run_paths(&config.workspace, &selection.task_id, &run_id);
    fs::create_dir_all(&paths.run_dir_abs)?;
    fs::create_dir_all(&paths.pack_dir_abs)?;

    fs::write(
        &paths.task_snapshot_path,
        format!("{}\n", selection.raw_json),
    )?;
    let assembly_task_input = build_assembly_task_input(&selection);
    let assembly_task_json = serde_json::to_string_pretty(&assembly_task_input)?;
    fs::write(
        &paths.assembly_task_path,
        format!("{}\n", assembly_task_json),
    )?;
    if config.context_compile.enabled {
        if is_shutdown(shutdown_flag) {
            return handle_interrupt(
                &config.tasks_path,
                &config.workspace,
                &selection.task_id,
                &selection.title,
                &run_id,
                run_attempt,
            );
        }

        let assembly_outcome = match run_assembly(
            &config.workspace,
            &selection.task_id,
            &paths,
            &config.context_compile,
            shutdown_flag,
        ) {
            Ok(outcome) => outcome,
            Err(err) => AssemblyOutcome::Failed {
                code: None,
                message: format!(
                    "Failed to run assembly {}: {}",
                    config.context_compile.assembly_path.display(),
                    err
                ),
            },
        };

        match assembly_outcome {
            AssemblyOutcome::Success => {
                log_line(
                    "INFO",
                    "Assembly build succeeded",
                    &[
                        format!("task_id={}", selection.task_id),
                        format!("run_id={}", run_id),
                        format!("stdout={}", paths.assembly_stdout_path.display()),
                        format!("stderr={}", paths.assembly_stderr_path.display()),
                    ],
                );
            }
            AssemblyOutcome::Interrupted => {
                return handle_interrupt(
                    &config.tasks_path,
                    &config.workspace,
                    &selection.task_id,
                    &selection.title,
                    &run_id,
                    run_attempt,
                );
            }
            AssemblyOutcome::Failed { code, message } => {
                let exit_detail = match code {
                    Some(value) => format!("exit={}", value),
                    None => "exit=signal".to_string(),
                };
                let note = format!(
                    "Assembly failed ({}) for run {}. {} See stdout={} stderr={}",
                    exit_detail.as_str(),
                    run_id,
                    message,
                    paths.assembly_stdout_path.display(),
                    paths.assembly_stderr_path.display()
                );
                if config.context_compile.policy == ContextFailurePolicy::Required {
                    increment_attempt_count(&config.tasks_path, &selection.task_id)?;
                    update_task_status(
                        &config.tasks_path,
                        &selection.task_id,
                        "blocked",
                        &run_id,
                        &note,
                    )?;
                    git_commit_progress(&config.workspace, &selection.title, &selection.task_id)?;
                    log_line(
                        "ERROR",
                        "Assembly build failed",
                        &[
                            format!("task_id={}", selection.task_id),
                            format!("run_id={}", run_id),
                            exit_detail.clone(),
                            format!("stdout={}", paths.assembly_stdout_path.display()),
                            format!("stderr={}", paths.assembly_stderr_path.display()),
                        ],
                    );
                    eprintln!("Blocked: {}", note);
                    return Ok(ASSEMBLY_REQUIRED_FAILURE_EXIT_CODE);
                }

                log_line(
                    "WARN",
                    "Assembly build failed (best-effort)",
                    &[
                        format!("task_id={}", selection.task_id),
                        format!("run_id={}", run_id),
                        exit_detail.clone(),
                        format!("stdout={}", paths.assembly_stdout_path.display()),
                        format!("stderr={}", paths.assembly_stderr_path.display()),
                    ],
                );
                eprintln!("Warning: {}", note);
            }
        }
    }

    ensure_schema_file(&config.workspace)?;

    if is_shutdown(shutdown_flag) {
        return handle_interrupt(
            &config.tasks_path,
            &config.workspace,
            &selection.task_id,
            &selection.title,
            &run_id,
            run_attempt,
        );
    }

    log_line(
        "INFO",
        "Run started",
        &[
            format!("task_id={}", selection.task_id),
            format!("title={}", selection.title),
            format!("run_id={}", run_id),
            format!("assignee={}", std::env::var("ASSIGNEE").unwrap_or_default()),
        ],
    );

    build_prompt(
        &config.prompt_path,
        &paths.prompt_path,
        &selection.title,
        &selection.definition_of_done,
        &selection.recommended_approach,
        &paths.task_snapshot_path,
    )?;

    let codex_stream = CodexLogStream::start(&paths.codex_log_abs, &selection.task_id, &run_id)?;

    let estimated_tokens = rate_limit::estimate_prompt_tokens(&paths.prompt_path);
    rate_limit_sleep(
        &config.workspace.join(RATE_LIMIT_FILE),
        &selection.model,
        estimated_tokens,
        shutdown_flag,
    )?;
    if is_shutdown(shutdown_flag) {
        codex_stream.stop();
        return handle_interrupt(
            &config.tasks_path,
            &config.workspace,
            &selection.task_id,
            &selection.title,
            &run_id,
            run_attempt,
        );
    }

    let mut codex_exit = 1;
    for attempt in 1..=3 {
        log_line(
            "INFO",
            "Codex exec start",
            &[
                format!("task_id={}", selection.task_id),
                format!("run_id={}", run_id),
                format!("attempt={}", attempt),
                format!("model={}", selection.model),
            ],
        );
        codex_exit = run_codex(
            &config.workspace,
            &selection.model,
            &paths.prompt_path,
            Path::new(SCHEMA_PATH),
            &paths.result_path_rel,
            &paths.codex_log_rel,
            shutdown_flag,
        )?;
        log_line(
            "INFO",
            "Codex exec end",
            &[
                format!("task_id={}", selection.task_id),
                format!("run_id={}", run_id),
                format!("attempt={}", attempt),
                format!("exit={}", codex_exit),
            ],
        );

        if codex_exit == 130 || is_shutdown(shutdown_flag) {
            codex_stream.stop();
            return handle_interrupt(
                &config.tasks_path,
                &config.workspace,
                &selection.task_id,
                &selection.title,
                &run_id,
                run_attempt,
            );
        }

        if paths.result_path_abs.is_file()
            && paths
                .result_path_abs
                .metadata()
                .map(|m| m.len())
                .unwrap_or(0)
                > 0
        {
            break;
        }

        if let Some(delay) = rate_limit_retry_delay(&paths.codex_log_abs)? {
            if delay > 0 {
                eprintln!(
                    "Rate limit retry: sleeping {}s before retry {}/3.",
                    delay, attempt
                );
                std::thread::sleep(Duration::from_secs(delay));
            }
            continue;
        }

        break;
    }

    codex_stream.stop();

    let tokens_used = parse_usage_tokens(&paths.codex_log_abs).unwrap_or(estimated_tokens);
    record_rate_usage(
        &config.workspace.join(RATE_LIMIT_FILE),
        &selection.model,
        tokens_used,
    )?;

    if !paths.result_path_abs.is_file()
        || paths
            .result_path_abs
            .metadata()
            .map(|m| m.len())
            .unwrap_or(0)
            == 0
    {
        increment_attempt_count(&config.tasks_path, &selection.task_id)?;
        update_task_status(
            &config.tasks_path,
            &selection.task_id,
            "blocked",
            &run_id,
            &format!(
                "Codex produced no result.json (exit={}). See {}",
                codex_exit,
                paths.codex_log_rel.display()
            ),
        )?;
        git_commit_progress(&config.workspace, &selection.title, &selection.task_id)?;
        log_line(
            "ERROR",
            "Missing result.json",
            &[
                format!("task_id={}", selection.task_id),
                format!("run_id={}", run_id),
                format!("exit={}", codex_exit),
            ],
        );
        eprintln!(
            "Blocked: missing result.json. See {}",
            paths.codex_log_rel.display()
        );
        return Ok(10);
    }

    let result: Value = serde_json::from_str(&fs::read_to_string(&paths.result_path_abs)?)?;
    let reported_outcome = result
        .get("outcome")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let dod_met = result
        .get("dod_met")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    if let Some(summary) = result.get("summary").and_then(Value::as_str) {
        if !summary.trim().is_empty() {
            log_line(
                "INFO",
                &format!("Result summary: {}", compact_text(summary, 220)),
                &[
                    format!("task_id={}", selection.task_id),
                    format!("run_id={}", run_id),
                    format!("outcome={}", reported_outcome),
                    format!("dod_met={}", dod_met),
                    format!(
                        "tests_ran={}",
                        result
                            .get("tests")
                            .and_then(Value::as_object)
                            .and_then(|tests| tests.get("ran"))
                            .and_then(Value::as_bool)
                            .unwrap_or(false)
                    ),
                    format!(
                        "tests_passed={}",
                        result
                            .get("tests")
                            .and_then(Value::as_object)
                            .and_then(|tests| tests.get("passed"))
                            .and_then(Value::as_bool)
                            .unwrap_or(false)
                    ),
                ],
            );
        }
    }

    if !dod_met {
        let notes = result.get("notes").and_then(Value::as_str).unwrap_or("");
        if !notes.trim().is_empty() {
            log_line(
                "WARN",
                &format!("Definition of done not met: {}", compact_text(notes, 220)),
                &[
                    format!("task_id={}", selection.task_id),
                    format!("run_id={}", run_id),
                    format!("outcome={}", reported_outcome),
                ],
            );
        } else {
            log_line(
                "WARN",
                "Definition of done not met",
                &[
                    format!("task_id={}", selection.task_id),
                    format!("run_id={}", run_id),
                    format!("outcome={}", reported_outcome),
                ],
            );
        }
    }

    let verify = if dod_met {
        run_verification(
            &config.workspace,
            &paths.run_dir_abs,
            &selection.verification_commands,
        )?
    } else {
        VerificationResult::skipped()
    };

    if !verify.log_command.as_deref().unwrap_or("").is_empty() {
        if verify.ok {
            log_line(
                "INFO",
                "Verification succeeded",
                &[
                    format!("task_id={}", selection.task_id),
                    format!("run_id={}", run_id),
                    format!("command={}", verify.log_command.as_deref().unwrap_or("")),
                    format!("log={}", paths.run_dir_abs.join("verify.log").display()),
                ],
            );
        } else {
            log_line(
                "WARN",
                "Verification failed",
                &[
                    format!("task_id={}", selection.task_id),
                    format!("run_id={}", run_id),
                    format!("command={}", verify.log_command.as_deref().unwrap_or("")),
                    format!("log={}", paths.run_dir_abs.join("verify.log").display()),
                ],
            );
        }
    }

    if dod_met && verify.ok {
        increment_attempt_count(&config.tasks_path, &selection.task_id)?;
        update_task_status(
            &config.tasks_path,
            &selection.task_id,
            "completed",
            &run_id,
            &format!("Run {} completed", run_id),
        )?;
        git_commit_progress(&config.workspace, &selection.title, &selection.task_id)?;
        finalize_successful_task(&config.workspace, &selection.task_id, &selection.title)?;
        log_line(
            "INFO",
            "Run completed",
            &[
                format!("task_id={}", selection.task_id),
                format!("run_id={}", run_id),
                format!("verify_ok={}", verify.ok),
            ],
        );
        print_line(
            true,
            &format!(
                "COMPLETED {} (model={}, run={})",
                selection.task_id, selection.model, run_id
            ),
        );
        return Ok(0);
    }

    if reported_outcome == "blocked" {
        log_line(
            "WARN",
            "Run reported blocked; keeping task started for deterministic retry",
            &[
                format!("task_id={}", selection.task_id),
                format!("run_id={}", run_id),
            ],
        );
    }

    let note = format!(
        "Run {} progress. reported_outcome={} dod_met={} verify_ok={}. See {}",
        run_id,
        reported_outcome,
        dod_met,
        verify.ok,
        paths.result_path_rel.display()
    );
    increment_attempt_count(&config.tasks_path, &selection.task_id)?;
    update_task_status(
        &config.tasks_path,
        &selection.task_id,
        "started",
        &run_id,
        &note,
    )?;
    git_commit_progress(&config.workspace, &selection.title, &selection.task_id)?;
    log_line(
        "INFO",
        "Run started/progress",
        &[
            format!("task_id={}", selection.task_id),
            format!("run_id={}", run_id),
            format!("outcome={}", reported_outcome),
            format!("dod_met={}", dod_met),
            format!("verify_ok={}", verify.ok),
        ],
    );
    print_line(
        false,
        &format!(
            "STARTED {} (model={}, run={})",
            selection.task_id, selection.model, run_id
        ),
    );
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
    verification_commands: Vec<String>,
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

    let model = first_task
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("");
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

    let verification_commands = first_task
        .get("verification")
        .and_then(Value::as_object)
        .and_then(|map| map.get("commands"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|entry| !entry.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let raw = first_task.clone();
    let raw_json = serde_json::to_string(&raw).map_err(|_| 2)?;

    Ok(SelectedTask {
        task_id: first_task_id,
        status,
        model: model.to_string(),
        title,
        definition_of_done,
        recommended_approach,
        verification_commands,
        raw,
        raw_json,
    })
}

fn model_supported(model: &str) -> bool {
    matches!(
        model,
        "gpt-5.1-codex-mini" | "gpt-5.1-codex" | "gpt-5.2-codex"
    )
}

fn run_id() -> Result<String, DynError> {
    let stamp = utc_timestamp("+%Y%m%dT%H%M%SZ")?;
    Ok(format!("{}-{}", stamp, std::process::id()))
}

fn utc_timestamp(format: &str) -> Result<String, DynError> {
    let format = if format.starts_with('+') {
        format.to_string()
    } else {
        format!("+{}", format)
    };
    let output = Command::new("date").arg("-u").arg(&format).output()?;
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

fn build_assembly_task_input(selection: &SelectedTask) -> Value {
    let mut payload = json!({
        "task_id": selection.task_id.clone(),
        "title": selection.title.clone(),
        "status": selection.status.clone(),
        "model": selection.model.clone(),
        "definition_of_done": selection.definition_of_done.clone(),
        "recommended": {
            "approach": selection.recommended_approach.clone(),
        },
    });

    if !selection.verification_commands.is_empty() {
        if let Some(map) = payload.as_object_mut() {
            map.insert(
                "verification".to_string(),
                json!({
                    "commands": selection.verification_commands.clone(),
                }),
            );
        }
    }

    payload
}

fn build_assembly_command_args(
    workspace: &Path,
    task_id: &str,
    task_input: &Path,
    pack_dir: &Path,
    summary_json: &Path,
    config: &ContextCompileConfig,
) -> Vec<OsString> {
    let mut args = vec![
        OsString::from("build"),
        OsString::from("--repo"),
        workspace.as_os_str().to_os_string(),
        OsString::from("--task"),
    ];
    let mut task_arg = OsString::from("@");
    task_arg.push(task_input);
    args.push(task_arg);
    args.push("--task-id".into());
    args.push(task_id.into());
    args.push("--out".into());
    args.push(pack_dir.as_os_str().to_os_string());
    args.push("--token-budget".into());
    args.push(config.token_budget.to_string().into());

    for glob in &config.exclude_globs {
        args.push("--exclude".into());
        args.push(glob.into());
    }

    for glob in &config.exclude_runtime_globs {
        args.push("--exclude-runtime".into());
        args.push(glob.into());
    }

    args.push("--summary-json".into());
    args.push(summary_json.as_os_str().to_os_string());

    args
}

fn rate_limit_sleep(
    rate_file: &Path,
    model: &str,
    estimated_tokens: u64,
    shutdown_flag: Option<&AtomicBool>,
) -> Result<(), DynError> {
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
        if let Some(flag) = shutdown_flag {
            let mut remaining = sleep_seconds;
            while remaining > 0 {
                if flag.load(Ordering::SeqCst) {
                    break;
                }
                let chunk = std::cmp::min(1, remaining);
                std::thread::sleep(Duration::from_secs(chunk));
                remaining = remaining.saturating_sub(chunk);
            }
        } else {
            std::thread::sleep(Duration::from_secs(sleep_seconds));
        }
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
    shutdown_flag: Option<&AtomicBool>,
) -> Result<i32, DynError> {
    let prompt_file = File::open(prompt_path)?;
    let log_file = File::create(workspace.join(log_path))?;
    let log_file_err = log_file.try_clone()?;

    let mut child = Command::new("codex")
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
        .spawn()?;

    loop {
        if let Some(flag) = shutdown_flag {
            if flag.load(Ordering::SeqCst) {
                let _ = child.kill();
                let _ = child.wait();
                return Ok(130);
            }
        }

        match child.try_wait()? {
            Some(status) => return Ok(status.code().unwrap_or(1)),
            None => thread::sleep(Duration::from_millis(100)),
        }
    }
}

enum AssemblyOutcome {
    Success,
    Failed { code: Option<i32>, message: String },
    Interrupted,
}

fn run_assembly(
    workspace: &Path,
    task_id: &str,
    paths: &crate::run_paths::RunPaths,
    config: &ContextCompileConfig,
    shutdown_flag: Option<&AtomicBool>,
) -> Result<AssemblyOutcome, DynError> {
    let args = build_assembly_command_args(
        workspace,
        task_id,
        &paths.assembly_task_path,
        &paths.pack_dir_abs,
        &paths.assembly_summary_path,
        config,
    );

    let stdout_file = File::create(&paths.assembly_stdout_path)?;
    let stderr_file = File::create(&paths.assembly_stderr_path)?;

    let mut child = Command::new(&config.assembly_path)
        .current_dir(workspace)
        .args(args)
        .stdout(stdout_file)
        .stderr(stderr_file)
        .spawn()?;

    loop {
        if let Some(flag) = shutdown_flag {
            if flag.load(Ordering::SeqCst) {
                let _ = child.kill();
                let _ = child.wait();
                return Ok(AssemblyOutcome::Interrupted);
            }
        }

        match child.try_wait()? {
            Some(status) => {
                if status.success() {
                    return Ok(AssemblyOutcome::Success);
                }
                return Ok(AssemblyOutcome::Failed {
                    code: status.code(),
                    message: "Assembly exited with non-zero status".to_string(),
                });
            }
            None => thread::sleep(Duration::from_millis(100)),
        }
    }
}

fn parse_usage_tokens(log_path: &Path) -> Option<u64> {
    let mut usage_tokens = None;
    let file = File::open(log_path).ok()?;
    let reader = io::BufReader::new(file);
    for line in reader.lines().map_while(Result::ok) {
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

fn is_shutdown(shutdown_flag: Option<&AtomicBool>) -> bool {
    shutdown_flag
        .map(|flag| flag.load(Ordering::SeqCst))
        .unwrap_or(false)
}

fn handle_interrupt(
    tasks_path: &Path,
    workspace: &Path,
    task_id: &str,
    task_title: &str,
    run_id: &str,
    run_attempt: u64,
) -> Result<i32, DynError> {
    increment_attempt_count(tasks_path, task_id)?;
    let note = format!("Run {} interrupted on attempt {}", run_id, run_attempt);
    update_task_status(tasks_path, task_id, "started", run_id, &note)?;
    git_commit_progress(workspace, task_title, task_id)?;
    log_line(
        "WARN",
        "Run interrupted",
        &[
            format!("task_id={}", task_id),
            format!("run_id={}", run_id),
            format!("attempt={}", run_attempt),
        ],
    );
    Ok(130)
}

struct CodexLogStream {
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl CodexLogStream {
    fn start(log_path: &Path, task_id: &str, run_id: &str) -> Result<Self, DynError> {
        if let Some(parent) = log_path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        let _ = File::create(log_path)?;

        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&stop);
        let log_path = log_path.to_path_buf();
        let task_id = task_id.to_string();
        let run_id = run_id.to_string();

        let handle = thread::spawn(move || {
            let mut file = match File::open(&log_path) {
                Ok(file) => file,
                Err(_) => return,
            };
            let mut reader = io::BufReader::new(&mut file);
            loop {
                if stop_flag.load(Ordering::SeqCst) {
                    break;
                }
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => {
                        thread::sleep(Duration::from_millis(100));
                        continue;
                    }
                    Ok(_) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        let ts = utc_timestamp("%Y-%m-%dT%H:%M:%SZ")
                            .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string());
                        let raw = compact_text(trimmed, 400);
                        print_line(
                            true,
                            &format!(
                                "{} INFO codex raw {} task_id={} run_id={}",
                                ts, raw, task_id, run_id
                            ),
                        );
                    }
                    Err(_) => {
                        thread::sleep(Duration::from_millis(100));
                    }
                }
            }
        });

        Ok(Self {
            stop,
            handle: Some(handle),
        })
    }

    fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn compact_text(input: &str, limit: usize) -> String {
    let mut normalized = input.replace(['\n', '\r'], " ");
    if normalized.len() > limit {
        normalized.truncate(limit);
        normalized.push_str("...");
    }
    normalized
}

fn log_line(level: &str, message: &str, kv: &[String]) {
    let ts =
        utc_timestamp("%Y-%m-%dT%H:%M:%SZ").unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string());
    let mut line = format!("{} {} task-agent {}", ts, level, message.replace('\n', " "));
    if !kv.is_empty() {
        line.push(' ');
        line.push_str(&kv.join(" "));
    }
    let prefer_stdout = !(level == "ERROR" || level == "WARN");
    print_line(prefer_stdout, &line);
}

fn print_line(prefer_stdout: bool, line: &str) {
    let use_stdout = prefer_stdout && io::stdout().is_terminal();
    if use_stdout {
        println!("{}", line);
        let _ = io::stdout().flush();
    } else {
        eprintln!("{}", line);
        let _ = io::stderr().flush();
    }
}

fn git_output(workspace: &Path, args: &[&str]) -> Result<String, DynError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(workspace)
        .output()
        .map_err(|err| format!("Failed to run git {}: {}", args.join(" "), err))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git {} failed: {}", args.join(" "), stderr.trim()).into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn git_status(workspace: &Path, args: &[&str]) -> Result<(), DynError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(workspace)
        .output()
        .map_err(|err| format!("Failed to run git {}: {}", args.join(" "), err))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("git {} failed: {}", args.join(" "), stderr.trim()).into())
    }
}

fn commit_subject_from_title(title: &str, task_id: &str) -> String {
    let normalized = title.replace(['\n', '\r'], " ");
    let mut subject = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
    subject = subject.trim_end_matches('.').trim().to_string();
    if subject.is_empty() {
        subject = format!("Update {}", task_id);
    }
    subject = truncate_commit_subject(&subject, 50);
    subject = subject.trim_end_matches('.').trim().to_string();
    if subject.is_empty() {
        subject = format!("Update {}", task_id);
    }
    capitalize_first_char(subject)
}

fn truncate_commit_subject(subject: &str, max_chars: usize) -> String {
    let mut count = 0;
    let mut end_byte = 0;
    let mut last_space = None;
    for (idx, ch) in subject.char_indices() {
        if count == max_chars {
            break;
        }
        if ch.is_whitespace() {
            last_space = Some(idx);
        }
        count += 1;
        end_byte = idx + ch.len_utf8();
    }
    if count < max_chars {
        return subject.to_string();
    }
    let cut = last_space.unwrap_or(end_byte);
    subject[..cut].trim_end().to_string()
}

fn capitalize_first_char(subject: String) -> String {
    let mut chars = subject.chars();
    let Some(first) = chars.next() else {
        return subject;
    };
    if first.is_ascii_lowercase() {
        let mut updated = String::new();
        updated.push(first.to_ascii_uppercase());
        updated.extend(chars);
        updated
    } else {
        subject
    }
}

fn git_commit_progress(workspace: &Path, task_title: &str, task_id: &str) -> Result<(), DynError> {
    let status = git_output(workspace, &["status", "--porcelain"])?;
    if status.trim().is_empty() {
        return Ok(());
    }
    let message = commit_subject_from_title(task_title, task_id);
    git_status(workspace, &["add", "-A"])?;
    git_status(workspace, &["commit", "-m", &message])?;
    Ok(())
}

fn finalize_successful_task(
    workspace: &Path,
    task_id: &str,
    task_title: &str,
) -> Result<(), DynError> {
    let task_branch = format!("ralph/{}", task_id);
    let base_branch = base_branch();
    let msg = commit_subject_from_title(task_title, task_id);

    git_status(workspace, &["checkout", &task_branch])?;
    let _ = git_status(workspace, &["rebase", &base_branch]);
    git_status(workspace, &["reset", "--soft", &base_branch])?;
    git_status(workspace, &["add", "-A"])?;
    git_status(workspace, &["commit", "-m", &msg])?;
    git_status(workspace, &["checkout", &base_branch])?;
    git_status(workspace, &["merge", "--ff-only", &task_branch])?;
    git_status(workspace, &["branch", "-D", &task_branch])?;
    Ok(())
}

fn base_branch() -> String {
    std::env::var("BASE_BRANCH").unwrap_or_else(|_| "main".to_string())
}

fn load_tasks_root(path: &Path) -> Result<Value, DynError> {
    let raw = fs::read_to_string(path)
        .map_err(|err| format!("Failed to read tasks file {}: {}", path.display(), err))?;
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
    let current = obs.get("run_attempts").and_then(Value::as_u64).unwrap_or(0);
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
    log_command: Option<String>,
}

impl VerificationResult {
    fn skipped() -> Self {
        Self {
            ok: true,
            log_command: None,
        }
    }
}

fn run_verification(
    workspace: &Path,
    run_dir: &Path,
    task_verification_commands: &[String],
) -> Result<VerificationResult, DynError> {
    let verify_log = run_dir.join("verify.log");
    let log_file = File::create(&verify_log)?;
    let mut selected_cmd = None;

    if !task_verification_commands.is_empty() {
        let script = format!(
            "set -euo pipefail\n{}\n",
            task_verification_commands.join("\n")
        );
        let status = Command::new("bash")
            .arg("-lc")
            .arg(script)
            .current_dir(workspace)
            .stdout(log_file.try_clone()?)
            .stderr(log_file)
            .status()?;
        return Ok(VerificationResult {
            ok: status.success(),
            log_command: Some("task.verification.commands".to_string()),
        });
    }

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
        return Ok(VerificationResult {
            ok: true,
            log_command: None,
        });
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
    Ok(VerificationResult {
        ok,
        log_command: Some(cmd.join(" ")),
    })
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
    dir_contains_py(&tests_dir)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    fn args_to_strings(args: Vec<OsString>) -> Vec<String> {
        args.into_iter()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect()
    }

    #[test]
    fn assembly_command_args_include_expected_fields() {
        let workspace = PathBuf::from("/repo");
        let task_id = "T1";
        let run_id = "run-123";
        let paths = run_paths(&workspace, task_id, run_id);
        let config = ContextCompileConfig {
            enabled: true,
            token_budget: 9001,
            exclude_globs: vec![".git/**".to_string(), "node_modules/**".to_string()],
            exclude_runtime_globs: vec![".ralph/**".to_string()],
            ..Default::default()
        };

        let args = build_assembly_command_args(
            &workspace,
            task_id,
            &paths.assembly_task_path,
            &paths.pack_dir_abs,
            &paths.assembly_summary_path,
            &config,
        );
        let args = args_to_strings(args);

        let expected = vec![
            "build".to_string(),
            "--repo".to_string(),
            "/repo".to_string(),
            "--task".to_string(),
            format!("@{}", paths.assembly_task_path.display()),
            "--task-id".to_string(),
            "T1".to_string(),
            "--out".to_string(),
            paths.pack_dir_abs.display().to_string(),
            "--token-budget".to_string(),
            "9001".to_string(),
            "--exclude".to_string(),
            ".git/**".to_string(),
            "--exclude".to_string(),
            "node_modules/**".to_string(),
            "--exclude-runtime".to_string(),
            ".ralph/**".to_string(),
            "--summary-json".to_string(),
            paths.assembly_summary_path.display().to_string(),
        ];

        assert_eq!(args, expected);
    }
}

use std::{
    collections::HashSet,
    error::Error,
    ffi::OsString,
    fmt::{self, Display, Formatter},
    fs,
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use crate::task_metadata::{
    validate_task_metadata as validate_task_metadata_raw, TaskMetadataError,
};
use clap::{value_parser, Parser, ValueEnum};
use lever::context_compile::{ContextCompileConfig, ContextFailurePolicy};
use serde_json::Value;

mod rate_limit;
mod task_agent;
mod task_metadata;

const DEFAULT_COMMAND_PATH: &str = "internal";
const LEGACY_TASK_AGENT_PATH: &str = "bin/task-agent.sh";
const TASK_FILE_SEARCH_ORDER: [&str; 2] = ["prd.json", "tasks.json"];

#[derive(Debug, Clone)]
struct TaskRecord {
    task_id: String,
    status: Option<String>,
    model: Option<String>,
    raw: Value,
}

type DynError = Box<dyn Error + Send + Sync + 'static>;

struct ExecutionConfig {
    command_path: PathBuf,
    tasks_path: PathBuf,
    prompt: PathBuf,
    explicit_task_id: Option<String>,
    workspace: PathBuf,
    assignee: Option<String>,
    reset_task: bool,
    context_compile: ContextCompileConfig,
    context_compile_override: Option<bool>,
    context_failure_policy_override: Option<ContextFailurePolicy>,
}

struct GitWorkspaceGuard {
    workspace: PathBuf,
    orig_branch: String,
    orig_head: String,
    pre_run_head: String,
    dirty_files: Option<HashSet<String>>,
    stash_ref: Option<String>,
}

#[derive(Debug)]
struct TaskAgentExit {
    command: PathBuf,
    status: ExitStatus,
}

impl TaskAgentExit {
    fn exit_code(&self) -> Option<i32> {
        self.status.code()
    }
}

impl Display for TaskAgentExit {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "task agent {} exited with status {}",
            self.command.display(),
            self.status
        )
    }
}

impl std::error::Error for TaskAgentExit {}

#[derive(Debug)]
struct StopReasonError {
    reason: StopReason,
}

impl StopReasonError {
    fn exit_code(&self) -> i32 {
        self.reason.exit_code()
    }
}

impl Display for StopReasonError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.reason.message())
    }
}

impl std::error::Error for StopReasonError {}

#[derive(Debug, Clone)]
enum StopReason {
    Human { task_id: String, is_next: bool },
    Dependencies { task_id: String },
    Blocked { task_id: String },
}

impl StopReason {
    fn exit_code(&self) -> i32 {
        1
    }

    fn message(&self) -> String {
        match self {
            StopReason::Human { task_id, is_next } => {
                if *is_next {
                    format!("Next task {} requires human input.", task_id)
                } else {
                    format!("Task {} requires human input.", task_id)
                }
            }
            StopReason::Dependencies { task_id } => {
                format!("Task {} cannot start due to unmet dependencies.", task_id)
            }
            StopReason::Blocked { task_id } => {
                format!("Task {} blocked; manual intervention required.", task_id)
            }
        }
    }
}

#[derive(Parser, Debug)]
#[command(
    name = "lever",
    author,
    version,
    about = "Command center for Codex-driven workflows",
    long_about = None
)]
struct LeverArgs {
    #[arg(
        long,
        value_name = "PATH",
        help = "Tasks JSON file leveraged by the run (auto-discovered if omitted)"
    )]
    tasks: Option<PathBuf>,

    #[arg(
        long,
        value_name = "PATH",
        help = "Optional prompt file supplied to the agent"
    )]
    prompt: Option<PathBuf>,

    #[arg(
        long,
        value_name = "ID",
        help = "Explicit task ID leveraged by this invocation"
    )]
    task_id: Option<String>,

    #[arg(
        long,
        help = "Select first task whose status != completed and model != human (cannot combine with --task-id)"
    )]
    next: bool,

    #[arg(
        long,
        value_name = "PATH",
        help = "Workspace directory for the run (defaults to current directory)"
    )]
    workspace: Option<PathBuf>,

    #[arg(
        long,
        value_name = "NAME",
        help = "Assignee label forwarded to the downstream task agent"
    )]
    assignee: Option<String>,

    #[arg(
        long = "loop",
        alias = "loop-count",
        value_name = "COUNT",
        num_args = 0..=1,
        value_parser = value_parser!(u64),
        default_missing_value = "0",
        help = "Loop mode (no value = continuous, 0 = infinite loop, >0 fixed iterations)"
    )]
    loop_count: Option<u64>,

    #[arg(
        long,
        help = "Reset the selected task's attempts/status before running"
    )]
    reset_task: bool,

    #[arg(
        long,
        value_name = "SECONDS",
        value_parser = value_parser!(u64),
        help = "Delay between loop iterations (seconds, only used with --loop; default: 0)"
    )]
    delay: Option<u64>,

    #[arg(
        long = "context-compile",
        conflicts_with = "no_context_compile",
        help = "Enable context compilation for each run"
    )]
    context_compile: bool,

    #[arg(
        long = "no-context-compile",
        conflicts_with = "context_compile",
        help = "Disable context compilation for each run"
    )]
    no_context_compile: bool,

    #[arg(
        long = "context-failure-policy",
        value_enum,
        value_name = "POLICY",
        help = "Context compile failure policy (best-effort continues, required fails the run)"
    )]
    context_failure_policy: Option<ContextFailurePolicyArg>,

    #[arg(
        long = "command-path",
        value_name = "PATH",
        default_value = DEFAULT_COMMAND_PATH,
        help = "Executable invoked for each iteration (use 'internal' for Rust task agent)"
    )]
    command_path: PathBuf,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, ValueEnum)]
enum ContextFailurePolicyArg {
    #[value(name = "best-effort")]
    BestEffort,
    #[value(name = "required")]
    Required,
}

impl From<ContextFailurePolicyArg> for ContextFailurePolicy {
    fn from(value: ContextFailurePolicyArg) -> Self {
        match value {
            ContextFailurePolicyArg::BestEffort => ContextFailurePolicy::BestEffort,
            ContextFailurePolicyArg::Required => ContextFailurePolicy::Required,
        }
    }
}

fn validate_lever_args(args: &LeverArgs) -> Result<(), DynError> {
    let loop_mode = resolve_loop_mode(args.loop_count);
    if args.next && args.task_id.is_some() {
        Err("--next cannot be combined with --task-id"
            .to_string()
            .into())
    } else if args.delay.is_some() && matches!(loop_mode, LoopMode::Single) {
        Err("--delay requires --loop".to_string().into())
    } else {
        Ok(())
    }
}

fn resolve_context_compile_config(
    enable_flag: bool,
    disable_flag: bool,
    policy: Option<ContextFailurePolicyArg>,
) -> (
    ContextCompileConfig,
    Option<bool>,
    Option<ContextFailurePolicy>,
) {
    let mut config = ContextCompileConfig::default();
    let mut override_flag = None;
    let mut policy_override = None;
    if enable_flag {
        config.enabled = true;
        override_flag = Some(true);
    } else if disable_flag {
        config.enabled = false;
        override_flag = Some(false);
    }
    if let Some(policy) = policy {
        let resolved = ContextFailurePolicy::from(policy);
        config.policy = resolved;
        policy_override = Some(resolved);
    }
    (config, override_flag, policy_override)
}

fn main() -> Result<(), DynError> {
    let args = LeverArgs::parse();
    validate_lever_args(&args)?;

    let LeverArgs {
        tasks,
        prompt,
        task_id,
        next: _,
        workspace,
        assignee,
        loop_count,
        reset_task,
        delay,
        context_compile,
        no_context_compile,
        context_failure_policy,
        command_path,
    } = args;

    let (context_compile, context_compile_override, context_failure_policy_override) =
        resolve_context_compile_config(context_compile, no_context_compile, context_failure_policy);

    let resolved = resolve_paths(workspace, tasks, prompt, command_path)?;
    let ResolvedPaths {
        workspace,
        tasks_path,
        prompt_path,
        command_path,
    } = resolved;
    let tasks = load_tasks(&tasks_path)?;
    let loop_mode = resolve_loop_mode(loop_count);
    let selecting_next = task_id.is_none() && matches!(loop_mode, LoopMode::Single);
    let selected_task =
        determine_selected_task(&tasks, task_id.as_deref(), selecting_next, &tasks_path)?;
    if let Some(task) = &selected_task {
        if let Err(err) = validate_task_metadata(task) {
            eprintln!("{}", err);
            std::process::exit(err.exit_code());
        }
    }

    let shutdown_flag = Arc::new(AtomicBool::new(false));
    {
        let handler_flag = Arc::clone(&shutdown_flag);
        ctrlc::set_handler(move || {
            handler_flag.store(true, Ordering::SeqCst);
        })
        .map_err(DynError::from)?;
    }

    println!(
        "lever: tasks={} prompt={} command={}",
        tasks_path.display(),
        prompt_path.display(),
        command_path.display()
    );

    if let Some(task) = &selected_task {
        println!(
            "lever: selected task {} (status={} model={})",
            task.task_id,
            task.status.as_deref().unwrap_or("unstarted"),
            task.model.as_deref().unwrap_or("unset")
        );
    } else if loop_mode.is_looping() {
        println!("lever: loop mode active; deferring task selection");
    }

    let delay_duration = Duration::from_secs(delay.unwrap_or(0));

    let exec_config = ExecutionConfig {
        command_path,
        tasks_path: tasks_path.clone(),
        prompt: prompt_path.clone(),
        explicit_task_id: task_id.clone(),
        workspace: workspace.clone(),
        assignee,
        reset_task,
        context_compile,
        context_compile_override,
        context_failure_policy_override,
    };

    if let Err(err) = run_iterations(&exec_config, loop_mode, delay_duration, &shutdown_flag) {
        if let Some(task_err) = err.downcast_ref::<TaskAgentExit>() {
            eprintln!("{}", task_err);
            let code = task_err.exit_code().unwrap_or(1);
            std::process::exit(code);
        }
        if let Some(stop_err) = err.downcast_ref::<StopReasonError>() {
            eprintln!("{}", stop_err);
            std::process::exit(stop_err.exit_code());
        }
        if let Some(metadata_err) = err.downcast_ref::<TaskMetadataError>() {
            eprintln!("{}", metadata_err);
            std::process::exit(metadata_err.exit_code());
        }
        return Err(err);
    }

    Ok(())
}

fn load_tasks(path: &Path) -> Result<Vec<TaskRecord>, DynError> {
    let raw = fs::read_to_string(path).map_err(|err| {
        DynError::from(format!(
            "Failed to read tasks file {}: {}",
            path.display(),
            err
        ))
    })?;
    let root: Value = serde_json::from_str(&raw)?;
    let tasks_value = if let Some(tasks_field) = root.get("tasks") {
        tasks_field.clone()
    } else {
        root.clone()
    };

    match tasks_value {
        Value::Array(items) => {
            let mut tasks = Vec::with_capacity(items.len());
            for (index, item) in items.into_iter().enumerate() {
                let task_id = item
                    .get("task_id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .ok_or_else(|| {
                        format!(
                            "Task at index {} in {} is missing task_id",
                            index,
                            path.display()
                        )
                    })?;
                let status = item
                    .get("status")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let model = item
                    .get("model")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                tasks.push(TaskRecord {
                    task_id,
                    status,
                    model,
                    raw: item,
                });
            }
            Ok(tasks)
        }
        _ => Err(format!(
            "Tasks file {} does not contain an array of tasks",
            path.display()
        )
        .into()),
    }
}

fn determine_selected_task(
    tasks: &[TaskRecord],
    explicit_task_id: Option<&str>,
    should_select_next: bool,
    tasks_path: &Path,
) -> Result<Option<TaskRecord>, DynError> {
    if let Some(task_id) = explicit_task_id {
        let found = tasks
            .iter()
            .find(|task| task.task_id == task_id)
            .ok_or_else(|| {
                format!(
                    "Task ID '{}' was not found in {}",
                    task_id,
                    tasks_path.display()
                )
            })?;
        return Ok(Some(found.clone()));
    }

    if should_select_next {
        if let Some(next) = select_next_runnable(tasks) {
            return Ok(Some(next.clone()));
        }

        return Err(format!("No runnable task found in {}", tasks_path.display()).into());
    }

    Ok(None)
}

fn select_next_non_completed(tasks: &[TaskRecord]) -> Option<&TaskRecord> {
    tasks
        .iter()
        .find(|task| !status_is_completed(task.status.as_deref()))
}

fn select_next_runnable(tasks: &[TaskRecord]) -> Option<&TaskRecord> {
    tasks.iter().find(|task| {
        !status_is_completed(task.status.as_deref()) && !model_is_human(task.model.as_deref())
    })
}

fn status_is_completed(status: Option<&str>) -> bool {
    matches!(status.unwrap_or("unstarted"), "completed")
}

fn model_is_human(model: Option<&str>) -> bool {
    matches!(model.unwrap_or(""), "human")
}

fn resolve_tasks_path(tasks_arg: Option<PathBuf>, workspace: &Path) -> Result<PathBuf, DynError> {
    if let Some(explicit) = tasks_arg {
        let explicit_label = explicit.display().to_string();
        let candidate = resolve_relative_to_workspace(explicit, workspace);
        if candidate.is_file() {
            return canonicalize_existing_path(candidate);
        }
        Err(format!(
            "The specified tasks file {} does not exist or is not a file",
            explicit_label
        )
        .into())
    } else {
        for candidate in TASK_FILE_SEARCH_ORDER {
            let candidate_path = workspace.join(candidate);
            if candidate_path.is_file() {
                return canonicalize_existing_path(candidate_path);
            }
        }

        let location = if workspace_is_current_dir(workspace) {
            "the current directory".to_string()
        } else {
            workspace.display().to_string()
        };

        Err(format!(
            "No tasks file specified and neither {} exist in {}",
            TASK_FILE_SEARCH_ORDER.join(" nor "),
            location
        )
        .into())
    }
}

struct ResolvedPaths {
    workspace: PathBuf,
    tasks_path: PathBuf,
    prompt_path: PathBuf,
    command_path: PathBuf,
}

fn resolve_paths(
    workspace_arg: Option<PathBuf>,
    tasks_arg: Option<PathBuf>,
    prompt_arg: Option<PathBuf>,
    command_path_arg: PathBuf,
) -> Result<ResolvedPaths, DynError> {
    let workspace = resolve_workspace(workspace_arg)?;
    let tasks_path = resolve_tasks_path(tasks_arg, &workspace)?;
    let prompt_path = resolve_prompt_path(prompt_arg, &workspace)?;
    let command_path = resolve_command_path(command_path_arg, &workspace)?;
    Ok(ResolvedPaths {
        workspace,
        tasks_path,
        prompt_path,
        command_path,
    })
}

fn resolve_workspace(workspace_arg: Option<PathBuf>) -> Result<PathBuf, DynError> {
    let candidate = workspace_arg.unwrap_or_else(|| PathBuf::from("."));
    if candidate.is_dir() {
        canonicalize_existing_path(candidate)
    } else {
        Err(format!("Workspace not found: {}", candidate.display()).into())
    }
}

fn workspace_is_current_dir(workspace: &Path) -> bool {
    let current = std::env::current_dir()
        .ok()
        .and_then(|dir| fs::canonicalize(dir).ok());
    matches!(current, Some(dir) if dir == workspace)
}

fn resolve_relative_to_workspace(path: PathBuf, workspace: &Path) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        workspace.join(path)
    }
}

fn resolve_prompt_path(prompt_arg: Option<PathBuf>, workspace: &Path) -> Result<PathBuf, DynError> {
    let candidate = match prompt_arg {
        Some(explicit) => resolve_relative_to_workspace(explicit, workspace),
        None => default_prompt_path(workspace),
    };

    if candidate.is_file() {
        canonicalize_existing_path(candidate)
    } else {
        Err(format!("Prompt file not found: {}", candidate.display()).into())
    }
}

fn default_prompt_path(workspace: &Path) -> PathBuf {
    workspace.join("prompts/autonomous-senior-engineer.prompt.md")
}

fn resolve_command_path(path: PathBuf, workspace: &Path) -> Result<PathBuf, DynError> {
    let path_str = path.as_os_str().to_string_lossy();
    if path.is_absolute() {
        return canonicalize_or_fallback(&path, &path);
    }

    if path_str.contains('/') || path_str.contains('\\') {
        let anchored = workspace.join(&path);
        canonicalize_or_fallback(&anchored, &path)
    } else {
        Ok(path)
    }
}

fn canonicalize_existing_path(path: PathBuf) -> Result<PathBuf, DynError> {
    fs::canonicalize(&path)
        .map_err(|err| format!("Failed to resolve {}: {}", path.display(), err).into())
}

fn canonicalize_or_fallback(candidate: &Path, original: &Path) -> Result<PathBuf, DynError> {
    match fs::canonicalize(candidate) {
        Ok(resolved) => Ok(resolved),
        Err(err) => {
            if is_legacy_task_agent_path(original) {
                eprintln!(
                    "lever: warning: legacy --command-path {} not found; using internal task agent",
                    candidate.display()
                );
                Ok(PathBuf::from(DEFAULT_COMMAND_PATH))
            } else {
                Err(format!("Failed to resolve {}: {}", candidate.display(), err).into())
            }
        }
    }
}

fn is_legacy_task_agent_path(path: &Path) -> bool {
    let normalized = path.to_string_lossy().replace('\\', "/");
    normalized == LEGACY_TASK_AGENT_PATH || normalized.ends_with("/bin/task-agent.sh")
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum LoopMode {
    Single,
    Continuous,
    Count(u64),
}

impl LoopMode {
    fn is_looping(self) -> bool {
        !matches!(self, LoopMode::Single)
    }
}

fn resolve_loop_mode(loop_count: Option<u64>) -> LoopMode {
    match loop_count {
        None => LoopMode::Single,
        Some(0) => LoopMode::Continuous,
        Some(n) => LoopMode::Count(n),
    }
}

fn run_iterations(
    config: &ExecutionConfig,
    loop_mode: LoopMode,
    delay: Duration,
    shutdown_flag: &AtomicBool,
) -> Result<(), DynError> {
    match loop_mode {
        LoopMode::Single => run_single_iteration(config, shutdown_flag),
        LoopMode::Continuous => run_loop_iterations(config, None, delay, shutdown_flag),
        LoopMode::Count(limit) => run_loop_iterations(config, Some(limit), delay, shutdown_flag),
    }
}

fn run_single_iteration(
    config: &ExecutionConfig,
    shutdown_flag: &AtomicBool,
) -> Result<(), DynError> {
    let status = run_once(
        config,
        None,
        config.explicit_task_id.is_none(),
        shutdown_flag,
    )?;
    if shutdown_flag.load(Ordering::SeqCst) && matches!(status.code(), Some(130)) {
        println!("lever: shutdown requested during task-agent execution");
        return Ok(());
    }

    if status.success() {
        println!("lever: task-agent execution finished");
        return Ok(());
    }

    Err(Box::new(TaskAgentExit {
        command: config.command_path.clone(),
        status,
    }))
}

fn run_loop_iterations(
    config: &ExecutionConfig,
    max_iterations: Option<u64>,
    delay: Duration,
    shutdown_flag: &AtomicBool,
) -> Result<(), DynError> {
    let mut iteration = 0;

    loop {
        if shutdown_flag.load(Ordering::SeqCst) {
            println!(
                "lever: shutdown requested before starting iteration {}",
                iteration + 1
            );
            break;
        }

        iteration += 1;
        println!("lever: starting iteration {}", iteration);
        let mut selected_task = None;
        if config.explicit_task_id.is_none() {
            let tasks = load_tasks(&config.tasks_path)?;
            let next = select_next_non_completed(&tasks);
            if let Some(task) = next {
                if let Err(err) = validate_task_metadata(task) {
                    return Err(Box::new(err));
                }
                if model_is_human(task.model.as_deref()) {
                    return Err(Box::new(StopReasonError {
                        reason: StopReason::Human {
                            task_id: task.task_id.clone(),
                            is_next: true,
                        },
                    }));
                }
                if task.status.as_deref() == Some("blocked") {
                    println!("lever: resuming blocked task {}", task.task_id);
                }
                selected_task = Some(task.clone());
            } else {
                println!("lever: no remaining tasks to drive.");
                break;
            }
        }

        let status = run_once(
            config,
            selected_task.as_ref().map(|task| task.task_id.as_str()),
            false,
            shutdown_flag,
        )?;

        if shutdown_flag.load(Ordering::SeqCst) {
            println!(
                "lever: shutdown requested during task-agent execution (iteration {})",
                iteration
            );
            break;
        }

        match status.code() {
            Some(0) => {
                println!("lever: iteration {} completed", iteration);
            }
            Some(3) => {
                println!("lever: task agent reported no runnable tasks (code 3); stopping.");
                break;
            }
            Some(4) => {
                let task_id = selected_task
                    .as_ref()
                    .map(|task| task.task_id.clone())
                    .unwrap_or_else(|| {
                        config
                            .explicit_task_id
                            .clone()
                            .unwrap_or_else(|| "unknown".to_string())
                    });
                return Err(Box::new(StopReasonError {
                    reason: StopReason::Human {
                        task_id,
                        is_next: false,
                    },
                }));
            }
            Some(5) | Some(6) => {
                let task_id = selected_task
                    .as_ref()
                    .map(|task| task.task_id.clone())
                    .unwrap_or_else(|| {
                        config
                            .explicit_task_id
                            .clone()
                            .unwrap_or_else(|| "unknown".to_string())
                    });
                return Err(Box::new(StopReasonError {
                    reason: StopReason::Dependencies { task_id },
                }));
            }
            Some(10) | Some(11) => {
                let task_id = selected_task
                    .as_ref()
                    .map(|task| task.task_id.clone())
                    .unwrap_or_else(|| {
                        config
                            .explicit_task_id
                            .clone()
                            .unwrap_or_else(|| "unknown".to_string())
                    });
                return Err(Box::new(StopReasonError {
                    reason: StopReason::Blocked { task_id },
                }));
            }
            Some(130) => {
                if shutdown_flag.load(Ordering::SeqCst) {
                    println!(
                        "lever: shutdown requested during task-agent execution (iteration {})",
                        iteration
                    );
                    break;
                }
                return Err(Box::new(TaskAgentExit {
                    command: config.command_path.clone(),
                    status,
                }));
            }
            Some(code) if code < 10 => {
                return Err(Box::new(TaskAgentExit {
                    command: config.command_path.clone(),
                    status,
                }));
            }
            Some(code) => {
                println!("lever: task agent ended with {} (continuing).", code);
            }
            None => {
                return Err(Box::new(TaskAgentExit {
                    command: config.command_path.clone(),
                    status,
                }));
            }
        }

        if let Some(limit) = max_iterations {
            if iteration >= limit {
                println!("lever: --loop limit reached ({})", limit);
                break;
            }
        }

        if delay > Duration::ZERO && sleep_with_shutdown(delay, shutdown_flag) {
            println!(
                "lever: shutdown requested during delay before iteration {}",
                iteration + 1
            );
            break;
        }
    }

    Ok(())
}

fn sleep_with_shutdown(delay: Duration, shutdown_flag: &AtomicBool) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < delay {
        if shutdown_flag.load(Ordering::SeqCst) {
            return true;
        }
        let remaining = delay.saturating_sub(start.elapsed());
        let nap = remaining.min(Duration::from_millis(100));
        std::thread::sleep(nap);
    }
    false
}

fn run_once(
    config: &ExecutionConfig,
    task_id_override: Option<&str>,
    allow_next: bool,
    shutdown_flag: &AtomicBool,
) -> Result<ExitStatus, DynError> {
    let task_id_for_git = resolve_task_id_for_git(config, task_id_override, allow_next)?;
    let prompt_content = read_prompt_content(&config.prompt)?;
    let internal = is_internal_task_agent(&config.command_path);
    let temp_prompt_path = if internal {
        Some(write_temp_prompt(&prompt_content)?)
    } else {
        None
    };
    let _git_guard = GitWorkspaceGuard::prepare(&config.workspace, task_id_for_git.as_deref())?;
    let mut restored_prompt = false;
    if !internal && !config.prompt.is_file() {
        if let Some(parent) = config.prompt.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&config.prompt, &prompt_content)?;
        restored_prompt = true;
    }

    let result: Result<ExitStatus, DynError> = if internal {
        let agent_config = task_agent::TaskAgentConfig {
            tasks_path: config.tasks_path.clone(),
            prompt_path: temp_prompt_path.clone().unwrap(),
            workspace: config.workspace.clone(),
            reset_task: config.reset_task,
            explicit_task_id: config.explicit_task_id.clone(),
            context_compile: config.context_compile.clone(),
        };
        let exit_code = task_agent::run_task_agent(
            &agent_config,
            task_id_override,
            allow_next,
            Some(shutdown_flag),
        )?;
        Ok(exit_status_from_code(exit_code))
    } else {
        let mut command = Command::new(&config.command_path);
        command.args(config.task_agent_args(task_id_override, allow_next, &config.prompt));
        command.current_dir(&config.workspace);
        Ok(command.status()?)
    };

    if restored_prompt {
        if let Err(err) = fs::remove_file(&config.prompt) {
            eprintln!(
                "Warning: failed to remove restored prompt file {}: {}",
                config.prompt.display(),
                err
            );
        }
    }
    if let Some(temp_prompt_path) = temp_prompt_path {
        let _ = fs::remove_file(temp_prompt_path);
    }

    let status = result?;

    if shutdown_flag.load(Ordering::SeqCst) && matches!(status.code(), Some(130)) {
        return Ok(status);
    }

    Ok(status)
}

fn is_internal_task_agent(path: &Path) -> bool {
    path == Path::new("internal")
}

fn exit_status_from_code(code: i32) -> ExitStatus {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        ExitStatus::from_raw(code << 8)
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::ExitStatusExt;
        ExitStatus::from_raw(code as u32)
    }
}

fn validate_task_metadata(task: &TaskRecord) -> Result<(), TaskMetadataError> {
    validate_task_metadata_raw(&task.task_id, &task.raw)
}

impl ExecutionConfig {
    fn task_agent_args(
        &self,
        task_id_override: Option<&str>,
        allow_next: bool,
        prompt_path: &Path,
    ) -> Vec<OsString> {
        let mut args = vec![
            "--tasks".into(),
            self.tasks_path.clone().into_os_string(),
            "--workspace".into(),
            self.workspace.clone().into_os_string(),
            "--prompt".into(),
            prompt_path.as_os_str().to_os_string(),
        ];

        if let Some(assignee) = &self.assignee {
            args.push("--assignee".into());
            args.push(assignee.clone().into());
        }

        if let Some(task_id) = task_id_override {
            args.push("--task-id".into());
            args.push(task_id.into());
        } else if let Some(task_id) = &self.explicit_task_id {
            args.push("--task-id".into());
            args.push(task_id.clone().into());
        } else if allow_next {
            args.push("--next".into());
        }

        if self.reset_task {
            args.push("--reset-task".into());
        }

        if let Some(enabled) = self.context_compile_override {
            if enabled {
                args.push("--context-compile".into());
            } else {
                args.push("--no-context-compile".into());
            }
        }

        if let Some(policy) = self.context_failure_policy_override {
            args.push("--context-failure-policy".into());
            args.push(context_failure_policy_arg(policy).into());
        }

        args
    }
}

fn context_failure_policy_arg(policy: ContextFailurePolicy) -> &'static str {
    match policy {
        ContextFailurePolicy::BestEffort => "best-effort",
        ContextFailurePolicy::Required => "required",
    }
}

fn read_prompt_content(prompt_path: &Path) -> Result<String, DynError> {
    fs::read_to_string(prompt_path).map_err(|err| {
        DynError::from(format!(
            "Failed to read prompt file {}: {}",
            prompt_path.display(),
            err
        ))
    })
}

fn write_temp_prompt(content: &str) -> Result<PathBuf, DynError> {
    let stamp = utc_timestamp()?;
    let filename = format!("lever-prompt-{}-{}.md", stamp, std::process::id());
    let temp_path = std::env::temp_dir().join(filename);
    fs::write(&temp_path, content).map_err(|err| {
        DynError::from(format!(
            "Failed to write prompt copy {}: {}",
            temp_path.display(),
            err
        ))
    })?;
    Ok(temp_path)
}

fn resolve_task_id_for_git(
    config: &ExecutionConfig,
    task_id_override: Option<&str>,
    allow_next: bool,
) -> Result<Option<String>, DynError> {
    if let Some(task_id) = task_id_override {
        return Ok(Some(task_id.to_string()));
    }
    if let Some(task_id) = &config.explicit_task_id {
        return Ok(Some(task_id.clone()));
    }
    if allow_next {
        let tasks = load_tasks(&config.tasks_path)?;
        if let Some(task) = select_next_runnable(&tasks) {
            return Ok(Some(task.task_id.clone()));
        }
        return Err(format!("No runnable task found in {}", config.tasks_path.display()).into());
    }
    Ok(None)
}

impl GitWorkspaceGuard {
    fn prepare(workspace: &Path, task_id: Option<&str>) -> Result<Self, DynError> {
        ensure_git_available()?;
        ensure_git_repo(workspace)?;

        let orig_branch = git_output(workspace, &["rev-parse", "--abbrev-ref", "HEAD"])?
            .trim()
            .to_string();
        let orig_head = git_output(workspace, &["rev-parse", "HEAD"])?
            .trim()
            .to_string();
        let pre_run_head = orig_head.clone();

        let mut dirty_files = None;
        let mut stash_ref = None;

        let status = git_output(workspace, &["status", "--porcelain"])?;
        if !status.trim().is_empty() {
            dirty_files = Some(record_dirty_files(workspace)?);
            let stash_msg = format!(
                "ralph(task-agent): auto-stash {}-{}",
                utc_timestamp()?,
                std::process::id()
            );
            git_status(workspace, &["stash", "push", "-u", "-m", &stash_msg])?;
            stash_ref = find_stash_ref(workspace, &stash_msg)?;
            if let Some(stash) = &stash_ref {
                eprintln!("Stashed local changes as {}.", stash);
            } else {
                eprintln!("Warning: auto-stash created but ref not found; check git stash list.");
            }
        }

        if let Some(task_id) = task_id {
            let base_branch = base_branch();
            checkout_task_branch(workspace, &base_branch, task_id)?;
        }

        Ok(Self {
            workspace: workspace.to_path_buf(),
            orig_branch,
            orig_head,
            pre_run_head,
            dirty_files,
            stash_ref,
        })
    }

    fn restore_local_changes(&self) -> Result<(), DynError> {
        let stash_ref = match &self.stash_ref {
            Some(stash_ref) => stash_ref,
            None => return Ok(()),
        };

        let dirty_files = match &self.dirty_files {
            Some(dirty_files) => dirty_files,
            None => {
                eprintln!(
                    "Warning: missing dirty file list; leaving {} for manual apply.",
                    stash_ref
                );
                return Ok(());
            }
        };

        let run_files_output = match git_output(
            &self.workspace,
            &["diff", "--name-only", &self.pre_run_head, "HEAD"],
        ) {
            Ok(output) => output,
            Err(_) => {
                eprintln!(
                    "Warning: unable to compute run changes; leaving {} for manual apply.",
                    stash_ref
                );
                return Ok(());
            }
        };

        let run_files: HashSet<String> = run_files_output
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect();

        if dirty_files.iter().any(|file| run_files.contains(file)) {
            eprintln!(
                "Warning: stash {} overlaps run changes; apply manually.",
                stash_ref
            );
            return Ok(());
        }

        if self.orig_branch == "HEAD" {
            if git_status(&self.workspace, &["checkout", "--detach", &self.orig_head]).is_err() {
                eprintln!(
                    "Warning: unable to restore detached HEAD; leaving {}.",
                    stash_ref
                );
                return Ok(());
            }
        } else if git_status(&self.workspace, &["checkout", &self.orig_branch]).is_err() {
            eprintln!(
                "Warning: unable to checkout {}; leaving {}.",
                self.orig_branch, stash_ref
            );
            return Ok(());
        }

        if git_status(&self.workspace, &["stash", "apply", stash_ref]).is_ok() {
            let _ = git_status(&self.workspace, &["stash", "drop", stash_ref]);
        } else {
            eprintln!(
                "Warning: stash {} could not be applied cleanly; leaving stash for manual apply.",
                stash_ref
            );
        }

        Ok(())
    }
}

impl Drop for GitWorkspaceGuard {
    fn drop(&mut self) {
        if let Err(err) = self.restore_local_changes() {
            eprintln!("Warning: failed to restore local changes: {}", err);
        }
    }
}

fn ensure_git_available() -> Result<(), DynError> {
    let output = Command::new("git")
        .arg("--version")
        .output()
        .map_err(|_| "Missing dependency: git".to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err("Missing dependency: git".to_string().into())
    }
}

fn ensure_git_repo(workspace: &Path) -> Result<(), DynError> {
    let output = Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(workspace)
        .output()
        .map_err(|err| format!("Failed to run git: {}", err))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!("Not a git repository: {}", workspace.display()).into())
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

fn record_dirty_files(workspace: &Path) -> Result<HashSet<String>, DynError> {
    let mut files = HashSet::new();
    for args in [
        ["diff", "--name-only"].as_slice(),
        ["diff", "--name-only", "--cached"].as_slice(),
        ["ls-files", "--others", "--exclude-standard"].as_slice(),
    ] {
        let output = git_output(workspace, args)?;
        for line in output.lines() {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                files.insert(trimmed.to_string());
            }
        }
    }
    Ok(files)
}

fn find_stash_ref(workspace: &Path, stash_msg: &str) -> Result<Option<String>, DynError> {
    let output = git_output(workspace, &["stash", "list", "--format=%gd %gs"])?;
    for line in output.lines() {
        if line.contains(stash_msg) {
            if let Some(reference) = line.split_whitespace().next() {
                return Ok(Some(reference.to_string()));
            }
        }
    }
    Ok(None)
}

fn base_branch() -> String {
    std::env::var("BASE_BRANCH").unwrap_or_else(|_| "main".to_string())
}

fn checkout_task_branch(
    workspace: &Path,
    base_branch: &str,
    task_id: &str,
) -> Result<(), DynError> {
    let task_branch = format!("ralph/{}", task_id);
    git_status(workspace, &["checkout", base_branch])?;
    let exists = Command::new("git")
        .args([
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{}", task_branch),
        ])
        .current_dir(workspace)
        .output()
        .map_err(|err| format!("Failed to run git show-ref: {}", err))?
        .status
        .success();
    if exists {
        git_status(workspace, &["checkout", &task_branch])?;
    } else {
        git_status(workspace, &["checkout", "-b", &task_branch])?;
    }
    Ok(())
}

fn utc_timestamp() -> Result<String, DynError> {
    let output = Command::new("date")
        .args(["-u", "+%Y%m%dT%H%M%SZ"])
        .output()
        .map_err(|err| format!("Failed to run date: {}", err))?;
    if !output.status.success() {
        return Err("Failed to resolve UTC timestamp".to_string().into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(task_id: &str, status: Option<&str>, model: Option<&str>) -> TaskRecord {
        TaskRecord {
            task_id: task_id.to_string(),
            status: status.map(str::to_string),
            model: model.map(str::to_string),
            raw: Value::Null,
        }
    }

    #[test]
    fn determine_selected_task_uses_explicit_task_id() {
        let tasks = vec![task("ALPHA", None, None), task("BETA", None, None)];
        let selected = determine_selected_task(&tasks, Some("BETA"), false, Path::new("prd.json"))
            .expect("selection failed");
        let selected = selected.expect("expected task selection");
        assert_eq!(selected.task_id, "BETA");
    }

    #[test]
    fn determine_selected_task_selects_next_runnable() {
        let tasks = vec![
            task("DONE", Some("completed"), None),
            task("HUMAN", None, Some("human")),
            task("NEXT", None, None),
        ];
        let selected = determine_selected_task(&tasks, None, true, Path::new("prd.json"))
            .expect("selection failed");
        let selected = selected.expect("expected task selection");
        assert_eq!(selected.task_id, "NEXT");
    }

    #[test]
    fn determine_selected_task_default_is_none() {
        let tasks = vec![task("ALPHA", None, None)];
        let selected = determine_selected_task(&tasks, None, false, Path::new("prd.json"))
            .expect("selection failed");
        assert!(selected.is_none());
    }

    #[test]
    fn stop_reason_exit_codes_map_to_nonzero() {
        let reasons = vec![
            StopReason::Human {
                task_id: "T1".to_string(),
                is_next: false,
            },
            StopReason::Dependencies {
                task_id: "T2".to_string(),
            },
            StopReason::Blocked {
                task_id: "T3".to_string(),
            },
        ];

        for reason in reasons {
            let err = StopReasonError { reason };
            assert_eq!(err.exit_code(), 1);
        }
    }
}

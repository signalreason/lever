use std::{
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

use clap::{value_parser, Parser};
use serde_json::Value;

mod rate_limit;

const DEFAULT_COMMAND_PATH: &str = "bin/task-agent.sh";
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

#[derive(Debug)]
struct TaskMetadataError {
    task_id: String,
    missing: Vec<&'static str>,
}

impl TaskMetadataError {
    fn exit_code(&self) -> i32 {
        2
    }
}

impl Display for TaskMetadataError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Task {} missing required metadata: {}",
            self.task_id,
            self.missing.join(", ")
        )
    }
}

impl std::error::Error for TaskMetadataError {}

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
        long = "command-path",
        value_name = "PATH",
        default_value = DEFAULT_COMMAND_PATH,
        help = "Executable invoked for each iteration (defaults to bin/task-agent.sh)"
    )]
    command_path: PathBuf,
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
        command_path,
    } = args;

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
        None => default_prompt_path()?,
    };

    if candidate.is_file() {
        canonicalize_existing_path(candidate)
    } else {
        Err(format!("Prompt file not found: {}", candidate.display()).into())
    }
}

fn default_prompt_path() -> Result<PathBuf, DynError> {
    let home = std::env::var_os("HOME")
        .ok_or_else(|| "HOME environment variable is not set; cannot resolve default prompt file")?;
    Ok(PathBuf::from(home).join(".prompts/autonomous-senior-engineer.prompt.md"))
}

fn resolve_command_path(path: PathBuf, workspace: &Path) -> Result<PathBuf, DynError> {
    let path_str = path.as_os_str().to_string_lossy();
    if path.is_absolute() {
        return canonicalize_existing_path(path);
    }

    if path_str.contains('/') || path_str.contains('\\') {
        let anchored = workspace.join(&path);
        return canonicalize_existing_path(anchored);
    } else {
        Ok(path)
    }
}

fn canonicalize_existing_path(path: PathBuf) -> Result<PathBuf, DynError> {
    fs::canonicalize(&path)
        .map_err(|err| format!("Failed to resolve {}: {}", path.display(), err).into())
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
    let status = run_once(config, None, config.explicit_task_id.is_none(), shutdown_flag)?;
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
                    .unwrap_or_else(|| config.explicit_task_id.clone().unwrap_or_else(|| "unknown".to_string()));
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
                    .unwrap_or_else(|| config.explicit_task_id.clone().unwrap_or_else(|| "unknown".to_string()));
                return Err(Box::new(StopReasonError {
                    reason: StopReason::Dependencies { task_id },
                }));
            }
            Some(10) | Some(11) => {
                let task_id = selected_task
                    .as_ref()
                    .map(|task| task.task_id.clone())
                    .unwrap_or_else(|| config.explicit_task_id.clone().unwrap_or_else(|| "unknown".to_string()));
                return Err(Box::new(StopReasonError {
                    reason: StopReason::Blocked { task_id },
                }));
            }
            Some(130) => {
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

        if delay > Duration::ZERO {
            std::thread::sleep(delay);
        }
    }

    Ok(())
}

fn run_once(
    config: &ExecutionConfig,
    task_id_override: Option<&str>,
    allow_next: bool,
    shutdown_flag: &AtomicBool,
) -> Result<ExitStatus, DynError> {
    let mut command = Command::new(&config.command_path);
    command.args(config.task_agent_args(task_id_override, allow_next));
    command.current_dir(&config.workspace);

    let status = command.status()?;
    if shutdown_flag.load(Ordering::SeqCst) && matches!(status.code(), Some(130)) {
        return Ok(status);
    }

    Ok(status)
}

fn validate_task_metadata(task: &TaskRecord) -> Result<(), TaskMetadataError> {
    let title_valid = matches!(
        task.raw.get("title"),
        Some(Value::String(value)) if !value.is_empty()
    );

    let dod_valid = match task.raw.get("definition_of_done") {
        Some(Value::Array(items)) => {
            !items.is_empty()
                && items.iter().all(|item| match item {
                    Value::String(value) => !value.is_empty(),
                    _ => false,
                })
        }
        _ => false,
    };

    let recommended_valid = match task.raw.get("recommended") {
        Some(Value::Object(map)) => {
            if map.len() != 1 {
                false
            } else {
                matches!(
                    map.get("approach"),
                    Some(Value::String(value)) if !value.is_empty()
                )
            }
        }
        _ => false,
    };

    let mut missing = Vec::new();
    if !title_valid {
        missing.push("title");
    }
    if !dod_valid {
        missing.push("definition_of_done");
    }
    if !recommended_valid {
        missing.push("recommended.approach");
    }

    if missing.is_empty() {
        Ok(())
    } else {
        Err(TaskMetadataError {
            task_id: task.task_id.clone(),
            missing,
        })
    }
}

impl ExecutionConfig {
    fn task_agent_args(&self, task_id_override: Option<&str>, allow_next: bool) -> Vec<OsString> {
        let mut args = Vec::new();
        args.push("--tasks".into());
        args.push(self.tasks_path.clone().into_os_string());
        args.push("--workspace".into());
        args.push(self.workspace.clone().into_os_string());
        args.push("--prompt".into());
        args.push(self.prompt.clone().into_os_string());

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

        args
    }
}

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
};

use clap::{value_parser, Parser};
use serde::Deserialize;
use serde_json::Value;

const DEFAULT_COMMAND_PATH: &str = "bin/task-agent.sh";
const TASK_FILE_SEARCH_ORDER: [&str; 2] = ["prd.json", "tasks.json"];

#[derive(Debug, Clone, Deserialize)]
struct Task {
    task_id: String,
    status: Option<String>,
    model: Option<String>,
}

type DynError = Box<dyn Error + Send + Sync + 'static>;

struct ExecutionConfig {
    command_path: PathBuf,
    tasks_path: PathBuf,
    prompt: Option<PathBuf>,
    explicit_task_id: Option<String>,
    workspace: PathBuf,
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

impl Error for TaskAgentExit {}

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
        long = "command-path",
        value_name = "PATH",
        default_value = DEFAULT_COMMAND_PATH,
        help = "Executable invoked for each iteration (defaults to bin/task-agent.sh)"
    )]
    command_path: PathBuf,
}

fn main() -> Result<(), DynError> {
    let LeverArgs {
        tasks,
        prompt,
        task_id,
        loop_count,
        command_path,
    } = LeverArgs::parse();

    let tasks_path = resolve_tasks_path(tasks)?;
    let workspace = tasks_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let command_path = resolve_command_path(command_path, &workspace)?;
    let tasks = load_tasks(&tasks_path)?;
    let loop_mode = resolve_loop_mode(loop_count);
    let selecting_next = task_id.is_none() && matches!(loop_mode, LoopMode::Single);
    let selected_task =
        determine_selected_task(&tasks, task_id.as_deref(), selecting_next, &tasks_path)?;

    let shutdown_flag = Arc::new(AtomicBool::new(false));
    {
        let handler_flag = Arc::clone(&shutdown_flag);
        ctrlc::set_handler(move || {
            handler_flag.store(true, Ordering::SeqCst);
        })
        .map_err(DynError::from)?;
    }

    let prompt_label = prompt
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "unset".into());

    println!(
        "lever: tasks={} prompt={} command={}",
        tasks_path.display(),
        prompt_label,
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

    let exec_config = ExecutionConfig {
        command_path,
        tasks_path: tasks_path.clone(),
        prompt: prompt.clone(),
        explicit_task_id: task_id.clone(),
        workspace,
    };

    if let Err(err) = run_iterations(&exec_config, loop_mode, &shutdown_flag) {
        if let Some(task_err) = err.downcast_ref::<TaskAgentExit>() {
            eprintln!("{}", task_err);
            let code = task_err.exit_code().unwrap_or(1);
            std::process::exit(code);
        }
        return Err(err);
    }

    Ok(())
}

fn load_tasks(path: &Path) -> Result<Vec<Task>, DynError> {
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
                let task: Task = serde_json::from_value(item).map_err(|err| {
                    format!(
                        "Failed to decode task at index {} in {}: {}",
                        index,
                        path.display(),
                        err
                    )
                })?;
                tasks.push(task);
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
    tasks: &[Task],
    explicit_task_id: Option<&str>,
    should_select_next: bool,
    tasks_path: &Path,
) -> Result<Option<Task>, DynError> {
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
        if let Some(next) = select_next_non_completed(tasks) {
            return Ok(Some(next.clone()));
        }

        return Err(format!("No non-completed task found in {}", tasks_path.display()).into());
    }

    Ok(None)
}

fn select_next_non_completed(tasks: &[Task]) -> Option<&Task> {
    tasks
        .iter()
        .find(|task| !status_is_completed(task.status.as_deref()))
}

fn status_is_completed(status: Option<&str>) -> bool {
    matches!(status.unwrap_or("unstarted"), "completed")
}

fn resolve_tasks_path(tasks_arg: Option<PathBuf>) -> Result<PathBuf, DynError> {
    if let Some(explicit) = tasks_arg {
        if explicit.is_file() {
            return canonicalize_existing_path(explicit);
        } else {
            Err(format!(
                "The specified tasks file {} does not exist or is not a file",
                explicit.display()
            )
            .into())
        }
    } else {
        for candidate in TASK_FILE_SEARCH_ORDER {
            let candidate_path = Path::new(candidate);
            if candidate_path.is_file() {
                return canonicalize_existing_path(candidate_path.to_path_buf());
            }
        }

        Err(format!(
            "No tasks file specified and neither {} exist in the current directory",
            TASK_FILE_SEARCH_ORDER.join(" nor ")
        )
        .into())
    }
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
    shutdown_flag: &AtomicBool,
) -> Result<(), DynError> {
    match loop_mode {
        LoopMode::Single => run_single_iteration(config, shutdown_flag),
        LoopMode::Continuous => run_loop_iterations(config, None, shutdown_flag),
        LoopMode::Count(limit) => run_loop_iterations(config, Some(limit), shutdown_flag),
    }
}

fn run_single_iteration(
    config: &ExecutionConfig,
    shutdown_flag: &AtomicBool,
) -> Result<(), DynError> {
    run_once(config, config.explicit_task_id.is_none(), shutdown_flag)?;
    if shutdown_flag.load(Ordering::SeqCst) {
        println!("lever: shutdown requested during task-agent execution");
    } else {
        println!("lever: task-agent execution finished");
    }

    Ok(())
}

fn run_loop_iterations(
    config: &ExecutionConfig,
    max_iterations: Option<u64>,
    shutdown_flag: &AtomicBool,
) -> Result<(), DynError> {
    let mut iteration = 0;
    let use_next = config.explicit_task_id.is_none();

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
        run_once(config, use_next, shutdown_flag)?;

        if shutdown_flag.load(Ordering::SeqCst) {
            println!(
                "lever: shutdown requested during task-agent execution (iteration {})",
                iteration
            );
            break;
        } else {
            println!("lever: iteration {} completed", iteration);
        }

        if let Some(limit) = max_iterations {
            if iteration >= limit {
                println!("lever: --loop limit reached ({})", limit);
                break;
            }
        }
    }

    Ok(())
}

fn run_once(
    config: &ExecutionConfig,
    allow_next: bool,
    shutdown_flag: &AtomicBool,
) -> Result<(), DynError> {
    let mut command = Command::new(&config.command_path);
    command.args(config.task_agent_args(allow_next));
    command.current_dir(&config.workspace);

    let status = command.status()?;

    if status.success()
        || (shutdown_flag.load(Ordering::SeqCst) && matches!(status.code(), Some(130)))
    {
        Ok(())
    } else {
        Err(Box::new(TaskAgentExit {
            command: config.command_path.clone(),
            status,
        }))
    }
}

impl ExecutionConfig {
    fn task_agent_args(&self, allow_next: bool) -> Vec<OsString> {
        let mut args = Vec::new();
        args.push("--tasks".into());
        args.push(self.tasks_path.clone().into_os_string());
        args.push("--workspace".into());
        args.push(self.workspace.clone().into_os_string());

        if let Some(prompt) = &self.prompt {
            args.push("--prompt".into());
            args.push(prompt.clone().into_os_string());
        }

        if let Some(task_id) = &self.explicit_task_id {
            args.push("--task-id".into());
            args.push(task_id.clone().into());
        } else if allow_next {
            args.push("--next".into());
        }

        args
    }
}

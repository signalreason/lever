use std::{
    error::Error,
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc,
};

use clap::Parser;

const PLACEHOLDER_COMMAND: &str = "/bin/true";
type DynError = Box<dyn Error + Send + Sync + 'static>;

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
        long = "command-path",
        value_name = "PATH",
        default_value = PLACEHOLDER_COMMAND,
        help = "Executable that the placeholder runner will invoke"
    )]
    command_path: PathBuf,
}

const TASK_FILE_SEARCH_ORDER: [&str; 2] = ["prd.json", "tasks.json"];

fn main() -> Result<(), DynError> {
    let LeverArgs {
        tasks,
        prompt,
        command_path,
    } = LeverArgs::parse();

    let tasks_path = resolve_tasks_path(tasks)?;

    let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>();
    ctrlc::set_handler(move || {
        let _ = shutdown_tx.send(());
    })
    .map_err(DynError::from)?;

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

    run_placeholder_command(&command_path)?;

    if shutdown_rx.try_recv().is_ok() {
        println!("lever: shutdown requested during placeholder execution");
    } else {
        println!("lever: placeholder execution finished");
    }

    Ok(())
}

fn resolve_tasks_path(tasks_arg: Option<PathBuf>) -> Result<PathBuf, DynError> {
    if let Some(explicit) = tasks_arg {
        if explicit.is_file() {
            Ok(explicit)
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
                return Ok(candidate_path.to_path_buf());
            }
        }

        Err(format!(
            "No tasks file specified and neither {} exist in the current directory",
            TASK_FILE_SEARCH_ORDER.join(" nor ")
        )
        .into())
    }
}

fn run_placeholder_command(command_path: &Path) -> Result<(), DynError> {
    let status = Command::new(command_path).status()?;

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "placeholder command {} exited with status {}",
            command_path.display(),
            status
        )
        .into())
    }
}

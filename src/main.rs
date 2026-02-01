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
        default_value = "prd.json",
        help = "Tasks JSON file leveraged by the run"
    )]
    tasks: PathBuf,

    #[arg(long, value_name = "PATH", help = "Optional prompt file supplied to the agent")]
    prompt: Option<PathBuf>,

    #[arg(
        long = "command-path",
        value_name = "PATH",
        default_value = PLACEHOLDER_COMMAND,
        help = "Executable that the placeholder runner will invoke"
    )]
    command_path: PathBuf,
}

fn main() -> Result<(), DynError> {
    let args = LeverArgs::parse();

    let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>();
    ctrlc::set_handler(move || {
        let _ = shutdown_tx.send(());
    })
    .map_err(DynError::from)?;

    let prompt_label = args
        .prompt
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "unset".into());

    println!(
        "lever: tasks={} prompt={} command={}",
        args.tasks.display(),
        prompt_label,
        args.command_path.display()
    );

    run_placeholder_command(&args.command_path)?;

    if shutdown_rx.try_recv().is_ok() {
        println!("lever: shutdown requested during placeholder execution");
    } else {
        println!("lever: placeholder execution finished");
    }

    Ok(())
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

use std::fmt::{self, Display, Formatter};
use std::path::Path;
use std::process::Command;

pub const CONTRACT_VERSION: &str = "2026-02-16";
pub const REQUIRED_BUILD_FLAGS: &[&str] = &[
    "--repo",
    "--task",
    "--task-id",
    "--out",
    "--token-budget",
    "--exclude",
    "--exclude-runtime",
    "--summary-json",
];
pub const REQUIRED_PACK_FILES: &[&str] = &[
    "manifest.json",
    "index.json",
    "context.md",
    "policy.md",
    "lint.json",
];

#[derive(Debug)]
pub enum AssemblyContractError {
    MissingDependency {
        command: String,
    },
    CommandFailed {
        command: String,
        status: Option<i32>,
        output: String,
    },
    MissingBuildFlags {
        missing: Vec<&'static str>,
    },
}

impl Display for AssemblyContractError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            AssemblyContractError::MissingDependency { command } => {
                write!(f, "Missing dependency: {}", command)
            }
            AssemblyContractError::CommandFailed {
                command,
                status,
                output,
            } => write!(
                f,
                "Assembly contract validation failed (version {}): command '{}' exited {:?}. {}",
                CONTRACT_VERSION, command, status, output
            ),
            AssemblyContractError::MissingBuildFlags { missing } => write!(
                f,
                "Assembly CLI contract mismatch (version {}): missing required build flags: {}. See docs/assembly-contract.md.",
                CONTRACT_VERSION,
                missing.join(", ")
            ),
        }
    }
}

impl std::error::Error for AssemblyContractError {}

pub fn validate_assembly_contract(assembly_path: &Path) -> Result<(), AssemblyContractError> {
    run_command(assembly_path, &["--version"])?;
    let help_output = run_command(assembly_path, &["build", "--help"])?;
    validate_build_help(&help_output)
}

pub fn validate_build_help(help_output: &str) -> Result<(), AssemblyContractError> {
    let missing: Vec<&'static str> = REQUIRED_BUILD_FLAGS
        .iter()
        .copied()
        .filter(|flag| !help_output.contains(flag))
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(AssemblyContractError::MissingBuildFlags { missing })
    }
}

fn run_command(command_path: &Path, args: &[&str]) -> Result<String, AssemblyContractError> {
    let output = Command::new(command_path)
        .args(args)
        .output()
        .map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                AssemblyContractError::MissingDependency {
                    command: command_path.display().to_string(),
                }
            } else {
                AssemblyContractError::CommandFailed {
                    command: format!("{} {}", command_path.display(), args.join(" ")),
                    status: None,
                    output: err.to_string(),
                }
            }
        })?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{}{}", stdout, stderr);
    if output.status.success() {
        Ok(combined)
    } else {
        Err(AssemblyContractError::CommandFailed {
            command: format!("{} {}", command_path.display(), args.join(" ")),
            status: output.status.code(),
            output: combined.trim().to_string(),
        })
    }
}

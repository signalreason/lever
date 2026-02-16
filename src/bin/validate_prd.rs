use std::{error::Error, fs, io, path::PathBuf, process};

use clap::Parser;
use jsonschema::validator_for;
use serde_json::Value;

type DynError = Box<dyn Error + Send + Sync + 'static>;

#[derive(Parser, Debug)]
#[command(
    name = "validate_prd",
    about = "Validate a tasks file against the Lever PRD schema"
)]
struct ValidatePrdArgs {
    #[arg(
        long,
        value_name = "PATH",
        default_value = "prd.json",
        help = "Tasks JSON file to validate"
    )]
    tasks: PathBuf,

    #[arg(
        long,
        value_name = "PATH",
        default_value = "prd.schema.json",
        help = "JSON Schema file used for validation"
    )]
    schema: PathBuf,
}

fn main() -> Result<(), DynError> {
    let args = ValidatePrdArgs::parse();

    let schema_raw = fs::read_to_string(&args.schema).map_err(|err| {
        io::Error::other(format!(
            "Failed to read schema file {}: {}",
            args.schema.display(),
            err
        ))
    })?;
    let schema: Value = serde_json::from_str(&schema_raw).map_err(|err| {
        io::Error::other(format!(
            "Failed to parse schema file {} as JSON: {}",
            args.schema.display(),
            err
        ))
    })?;

    let validator = validator_for(&schema).map_err(|err| {
        io::Error::other(format!(
            "Failed to compile JSON Schema {}: {}",
            args.schema.display(),
            err
        ))
    })?;

    let tasks_raw = fs::read_to_string(&args.tasks).map_err(|err| {
        io::Error::other(format!(
            "Failed to read tasks file {}: {}",
            args.tasks.display(),
            err
        ))
    })?;
    let tasks: Value = serde_json::from_str(&tasks_raw).map_err(|err| {
        io::Error::other(format!(
            "Failed to parse tasks file {} as JSON: {}",
            args.tasks.display(),
            err
        ))
    })?;

    let mut errors = validator.iter_errors(&tasks).peekable();
    if errors.peek().is_none() {
        println!(
            "Schema validation passed: {} matches {}",
            args.tasks.display(),
            args.schema.display()
        );
        return Ok(());
    }

    eprintln!(
        "Validation failed: {} does not match {}",
        args.tasks.display(),
        args.schema.display()
    );
    for error in errors {
        eprintln!("- {}", error);
    }
    process::exit(1);
}

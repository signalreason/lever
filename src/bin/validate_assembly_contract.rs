use std::path::PathBuf;

use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "validate_assembly_contract",
    author,
    version,
    about = "Validate the Assembly CLI contract expected by lever",
    long_about = None
)]
struct Args {
    #[arg(
        long,
        value_name = "PATH",
        default_value = "assembly",
        help = "Assembly executable path to validate"
    )]
    assembly: PathBuf,
}

fn main() {
    let args = Args::parse();
    if let Err(err) = lever::assembly_contract::validate_assembly_contract(&args.assembly) {
        eprintln!("{}", err);
        std::process::exit(1);
    }
    println!(
        "Assembly contract validated (version {})",
        lever::assembly_contract::CONTRACT_VERSION
    );
}

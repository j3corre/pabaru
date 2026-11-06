//! Pabaru command line.
//!
//! A thin layer: parses the arguments, calls `pabaru-core` and prints the result.
//! All backup logic lives in `pabaru-core`.

mod args;
mod commands;

use clap::Parser;

fn main() {
    let cli = args::Cli::parse();
    if let Err(e) = commands::run(cli.command) {
        // Print the full chain of error causes.
        eprintln!("Error: {e:#}");
        std::process::exit(1);
    }
}

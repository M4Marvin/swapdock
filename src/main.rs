//! Binary entry point. All logic lives in the library so it can be tested.

use std::process::ExitCode;

use clap::Parser;

use swapdock::cli::Cli;

fn main() -> ExitCode {
    // Human-facing progress goes to stderr so stdout stays a clean data stream.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_target(false)
        .init();

    Cli::parse().dispatch()
}

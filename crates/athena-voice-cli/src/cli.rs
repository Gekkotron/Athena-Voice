use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "athena-voice",
    version,
    about = "Extensible voice-assistant framework.",
    propagate_version = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Start the Athena-Voice server.
    Serve(ServeArgs),
}

#[derive(Debug, clap::Args)]
pub struct ServeArgs {
    /// Path to the TOML config file.
    #[arg(long, default_value = "./athena.toml", env = "ATHENA_CONFIG")]
    pub config: PathBuf,

    /// Load config + open storage, then exit without accepting traffic.
    #[arg(long)]
    pub dry_run: bool,
}

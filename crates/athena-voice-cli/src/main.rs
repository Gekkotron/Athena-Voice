#![deny(warnings)]

use clap::Parser;

fn main() -> anyhow::Result<()> {
    let cli = athena_voice_cli::cli::Cli::parse();
    match cli.command {
        athena_voice_cli::cli::Command::Serve(args) => {
            let cfg = athena_voice_cli::config::load(&args.config)?;
            println!("stub: serve {args:?} → {cfg:?}");
            Ok(())
        }
    }
}

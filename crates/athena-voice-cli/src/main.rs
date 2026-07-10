#![deny(warnings)]

use clap::Parser;

mod cli;

fn main() -> anyhow::Result<()> {
    let cli = cli::Cli::parse();
    match cli.command {
        cli::Command::Serve(args) => {
            println!("stub: serve {args:?}");
            Ok(())
        }
    }
}

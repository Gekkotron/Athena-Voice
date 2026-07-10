#![deny(warnings)]

use clap::Parser;

fn main() -> anyhow::Result<()> {
    let cli = athena_voice_cli::cli::Cli::parse();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async move {
        match cli.command {
            athena_voice_cli::cli::Command::Serve(args) => athena_voice_cli::serve::run(args).await,
        }
    })
}

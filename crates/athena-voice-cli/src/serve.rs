use athena_voice_storage::SqliteStore;

use crate::cli::ServeArgs;
use crate::{config, logging};

pub async fn run(args: ServeArgs) -> anyhow::Result<()> {
    let cfg = config::load(&args.config)?;

    match logging::init() {
        Ok(()) | Err(logging::LoggingError::AlreadyInit) => {}
        Err(e) => anyhow::bail!("logging init failed: {e}"),
    }

    let _store = SqliteStore::open(&cfg.storage.database_url).await?;

    tracing::info!(
        host = %cfg.server.host,
        port = cfg.server.port,
        locales = ?cfg.locales.iter().map(athena_voice_core::ids::Locale::as_str).collect::<Vec<_>>(),
        "ready"
    );

    if args.dry_run {
        tracing::info!("dry-run: exiting");
        return Ok(());
    }

    tokio::signal::ctrl_c().await.ok();
    tracing::info!("shutting down");
    Ok(())
}

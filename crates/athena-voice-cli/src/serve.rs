use std::sync::Arc;

use athena_voice_providers::ProviderFactory;
use athena_voice_runtime::Runtime;
use athena_voice_runtime::mqtt::MqttConfig as RuntimeMqttConfig;
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

    let broker = athena_voice_providers::factory::MqttBrokerAddr {
        host: cfg.mqtt.host.clone(),
        port: cfg.mqtt.port,
    };
    let factory = Arc::new(
        ProviderFactory::build(&cfg.providers, Some(&broker))
            .await
            .map_err(|e| anyhow::anyhow!("provider factory: {e}"))?,
    );
    let runtime_mqtt = RuntimeMqttConfig {
        host: cfg.mqtt.host.clone(),
        port: cfg.mqtt.port,
        client_id: cfg.mqtt.client_id.clone(),
        username: cfg.mqtt.username.clone(),
        password: cfg.mqtt.password.clone(),
        keep_alive_secs: cfg.mqtt.keep_alive_secs,
    };
    let runtime = Runtime::spawn(runtime_mqtt, factory)?;
    tracing::info!("runtime spawned; awaiting SIGINT");

    tokio::signal::ctrl_c().await.ok();
    tracing::info!("shutting down");
    runtime.shutdown().await;
    Ok(())
}

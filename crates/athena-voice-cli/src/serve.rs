use std::sync::Arc;

use athena_voice_providers::ProviderFactory;
use athena_voice_runtime::mqtt::MqttConfig as RuntimeMqttConfig;
use athena_voice_runtime::wasm::registry::SkillConfig as RuntimeSkillConfig;
use athena_voice_runtime::{Runtime, SkillsInit};
use athena_voice_storage::{SqliteStore, Store};

use crate::cli::ServeArgs;
use crate::{config, logging};

pub async fn run(args: ServeArgs) -> anyhow::Result<()> {
    let cfg = config::load(&args.config)?;

    match logging::init() {
        Ok(()) | Err(logging::LoggingError::AlreadyInit) => {}
        Err(e) => anyhow::bail!("logging init failed: {e}"),
    }

    let store: Arc<dyn Store> = Arc::new(SqliteStore::open(&cfg.storage.database_url).await?);

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
        // Suffix with the pid: MQTT brokers disconnect duplicate client ids
        // in an endless mutual-kick loop, so two serve instances (or a
        // restart racing its predecessor) must never share one.
        client_id: format!("{}-{}", cfg.mqtt.client_id, std::process::id()),
        username: cfg.mqtt.username.clone(),
        password: cfg.mqtt.password.clone(),
        keep_alive_secs: cfg.mqtt.keep_alive_secs,
    };
    let skills = cfg.skills.dir.clone().map(|dir| SkillsInit {
        dir,
        store: store.clone(),
        locales: cfg
            .locales
            .iter()
            .map(|l| l.as_str().to_string())
            .collect(),
        per_skill: cfg
            .skills
            .per_skill
            .iter()
            .map(|(name, c)| {
                (
                    name.clone(),
                    RuntimeSkillConfig {
                        http_allowlist: c.http_allowlist.clone(),
                        mqtt_publish_allowlist: c.mqtt_publish_allowlist.clone(),
                        config: c.config.clone(),
                        retention_gc_after_sec: c.retention.gc_after_sec,
                        config_file: c.config_file.clone(),
                    },
                )
            })
            .collect(),
        disabled: vec![],
    });

    let runtime = Runtime::spawn(
        runtime_mqtt,
        factory,
        skills,
        std::time::Duration::from_secs(cfg.server.session_idle_secs),
    )?;
    tracing::info!("runtime spawned; awaiting SIGINT");

    tokio::signal::ctrl_c().await.ok();
    tracing::info!("shutting down");
    runtime.shutdown().await;
    Ok(())
}

use std::collections::HashMap;
use std::sync::Arc;

use athena_voice_providers::ProviderFactory;
use athena_voice_runtime::mqtt::MqttConfig as RuntimeMqttConfig;
use athena_voice_runtime::wasm::registry::SkillConfig as RuntimeSkillConfig;
use athena_voice_runtime::{Runtime, SkillsInit};
use athena_voice_storage::models::SkillSettingRow;
use athena_voice_storage::{SqliteStore, Store};

use crate::cli::ServeArgs;
use crate::{config, logging};

/// TOML `[skills.<name>]` entries converted to the runtime's `SkillConfig`
/// shape — the merge base that DB-stored settings override key-by-key.
fn base_per_skill_from_config(
    per_skill: &HashMap<String, config::PerSkillConfig>,
) -> HashMap<String, RuntimeSkillConfig> {
    per_skill
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
        .collect()
}

/// Overlays web-edited settings rows onto the TOML base, one skill at a
/// time, via [`athena_voice_runtime::wasm::settings::apply_settings`].
fn merge_db_settings(
    base: &HashMap<String, RuntimeSkillConfig>,
    db_settings: Vec<SkillSettingRow>,
) -> HashMap<String, RuntimeSkillConfig> {
    use athena_voice_runtime::wasm::settings::apply_settings;

    let mut by_skill: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for row in db_settings {
        by_skill
            .entry(row.skill)
            .or_default()
            .push((row.key, row.value));
    }

    let mut merged = base.clone();
    for (skill, rows) in by_skill {
        let skill_base = base.get(&skill).cloned().unwrap_or_default();
        merged.insert(skill, apply_settings(&skill_base, &rows));
    }
    merged
}

/// Ensures the one-time admin token exists (printing it on first run) and
/// spawns the admin web UI as a background task on `cfg.server.{host,port}`.
async fn spawn_admin_ui(
    cfg: &config::Config,
    store: Arc<dyn Store>,
    runtime: &Runtime,
    base_per_skill: HashMap<String, RuntimeSkillConfig>,
) -> anyhow::Result<()> {
    if let Some(token) = athena_voice_admin::auth::ensure_token(&store).await? {
        println!("\n==============================================================");
        println!(" Admin UI token (shown once — save it now):");
        println!("   {token}");
        println!(
            " Open http://{}:{} and paste it when prompted.",
            cfg.server.host, cfg.server.port
        );
        println!("==============================================================\n");
    }
    let token_hash = store
        .admin_token_hash()
        .await?
        .expect("ensure_token guarantees a stored hash");
    let admin_addr: std::net::SocketAddr = format!("{}:{}", cfg.server.host, cfg.server.port)
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid [server] host/port: {e}"))?;
    let admin_deps = athena_voice_admin::AdminDeps {
        store,
        skills: runtime.skills.clone(),
        base_per_skill,
        token_hash,
        bundled_dir: cfg.skills.bundled_dir.clone(),
    };
    drop(tokio::spawn(async move {
        if let Err(e) = athena_voice_admin::serve(admin_addr, admin_deps).await {
            tracing::error!(error = %e, "admin server exited");
        }
    }));
    Ok(())
}

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
    // Web-edited settings override TOML key-by-key; disabled skills are
    // unloaded right after the directory scan.
    let db_settings = store.skill_settings_all().await?;
    let disabled = store.skills_disabled().await?;

    let base_per_skill = base_per_skill_from_config(&cfg.skills.per_skill);
    let merged_per_skill = merge_db_settings(&base_per_skill, db_settings);

    let skills = cfg.skills.dir.clone().map(|dir| SkillsInit {
        dir,
        store: store.clone(),
        locales: cfg.locales.iter().map(|l| l.as_str().to_string()).collect(),
        per_skill: merged_per_skill,
        disabled,
    });

    let runtime = Runtime::spawn(
        runtime_mqtt,
        factory,
        skills,
        std::time::Duration::from_secs(cfg.server.session_idle_secs),
    )?;
    tracing::info!("runtime spawned; awaiting SIGINT");

    // Admin web UI — first run prints the token exactly once.
    spawn_admin_ui(&cfg, store.clone(), &runtime, base_per_skill).await?;

    tokio::signal::ctrl_c().await.ok();
    tracing::info!("shutting down");
    runtime.shutdown().await;
    Ok(())
}

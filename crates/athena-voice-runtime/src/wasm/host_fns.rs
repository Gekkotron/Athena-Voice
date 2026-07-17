//! Extism host functions exposed to WASM skills.
//!
//! Every host function is registered with a `UserData<SkillCtx>` payload that
//! carries the skill's identity plus the runtime handles it is allowed to
//! reach — the per-skill `Store` view, the MQTT client, the HTTP allowlist,
//! and the per-skill config map. The security-relevant checks (MQTT topic
//! ACL, HTTP host allowlist) live in this module and are enforced BEFORE any
//! side effect is executed.

use std::collections::HashMap;
use std::sync::Arc;

use extism::{CurrentPlugin, Function, PTR, UserData, Val, ValType};
use reqwest::Url;
use rumqttc::{AsyncClient, QoS};
use tokio::runtime::Handle;

use athena_voice_storage::Store;

use crate::wasm::error::HostFnError;

/// Per-skill context that every host function receives via `UserData`.
///
/// A single `SkillCtx` is constructed by the registry when a skill is loaded
/// and cloned into each of the six `Function`s. All fields are cheap to clone
/// (`Arc` / `Vec<String>` / `HashMap`), so cloning the ctx per host function
/// is a wash.
#[derive(Clone)]
pub struct SkillCtx {
    /// Logical skill name — used for tracing spans and for the MQTT ACL
    /// prefix (`athena/skills/<name>/…`).
    pub name: String,
    /// Storage backend, scoped by `Store::skill_kv_{get,set}(&name, …)`.
    pub store: Arc<dyn Store>,
    /// MQTT client shared with the rest of the runtime.
    pub mqtt: AsyncClient,
    /// Allowlisted hosts (bare host names, no scheme) the skill may reach via
    /// `host_http_get_json`.
    pub http_allowlist: Vec<String>,
    /// Config map exposed via `host_config_get`.
    pub config: HashMap<String, String>,
    /// Tokio runtime handle used to bridge async runtime APIs (Store, MQTT,
    /// reqwest) from the sync host-fn callback.
    pub tokio: Handle,
    /// HTTP client used by `host_http_get_json` — cloning is cheap (internal
    /// `Arc`), so we keep one instance per skill.
    pub http: reqwest::Client,
}

/// Prefix that every MQTT topic published by a skill must start with.
#[must_use]
pub fn mqtt_topic_prefix(skill: &str) -> String {
    format!("athena/skills/{skill}/")
}

/// Returns `true` iff `topic` is inside the skill's ACL namespace.
#[must_use]
pub fn mqtt_topic_allowed(skill: &str, topic: &str) -> bool {
    topic.starts_with(&mqtt_topic_prefix(skill))
}

/// Parses `url` and returns it iff its host is on `allowlist` and its scheme
/// is `http` or `https`.
pub fn http_url_allowed(allowlist: &[String], url: &str) -> Result<Url, HostFnError> {
    let parsed = Url::parse(url).map_err(|e| HostFnError::HttpBadUrl(e.to_string()))?;
    match parsed.scheme() {
        "http" | "https" => {}
        other => return Err(HostFnError::HttpBadScheme(other.to_string())),
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| HostFnError::HttpBadUrl("missing host".into()))?;
    if allowlist.iter().any(|h| h == host) {
        Ok(parsed)
    } else {
        Err(HostFnError::HttpHostNotAllowed(host.to_string()))
    }
}

/// Returns the six `Function`s that make up the skill host ABI, all bound to
/// the same `SkillCtx`. Names are stable — the guest ABI (Task 8) imports by
/// name from the `extism:host/user` namespace.
#[must_use]
pub fn host_functions(ctx: SkillCtx) -> Vec<Function> {
    let user_data = UserData::new(ctx);
    vec![
        Function::new(
            "host_log",
            [ValType::I64, ValType::I64],
            [],
            user_data.clone(),
            host_log,
        )
        .with_namespace("extism:host/user"),
        Function::new(
            "host_config_get",
            [PTR],
            [PTR],
            user_data.clone(),
            host_config_get,
        )
        .with_namespace("extism:host/user"),
        Function::new(
            "host_state_get",
            [PTR],
            [PTR],
            user_data.clone(),
            host_state_get,
        )
        .with_namespace("extism:host/user"),
        Function::new(
            "host_state_set",
            [PTR, PTR],
            [],
            user_data.clone(),
            host_state_set,
        )
        .with_namespace("extism:host/user"),
        Function::new(
            "host_mqtt_publish",
            [PTR, PTR],
            [PTR],
            user_data.clone(),
            host_mqtt_publish,
        )
        .with_namespace("extism:host/user"),
        Function::new(
            "host_http_get_json",
            [PTR],
            [PTR],
            user_data,
            host_http_get_json,
        )
        .with_namespace("extism:host/user"),
    ]
}

fn with_ctx<R>(
    ud: &UserData<SkillCtx>,
    f: impl FnOnce(&SkillCtx) -> R,
) -> Result<R, extism::Error> {
    let arc = ud.get()?;
    let guard = arc
        .lock()
        .map_err(|_| extism::Error::msg("SkillCtx mutex poisoned"))?;
    Ok(f(&guard))
}

fn host_log(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    _outputs: &mut [Val],
    ud: UserData<SkillCtx>,
) -> Result<(), extism::Error> {
    let level: String = plugin.memory_get_val(&inputs[0])?;
    let msg: String = plugin.memory_get_val(&inputs[1])?;
    with_ctx(&ud, |ctx| log_line(&ctx.name, &level, &msg))
}

fn log_line(skill: &str, level: &str, msg: &str) {
    match level.to_ascii_lowercase().as_str() {
        "error" => tracing::error!(skill, "{msg}"),
        "warn" => tracing::warn!(skill, "{msg}"),
        "debug" => tracing::debug!(skill, "{msg}"),
        "trace" => tracing::trace!(skill, "{msg}"),
        _ => tracing::info!(skill, "{msg}"),
    }
}

fn host_config_get(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    ud: UserData<SkillCtx>,
) -> Result<(), extism::Error> {
    let key: String = plugin.memory_get_val(&inputs[0])?;
    let value = with_ctx(&ud, |ctx| ctx.config.get(&key).cloned())?.unwrap_or_default();
    let handle = plugin.memory_new(&value)?;
    outputs[0] = plugin.memory_to_val(handle);
    Ok(())
}

fn host_state_get(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    ud: UserData<SkillCtx>,
) -> Result<(), extism::Error> {
    let key: String = plugin.memory_get_val(&inputs[0])?;
    let (store, name, tokio) = with_ctx(&ud, |ctx| {
        (ctx.store.clone(), ctx.name.clone(), ctx.tokio.clone())
    })?;
    let bytes = tokio
        .block_on(async move { store.skill_kv_get(&name, &key).await })
        .map_err(|e| extism::Error::msg(format!("skill_kv_get failed: {e}")))?
        .unwrap_or_default();
    let handle = plugin.memory_new(bytes.as_slice())?;
    outputs[0] = plugin.memory_to_val(handle);
    Ok(())
}

fn host_state_set(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    _outputs: &mut [Val],
    ud: UserData<SkillCtx>,
) -> Result<(), extism::Error> {
    let key: String = plugin.memory_get_val(&inputs[0])?;
    let val: Vec<u8> = plugin.memory_get_val(&inputs[1])?;
    let (store, name, tokio) = with_ctx(&ud, |ctx| {
        (ctx.store.clone(), ctx.name.clone(), ctx.tokio.clone())
    })?;
    tokio
        .block_on(async move { store.skill_kv_set(&name, &key, &val).await })
        .map_err(|e| extism::Error::msg(format!("skill_kv_set failed: {e}")))?;
    Ok(())
}

/// Result codes returned in the single `i64` output of `host_mqtt_publish`.
///
/// The wire type is `i64` because `extism-pdk`'s `host_fn!` widens every
/// scalar return to `i64` on the guest side; the host must match. Guest-side
/// (Task 8) turns non-zero codes into a `SkillError`.
pub const MQTT_OK: i64 = 0;
pub const MQTT_ERR_ACL: i64 = 1;
pub const MQTT_ERR_CLIENT: i64 = 2;

fn host_mqtt_publish(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    ud: UserData<SkillCtx>,
) -> Result<(), extism::Error> {
    let topic: String = plugin.memory_get_val(&inputs[0])?;
    let payload: Vec<u8> = plugin.memory_get_val(&inputs[1])?;
    let (name, mqtt, tokio) = with_ctx(&ud, |ctx| {
        (ctx.name.clone(), ctx.mqtt.clone(), ctx.tokio.clone())
    })?;
    let code: i64 = if mqtt_topic_allowed(&name, &topic) {
        let publish = tokio
            .block_on(async move { mqtt.publish(topic, QoS::AtLeastOnce, false, payload).await });
        match publish {
            Ok(()) => MQTT_OK,
            Err(e) => {
                tracing::warn!(skill = %name, error = %e, "mqtt publish failed");
                MQTT_ERR_CLIENT
            }
        }
    } else {
        tracing::warn!(
            skill = %name,
            topic = %topic,
            "skill attempted to publish outside its ACL namespace"
        );
        MQTT_ERR_ACL
    };
    // `extism-pdk` on the guest treats every non-void host-fn return as a
    // memory-handle pointing to the little-endian bytes of the actual value
    // (see `extism_convert::FromBytesOwned for i64`), so we can't return via
    // `Val::I64` — we have to allocate memory and hand back its offset.
    let handle = plugin.memory_new(code.to_le_bytes().as_slice())?;
    outputs[0] = plugin.memory_to_val(handle);
    Ok(())
}

fn host_http_get_json(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    ud: UserData<SkillCtx>,
) -> Result<(), extism::Error> {
    let url: String = plugin.memory_get_val(&inputs[0])?;
    let (allowlist, http, tokio) = with_ctx(&ud, |ctx| {
        (
            ctx.http_allowlist.clone(),
            ctx.http.clone(),
            ctx.tokio.clone(),
        )
    })?;
    let response = match http_url_allowed(&allowlist, &url) {
        Ok(parsed) => tokio.block_on(async move {
            let resp = http
                .get(parsed)
                .send()
                .await
                .map_err(|e| HostFnError::HttpFailed(e.to_string()))?;
            let status = resp.status();
            if !status.is_success() {
                return Err(HostFnError::HttpFailed(format!("status {status}")));
            }
            resp.json::<serde_json::Value>()
                .await
                .map_err(|e| HostFnError::HttpFailed(e.to_string()))
        }),
        Err(e) => Err(e),
    };
    let json_out = match response {
        Ok(v) => v,
        Err(e) => serde_json::json!({ "error": e.to_string() }),
    };
    let bytes = serde_json::to_vec(&json_out)?;
    let handle = plugin.memory_new(bytes.as_slice())?;
    outputs[0] = plugin.memory_to_val(handle);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use athena_voice_storage::SqliteStore;
    use rumqttc::MqttOptions;

    fn dummy_mqtt() -> AsyncClient {
        let (client, _eventloop) =
            AsyncClient::new(MqttOptions::new("test-client", "127.0.0.1", 1), 8);
        client
    }

    async fn make_ctx(name: &str) -> SkillCtx {
        let store = Arc::new(SqliteStore::open("sqlite::memory:").await.unwrap());
        SkillCtx {
            name: name.into(),
            store,
            mqtt: dummy_mqtt(),
            http_allowlist: vec!["api.example.com".into()],
            config: HashMap::from([("greeting".into(), "bonjour".into())]),
            tokio: Handle::current(),
            http: reqwest::Client::new(),
        }
    }

    #[test]
    fn mqtt_acl_only_allows_own_namespace() {
        assert!(mqtt_topic_allowed("clock", "athena/skills/clock/tick"));
        assert!(mqtt_topic_allowed("clock", "athena/skills/clock/"));
        assert!(!mqtt_topic_allowed("clock", "athena/skills/timer/tick"));
        assert!(!mqtt_topic_allowed("clock", "athena/system/reboot"));
        assert!(!mqtt_topic_allowed("clock", "clock/tick"));
    }

    #[test]
    fn http_allowlist_accepts_listed_host() {
        let allow = vec!["api.example.com".to_string()];
        assert!(http_url_allowed(&allow, "https://api.example.com/x").is_ok());
        assert!(http_url_allowed(&allow, "http://api.example.com/y?z=1").is_ok());
    }

    #[test]
    fn http_allowlist_rejects_unlisted_host() {
        let allow = vec!["api.example.com".to_string()];
        let err = http_url_allowed(&allow, "https://evil.example.com/x").unwrap_err();
        assert!(matches!(err, HostFnError::HttpHostNotAllowed(_)));
    }

    #[test]
    fn http_allowlist_rejects_non_http_scheme() {
        let allow = vec!["api.example.com".to_string()];
        let err = http_url_allowed(&allow, "file:///etc/passwd").unwrap_err();
        assert!(matches!(err, HostFnError::HttpBadScheme(_)));
    }

    #[test]
    fn http_allowlist_rejects_garbage_url() {
        let err = http_url_allowed(&[], "not a url").unwrap_err();
        assert!(matches!(err, HostFnError::HttpBadUrl(_)));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_functions_expose_expected_names() {
        let ctx = make_ctx("clock").await;
        let fns = host_functions(ctx);
        let names: Vec<&str> = fns.iter().map(Function::name).collect();
        assert_eq!(
            names,
            vec![
                "host_log",
                "host_config_get",
                "host_state_get",
                "host_state_set",
                "host_mqtt_publish",
                "host_http_get_json",
            ]
        );
        for f in &fns {
            assert_eq!(f.namespace(), Some("extism:host/user"));
        }
    }

    // The following tests exercise host-fn business logic without a WASM
    // guest: we drive the same code paths a real plugin would trigger, but
    // through the pure-Rust helpers (`Store`, `SkillCtx`) instead of going
    // through Extism's memory ABI. This is the "mock the WASM plugin side"
    // approach called for by Plan 4 Task 5.

    #[tokio::test(flavor = "multi_thread")]
    async fn state_roundtrip_through_store() {
        let ctx = make_ctx("clock").await;
        ctx.store
            .skill_kv_set(&ctx.name, "last_tick", b"42")
            .await
            .unwrap();
        let got = ctx
            .store
            .skill_kv_get(&ctx.name, "last_tick")
            .await
            .unwrap();
        assert_eq!(got.as_deref(), Some(&b"42"[..]));
        let miss = ctx.store.skill_kv_get("other", "last_tick").await.unwrap();
        assert!(miss.is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn config_get_returns_bound_map() {
        let ctx = make_ctx("clock").await;
        assert_eq!(
            ctx.config.get("greeting").map(String::as_str),
            Some("bonjour"),
        );
        assert_eq!(ctx.config.get("missing"), None);
    }
}

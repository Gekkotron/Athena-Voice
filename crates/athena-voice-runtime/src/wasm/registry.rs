//! Skill registry: loads WASM skills from disk, keeps their Extism plugin
//! instances alive, and dispatches intents to them.
//!
//! Structure follows Plan 4 Task 6: the registry owns a `HashMap` of loaded
//! plugins keyed by skill name (filename stem) plus a `RuleIndex` populated
//! by calling each skill's exported `pattern_rules(locale)` function.
//!
//! Task B (hot-reload) rewired the internals for live edits: the plugin map
//! is behind an `RwLock` and the pattern index sits inside an `ArcSwap` so
//! the router picks up a swap on the next dispatch without any restart.
//!
//! The plugin surface is abstracted behind [`SkillPlugin`] so tests can
//! substitute a pure-Rust mock in place of a real Extism plugin — the plan
//! calls for "fixture wasm file OR mocked plugin", and mocking keeps the
//! test suite free of a build-time wasm dependency.
//!
//! Guest ABI (host functions every skill links against, all under the
//! `extism:host/user` namespace): `host_log`, `host_config_get`,
//! `host_state_get`/`host_state_set`, `host_mqtt_publish`,
//! `host_http_get_json`, and `host_schedule_mqtt(fires_at_ms, topic,
//! payload) -> i64` — schedules a future MQTT publish under the skill's own
//! namespace and returns the row id (negative on ACL/store error). See
//! `wasm/host_fns.rs` for the full signatures and `wasm/scheduler.rs` for the
//! task that later drains and publishes those scheduled events.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex, RwLock};

use arc_swap::ArcSwap;
use extism::{Manifest, Plugin, PluginBuilder, Wasm};
use thiserror::Error;
use tokio::runtime::Handle;
use tokio::sync::broadcast;
use tracing::warn;

use athena_voice_core::event::Event;
use athena_voice_skill_sdk::{ConfigSchema, Intent, PatternRule, SkillError, SkillResponse};
use athena_voice_storage::Store;

use crate::intent::{HostPatternRule, RuleIndex};
use crate::wasm::host_fns::{AsyncClientPublisher, SkillCtx, host_functions};
use std::time::{SystemTime, UNIX_EPOCH};

/// Per-skill configuration merged into a `SkillCtx` at load time.
#[derive(Debug, Clone, Default)]
pub struct SkillConfig {
    pub http_allowlist: Vec<String>,
    pub mqtt_publish_allowlist: Vec<String>,
    pub config: HashMap<String, String>,
    pub retention_gc_after_sec: Option<u64>,
    /// Optional INI/TOML file for skill config.
    /// Values: path to file (INI/TOML), or empty to use `config`.
    /// Accessed via `host_config_get` → `IniSlice`/`toml::from_slice`.
    pub config_file: Option<String>,
}

/// Runtime handles + per-skill config passed into
/// [`SkillRegistry::load_dir`]. One `SkillDeps` is enough to load an entire
/// directory of skills.
#[derive(Clone)]
pub struct SkillDeps {
    pub store: Arc<dyn Store>,
    pub mqtt: rumqttc::AsyncClient,
    pub tokio: Handle,
    pub http: reqwest::Client,
    /// Locales for which each skill's `pattern_rules(locale)` will be
    /// queried; entries land in the `RuleIndex` under that locale key.
    pub locales: Vec<String>,
    /// Per-skill config, keyed by skill name (filename stem). Missing entries
    /// default to an empty allowlist and empty config map.
    pub per_skill: HashMap<String, SkillConfig>,
    /// Event bus for `SkillReloaded` / `SkillReloadFailed`. `None` means
    /// hot-reload observability is disabled — the registry falls back to
    /// tracing only.
    pub event_tx: Option<broadcast::Sender<Event>>,
    /// Event bus for skill-driven audio playback.
    pub audio_event_tx: broadcast::Sender<Event>,
}

#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("io error while reading skills dir {dir}: {source}")]
    Io {
        dir: String,
        #[source]
        source: std::io::Error,
    },
    #[error("skill file has no filename stem: {path}")]
    NoStem { path: String },
    #[error("failed to build skill {skill}: {source}")]
    Build {
        skill: String,
        #[source]
        source: extism::Error,
    },
    #[error("skill {skill} pattern_rules({locale}) failed: {source}")]
    Patterns {
        skill: String,
        locale: String,
        #[source]
        source: extism::Error,
    },
}

/// Loaded skill instance — either a real Extism plugin or a test mock. Kept
/// deliberately minimal: the guest ABI needed by the registry is just the
/// two exports `pattern_rules(locale)` and `handle(intent)`.
pub trait SkillPlugin: Send {
    fn pattern_rules(&mut self, locale: &str) -> Result<Vec<PatternRule>, extism::Error>;
    fn handle(&mut self, intent: &Intent) -> Result<SkillResponse, SkillError>;
    /// Returns the `SkillCtx` if this plugin wraps a `ExtismSkillPlugin`.
    fn ctx(&self) -> Option<&SkillCtx> {
        None
    }
    /// Parsed `config_schema` guest export, if the skill provides one.
    fn config_schema(&mut self) -> Option<ConfigSchema> {
        None
    }
}

/// `SkillPlugin` backed by a live `extism::Plugin`. Guest exports are called
/// with JSON strings on the wire; format decisions here mirror the pending
/// guest ABI in Task 8 and stay local so Task 8 can adjust freely.
pub struct ExtismSkillPlugin {
    plugin: Plugin,
    ctx: Option<SkillCtx>,
}

impl ExtismSkillPlugin {
    #[must_use]
    pub fn new(plugin: Plugin) -> Self {
        Self { plugin, ctx: None }
    }

    /// Wraps a plugin together with the `SkillCtx` its host functions were
    /// built from, so the registry can consult per-skill settings (e.g.
    /// retention GC) at dispatch time.
    #[must_use]
    pub fn with_ctx(plugin: Plugin, ctx: SkillCtx) -> Self {
        Self {
            plugin,
            ctx: Some(ctx),
        }
    }
}

impl SkillPlugin for ExtismSkillPlugin {
    fn pattern_rules(&mut self, locale: &str) -> Result<Vec<PatternRule>, extism::Error> {
        let out: &str = self.plugin.call("pattern_rules", locale)?;
        serde_json::from_str(out).map_err(|e| extism::Error::msg(e.to_string()))
    }

    fn handle(&mut self, intent: &Intent) -> Result<SkillResponse, SkillError> {
        let payload = serde_json::to_string(intent)
            .map_err(|e| SkillError::Custom(format!("intent encode: {e}")))?;
        let out: &str = self
            .plugin
            .call("handle", payload.as_str())
            .map_err(|e| SkillError::Custom(format!("plugin call: {e}")))?;
        serde_json::from_str::<Result<SkillResponse, SkillError>>(out)
            .map_err(|e| SkillError::Custom(format!("response decode: {e}")))?
    }

    fn ctx(&self) -> Option<&SkillCtx> {
        self.ctx.as_ref()
    }

    fn config_schema(&mut self) -> Option<ConfigSchema> {
        if !self.plugin.function_exists("config_schema") {
            return None;
        }
        let out = self.plugin.call::<&str, String>("config_schema", "").ok()?;
        match serde_json::from_str(&out) {
            Ok(schema) => Some(schema),
            Err(e) => {
                warn!(error = %e, "config_schema export returned invalid JSON; ignoring");
                None
            }
        }
    }
}

/// Rules a single plugin contributed to the aggregate index, keyed by
/// locale. Kept alongside the `plugins` map so `remove` / `reload_path` can
/// rebuild the aggregate `RuleIndex` without re-querying every remaining
/// plugin.
type PluginRules = HashMap<String, Vec<HostPatternRule>>;
type PluginHandle = Arc<Mutex<dyn SkillPlugin>>;
type PluginMap = HashMap<String, PluginHandle>;

pub struct SkillRegistry {
    plugins: Arc<RwLock<PluginMap>>,
    /// Cache of the rules each plugin contributed, so the aggregate index
    /// can be rebuilt cheaply when one plugin is removed or replaced.
    plugin_rules: Mutex<HashMap<String, PluginRules>>,
    patterns: Arc<ArcSwap<RuleIndex>>,
    /// Config schema cached at install time, keyed by skill name.
    schemas: RwLock<HashMap<String, ConfigSchema>>,
}

impl SkillRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            plugins: Arc::new(RwLock::new(HashMap::new())),
            plugin_rules: Mutex::new(HashMap::new()),
            patterns: Arc::new(ArcSwap::from_pointee(RuleIndex::new())),
            schemas: RwLock::new(HashMap::new()),
        }
    }

    /// A cloneable handle to the aggregate pattern index. Consumers (router,
    /// tests) read via `handle.load()` — every dispatch sees the latest swap
    /// with no lock contention.
    #[must_use]
    pub fn patterns_handle(&self) -> Arc<ArcSwap<RuleIndex>> {
        self.patterns.clone()
    }

    /// Snapshot of the current pattern index. Prefer `patterns_handle` for
    /// long-lived readers so they see reloads.
    #[must_use]
    pub fn patterns_snapshot(&self) -> Arc<RuleIndex> {
        self.patterns.load_full()
    }

    #[must_use]
    pub fn skill_names(&self) -> Vec<String> {
        self.plugins
            .read()
            .expect("skills map lock poisoned")
            .keys()
            .cloned()
            .collect()
    }

    /// Iterate `dir` for `*.wasm` files and load each with Extism, wiring in
    /// the host functions from [`crate::wasm::host_fns`] and populating the
    /// pattern index from every skill's `pattern_rules(locale)` export.
    pub fn load_dir(dir: &Path, deps: &SkillDeps) -> Result<Self, RegistryError> {
        let registry = Self::new();
        let read = fs::read_dir(dir).map_err(|source| RegistryError::Io {
            dir: dir.display().to_string(),
            source,
        })?;
        for entry in read {
            let entry = entry.map_err(|source| RegistryError::Io {
                dir: dir.display().to_string(),
                source,
            })?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("wasm") {
                continue;
            }
            // A single unbuildable skill must not brick every OTHER skill's
            // startup (and, transitively, the whole process): warn and move
            // on to the next file instead of propagating. Io/read_dir errors
            // above stay fatal — those indicate the skills dir itself is
            // unusable, not a problem with one file in it.
            let (name, plugin, _retention_gc_after_sec) = match build_plugin_from_file(&path, deps)
            {
                Ok(built) => built,
                Err(e) => {
                    let name = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("<unknown>");
                    warn!(skill = %name, error = %e, "failed to build skill; skipping");
                    continue;
                }
            };
            registry.install(&name, plugin, &deps.locales)?;
        }
        Ok(registry)
    }

    /// Register a plugin (real or mock) and populate the pattern index by
    /// calling its `pattern_rules(locale)` export for every locale. If the
    /// name is already present, its previous plugin is replaced and only its
    /// rules are re-populated in the aggregate index.
    pub fn install(
        &self,
        name: &str,
        plugin: Arc<Mutex<dyn SkillPlugin>>,
        locales: &[String],
    ) -> Result<(), RegistryError> {
        let mut this_rules: PluginRules = HashMap::new();
        for locale in locales {
            let rules = {
                let mut guard = plugin
                    .lock()
                    .expect("skill plugin mutex poisoned during install");
                guard
                    .pattern_rules(locale)
                    .map_err(|source| RegistryError::Patterns {
                        skill: name.to_string(),
                        locale: locale.clone(),
                        source,
                    })?
            };
            let converted: Vec<HostPatternRule> =
                rules.into_iter().map(HostPatternRule::from).collect();
            this_rules.insert(locale.clone(), converted);
        }

        let schema = {
            let mut guard = plugin
                .lock()
                .expect("skill plugin mutex poisoned during install");
            guard.config_schema()
        };

        // Update the plugin map + per-plugin rule cache, then rebuild the
        // aggregate index atomically. The map lock is held only for the
        // insert; the ArcSwap store is what matters for observers.
        {
            let mut plugins = self.plugins.write().expect("skills map lock poisoned");
            plugins.insert(name.to_string(), plugin);
        }
        {
            let mut rules_map = self
                .plugin_rules
                .lock()
                .expect("plugin_rules lock poisoned");
            rules_map.insert(name.to_string(), this_rules);
        }
        {
            let mut schemas = self.schemas.write().expect("schemas lock poisoned");
            match schema {
                Some(s) => {
                    schemas.insert(name.to_string(), s);
                }
                None => {
                    schemas.remove(name);
                }
            }
        }
        self.rebuild_index();
        Ok(())
    }

    /// Drop the plugin under `name` and rebuild the aggregate `RuleIndex` so
    /// the router stops matching its rules on the next dispatch. Idempotent
    /// — removing a name that isn't loaded is a no-op.
    pub fn remove(&self, name: &str) -> bool {
        let existed = {
            let mut plugins = self.plugins.write().expect("skills map lock poisoned");
            plugins.remove(name).is_some()
        };
        {
            let mut rules_map = self
                .plugin_rules
                .lock()
                .expect("plugin_rules lock poisoned");
            rules_map.remove(name);
        }
        self.schemas
            .write()
            .expect("schemas lock poisoned")
            .remove(name);
        if existed {
            self.rebuild_index();
        }
        existed
    }

    /// Config schema cached at install time; `None` for skills without the
    /// export (the UI falls back to a key/value editor).
    #[must_use]
    pub fn config_schema(&self, name: &str) -> Option<ConfigSchema> {
        self.schemas
            .read()
            .expect("schemas lock poisoned")
            .get(name)
            .cloned()
    }

    /// Rebuild one plugin from a single file path and re-run [`install`].
    ///
    /// The `name` is derived from the file stem. On failure the previous
    /// plugin (if any) is left untouched — the registry is never in a
    /// half-loaded state. Success emits `Event::SkillReloaded { name }` on
    /// the deps bus, failure emits `Event::SkillReloadFailed`.
    pub fn reload_path(&self, path: &Path, deps: &SkillDeps) -> Result<String, RegistryError> {
        let outcome = (|| -> Result<String, RegistryError> {
            let (name, plugin, _retention_gc_after_sec) = build_plugin_from_file(path, deps)?;
            self.install(&name, plugin, &deps.locales)?;
            Ok(name)
        })();

        match &outcome {
            Ok(name) => {
                if let Some(tx) = deps.event_tx.as_ref() {
                    let _ = tx.send(Event::SkillReloaded { name: name.clone() });
                }
            }
            Err(err) => {
                let name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("<unknown>")
                    .to_string();
                let reason = err.to_string();
                warn!(skill = %name, error = %reason, "skill reload failed");
                if let Some(tx) = deps.event_tx.as_ref() {
                    let _ = tx.send(Event::SkillReloadFailed { name, reason });
                }
            }
        }
        outcome
    }

    /// Dispatch `intent` to the named skill. Unknown-skill and mutex-poison
    /// conditions surface as `SkillError::Custom` so the caller can treat
    /// dispatch as a single unified fail-path.
    pub fn dispatch(&self, skill: &str, intent: Intent) -> Result<SkillResponse, SkillError> {
        let now_sec = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| SkillError::Custom(format!("time error: {e}")))?
            .as_secs();

        let plugin = {
            let plugins = self
                .plugins
                .read()
                .map_err(|_| SkillError::Custom("skills map lock poisoned".into()))?;
            plugins
                .get(skill)
                .cloned()
                .ok_or_else(|| SkillError::Custom(format!("unknown skill: {skill}")))?
        };

        // GC expired keys if retention is enabled (spawned to avoid blocking)
        if let Some(ctx) = plugin.lock().unwrap().ctx() {
            if let Some(gc_sec) = ctx.retention_gc_after_sec {
                if gc_sec > 0 {
                    let store = ctx.store.clone();
                    let skill_name = skill.to_string();
                    let gc_threshold = now_sec - gc_sec;
                    ctx.tokio.spawn(async move {
                        let _ = store.skill_kv_gc(&skill_name, gc_threshold).await;
                    });
                }
            }
        }

        let mut guard = plugin
            .lock()
            .map_err(|_| SkillError::Custom("skill plugin mutex poisoned".into()))?;
        guard.handle(&intent)
    }

    fn rebuild_index(&self) {
        let rules_map = self
            .plugin_rules
            .lock()
            .expect("plugin_rules lock poisoned");
        let mut fresh = RuleIndex::new();
        for (skill, per_locale) in rules_map.iter() {
            for (locale, rules) in per_locale {
                for rule in rules {
                    fresh.insert(locale.clone(), rule.clone(), skill.clone());
                }
            }
        }
        self.patterns.store(Arc::new(fresh));
    }
}

/// Build one plugin instance from a wasm file on disk. Returns the derived
/// skill name (file stem) alongside the wrapped plugin and its retention TTL.
fn build_plugin_from_file(
    path: &Path,
    deps: &SkillDeps,
) -> Result<(String, Arc<Mutex<dyn SkillPlugin>>, Option<u64>), RegistryError> {
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| RegistryError::NoStem {
            path: path.display().to_string(),
        })?
        .to_string();
    let cfg = deps.per_skill.get(&name).cloned().unwrap_or_default();
    let retention_gc_after_sec = cfg.retention_gc_after_sec;
    let ctx = SkillCtx {
        name: name.clone(),
        store: deps.store.clone(),
        mqtt: Arc::new(AsyncClientPublisher(deps.mqtt.clone())),
        http_allowlist: cfg.http_allowlist,
        mqtt_publish_allowlist: cfg.mqtt_publish_allowlist,
        config: cfg.config,
        tokio: deps.tokio.clone(),
        http: deps.http.clone(),
        retention_gc_after_sec: cfg.retention_gc_after_sec,
        event_bus: deps.audio_event_tx.clone(),
        config_file: cfg.config_file,
    };
    let manifest = Manifest::new([Wasm::file(path)]);
    let builder = PluginBuilder::new(manifest)
        .with_wasi(true)
        .with_functions(host_functions(ctx.clone()));
    let plugin = builder.build().map_err(|source| RegistryError::Build {
        skill: name.clone(),
        source,
    })?;
    let plugin: Arc<Mutex<dyn SkillPlugin>> =
        Arc::new(Mutex::new(ExtismSkillPlugin::with_ctx(plugin, ctx)));
    Ok((name, plugin, retention_gc_after_sec))
}

impl Default for SkillRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use athena_voice_skill_sdk::{ConfigField, ConfigSchema, FieldKind, SlotKind, SlotSpec};

    struct MockPlugin {
        rules_by_locale: HashMap<String, Vec<PatternRule>>,
        response: Result<SkillResponse, SkillError>,
        handle_calls: Arc<AtomicUsize>,
        last_intent: Arc<Mutex<Option<Intent>>>,
        schema: Option<ConfigSchema>,
    }

    impl SkillPlugin for MockPlugin {
        fn pattern_rules(&mut self, locale: &str) -> Result<Vec<PatternRule>, extism::Error> {
            Ok(self
                .rules_by_locale
                .get(locale)
                .cloned()
                .unwrap_or_default())
        }
        fn handle(&mut self, intent: &Intent) -> Result<SkillResponse, SkillError> {
            self.handle_calls.fetch_add(1, Ordering::SeqCst);
            *self.last_intent.lock().unwrap() = Some(intent.clone());
            self.response
                .as_ref()
                .map(Clone::clone)
                .map_err(|e| SkillError::Custom(e.to_string()))
        }
        fn config_schema(&mut self) -> Option<ConfigSchema> {
            self.schema.clone()
        }
    }

    fn rule(intent: &str, phrase: &str) -> PatternRule {
        PatternRule {
            intent: intent.into(),
            phrases: vec![phrase.into()],
            slots: vec![SlotSpec {
                name: "x".into(),
                kind: SlotKind::String,
            }],
        }
    }

    fn arc_mock(mock: MockPlugin) -> Arc<Mutex<dyn SkillPlugin>> {
        Arc::new(Mutex::new(mock))
    }

    fn simple_mock(locale_rules: &[(&str, Vec<PatternRule>)]) -> Arc<Mutex<dyn SkillPlugin>> {
        let rules_by_locale = locale_rules
            .iter()
            .map(|(l, r)| ((*l).to_string(), r.clone()))
            .collect();
        arc_mock(MockPlugin {
            rules_by_locale,
            response: Ok(SkillResponse::empty()),
            handle_calls: Arc::new(AtomicUsize::new(0)),
            last_intent: Arc::new(Mutex::new(None)),
            schema: None,
        })
    }

    fn mock_plugin_with_schema(schema: Option<ConfigSchema>) -> Arc<Mutex<dyn SkillPlugin>> {
        arc_mock(MockPlugin {
            rules_by_locale: HashMap::new(),
            response: Ok(SkillResponse::empty()),
            handle_calls: Arc::new(AtomicUsize::new(0)),
            last_intent: Arc::new(Mutex::new(None)),
            schema,
        })
    }

    #[test]
    fn install_populates_patterns_per_locale() {
        let reg = SkillRegistry::new();
        let mock = MockPlugin {
            rules_by_locale: HashMap::from([
                ("fr".into(), vec![rule("hello", "bonjour")]),
                (
                    "en".into(),
                    vec![rule("hello", "hello"), rule("bye", "bye")],
                ),
            ]),
            response: Ok(SkillResponse::empty()),
            handle_calls: Arc::new(AtomicUsize::new(0)),
            last_intent: Arc::new(Mutex::new(None)),
            schema: None,
        };
        reg.install(
            "greeter",
            arc_mock(mock),
            &["fr".to_string(), "en".to_string()],
        )
        .unwrap();

        let idx = reg.patterns_snapshot();
        assert_eq!(idx.locale_count(), 2);
        assert_eq!(idx.for_locale("fr").unwrap().len(), 1);
        assert_eq!(idx.for_locale("en").unwrap().len(), 2);
        assert_eq!(reg.skill_names(), vec!["greeter".to_string()]);
    }

    #[test]
    fn dispatch_calls_plugin_and_returns_response() {
        let reg = SkillRegistry::new();
        let handle_calls = Arc::new(AtomicUsize::new(0));
        let last_intent = Arc::new(Mutex::new(None));
        let mock = MockPlugin {
            rules_by_locale: HashMap::new(),
            response: Ok(SkillResponse::speak("il est huit heures")),
            handle_calls: handle_calls.clone(),
            last_intent: last_intent.clone(),
            schema: None,
        };
        reg.install("clock", arc_mock(mock), &[]).unwrap();

        let intent = Intent {
            name: "time.query".into(),
            slots: BTreeMap::new(),
            locale: String::new(),
        };
        let resp = reg.dispatch("clock", intent).unwrap();
        assert!(matches!(resp, SkillResponse::Speak { text } if text == "il est huit heures"));
        assert_eq!(handle_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            last_intent.lock().unwrap().as_ref().unwrap().name,
            "time.query"
        );
    }

    #[test]
    fn dispatch_propagates_skill_error() {
        let reg = SkillRegistry::new();
        let mock = MockPlugin {
            rules_by_locale: HashMap::new(),
            response: Err(SkillError::HttpFailed("boom".into())),
            handle_calls: Arc::new(AtomicUsize::new(0)),
            last_intent: Arc::new(Mutex::new(None)),
            schema: None,
        };
        reg.install("weather", arc_mock(mock), &[]).unwrap();
        let err = reg
            .dispatch(
                "weather",
                Intent {
                    name: "weather.query".into(),
                    slots: BTreeMap::new(),
                    locale: String::new(),
                },
            )
            .unwrap_err();
        assert!(err.to_string().contains("boom"));
    }

    #[test]
    fn dispatch_unknown_skill_is_custom_error() {
        let reg = SkillRegistry::new();
        let err = reg
            .dispatch(
                "nope",
                Intent {
                    name: "x".into(),
                    slots: BTreeMap::new(),
                    locale: String::new(),
                },
            )
            .unwrap_err();
        assert!(matches!(err, SkillError::Custom(ref m) if m.contains("unknown skill")));
    }

    #[test]
    fn install_then_remove_clears_plugin_and_rules() {
        let reg = SkillRegistry::new();
        let mock = simple_mock(&[("fr", vec![rule("hello", "bonjour")])]);
        reg.install("greeter", mock, &["fr".to_string()]).unwrap();
        assert_eq!(reg.patterns_snapshot().for_locale("fr").unwrap().len(), 1);
        assert!(reg.skill_names().contains(&"greeter".to_string()));

        assert!(reg.remove("greeter"));
        assert!(reg.skill_names().is_empty());
        let idx = reg.patterns_snapshot();
        assert!(idx.for_locale("fr").is_none() || idx.for_locale("fr").unwrap().is_empty());
        // Second remove is a no-op.
        assert!(!reg.remove("greeter"));
    }

    #[test]
    fn install_twice_same_name_replaces_only_that_skills_rules() {
        let reg = SkillRegistry::new();
        // Skill A contributes rules under both fr and en.
        let a1 = simple_mock(&[
            ("fr", vec![rule("hello", "bonjour")]),
            ("en", vec![rule("hello", "hello")]),
        ]);
        reg.install("a", a1, &["fr".into(), "en".into()]).unwrap();

        // Second skill "b" contributes an fr rule — should survive re-install of "a".
        let b = simple_mock(&[("fr", vec![rule("bye", "au revoir")])]);
        reg.install("b", b, &["fr".into(), "en".into()]).unwrap();

        assert_eq!(reg.patterns_snapshot().for_locale("fr").unwrap().len(), 2);

        // Replace "a" with a new plugin exposing different rules.
        let a2 = simple_mock(&[
            ("fr", vec![rule("hello", "salut"), rule("cheer", "santé")]),
            ("en", vec![]),
        ]);
        reg.install("a", a2, &["fr".into(), "en".into()]).unwrap();

        let idx = reg.patterns_snapshot();
        // fr now has 2 (a) + 1 (b) = 3 rules; en has 0 (a's new rules were empty).
        assert_eq!(idx.for_locale("fr").unwrap().len(), 3);
        assert!(idx.for_locale("en").is_none() || idx.for_locale("en").unwrap().is_empty());
        // Both skills still registered.
        let mut names = reg.skill_names();
        names.sort();
        assert_eq!(names, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn reload_path_on_broken_wasm_leaves_prior_plugin_and_emits_failed_event() {
        let reg = SkillRegistry::new();
        // Prior good mock installed manually as "brokentest".
        let good = simple_mock(&[("fr", vec![rule("hello", "bonjour")])]);
        reg.install("brokentest", good, &["fr".into()]).unwrap();
        let rules_before = reg.patterns_snapshot().for_locale("fr").unwrap().len();
        assert_eq!(rules_before, 1);

        // Broken .wasm at a real path so `reload_path` walks the failure path.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("brokentest.wasm");
        std::fs::write(&path, b"not really wasm").unwrap();

        let (tx, mut rx) = broadcast::channel(4);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let store: Arc<dyn Store> = rt.block_on(async {
            Arc::new(
                athena_voice_storage::SqliteStore::open("sqlite::memory:")
                    .await
                    .unwrap(),
            )
        });
        let (mqtt, _eventloop) =
            rumqttc::AsyncClient::new(rumqttc::MqttOptions::new("reload-fail", "127.0.0.1", 1), 8);
        let deps = SkillDeps {
            store,
            mqtt,
            tokio: rt.handle().clone(),
            http: reqwest::Client::new(),
            locales: vec!["fr".into()],
            per_skill: HashMap::new(),
            event_tx: Some(tx),
            audio_event_tx: broadcast::channel(4).0,
        };

        let err = reg.reload_path(&path, &deps).unwrap_err();
        assert!(
            matches!(err, RegistryError::Build { ref skill, .. } if skill == "brokentest"),
            "unexpected reload error variant"
        );

        // Prior plugin still installed and its rules survive.
        assert!(reg.skill_names().contains(&"brokentest".to_string()));
        assert_eq!(reg.patterns_snapshot().for_locale("fr").unwrap().len(), 1);

        let ev = rx.try_recv().expect("expected an event");
        match ev {
            Event::SkillReloadFailed { name, .. } => assert_eq!(name, "brokentest"),
            other => panic!("expected SkillReloadFailed, got {other:?}"),
        }
    }

    fn make_deps(rt: &tokio::runtime::Runtime, client_id: &str) -> SkillDeps {
        let store: Arc<dyn Store> = rt.block_on(async {
            Arc::new(
                athena_voice_storage::SqliteStore::open("sqlite::memory:")
                    .await
                    .unwrap(),
            )
        });
        let (mqtt, _eventloop) =
            rumqttc::AsyncClient::new(rumqttc::MqttOptions::new(client_id, "127.0.0.1", 1), 8);
        SkillDeps {
            store,
            mqtt,
            tokio: rt.handle().clone(),
            http: reqwest::Client::new(),
            locales: vec!["fr".into()],
            per_skill: HashMap::new(),
            event_tx: None,
            audio_event_tx: broadcast::channel(4).0,
        }
    }

    #[test]
    fn load_dir_ignores_non_wasm_and_returns_empty_registry() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("readme.txt"), b"hi").unwrap();
        std::fs::write(dir.path().join("notes.md"), b"# skills").unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let deps = make_deps(&rt, "registry-test");
        let reg = SkillRegistry::load_dir(dir.path(), &deps).unwrap();
        assert!(reg.skill_names().is_empty());
        assert_eq!(reg.patterns_snapshot().locale_count(), 0);
    }

    #[test]
    fn load_dir_skips_invalid_wasm_and_continues() {
        // A single unbuildable .wasm must not kill startup for the whole
        // directory: load_dir logs a warning and moves on to the next file,
        // returning `Ok` with whatever skills DID load successfully.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("broken.wasm"), b"not really wasm").unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let deps = make_deps(&rt, "registry-test-2");
        let reg = SkillRegistry::load_dir(dir.path(), &deps)
            .expect("a broken file must not fail the whole directory load");
        assert!(!reg.skill_names().contains(&"broken".to_string()));
    }

    #[test]
    fn load_dir_skips_invalid_wasm_but_loads_valid_skill() {
        // One garbage file alongside one real, buildable skill: the registry
        // must come back with the valid skill loaded and no error, proving
        // the tolerant loop doesn't stop at (or get confused by) the bad file.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("broken.wasm"), b"not really wasm").unwrap();
        std::fs::copy(env!("SMOKE_TEST_WASM"), dir.path().join("smoke-test.wasm"))
            .expect("copy smoke-test.wasm fixture built by this crate's build.rs");

        let rt = tokio::runtime::Runtime::new().unwrap();
        let deps = make_deps(&rt, "registry-test-3");
        let reg = SkillRegistry::load_dir(dir.path(), &deps)
            .expect("valid skills must still load despite one broken file");
        assert!(reg.skill_names().contains(&"smoke-test".to_string()));
        assert!(!reg.skill_names().contains(&"broken".to_string()));
    }

    #[test]
    fn install_caches_config_schema() {
        let schema = ConfigSchema {
            fields: vec![ConfigField {
                key: "api_key".into(),
                label: "API key".into(),
                kind: FieldKind::Secret,
                required: true,
                help: String::new(),
                default: String::new(),
                item_fields: vec![],
            }],
        };
        let registry = SkillRegistry::new();
        let plugin = mock_plugin_with_schema(Some(schema.clone()));
        registry.install("jeedom", plugin, &["fr".into()]).unwrap();
        assert_eq!(registry.config_schema("jeedom"), Some(schema));
        assert_eq!(registry.config_schema("nope"), None);
        registry.remove("jeedom");
        assert_eq!(registry.config_schema("jeedom"), None);
    }
}

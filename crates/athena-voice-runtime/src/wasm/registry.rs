//! Skill registry: loads WASM skills from disk, keeps their Extism plugin
//! instances alive, and dispatches intents to them.
//!
//! Structure follows Plan 4 Task 6: the registry owns a `HashMap` of loaded
//! plugins keyed by skill name (filename stem) plus a `RuleIndex` populated
//! by calling each skill's exported `pattern_rules(locale)` function.
//!
//! The plugin surface is abstracted behind [`SkillPlugin`] so tests can
//! substitute a pure-Rust mock in place of a real Extism plugin — the plan
//! calls for "fixture wasm file OR mocked plugin", and mocking keeps the
//! test suite free of a build-time wasm dependency.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

use extism::{Manifest, Plugin, PluginBuilder, Wasm};
use thiserror::Error;
use tokio::runtime::Handle;

use athena_voice_skill_sdk::{Intent, PatternRule, SkillError, SkillResponse};
use athena_voice_storage::Store;

use crate::intent::{HostPatternRule, RuleIndex};
use crate::wasm::host_fns::{SkillCtx, host_functions};

/// Per-skill configuration merged into a `SkillCtx` at load time.
#[derive(Debug, Clone, Default)]
pub struct SkillConfig {
    pub http_allowlist: Vec<String>,
    pub config: HashMap<String, String>,
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
}

/// `SkillPlugin` backed by a live `extism::Plugin`. Guest exports are called
/// with JSON strings on the wire; format decisions here mirror the pending
/// guest ABI in Task 8 and stay local so Task 8 can adjust freely.
pub struct ExtismSkillPlugin {
    plugin: Plugin,
}

impl ExtismSkillPlugin {
    #[must_use]
    pub fn new(plugin: Plugin) -> Self {
        Self { plugin }
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
}

pub struct SkillRegistry {
    plugins: HashMap<String, Arc<Mutex<dyn SkillPlugin>>>,
    patterns: RuleIndex,
}

impl SkillRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            plugins: HashMap::new(),
            patterns: RuleIndex::new(),
        }
    }

    #[must_use]
    pub fn patterns(&self) -> &RuleIndex {
        &self.patterns
    }

    /// Consume the registry and return only its pattern index — useful when
    /// handing the index off to the intent matcher.
    #[must_use]
    pub fn into_patterns(self) -> RuleIndex {
        self.patterns
    }

    #[must_use]
    pub fn skill_names(&self) -> Vec<String> {
        self.plugins.keys().cloned().collect()
    }

    /// Iterate `dir` for `*.wasm` files and load each with Extism, wiring in
    /// the host functions from [`crate::wasm::host_fns`] and populating the
    /// pattern index from every skill's `pattern_rules(locale)` export.
    pub fn load_dir(dir: &Path, deps: &SkillDeps) -> Result<Self, RegistryError> {
        let mut registry = Self::new();
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
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or_else(|| RegistryError::NoStem {
                    path: path.display().to_string(),
                })?
                .to_string();
            let cfg = deps.per_skill.get(&name).cloned().unwrap_or_default();
            let ctx = SkillCtx {
                name: name.clone(),
                store: deps.store.clone(),
                mqtt: deps.mqtt.clone(),
                http_allowlist: cfg.http_allowlist,
                config: cfg.config,
                tokio: deps.tokio.clone(),
                http: deps.http.clone(),
            };
            let manifest = Manifest::new([Wasm::file(&path)]);
            let plugin = PluginBuilder::new(manifest)
                .with_wasi(true)
                .with_functions(host_functions(ctx))
                .build()
                .map_err(|source| RegistryError::Build {
                    skill: name.clone(),
                    source,
                })?;
            let boxed: Arc<Mutex<dyn SkillPlugin>> =
                Arc::new(Mutex::new(ExtismSkillPlugin::new(plugin)));
            registry.install(&name, boxed, &deps.locales)?;
        }
        Ok(registry)
    }

    /// Register a plugin (real or mock) and populate the pattern index by
    /// calling its `pattern_rules(locale)` export for every locale.
    pub fn install(
        &mut self,
        name: &str,
        plugin: Arc<Mutex<dyn SkillPlugin>>,
        locales: &[String],
    ) -> Result<(), RegistryError> {
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
            for rule in rules {
                self.patterns.insert(
                    locale.clone(),
                    HostPatternRule::from(rule),
                    name.to_string(),
                );
            }
        }
        self.plugins.insert(name.to_string(), plugin);
        Ok(())
    }

    /// Dispatch `intent` to the named skill. Unknown-skill and mutex-poison
    /// conditions surface as `SkillError::Custom` so the caller can treat
    /// dispatch as a single unified fail-path.
    pub fn dispatch(
        &self,
        skill: &str,
        intent: Intent,
    ) -> Result<SkillResponse, SkillError> {
        let plugin = self
            .plugins
            .get(skill)
            .ok_or_else(|| SkillError::Custom(format!("unknown skill: {skill}")))?;
        let mut guard = plugin
            .lock()
            .map_err(|_| SkillError::Custom("skill plugin mutex poisoned".into()))?;
        guard.handle(&intent)
    }
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

    use athena_voice_skill_sdk::{SlotKind, SlotSpec};

    struct MockPlugin {
        rules_by_locale: HashMap<String, Vec<PatternRule>>,
        response: Result<SkillResponse, SkillError>,
        handle_calls: Arc<AtomicUsize>,
        last_intent: Arc<Mutex<Option<Intent>>>,
    }

    impl SkillPlugin for MockPlugin {
        fn pattern_rules(&mut self, locale: &str) -> Result<Vec<PatternRule>, extism::Error> {
            Ok(self.rules_by_locale.get(locale).cloned().unwrap_or_default())
        }
        fn handle(&mut self, intent: &Intent) -> Result<SkillResponse, SkillError> {
            self.handle_calls.fetch_add(1, Ordering::SeqCst);
            *self.last_intent.lock().unwrap() = Some(intent.clone());
            self.response
                .as_ref()
                .map(Clone::clone)
                .map_err(|e| SkillError::Custom(e.to_string()))
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

    #[test]
    fn install_populates_patterns_per_locale() {
        let mut reg = SkillRegistry::new();
        let mock = MockPlugin {
            rules_by_locale: HashMap::from([
                ("fr".into(), vec![rule("hello", "bonjour")]),
                ("en".into(), vec![rule("hello", "hello"), rule("bye", "bye")]),
            ]),
            response: Ok(SkillResponse::empty()),
            handle_calls: Arc::new(AtomicUsize::new(0)),
            last_intent: Arc::new(Mutex::new(None)),
        };
        reg.install(
            "greeter",
            arc_mock(mock),
            &["fr".to_string(), "en".to_string()],
        )
        .unwrap();

        assert_eq!(reg.patterns().locale_count(), 2);
        assert_eq!(reg.patterns().for_locale("fr").unwrap().len(), 1);
        assert_eq!(reg.patterns().for_locale("en").unwrap().len(), 2);
        assert_eq!(reg.skill_names(), vec!["greeter".to_string()]);
    }

    #[test]
    fn dispatch_calls_plugin_and_returns_response() {
        let mut reg = SkillRegistry::new();
        let handle_calls = Arc::new(AtomicUsize::new(0));
        let last_intent = Arc::new(Mutex::new(None));
        let mock = MockPlugin {
            rules_by_locale: HashMap::new(),
            response: Ok(SkillResponse::speak("il est huit heures")),
            handle_calls: handle_calls.clone(),
            last_intent: last_intent.clone(),
        };
        reg.install("clock", arc_mock(mock), &[]).unwrap();

        let intent = Intent {
            name: "time.query".into(),
            slots: BTreeMap::new(),
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
        let mut reg = SkillRegistry::new();
        let mock = MockPlugin {
            rules_by_locale: HashMap::new(),
            response: Err(SkillError::HttpFailed("boom".into())),
            handle_calls: Arc::new(AtomicUsize::new(0)),
            last_intent: Arc::new(Mutex::new(None)),
        };
        reg.install("weather", arc_mock(mock), &[]).unwrap();
        let err = reg
            .dispatch(
                "weather",
                Intent {
                    name: "weather.query".into(),
                    slots: BTreeMap::new(),
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
                },
            )
            .unwrap_err();
        assert!(matches!(err, SkillError::Custom(ref m) if m.contains("unknown skill")));
    }

    fn make_deps(rt: &tokio::runtime::Runtime, client_id: &str) -> SkillDeps {
        let store: Arc<dyn Store> = rt.block_on(async {
            Arc::new(
                athena_voice_storage::SqliteStore::open("sqlite::memory:")
                    .await
                    .unwrap(),
            )
        });
        let (mqtt, _eventloop) = rumqttc::AsyncClient::new(
            rumqttc::MqttOptions::new(client_id, "127.0.0.1", 1),
            8,
        );
        SkillDeps {
            store,
            mqtt,
            tokio: rt.handle().clone(),
            http: reqwest::Client::new(),
            locales: vec!["fr".into()],
            per_skill: HashMap::new(),
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
        assert_eq!(reg.patterns().locale_count(), 0);
    }

    #[test]
    fn load_dir_reports_build_error_on_invalid_wasm() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("broken.wasm"), b"not really wasm").unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let deps = make_deps(&rt, "registry-test-2");
        let Err(err) = SkillRegistry::load_dir(dir.path(), &deps) else {
            panic!("expected build error");
        };
        assert!(matches!(err, RegistryError::Build { ref skill, .. } if skill == "broken"));
    }
}

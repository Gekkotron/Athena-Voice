//! Merge web-edited skill settings (SQLite rows) over the TOML-derived
//! [`SkillConfig`] base — the DB wins key-by-key, so tracked TOML never
//! needs to hold secrets.

use crate::wasm::registry::SkillConfig;

/// Reserved settings key holding a JSON array of allowed HTTP hosts; the
/// admin API derives it from schema `host`/`url` fields on config save.
pub const HTTP_ALLOWLIST_KEY: &str = "$http_allowlist";

pub fn apply_settings(base: &SkillConfig, rows: &[(String, String)]) -> SkillConfig {
    let mut out = base.clone();
    for (key, value) in rows {
        if key == HTTP_ALLOWLIST_KEY {
            match serde_json::from_str::<Vec<String>>(value) {
                Ok(hosts) => out.http_allowlist = hosts,
                Err(e) => tracing::warn!(error = %e, "invalid $http_allowlist row ignored"),
            }
        } else {
            out.config.insert(key.clone(), value.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wasm::registry::SkillConfig;
    use std::collections::HashMap;

    #[test]
    fn db_rows_override_toml_key_by_key() {
        let base = SkillConfig {
            http_allowlist: vec!["old.example".into()],
            mqtt_publish_allowlist: vec!["home/+/light/set".into()],
            config: HashMap::from([
                ("base_url".into(), "http://toml".into()),
                ("kept".into(), "from-toml".into()),
            ]),
            retention_gc_after_sec: Some(60),
            config_file: None,
        };
        let rows = vec![
            ("base_url".to_string(), "http://db".to_string()),
            ("api_key".to_string(), "s3cret".to_string()),
            (
                HTTP_ALLOWLIST_KEY.to_string(),
                r#"["192.168.1.91"]"#.to_string(),
            ),
        ];
        let merged = apply_settings(&base, &rows);
        assert_eq!(merged.config["base_url"], "http://db"); // overridden
        assert_eq!(merged.config["kept"], "from-toml"); // preserved
        assert_eq!(merged.config["api_key"], "s3cret"); // added
        assert_eq!(merged.http_allowlist, vec!["192.168.1.91"]); // replaced
        assert_eq!(merged.mqtt_publish_allowlist, base.mqtt_publish_allowlist);
        assert_eq!(merged.retention_gc_after_sec, Some(60));
        assert!(!merged.config.contains_key(HTTP_ALLOWLIST_KEY)); // reserved key not leaked
    }

    #[test]
    fn invalid_allowlist_json_keeps_base_allowlist() {
        let base = SkillConfig::default();
        let rows = vec![(HTTP_ALLOWLIST_KEY.to_string(), "not-json".to_string())];
        let merged = apply_settings(&base, &rows);
        assert!(merged.http_allowlist.is_empty());
    }
}

//! Schema-driven validation for config writes; keys absent from `values`
//! are not validated (absent means "unchanged").

use std::collections::HashMap;

use athena_voice_skill_sdk::{ConfigSchema, FieldKind, ItemField};

pub(crate) fn validate(
    schema: Option<&ConfigSchema>,
    values: &HashMap<String, String>,
) -> Result<(), String> {
    let Some(schema) = schema else { return Ok(()) };
    for f in &schema.fields {
        let Some(raw) = values.get(&f.key) else {
            continue;
        };
        if raw.trim().is_empty() {
            // Fix 3: a blank secret submission means "leave unchanged" (the
            // UI never shows the stored value back), so it must not trip
            // `required` here — put_config skips persisting it and the
            // previously-stored value (if any) survives untouched.
            if f.required && !f.is_secret() {
                return Err(format!("`{}` is required", f.key));
            }
            continue;
        }
        check(f.kind, raw, &f.key, &f.item_fields)?;
    }
    Ok(())
}

fn check(kind: FieldKind, raw: &str, key: &str, items: &[ItemField]) -> Result<(), String> {
    match kind {
        FieldKind::String | FieldKind::Secret => Ok(()),
        FieldKind::Number => raw
            .trim()
            .parse::<f64>()
            .map(|_| ())
            .map_err(|_| format!("`{key}` must be a number")),
        FieldKind::Url => {
            if raw.starts_with("http://") || raw.starts_with("https://") {
                Ok(())
            } else {
                Err(format!("`{key}` must start with http:// or https://"))
            }
        }
        FieldKind::Host => {
            if raw.contains("://") || raw.contains('/') || raw.contains(' ') {
                Err(format!(
                    "`{key}` must be a bare host name (no scheme or path)"
                ))
            } else {
                Ok(())
            }
        }
        FieldKind::List => {
            let parsed: Vec<serde_json::Map<String, serde_json::Value>> = serde_json::from_str(raw)
                .map_err(|e| {
                    format!(
                        "`{key}` is not a JSON array of objects (parse error at line {}, column {})",
                        e.line(),
                        e.column()
                    )
                })?;
            for (i, item) in parsed.iter().enumerate() {
                for f in items {
                    let v = item
                        .get(&f.key)
                        .ok_or_else(|| format!("`{key}[{i}]` is missing `{}`", f.key))?;
                    let ok = match f.kind {
                        FieldKind::Number => v.is_number(),
                        _ => v.is_string(),
                    };
                    if !ok {
                        let want = if matches!(f.kind, FieldKind::Number) {
                            "number"
                        } else {
                            "string"
                        };
                        return Err(format!("`{key}[{i}].{}` must be a {want}", f.key));
                    }
                }
            }
            Ok(())
        }
    }
}

/// Hosts implied by the schema's `host` fields plus the host part of `url`
/// fields — becomes the skill's HTTP allowlist so users never edit it by hand.
pub(crate) fn derived_allowlist(
    schema: &ConfigSchema,
    merged_values: &HashMap<String, String>,
) -> Vec<String> {
    let mut hosts = Vec::new();
    for f in &schema.fields {
        let Some(raw) = merged_values.get(&f.key) else {
            continue;
        };
        if raw.trim().is_empty() {
            continue;
        }
        match f.kind {
            FieldKind::Host => hosts.push(raw.trim().to_string()),
            FieldKind::Url => {
                let no_scheme = raw.split("://").nth(1).unwrap_or(raw);
                let host = no_scheme.split('/').next().unwrap_or("");
                let host = host.split(':').next().unwrap_or("");
                if !host.is_empty() {
                    hosts.push(host.to_string());
                }
            }
            _ => {}
        }
    }
    hosts.sort();
    hosts.dedup();
    hosts
}

#[cfg(test)]
mod tests {
    use super::*;
    use athena_voice_skill_sdk::{ConfigField, ConfigSchema, FieldKind, ItemField};
    use std::collections::HashMap;

    fn field(key: &str, kind: FieldKind, required: bool) -> ConfigField {
        ConfigField {
            key: key.into(),
            label: key.into(),
            kind,
            required,
            help: String::new(),
            default: String::new(),
            item_fields: vec![],
        }
    }

    fn jeedom_schema() -> ConfigSchema {
        let mut sensors = field("sensors", FieldKind::List, false);
        sensors.item_fields = vec![
            ItemField {
                key: "name".into(),
                kind: FieldKind::String,
            },
            ItemField {
                key: "id".into(),
                kind: FieldKind::Number,
            },
            ItemField {
                key: "unit".into(),
                kind: FieldKind::String,
            },
        ];
        ConfigSchema {
            fields: vec![
                field("base_url", FieldKind::Url, true),
                field("api_key", FieldKind::Secret, true),
                sensors,
            ],
        }
    }

    #[test]
    fn accepts_valid_values_and_derives_allowlist() {
        let values = HashMap::from([
            ("base_url".to_string(), "http://192.168.1.91".to_string()),
            ("api_key".to_string(), "k".to_string()),
            (
                "sensors".to_string(),
                r#"[{"name":"salon","id":142,"unit":"degrés"}]"#.to_string(),
            ),
        ]);
        assert_eq!(validate(Some(&jeedom_schema()), &values), Ok(()));
        assert_eq!(
            derived_allowlist(&jeedom_schema(), &values),
            vec!["192.168.1.91"]
        );
    }

    #[test]
    fn rejects_bad_inputs() {
        let s = jeedom_schema();
        let bad_url = HashMap::from([("base_url".to_string(), "192.168.1.91".to_string())]);
        assert!(
            validate(Some(&s), &bad_url).is_err(),
            "url must carry a scheme"
        );

        let bad_list = HashMap::from([(
            "sensors".to_string(),
            r#"[{"name":"x","id":"142"}]"#.to_string(),
        )]);
        assert!(
            validate(Some(&s), &bad_list).is_err(),
            "id must be a JSON number"
        );

        let missing_required = HashMap::from([("base_url".to_string(), "  ".to_string())]);
        assert!(
            validate(Some(&s), &missing_required).is_err(),
            "required present-but-blank rejected"
        );

        let host_schema = ConfigSchema {
            fields: vec![field("host", FieldKind::Host, true)],
        };
        let bad_host = HashMap::from([("host".to_string(), "http://x/y".to_string())]);
        assert!(
            validate(Some(&host_schema), &bad_host).is_err(),
            "host must be bare"
        );
    }

    #[test]
    fn required_secret_blank_value_is_not_rejected() {
        // A blank secret submission means "leave unchanged" (see Fix 3 in
        // api.rs::put_config, which skips persisting it) — validation must
        // not reject it just because the field is marked required. Any OTHER
        // required-but-blank field kind still errors (covered above).
        let s = jeedom_schema(); // api_key: Secret, required: true
        let blank_secret = HashMap::from([("api_key".to_string(), String::new())]);
        assert_eq!(
            validate(Some(&s), &blank_secret),
            Ok(()),
            "blank required secret must pass validation; put_config skips persisting it"
        );
    }

    #[test]
    fn no_schema_means_no_validation() {
        let values = HashMap::from([("anything".to_string(), "goes".to_string())]);
        assert_eq!(validate(None, &values), Ok(()));
    }

    #[test]
    fn list_parse_error_never_echoes_submitted_value() {
        // serde_json's Display for a type-mismatch error embeds the offending
        // input verbatim (e.g. `invalid type: string "my-secret-value-123",
        // expected a sequence`) — the validator must never forward `{e}` (or
        // `raw`) into the message it returns, only position info.
        let s = jeedom_schema();
        let sentinel = "my-secret-value-123";
        let bad = HashMap::from([("sensors".to_string(), format!("\"{sentinel}\""))]);
        let err = validate(Some(&s), &bad).expect_err("a JSON string is not an array of objects");
        assert!(
            !err.contains(sentinel),
            "error must not echo the submitted value: {err}"
        );
        assert!(err.contains("sensors"), "error should name the key: {err}");
    }
}

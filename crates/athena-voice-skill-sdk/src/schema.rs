//! Skill config schema — the optional `config_schema` guest export returns
//! this as JSON so the admin UI can render a typed form. Skills without the
//! export get a free-form key/value editor instead.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigSchema {
    pub fields: Vec<ConfigField>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigField {
    pub key: String,
    pub label: String,
    #[serde(rename = "type")]
    pub kind: FieldKind,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub help: String,
    #[serde(default)]
    pub default: String,
    /// Item shape for `FieldKind::List` fields; empty otherwise.
    #[serde(default)]
    pub item_fields: Vec<ItemField>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldKind {
    String,
    Number,
    Secret,
    Url,
    Host,
    List,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemField {
    pub key: String,
    #[serde(rename = "type")]
    pub kind: FieldKind,
}

impl ConfigField {
    pub fn is_secret(&self) -> bool {
        matches!(self.kind, FieldKind::Secret)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_jeedom_style_schema() {
        let json = r#"{ "fields": [
            { "key": "base_url", "label": "Jeedom URL", "type": "url", "required": true },
            { "key": "api_key", "label": "API key", "type": "secret", "required": true },
            { "key": "sensors", "label": "Sensors", "type": "list",
              "item_fields": [
                { "key": "name", "type": "string" },
                { "key": "id",   "type": "number" },
                { "key": "unit", "type": "string" } ] }
        ] }"#;
        let schema: ConfigSchema = serde_json::from_str(json).unwrap();
        assert_eq!(schema.fields.len(), 3);
        assert_eq!(schema.fields[0].kind, FieldKind::Url);
        assert!(schema.fields[1].is_secret());
        assert!(!schema.fields[0].is_secret());
        assert_eq!(schema.fields[2].item_fields[1].kind, FieldKind::Number);
        // Optional fields default cleanly.
        assert!(!schema.fields[2].required);
        assert!(schema.fields[0].help.is_empty());
        let back = serde_json::to_string(&schema).unwrap();
        let again: ConfigSchema = serde_json::from_str(&back).unwrap();
        assert_eq!(schema, again);
    }
}

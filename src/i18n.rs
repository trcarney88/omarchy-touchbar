//! Translation catalog utilities. Catalogs are JSON objects whose string values
//! override the English source catalog.
use std::collections::BTreeMap;

pub const ENGLISH: &str = include_str!("../i18n/en.json");
pub const DANISH: &str = include_str!("../i18n/da.json");

pub fn merge_catalogs(catalogs: &[&str]) -> BTreeMap<String, String> {
    let mut merged = BTreeMap::new();
    for catalog in catalogs {
        if let Ok(serde_json::Value::Object(entries)) = serde_json::from_str(catalog) {
            for (key, value) in entries {
                if let Some(value) = value.as_str() {
                    merged.insert(key, value.to_owned());
                }
            }
        }
    }
    merged
}

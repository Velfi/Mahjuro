//! `{{token}}` placeholders in `assets/data/*.json` content files.
//!
//! Relic and memorial talisman descriptions expand at display time; unknown
//! tokens stay literal. Tests here verify every registered content file resolves
//! all placeholders and flag unregistered files that introduce new templates.

/// Every `{{token}}` still present in `s` after expansion (unknown or malformed).
pub fn leftover_template_tokens(s: &str) -> Vec<String> {
    let mut rest = s;
    let mut out = Vec::new();
    while let Some(start) = rest.find("{{") {
        rest = &rest[start + 2..];
        let Some(end) = rest.find("}}") else {
            break;
        };
        out.push(rest[..end].trim().to_string());
        rest = &rest[end + 2..];
    }
    out
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    use serde::Deserialize;
    use serde_json::Value;

    use super::leftover_template_tokens;
    use crate::core::memorial_desc_template::expand_memorial_description_templates;
    use crate::core::memorial_talisman::MemorialTalismanKind;
    use crate::core::relic::RelicId;
    use crate::core::relic_desc_template::{RelicDescContext, expand_relic_description_templates};

    /// Content JSON files whose `{{token}}` placeholders have dedicated expanders.
    const REGISTERED_TEMPLATE_FILES: &[&str] = &["relics.json", "memorial_talismans.json"];

    fn assets_data_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/data")
    }

    fn assets_data_path(name: &str) -> PathBuf {
        assets_data_dir().join(name)
    }

    fn read_json_value(path: &Path) -> Value {
        let bytes = std::fs::read(path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|e| panic!("failed to parse {}: {e}", path.display()))
    }

    fn json_contains_template_tokens(value: &Value) -> bool {
        match value {
            Value::String(s) => s.contains("{{"),
            Value::Array(items) => items.iter().any(json_contains_template_tokens),
            Value::Object(fields) => fields.values().any(json_contains_template_tokens),
            _ => false,
        }
    }

    fn check_relics_json() -> Vec<String> {
        #[derive(Deserialize)]
        struct RawRelic {
            id: RelicId,
            name: String,
            description: String,
        }

        let path = assets_data_path("relics.json");
        let entries: Vec<RawRelic> = serde_json::from_value(read_json_value(&path))
            .unwrap_or_else(|e| panic!("failed to decode {}: {e}", path.display()));

        let counters = BTreeMap::new();
        let mut failures = Vec::new();
        for entry in &entries {
            let ctx = RelicDescContext {
                id: entry.id,
                counters: &counters,
                gold: 0,
                relics: None,
                slot: None,
                ghost_hand_fu_preview: None,
                wing: None,
                live: false,
            };
            let expanded = expand_relic_description_templates(&entry.description, &ctx);
            let unknown = leftover_template_tokens(&expanded);
            if !unknown.is_empty() {
                failures.push(format!(
                    "relics.json {:?} ({}): unknown tokens {:?}",
                    entry.id, entry.name, unknown
                ));
            }
        }
        failures
    }

    fn check_memorial_talismans_json() -> Vec<String> {
        #[derive(Deserialize)]
        struct RawMemorial {
            id: MemorialTalismanKind,
            name: String,
            description: String,
        }

        let path = assets_data_path("memorial_talismans.json");
        let entries: Vec<RawMemorial> = serde_json::from_value(read_json_value(&path))
            .unwrap_or_else(|e| panic!("failed to decode {}: {e}", path.display()));

        let mut failures = Vec::new();
        for entry in &entries {
            let expanded =
                expand_memorial_description_templates(entry.id, &entry.description, None);
            let unknown = leftover_template_tokens(&expanded);
            if !unknown.is_empty() {
                failures.push(format!(
                    "memorial_talismans.json {:?} ({}): unknown tokens {:?}",
                    entry.id, entry.name, unknown
                ));
            }
        }
        failures
    }

    fn check_unregistered_template_files() -> Vec<String> {
        let dir = assets_data_dir();
        let mut failures = Vec::new();
        for entry in std::fs::read_dir(&dir).unwrap_or_else(|e| {
            panic!(
                "failed to read assets/data directory {}: {e}",
                dir.display()
            )
        }) {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.ends_with(".json") || REGISTERED_TEMPLATE_FILES.contains(&name.as_str()) {
                continue;
            }
            let value = read_json_value(&entry.path());
            if json_contains_template_tokens(&value) {
                failures.push(format!(
                    "{name} contains {{{{token}}}} placeholders but is not registered in content_json_templates tests — add an expander check"
                ));
            }
        }
        failures
    }

    #[test]
    fn all_content_json_description_tokens_expand() {
        let mut failures = Vec::new();
        failures.extend(check_relics_json());
        failures.extend(check_memorial_talismans_json());
        failures.extend(check_unregistered_template_files());
        assert!(
            failures.is_empty(),
            "content JSON template errors:\n{}",
            failures.join("\n")
        );
    }
}

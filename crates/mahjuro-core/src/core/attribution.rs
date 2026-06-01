//! Third-party asset attribution loaded from `assets/data/attribution.json`.

use std::sync::OnceLock;

use serde::Deserialize;

use crate::core::json_asset::load_json_asset;

const PATH: &str = "data/attribution.json";

#[derive(Clone, Debug, Deserialize)]
pub struct AttributionSection {
    pub title: String,
    pub entries: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct AttributionCatalog {
    pub title: String,
    pub subtitle: String,
    pub footer: String,
    pub sections: Vec<AttributionSection>,
}

#[derive(Deserialize)]
struct AttributionFileRaw {
    #[serde(default = "default_title")]
    title: String,
    #[serde(default)]
    subtitle: String,
    #[serde(default)]
    footer: String,
    sections: Vec<AttributionSection>,
}

fn default_title() -> String {
    "Attribution".into()
}

pub fn attribution_catalog() -> &'static AttributionCatalog {
    static CATALOG: OnceLock<AttributionCatalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let raw: AttributionFileRaw = load_json_asset(PATH, "attribution data");
        AttributionCatalog {
            title: raw.title,
            subtitle: raw.subtitle,
            footer: raw.footer,
            sections: raw.sections,
        }
    })
}

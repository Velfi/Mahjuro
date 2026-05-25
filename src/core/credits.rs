//! Credits roll loaded from `assets/data/credits.json`.

use std::sync::OnceLock;

use serde::Deserialize;

use crate::core::json_asset::load_json_asset;

const PATH: &str = "data/credits.json";

#[derive(Clone, Debug, Deserialize)]
pub struct CreditEntry {
    pub name: String,
    #[serde(default)]
    pub role: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CreditSection {
    pub title: String,
    pub entries: Vec<CreditEntry>,
}

#[derive(Clone, Debug)]
pub struct CreditsCatalog {
    pub title: String,
    pub subtitle: String,
    pub footer: String,
    pub sections: Vec<CreditSection>,
}

#[derive(Deserialize)]
struct CreditsFileRaw {
    #[serde(default = "default_title")]
    title: String,
    #[serde(default)]
    subtitle: String,
    #[serde(default)]
    footer: String,
    sections: Vec<CreditSection>,
}

fn default_title() -> String {
    "Credits".into()
}

pub fn credits_catalog() -> &'static CreditsCatalog {
    static CATALOG: OnceLock<CreditsCatalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let raw: CreditsFileRaw = load_json_asset(PATH, "credits data");
        CreditsCatalog {
            title: raw.title,
            subtitle: raw.subtitle,
            footer: raw.footer,
            sections: raw.sections,
        }
    })
}

//! Random post-ordeal copy from [`assets/data/staircase_flavor.json`](../../../assets/data/staircase_flavor.json).

use std::sync::OnceLock;

use rand::prelude::IndexedRandom;
use serde::Deserialize;

use crate::core::json_asset::load_json_asset;
use crate::core::relic::RelicFlavorSpan;

#[derive(Deserialize)]
struct StaircaseFlavorSpanRaw {
    text: String,
    #[serde(default)]
    bold: bool,
    #[serde(default)]
    italic: bool,
}

#[derive(Deserialize)]
struct StaircaseFlavorEntryRaw {
    #[serde(default, rename = "id")]
    _id: String,
    #[serde(default)]
    flavor_spans: Vec<StaircaseFlavorSpanRaw>,
}

fn leak_flavor_spans(raw: Vec<StaircaseFlavorSpanRaw>) -> &'static [RelicFlavorSpan] {
    let v: Vec<RelicFlavorSpan> = raw
        .into_iter()
        .filter(|s| !s.text.is_empty())
        .map(|s| RelicFlavorSpan {
            text: Box::leak(s.text.into_boxed_str()),
            bold: s.bold,
            italic: s.italic,
        })
        .collect();
    if v.is_empty() || v.iter().all(|s| s.text.chars().all(char::is_whitespace)) {
        &[]
    } else {
        Box::leak(v.into_boxed_slice())
    }
}

fn all_entries() -> &'static [&'static [RelicFlavorSpan]] {
    static ENTRIES: OnceLock<Vec<&'static [RelicFlavorSpan]>> = OnceLock::new();
    ENTRIES
        .get_or_init(|| {
            const PATH: &str = "data/staircase_flavor.json";
            let raw: Vec<StaircaseFlavorEntryRaw> = load_json_asset(PATH, "staircase flavor");
            raw.into_iter()
                .map(|e| leak_flavor_spans(e.flavor_spans))
                .filter(|spans| !spans.is_empty())
                .collect()
        })
        .as_slice()
}

/// One random flavor paragraph for the staircase interstitial; empty when data is missing.
pub fn random_entry_flavor() -> &'static [RelicFlavorSpan] {
    let entries = all_entries();
    if entries.is_empty() {
        return &[];
    }
    let mut rng = rand::rng();
    entries.choose(&mut rng).copied().unwrap_or(&[])
}

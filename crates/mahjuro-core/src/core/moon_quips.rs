//! Random hub-moon speech lines from [`assets/data/moon_quips.json`](../../../assets/data/moon_quips.json).

use std::collections::HashMap;
use std::sync::OnceLock;

use chrono::Datelike;
use rand::RngExt;
use rand::prelude::SliceRandom;
use serde::Deserialize;

use crate::core::json_asset::load_json_asset;

const PATH: &str = "data/moon_quips.json";

#[derive(Deserialize)]
struct MoonQuipsFileRaw {
    /// June hub lines (`pride_lines` preferred). Used when `pride_lines` is empty.
    #[serde(default = "default_june_line")]
    june_line: String,
    #[serde(default)]
    pride_lines: Vec<MoonQuipWeightedRaw>,
    #[serde(default)]
    category_weights: HashMap<String, f32>,
    #[serde(default)]
    lines: Vec<MoonQuipLineRaw>,
}

#[derive(Deserialize)]
struct MoonQuipWeightedRaw {
    text: String,
    #[serde(default = "default_line_weight")]
    weight: f32,
}

#[derive(Deserialize)]
struct MoonQuipLineRaw {
    text: String,
    category: String,
    #[serde(default = "default_line_weight")]
    weight: f32,
}

fn default_june_line() -> String {
    "Happy Pride!".into()
}

fn default_line_weight() -> f32 {
    1.0
}

fn leak_str(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

struct MoonQuipEntry {
    text: &'static str,
    #[allow(dead_code)]
    category: &'static str,
    weight: f32,
}

struct MoonQuipsCatalog {
    pride_entries: Vec<MoonQuipEntry>,
    entries: Vec<MoonQuipEntry>,
    /// Category name → roll weight when that category still has unseen lines.
    category_weights: HashMap<&'static str, f32>,
    /// Category name → entry indices (regular pool only).
    by_category: HashMap<&'static str, Vec<usize>>,
}

fn catalog() -> &'static MoonQuipsCatalog {
    static CAT: OnceLock<MoonQuipsCatalog> = OnceLock::new();
    CAT.get_or_init(|| {
        let raw: MoonQuipsFileRaw = load_json_asset(PATH, "moon quips");
        let mut pride_entries = Vec::new();
        let mut pride_raw = raw.pride_lines;
        if pride_raw.is_empty() && !raw.june_line.trim().is_empty() {
            pride_raw.push(MoonQuipWeightedRaw {
                text: raw.june_line,
                weight: 1.0,
            });
        }
        for line in pride_raw {
            let text = line.text.trim();
            if text.is_empty() {
                continue;
            }
            let weight = line.weight.max(0.0);
            pride_entries.push(MoonQuipEntry {
                text: leak_str(text.to_string()),
                category: "pride",
                weight: if weight > 0.0 { weight } else { 1.0 },
            });
        }

        let mut entries = Vec::new();
        let mut by_category: HashMap<&'static str, Vec<usize>> = HashMap::new();
        let mut category_weights: HashMap<&'static str, f32> = HashMap::new();
        for (k, w) in raw.category_weights {
            if w > 0.0 {
                category_weights.insert(leak_str(k), w);
            }
        }
        for line in raw.lines {
            let text = line.text.trim();
            if text.is_empty() {
                continue;
            }
            let category = leak_str(line.category);
            let weight = line.weight.max(0.0);
            let idx = entries.len();
            entries.push(MoonQuipEntry {
                text: leak_str(text.to_string()),
                category,
                weight: if weight > 0.0 { weight } else { 1.0 },
            });
            by_category.entry(category).or_default().push(idx);
            category_weights.entry(category).or_insert(1.0);
        }
        MoonQuipsCatalog {
            pride_entries,
            entries,
            category_weights,
            by_category,
        }
    })
}

fn active_pool(cat: &MoonQuipsCatalog) -> (&[MoonQuipEntry], bool) {
    if moon_quip_is_pride_month() {
        (&cat.pride_entries, true)
    } else {
        (&cat.entries, false)
    }
}

fn weighted_pick_index(weights: &[(usize, f32)]) -> Option<usize> {
    if weights.is_empty() {
        return None;
    }
    let total: f32 = weights.iter().map(|(_, w)| w).sum();
    if total <= 0.0 {
        return weights.first().map(|(i, _)| *i);
    }
    let mut roll = rand::rng().random::<f32>() * total;
    for &(idx, w) in weights {
        roll -= w;
        if roll <= 0.0 {
            return Some(idx);
        }
    }
    weights.last().map(|(i, _)| *i)
}

/// June (local calendar) — matches main-menu pride rainbow month.
#[inline]
pub fn moon_quip_is_pride_month() -> bool {
    chrono::Local::now().month() == 6
}

/// Number of quips in the active pool (pride lines in June, regular lines otherwise).
pub fn moon_quip_entry_count() -> usize {
    let (pool, _) = active_pool(catalog());
    pool.len()
}

/// Fill `remaining` with every line index for the active pool.
pub fn refill_moon_quip_bag(remaining: &mut Vec<usize>) {
    let n = moon_quip_entry_count();
    remaining.clear();
    remaining.extend(0..n);
    remaining.shuffle(&mut rand::rng());
}

/// Line shown when the hub moon is clicked.
/// `remaining` holds indices not yet shown this main-menu visit; refills when empty.
pub fn roll_moon_quip(remaining: &mut Vec<usize>) -> &'static str {
    let cat = catalog();
    let (pool, pride) = active_pool(cat);
    if pool.is_empty() {
        return "…";
    }
    let n = pool.len();
    if remaining.len() != n {
        refill_moon_quip_bag(remaining);
    }
    if remaining.is_empty() {
        refill_moon_quip_bag(remaining);
    }
    let chosen = if pride {
        pick_weighted_pride_index(pool, remaining)
    } else {
        pick_weighted_entry_index(cat, remaining)
    };
    let Some(idx) = chosen else {
        return "…";
    };
    remaining.retain(|&i| i != idx);
    pool.get(idx).map(|e| e.text).unwrap_or("…")
}

fn pick_weighted_pride_index(pool: &[MoonQuipEntry], remaining: &[usize]) -> Option<usize> {
    let weights: Vec<(usize, f32)> = remaining
        .iter()
        .filter_map(|&idx| pool.get(idx).map(|e| (idx, e.weight)))
        .collect();
    weighted_pick_index(&weights)
}

fn pick_weighted_entry_index(cat: &MoonQuipsCatalog, remaining: &[usize]) -> Option<usize> {
    let remaining_set: HashMap<usize, ()> = remaining.iter().map(|&i| (i, ())).collect();

    let mut category_choices: Vec<(&'static str, f32)> = Vec::new();
    for (&category, &cat_w) in &cat.category_weights {
        let has_line = cat.by_category.get(category).is_some_and(|indices| {
            indices.iter().any(|&idx| remaining_set.contains_key(&idx))
        });
        if has_line && cat_w > 0.0 {
            category_choices.push((category, cat_w));
        }
    }

    if let Some(category) = weighted_pick_category(&category_choices) {
        if let Some(indices) = cat.by_category.get(category) {
            let line_weights: Vec<(usize, f32)> = indices
                .iter()
                .filter(|&&idx| remaining_set.contains_key(&idx))
                .filter_map(|&idx| cat.entries.get(idx).map(|e| (idx, e.weight)))
                .collect();
            if let Some(idx) = weighted_pick_index(&line_weights) {
                return Some(idx);
            }
        }
    }

    let fallback: Vec<(usize, f32)> = remaining
        .iter()
        .filter_map(|&idx| cat.entries.get(idx).map(|e| (idx, e.weight)))
        .collect();
    weighted_pick_index(&fallback)
}

fn weighted_pick_category(choices: &[(&'static str, f32)]) -> Option<&'static str> {
    let weights: Vec<(usize, f32)> = choices
        .iter()
        .enumerate()
        .map(|(i, (_, w))| (i, *w))
        .collect();
    weighted_pick_index(&weights).map(|i| choices[i].0)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn moon_quips_json_loads_with_categories() {
        let cat = catalog();
        assert!(cat.entries.len() >= 20);
        assert!(cat.pride_entries.len() >= 2);
        assert_eq!(cat.pride_entries[0].text, "Happy Pride!");
        assert!(cat.by_category.contains_key("lunar"));
        assert!(cat.by_category.contains_key("rare"));
    }

    #[test]
    fn moon_quip_bag_no_repeat_until_exhausted() {
        if moon_quip_is_pride_month() {
            let cat = catalog();
            let n = cat.pride_entries.len();
            let mut remaining: Vec<usize> = (0..n).collect();
            let mut picked = HashSet::new();
            for _ in 0..n {
                let idx = pick_weighted_pride_index(&cat.pride_entries, &remaining).expect("pick");
                assert!(picked.insert(idx));
                remaining.retain(|&i| i != idx);
            }
            assert!(remaining.is_empty());
            return;
        }
        let cat = catalog();
        let n = cat.entries.len();
        let mut remaining: Vec<usize> = (0..n).collect();
        let mut picked = HashSet::new();
        for _ in 0..n {
            let idx = pick_weighted_entry_index(cat, &remaining).expect("pick");
            assert!(picked.insert(idx));
            remaining.retain(|&i| i != idx);
        }
        assert!(remaining.is_empty());
        refill_moon_quip_bag(&mut remaining);
        assert_eq!(remaining.len(), n);
    }
}

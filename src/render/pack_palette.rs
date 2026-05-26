//! Per-pack visual palette: the foil wrapper tint, the wax-seal red,
//! and the deep background that gets baked into the AI cover art.
//!
//! Tile packs have *four* color signals that need to agree:
//!
//! 1. The 3D foil shader on the rendered booster pack (`Object3d.color`).
//! 2. The wax seal disc baked onto the cover PNG.
//! 3. The runtime hover halo on the shop counter.
//! 4. The deep background color the AI cover art is generated against.
//!
//! Before this module, those four lived in three different places — a Rust
//! method on `TilePackKind` (#1, #3), a Python tuple in
//! `scripts/bake_pack_seals.py` (#2), and a Python prompt string in
//! `scripts/generate_pack_covers.py` (#4). Drift was inevitable.
//!
//! Now Rust is the canonical source for all four, mirrored into
//! `tools/pack_palette.json` (checked in). The Python bake / generate
//! scripts read the JSON instead of carrying their own copies. A unit
//! test below loads the JSON and asserts every field matches the Rust
//! constants — if anyone edits one side without the other, `cargo test
//! pack_palette` fails.
//!
//! The palette is *not* in `theme.rs` because the wrapper / seal / bg
//! values are deliberate per-pack signatures, not generic theme tokens.
//! Where a per-pack value happens to be a theme token (e.g. Honors seal
//! is `RUBY`), the constant references that token directly so the
//! relationship survives a re-skin.

use crate::core::tile_pack::TilePackKind;
use crate::render::theme::color;

/// Per-pack visual signature. All four colors are RGBA in linear-ish
/// 0..1 space matching the rest of the rendering pipeline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PackPalette {
    /// Stable filename slug (also the basename of `pack_<slug>.png`).
    pub slug: &'static str,
    /// Display name; matches `TilePackKind::name()`.
    pub display_name: &'static str,
    /// Shrink-wrap sleeve tint — multiplied on pack edges; cover art uses
    /// the decal. See `TilePackKind::foil_tint`.
    pub foil: [f32; 4],
    /// Wax-seal red baked onto the cover PNG and reused at runtime for
    /// the hover-glow halo. See `TilePackKind::seal_color`.
    pub seal: [f32; 4],
    /// Background the AI generates the cover art against; baked into
    /// the PNG as a solid edge-to-edge fill. Effectively the dominant
    /// color when looking at a pack on the shop counter.
    pub bg: [f32; 4],
    /// Short descriptor for the bg color used in AI prompts ("deep
    /// navy", "warm obsidian", etc.). The hex is interpolated next to
    /// it to disambiguate.
    pub bg_name: &'static str,
    /// Single-character insignia stamped into the wax seal at bake
    /// time. Picked from the ASCII subset that the bake script's
    /// fallback font always renders.
    pub insignia: &'static str,
}

pub const HONORS: PackPalette = PackPalette {
    slug: "honors",
    display_name: "Honors Pack",
    foil: [0.92, 0.78, 0.38, 1.0],
    seal: color::RUBY,
    bg: [0.055, 0.094, 0.220, 1.0], // #0E1838
    bg_name: "deep navy",
    insignia: "H",
};

pub const TERMINALS: PackPalette = PackPalette {
    slug: "terminals",
    display_name: "Terminals Pack",
    foil: [0.78, 0.52, 0.32, 1.0],
    seal: [0.56, 0.14, 0.12, 1.0],
    bg: [0.102, 0.078, 0.071, 1.0], // #1A1412
    bg_name: "warm obsidian",
    insignia: "9",
};

pub const FLOWERS: PackPalette = PackPalette {
    slug: "flowers",
    display_name: "Flowers Pack",
    foil: [0.92, 0.62, 0.70, 1.0],
    seal: [0.52, 0.14, 0.30, 1.0],
    bg: [0.110, 0.059, 0.118, 1.0], // #1C0F1E
    bg_name: "plum-black",
    insignia: "F",
};

pub const SOUZU: PackPalette = PackPalette {
    slug: "souzu",
    display_name: "Souzu Pack",
    foil: [0.48, 0.78, 0.52, 1.0],
    seal: [0.78, 0.18, 0.14, 1.0],
    bg: [0.039, 0.102, 0.055, 1.0], // #0A1A0E
    bg_name: "deep forest-black",
    insignia: "B",
};

pub const PINZU: PackPalette = PackPalette {
    slug: "pinzu",
    display_name: "Pinzu Pack",
    foil: [0.78, 0.82, 0.88, 1.0],
    seal: [0.58, 0.10, 0.18, 1.0],
    bg: [0.102, 0.055, 0.071, 1.0], // #1A0E12
    bg_name: "burgundy-black",
    insignia: "C",
};

pub const MANZU: PackPalette = PackPalette {
    slug: "manzu",
    display_name: "Manzu Pack",
    foil: [0.42, 0.48, 0.78, 1.0],
    seal: [0.72, 0.18, 0.18, 1.0],
    bg: [0.102, 0.078, 0.039, 1.0], // #1A140A
    bg_name: "sepia-black",
    insignia: "S",
};

/// Look up the palette for a pack kind. Cheap (`Copy` of a small struct).
pub const fn for_kind(kind: TilePackKind) -> PackPalette {
    match kind {
        TilePackKind::Honors => HONORS,
        TilePackKind::Terminals => TERMINALS,
        TilePackKind::Flowers => FLOWERS,
        TilePackKind::Souzu => SOUZU,
        TilePackKind::Pinzu => PINZU,
        TilePackKind::Manzu => MANZU,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::theme;
    use serde_json::{Map, Number, Value, json};
    use std::path::PathBuf;

    fn all_palettes() -> Vec<PackPalette> {
        TilePackKind::all().iter().copied().map(for_kind).collect()
    }

    fn canonical_json() -> String {
        fn round4(v: f32) -> Value {
            let n = ((v as f64) * 10_000.0).round() / 10_000.0;
            Number::from_f64(n)
                .map(Value::Number)
                .unwrap_or(Value::Null)
        }

        fn rgba(c: [f32; 4]) -> Value {
            Value::Array(c.iter().map(|v| round4(*v)).collect())
        }

        let mut packs = Map::new();
        for p in all_palettes() {
            let mut entry = Map::new();
            entry.insert("display_name".into(), Value::String(p.display_name.into()));
            entry.insert("foil".into(), rgba(p.foil));
            entry.insert("seal".into(), rgba(p.seal));
            entry.insert("bg".into(), rgba(p.bg));
            entry.insert("bg_name".into(), Value::String(p.bg_name.into()));
            entry.insert("insignia".into(), Value::String(p.insignia.into()));
            packs.insert(p.slug.to_string(), Value::Object(entry));
        }

        let root = json!({
            "_generated_by": "src/render/pack_palette.rs (cargo test pack_palette::tests::dump_pack_palette -- --nocapture)",
            "_kind_order": TilePackKind::all().iter().map(|k| for_kind(*k).slug).collect::<Vec<_>>(),
            "border_gold": rgba(theme::color::GOLD),
            "packs": Value::Object(packs),
        });

        serde_json::to_string_pretty(&root)
            .expect("pack_palette JSON serialization should never fail")
    }

    fn json_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tools/pack_palette.json")
    }

    fn assert_rgba_eq(field: &str, slug: &str, rust: [f32; 4], json: &Value) {
        let arr = json
            .as_array()
            .unwrap_or_else(|| panic!("{slug}.{field} not an array"));
        assert_eq!(arr.len(), 4, "{slug}.{field} has {} channels", arr.len());
        for (i, ch) in arr.iter().enumerate() {
            let json_v = ch
                .as_f64()
                .unwrap_or_else(|| panic!("{slug}.{field}[{i}] not a number"));
            let delta = (json_v - rust[i] as f64).abs();
            assert!(
                delta < 1e-6,
                "{slug}.{field}[{i}]: rust={} json={} delta={:.3e}",
                rust[i],
                json_v,
                delta,
            );
        }
    }

    #[test]
    fn json_mirror_matches_rust_constants() {
        let raw = std::fs::read_to_string(json_path()).expect(
            "tools/pack_palette.json must exist; regenerate via the dump_pack_palette test",
        );
        let parsed: Value = serde_json::from_str(&raw).expect("pack_palette.json is invalid JSON");
        let packs = parsed
            .get("packs")
            .and_then(Value::as_object)
            .expect("pack_palette.json missing top-level `packs` object");

        for palette in all_palettes() {
            let entry = packs
                .get(palette.slug)
                .unwrap_or_else(|| panic!("pack_palette.json missing slug `{}`", palette.slug));

            assert_eq!(
                entry["display_name"].as_str(),
                Some(palette.display_name),
                "{}.display_name diverged from Rust",
                palette.slug,
            );
            assert_eq!(
                entry["bg_name"].as_str(),
                Some(palette.bg_name),
                "{}.bg_name diverged from Rust",
                palette.slug,
            );
            assert_eq!(
                entry["insignia"].as_str(),
                Some(palette.insignia),
                "{}.insignia diverged from Rust",
                palette.slug,
            );
            assert_rgba_eq("foil", palette.slug, palette.foil, &entry["foil"]);
            assert_rgba_eq("seal", palette.slug, palette.seal, &entry["seal"]);
            assert_rgba_eq("bg", palette.slug, palette.bg, &entry["bg"]);
        }

        let border = parsed
            .get("border_gold")
            .expect("pack_palette.json missing top-level `border_gold`");
        assert_rgba_eq("border_gold", "_shared", theme::color::GOLD, border);

        assert_eq!(
            packs.len(),
            all_palettes().len(),
            "pack_palette.json has {} packs, Rust has {}",
            packs.len(),
            all_palettes().len(),
        );
    }

    #[test]
    fn dump_pack_palette() {
        println!("{}", canonical_json());
    }
}

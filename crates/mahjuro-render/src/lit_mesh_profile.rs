//! Runtime A/B toggles for `lit_mesh.wgsl` cost isolation (headless profiling only).
//!
//! Set `MAHJURO_LIT_MESH_PROFILE` to a comma-separated list of tokens:
//! `no_per_light_shadow`, `no_combined_shadow` (dynamic receiver indirect only),
//! `no_spec`, `one_light`, `no_pcf`.

use std::sync::OnceLock;

pub const NO_PER_LIGHT_SHADOW: u32 = 1;
pub const NO_COMBINED_SHADOW: u32 = 2;
pub const NO_SPEC: u32 = 4;
pub const ONE_LIGHT: u32 = 8;
pub const NO_PCF: u32 = 16;

static FLAGS: OnceLock<u32> = OnceLock::new();

fn parse_flags() -> u32 {
    let Some(raw) = std::env::var("MAHJURO_LIT_MESH_PROFILE").ok() else {
        return 0;
    };
    let mut flags = 0u32;
    for token in raw.split(',') {
        let t = token.trim().to_ascii_lowercase();
        if t.is_empty() || t == "baseline" {
            continue;
        }
        flags |= match t.as_str() {
            "no_per_light_shadow" | "no_shadow_per_light" => NO_PER_LIGHT_SHADOW,
            "no_combined_shadow" | "no_shadow_combined" => NO_COMBINED_SHADOW,
            "no_shadow" | "no_shadows" => NO_PER_LIGHT_SHADOW | NO_COMBINED_SHADOW,
            "no_spec" | "no_specular" => NO_SPEC,
            "one_light" | "single_light" => ONE_LIGHT,
            "no_pcf" | "pcf1" => NO_PCF,
            "diffuse_only" => NO_PER_LIGHT_SHADOW | NO_COMBINED_SHADOW | NO_SPEC,
            _ => {
                log::warn!("lit_mesh_profile: unknown token '{t}' in MAHJURO_LIT_MESH_PROFILE");
                0
            }
        };
    }
    flags
}

#[inline]
pub fn flags() -> u32 {
    *FLAGS.get_or_init(parse_flags)
}

#[inline]
pub fn flags_f32() -> f32 {
    flags() as f32
}

#[inline]
pub fn pcf_single_tap() -> bool {
    flags() & NO_PCF != 0
}

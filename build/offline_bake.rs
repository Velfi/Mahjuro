//! Check committed offline outputs from `build.rs` when stamps are stale (local dev).
//!
//! If stamps are stale, the build will panic with instructions on how to rebake.

use std::env;
use std::path::Path;

use mahjuro_bake_stamp::relic::Relic;
use mahjuro_bake_stamp::room_gi::RoomGi;
use mahjuro_bake_stamp::room_shadow::RoomShadow;
use mahjuro_bake_stamp::showcase_decal::ShowcaseDecal;
use mahjuro_bake_stamp::{assert_bake_current, skip_committed_bake_checks};

pub fn emit_rerun_if_changed() {
    println!("cargo:rerun-if-env-changed=CI");
    println!("cargo:rerun-if-env-changed=HOST");
    println!("cargo:rerun-if-env-changed=TARGET");
}

pub fn ensure_committed_offline_bakes_current(repo: &Path, _profile_dir: &Path) {
    if skip_committed_bake_freshness() {
        println!("cargo:info=skipping committed offline bake freshness checks");
        return;
    }

    assert_bake_current::<RoomGi>(repo);
    assert_bake_current::<RoomShadow>(repo);
    assert_bake_current::<ShowcaseDecal>(repo);
    assert_bake_current::<Relic>(repo);
}

fn skip_committed_bake_freshness() -> bool {
    skip_committed_bake_checks()
        || cargo_feature_enabled("CARGO_FEATURE_HEADLESS_SCREENSHOT")
        || cargo_feature_enabled("CARGO_FEATURE_OFFLINE_BAKE_SUPPORT")
}

fn cargo_feature_enabled(name: &str) -> bool {
    env::var(name).is_ok_and(|v| {
        let v = v.trim();
        !v.is_empty() && v != "0" && !v.eq_ignore_ascii_case("false")
    })
}

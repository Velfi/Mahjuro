//! Invoked from `build.rs`: verify committed showcase decal atlases match inputs.

use std::path::Path;

use mahjuro_bake_stamp::showcase_decal::{ShowcaseDecal, rerun_if_changed_paths};

pub fn emit_rerun_if_changed() {
    mahjuro_bake_stamp::emit_rerun_if_changed::<ShowcaseDecal>(rerun_if_changed_paths());
}

pub fn assert_showcase_decal_atlases_current(repo: &Path) {
    mahjuro_bake_stamp::assert_bake_current::<ShowcaseDecal>(repo);
}

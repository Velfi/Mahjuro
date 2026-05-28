//! Invoked from `build/offline_bake.rs`: verify committed showcase decal atlases match inputs.

use mahjuro_bake_stamp::showcase_decal::{ShowcaseDecal, rerun_if_changed_paths};

pub fn emit_rerun_if_changed() {
    mahjuro_bake_stamp::emit_rerun_if_changed::<ShowcaseDecal>(rerun_if_changed_paths());
}

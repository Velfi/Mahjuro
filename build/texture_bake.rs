//! Invoked from `build/offline_bake.rs`: verify committed generic BTX1 texture bakes match inputs.

use mahjuro_bake_stamp::texture::{Texture, rerun_if_changed_paths};

pub fn emit_rerun_if_changed() {
    mahjuro_bake_stamp::emit_rerun_if_changed::<Texture>(rerun_if_changed_paths());
}

//! Invoked from `build/offline_bake.rs`: verify committed relic RLC1 bakes match inputs.

use mahjuro_bake_stamp::relic::{Relic, rerun_if_changed_paths};

pub fn emit_rerun_if_changed() {
    mahjuro_bake_stamp::emit_rerun_if_changed::<Relic>(rerun_if_changed_paths());
}

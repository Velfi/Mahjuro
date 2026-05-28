//! Invoked from `build.rs`: verify committed relic RLC1 bakes match inputs.

use std::path::Path;

use mahjuro_bake_stamp::relic::{Relic, rerun_if_changed_paths};

pub fn emit_rerun_if_changed() {
    mahjuro_bake_stamp::emit_rerun_if_changed::<Relic>(rerun_if_changed_paths());
}

pub fn assert_relic_bakes_current(repo: &Path) {
    mahjuro_bake_stamp::assert_bake_current::<Relic>(repo);
}

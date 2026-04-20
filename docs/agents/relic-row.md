# Relic display row

Use `relic_row(relics, score_panel, window_w)` from [`scenes/mod.rs`](../../src/scenes/mod.rs) to render a horizontal row of relic badges below the score panel. Returns `(Vec<GpuInstance>, Vec<TextLabel>)` to extend into the scene output. Used in gameplay, shop, and pick-blind scenes.

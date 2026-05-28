# Tile pack opening (TPOS2)

Phase machine and timing live in [`pack_celebration.rs`](../../src/scenes/shop/pack_celebration.rs); presentation in [`tile_pack.rs`](../../src/scenes/showcase/tile_pack.rs).

## Phases

| Phase | Feel | Input |
|-------|------|-------|
| **Arrival** | Shooting-star wipe + pack title fade-in | Auto-advances when intro completes |
| **Anticipation** | Pack hero: seal glow, breathe | Confirm to break seal |
| **Unseal** | Short “break seal” punch (~0.55 s) | None |
| **Deal** | Tiles arc out and settle into the reveal row | Dismiss when settled |

## Key constants (`PackCelebration`)

- `UNSEAL_SECS` = 0.55
- `DEAL_STAGGER` = 0.14 s between tile launches
- `DEAL_TILE_FLY_SECS` = 0.42
- `SETTLE_SECS` = 0.25 after last tile lands
- `FAN_HALF_DEG` = 28° reveal fan

## Screenshots

`PackCelebration::screenshot_reveal_settled` jumps to Deal with all tiles revealed — used by `TilePackPresenter::new_headless_screenshot` and `mahjuro-screenshot --scene tile_pack_celebration`.

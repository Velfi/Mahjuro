# Mahjuro

**A mahjong roguelite where you break the rules, build impossible hands, and chase absurd combos.**

Mahjuro takes the tile-matching beauty of mahjong and reimagines it as a roguelite deckbuilder inspired by Balatro. No mahjong experience required — the game teaches you as you play, and every run is a new puzzle of pattern recognition, risk management, and combo stacking.

<!-- If you have screenshots, uncomment and add them here:
![Gameplay Screenshot](screenshots/gameplay.png)
-->

## How It Works

Each run is a series of **antes**. Each ante has three **blinds** with a score target you must beat before you run out of **plays** and **discards**.

During a blind you draw tiles, group them into **melds** (pairs, sequences, triplets, kongs), and **play** those melds into **structure** — your bank for the round. **Cash In** scores everything in structure at once as **chips × multiplier**. Nothing counts toward the target until you cash in.

Between blinds, visit the **shop** for **relics** (persistent run upgrades), **zodiac cards**, and **talismans**. Relics stack and interact; yaku and dora add big bonuses; boss blinds rewrite the rules.

For scoring details (tile values, yaku, dora, relic math), see [`docs/agents/scoring.md`](docs/agents/scoring.md).

## Progression

Meta progress persists across runs. You unlock new yaku, relics, tile materials, dora indicators, bosses, and difficulty seasons as you play more.

## Getting Started

### Requirements

- [Rust](https://rustup.rs/) (2024 edition)
- [Python](https://www.python.org/) 3 on `PATH` (`python3` or `python`) — `build.rs` runs `tools/bake_assets/bake_assets.py` so packs sit next to the binary (set `MAHJURO_SKIP_ASSET_BAKE=1` only if you supply packs or `MAHJURO_ASSETS` yourself)
- A GPU with Vulkan, Metal, or DX12 support
- **Minimum (discrete 4 GB VRAM @ 1080p):** choose **Low memory** in Options → Graphics (or let the game suggest it on first launch). **Recommended:** 6 GB+ VRAM for **Visuals** at native resolution.

### Build & Run

```bash
git clone https://github.com/Velfi/Mahjuro.git
cd mahjuro
cargo run --release
```

Release mode is recommended; the first build bakes asset packs via Python. Start a new run and follow the in-game tutorial.

### Gamepad Support

Mahjuro supports gamepads through **SDL3** (Xbox, PlayStation, Nintendo Switch / Switch 2, Steam Deck, Steam Controller, and generic USB pads).

## For Developers

### Bot Mode

Headless AI runs for balance testing. Always use `--release` — debug builds are much slower.

```bash
cargo run --release -- bot 200
cargo run --release -- sweep --runs 30
cargo run --release -- bot 1 --bot-log
```

See `BOT.md` for parameters, sweeps, and tuning.

### Project Structure

```
src/          Game loop, scenes, UI, audio, persistence
crates/       Core logic (`mahjuro-core`), rendering (`mahjuro-render`), headless tools
assets/       Models, textures, fonts, audio, scene art
```

### Documentation

| Document | Description |
| -------- | ----------- |
| `GAME_DESIGN.md` | Feature spec and design philosophy |
| `ARCHITECTURE.md` | System design and module structure |
| `PROGRESSION.md` | Unlock trees and meta-progression |
| `BOT.md` | Headless AI player guide and tuning |
| `docs/agents/scoring.md` | Play / cash-in loop and score math |
| `docs/agents/launch-options.md` | CLI flags and `MAHJURO_*` env vars |
| `docs/steam-uploads.md` | Publishing builds to Steam |

# Mahjuro

**A mahjong roguelite where you break the rules, build impossible hands, and chase absurd combos.**

Mahjuro takes the tile-matching beauty of mahjong and reimagines it as a roguelite deckbuilder inspired by Balatro. No mahjong experience required -- the game teaches you as you play, and every run is a new puzzle of pattern recognition, risk management, and combo stacking.

<!-- If you have screenshots, uncomment and add them here:
![Gameplay Screenshot](screenshots/gameplay.png)
-->

## How It Works

Each run is a series of **antes**, and each ante has three **blinds** you need to beat by hitting a target score. You draw tiles, arrange them into scoring patterns called **melds**, and try to rack up enough **chips x multiplier** to clear the target before you run out of plays.

Between rounds, visit the **shop** to buy relics, zodiac cards, and talismans that bend the rules in your favor. The deeper you go, the higher the stakes -- and the wilder the combos.

### Melds

| Pattern       | What It Is                        | Base Chips |
| ------------- | --------------------------------- | ---------- |
| **Pair**      | 2 identical tiles                 | 18         |
| **Triplet**   | 3 identical tiles                 | 50         |
| **Sequence**  | 3 consecutive numbers in one suit | 28         |
| **Kong**      | 4 identical tiles                 | 80         |
| **Full Hand** | 4 melds + 1 pair (14 tiles)       | Huge bonus |

### Yaku (Hand Bonuses)

Land specific tile patterns to trigger **yaku** -- named bonuses that multiply your score. Start with the basics and unlock more as you progress:

- **Tanyao** -- All middle tiles (2-8), no terminals or honors
- **Toitoi** -- All triplets, no sequences
- **Honitsu** -- One suit plus honors only
- **Chinitsu** -- Pure single suit, no honors
- **Sanshoku Doujun** -- Same sequence in all three suits
- ...and many more to discover

### Relics

Persistent upgrades that stack and interact in surprising ways. A few favorites:

- **Triplet Boost** -- Triplets get x2 multiplier
- **Kan Drum** -- Kongs grant +1 play and +4 mult
- **Snowball** -- Cleared blinds stack investment; each stack adds chips scaled to the current blind's target (see in-run tooltip)
- **Dora Crown** -- Extra dora indicator with +35 bonus chips

There are 50+ relics to find across your runs.

### Consumables

Carry up to 2 items at a time (expandable with relics):

- **Zodiac Cards** -- Level up your yaku patterns permanently for the run
- **Talismans** -- Enhance tiles with jade, pearl, gilded, or polychrome finishes for bonus chips and mult

### Boss Blinds

Every third blind is a **boss** with a special rule that forces you to adapt:

- **The Hermit** -- Pairs score zero
- **The Forest** -- Sequences are halved
- **The Bureaucrat** -- You must play exactly 5 tiles
- **The Dragon** -- Structure must contain honors

## Getting Started

### Requirements

- [Rust](https://rustup.rs/) (2024 edition)
- [Python](https://www.python.org/) 3 on `PATH` (`python3` or `python`) — `build.rs` runs `tools/bake_assets/bake_assets.py` so packs sit next to the binary (set `MAHJURO_SKIP_ASSET_BAKE=1` only if you supply packs or `MAHJURO_ASSETS` yourself)
- A GPU with Vulkan, Metal, or DX12 support

### Build & Run

```bash
# Clone the repo
git clone https://github.com/Velfi/Mahjuro.git
cd mahjuro

# Build and run (release mode recommended; first build bakes ZIP packs via Python)
cargo run --release
```

The game includes a **tutorial** that walks you through the basics -- just start a new run and follow the prompts.

### Gamepad Support

Mahjuro supports gamepads out of the box via the gilrs library.

## Progression

Your progress persists across runs. As you complete more runs, you unlock:

- New **yaku** patterns to score with
- New **relics** in the shop pool
- **Tile materials** with gameplay perks (Bamboo: +1 play, Plastic: +1 discard)
- **Dora indicators** for bonus scoring
- Harder **rule modifiers** and boss encounters
- 7 difficulty tiers total

## For Developers

### Bot Mode

Run a headless AI player to test game balance:

```bash
# Run 200 bot games
cargo run --release -- --bot 200

# Parameter sweep for tuning
cargo run --release -- --sweep --runs 30

# Custom parameters
cargo run --release -- --bot 200 --base-target 250 --target-scale 1.3 --plays 5

# Verbose trace for a single run
cargo run --release -- --bot 1 --bot-log
```

Always use `--release` for bot mode -- debug builds are 10-20x slower.

### Project Structure

```
src/
  core/       Game logic: tiles, hands, scoring, yaku, relics, progression
  game/       State machine, event bus, game modes, cascade animations
  render/     WGPU rendering, animation controller, draw commands
  ui/         Constraint-based layout (Cassowary), widget tree, input
  scenes/     Scene system: start screen, gameplay, shop, game over
  audio/      Sound effects and music
  persistence/  Save/load

assets/       Tile models, textures, fonts, audio, `scenes/` per-scene art, music
```

### Documentation

| Document          | Description                             |
| ----------------- | --------------------------------------- |
| `GAME_DESIGN.md`        | Full feature spec and design philosophy |
| `ARCHITECTURE.md`       | System design and module structure      |
| `PROGRESSION.md`        | Unlock trees and meta-progression       |
| `BOT.md`                | Headless AI player guide and tuning     |
| `docs/steam-uploads.md` | Publishing builds to Steam              |

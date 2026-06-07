# Multi-store distribution

Mahjuro ships three consumer SKUs from one codebase via mutually exclusive Cargo features.

| SKU | Feature flags | Achievements |
|-----|---------------|--------------|
| Steam (default) | `game`, `dist-steam` | Steamworks (app 4636490) |
| Mac App Store | `game`, `dist-mas` | Game Center |
| Microsoft Store | `game`, `dist-msstore` | Xbox Live (GDK shim) |

Enable **exactly one** `dist-*` feature. The `mahjuro-distribution` crate emits a `compile_error!` if more than one is set.

## Build commands

```bash
# Steam (default)
cargo build --release

# Mac App Store sandbox build
cargo build --release --no-default-features --features game,dist-mas
scripts/package-macos-store.sh [--universal] [--sign]

# Microsoft Store
cargo build --release --no-default-features --features game,dist-msstore --target x86_64-pc-windows-msvc
pwsh scripts/package-windows-store.ps1 -Configuration Release
```

Dev shortcut to skip platform sign-in on any SKU:

```bash
cargo run -- --no-platform-services
# Steam alias (hidden): --no-steam
```

## Architecture

- **`crates/mahjuro-distribution`** — `DistributionBackend` trait, `Achievement` ID table, `PlatformPaths` / `PlatformShell`.
- **Game code** — `App.dist: DistributionClient`; achievement triggers unchanged.
- **Store I/O** — saves/mods/crash logs under `PlatformPaths::data_root()`; exports use native save panels on store SKUs.
- **`MAHJURO_ASSETS`** — disabled when `mahjuro-assets/store-bundle-only` is enabled (`dist-mas`, `dist-msstore`).

## Achievement IDs (all backends)

Configure matching records in each partner portal. IDs are identical across Steam, Game Center, and Xbox (see `mahjuro_distribution::Achievement` and CI test `all_achievements_have_three_backend_ids`).

| ID | Trigger (summary) |
|----|-------------------|
| `TUTORIAL_COMPLETE` | Tutorial finished or skipped |
| `FIRST_STRUCTURE` | First structure scored |
| `FIRST_BLIND_CLEARED` | First chamber cleared post-tutorial |
| `FIRST_BOSS_DEFEATED` | First ordeal defeated |
| `FIRST_RUN_COMPLETED` | First full run victory |
| `TEN_RUNS_PLAYED` | Ten runs completed |
| `STAKE_2_UNLOCKED` | Summer unlocked on a material |
| `STAKE_3_UNLOCKED` | Autumn unlocked |
| `STAKE_4_UNLOCKED` | Winter unlocked |
| `ALL_BOSSES_SEEN` | Every non-final ordeal played once |
| `SILK_MOTH_EMERGED` | Silk Thread → Silk Moth |
| `TAOTIE_AWAKENED` | Melting Ice → Taotie |
| `GEESE_TAKE_FLIGHT` | XXXL Egg → Geese |
| `THIRTEEN_ORPHANS` | Kokushi Musō scored |
| `HOUSE_DEFEATED` | The House defeated |

### Profile stats (Steam / Xbox stub)

| Stat ID | Source |
|---------|--------|
| `RUNS_FINISHED` | `PlayerProgress::runs_completed` |
| `RUNS_WON` | Non-tutorial victories in `run_history` |
| `BEST_ENDING_ROUND_SCORE` | Top `high_scores` entry |

Game Center v1: achievements wired; leaderboard stat sync is logged pending App Store Connect leaderboard IDs.

## macOS App Store

See **[macos-app-store.md](macos-app-store.md)** for the full Connect / Transporter checklist.

- **Bundle ID:** `com.zelda-built-this.Mahjuro.store` (override with `MAS_BUNDLE_ID`)
- **Entitlements:** `Entitlements.mas.plist` (sandbox, network client, JIT, Game Center, user-selected export)
- **Signing:** `APPLE_MAS_APP_SIGNING_IDENTITY` + `APPLE_MAS_INSTALLER_SIGNING_IDENTITY`
- **Scripts:** `scripts/package-macos-store.sh`, `scripts/validate-macos-store.sh`
- **CI:** `.github/workflows/release-macos-store.yml`

## Microsoft Store

See **[windows-app-store.md](windows-app-store.md)** for the full Partner Center checklist.

- **Manifest:** `packaging/msix/AppxManifest.xml` — update `Publisher` CN before submission
- **Xbox shim:** `crates/mahjuro-distribution/cpp/xbox_shim/` — replace stubs with GDK `XGameRuntime` calls
- **Scripts:** `scripts/package-windows-store.ps1`, `scripts/validate-windows-store.ps1`
- **CI:** `.github/workflows/build-windows-store.yml`, `.github/workflows/release-windows-store.yml`

## Per-SKU QA checklist

### Steam

- [ ] Achievements unlock and appear in Steam overlay
- [ ] Profile stats sync after run end / profile switch
- [ ] `MAHJURO_ASSETS` loose tree works in dev
- [ ] Tileset mod folder opens via Options
- [ ] Play-stats export lands in Downloads

### Mac App Store

- [ ] Game Center sign-in on first launch
- [ ] All 15 achievements report in sandbox Apple ID
- [ ] Saves/mods stay in container (`~/Library/Containers/...` or Application Support)
- [ ] Export uses NSSavePanel; no broad filesystem access
- [ ] Crash log in container; no AppleScript dialog
- [ ] `MAHJURO_ASSETS` ignored; bundle packs only

### Microsoft Store

- [ ] MSIX installs and launches from Partner Center sandbox
- [ ] Xbox sign-in (after GDK shim wired)
- [ ] Achievements unlock via shim
- [ ] Saves in package `LOCALAPPDATA`
- [ ] Export uses IFileSaveDialog
- [ ] Mod folder reveal via Shell API (no raw `explorer` spawn on store build)

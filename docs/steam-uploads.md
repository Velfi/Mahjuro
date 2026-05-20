# Steam build uploads

Mahjuro publishes Windows and macOS builds to Steam (AppID `4636490`)
using `steamcmd` and the VDF scripts in [packaging/steam/](../packaging/steam/).
The driver is [scripts/steam-upload.sh](../scripts/steam-upload.sh).

## One-time setup

### 1. Vendor the Steamworks SDK

The SDK is large and Valve restricts redistribution, so it isn't checked in.
Download it from the partner site and unpack it under the repo root:

```
~/Documents/Mahjuro/steam_sdk/
├── tools/ContentBuilder/
├── public/
└── ...
```

Or set `STEAM_SDK_ROOT` to wherever you actually keep it.

### 2. Create / verify depots in the partner UI

Open [partner.steamgames.com/apps/depots/4636490](https://partner.steamgames.com/apps/depots/4636490)
and confirm three depots exist. The defaults the script assumes:

| Platform | Depot ID  | Env override            |
| -------- | --------- | ----------------------- |
| Windows  | `4636491` | `STEAM_DEPOT_WINDOWS`   |
| macOS    | `4636492` | `STEAM_DEPOT_MACOS`     |
| Linux    | `4636493` | `STEAM_DEPOT_LINUX`     | *(not uploaded by `steam-upload.sh`; depot exists for a future Linux Steam build)* |

If Valve assigned different IDs, export the matching `STEAM_DEPOT_*` env var.

### 3. Build account + Steam Guard

Create (or use) a dedicated Steam account with **Edit App Metadata** and
**Publish Builds to Steam** permissions on Mahjuro. Don't use your personal
account — CI will eventually share these credentials.

Bootstrap Steam Guard once on the machine that will upload:

```sh
cd "$STEAM_SDK_ROOT/tools/ContentBuilder/builder_osx"   # or builder_linux
./steamcmd.sh +login <build_account>
# → enter password
# → enter Steam Guard code from email/Authenticator
# → wait for "Logged in OK"
quit
```

The sentry / login token is now cached under
`~/Library/Application Support/Steam/` (macOS) or `~/Steam/` (Linux).
Future runs of `steam-upload.sh` will skip the 2FA prompt.

## Uploading a build

Always preview first when you've changed a VDF or the staging logic:

```sh
export STEAM_BUILD_USER=mahjuro_ci
scripts/steam-upload.sh --preview 0.4.2
```

A preview validates the depot layout and writes logs to
`build-staging/output/` without uploading.

For a real upload from the GitHub release artifacts:

```sh
export STEAM_BUILD_USER=mahjuro_ci
scripts/steam-upload.sh --branch internal 0.4.2
```

This downloads `mahjuro-v0.4.2-{windows,macos}-*` from the GitHub release,
stages them under `build-staging/content/<platform>/`, renders the
VDFs, and runs `steamcmd +run_app_build`. The `--branch internal` flag sets
the build live on the `internal` beta branch — leave it off to upload without
promoting (then promote manually in the partner UI).

For a Steam branch named `beta`, use `--beta` (same as
`--branch beta`). To default to a different partner branch name, set
`STEAM_BETA_BRANCH` (e.g. `STEAM_BETA_BRANCH=publicbeta scripts/steam-upload.sh --beta 0.4.2`).
Create the branch under **App Admin → SteamPipe → Branches** if it does not
exist yet.

To upload from a local build instead (host platform only — useful for smoke
tests), pass `--local`:

```sh
scripts/package-macos.sh         # produces Mahjuro.app at the repo root
scripts/steam-upload.sh --local --preview 0.4.2
```

## Promoting a build

Builds default to "uploaded but not live". Promote them via
[partner.steamgames.com/apps/builds/4636490](https://partner.steamgames.com/apps/builds/4636490):
pick the build, set the branch (`default` for production), and save.

## Controller input

Mahjuro uses **SDL3** for gamepads. Steam Input in-game actions and bundled
`game_actions_*.vdf` manifests are not used. Players remap hardware through
their OS, Steam’s desktop/Big Picture controller settings, or device apps; the
in-game Options menu can swap face-button semantics (South/East, West/North).

## Troubleshooting

- **`Login Failure: Invalid Login Auth Code`** — Steam Guard token expired or
  was rotated. Re-run the bootstrap (`+login <account>`, enter the new code).
- **`ERROR! Failed to get application info`** — the build account is missing
  Publish-Builds permission for AppID `4636490`. Fix in partner UI.
- **`ERROR! Depot N not found in app M`** — depot ID mismatch. Verify in the
  partner UI and override `STEAM_DEPOT_*` env vars if needed.
- **`hdiutil: attach failed`** when staging from release on macOS — the DMG
  is busy or already mounted. `hdiutil detach /Volumes/Mahjuro` and retry.
- **`gh: command not found`** — install the GitHub CLI (`brew install gh`)
  and `gh auth login`. Required for the default (release-pull) staging mode.

## Files

- [packaging/steam/app_build.vdf.template](../packaging/steam/app_build.vdf.template) — top-level build script
- [packaging/steam/depot_build_*.vdf.template](../packaging/steam/) — per-depot file mappings
- [scripts/steam-upload.sh](../scripts/steam-upload.sh) — driver
- `build-staging/` — generated; gitignored

---

# Steamworks runtime integration

The shipped binary links and dlopens the Steamworks SDK at runtime to
provide achievements, the Steam overlay, and Steam Cloud saves.

## Build-time requirement

The build script ([build.rs](../build.rs)) copies the Steamworks
redistributable (`libsteam_api.dylib`, `libsteam_api.so`, or
`steam_api64.dll`) into the cargo **profile** directory next to `mahjuro`
so packaged builds and `cargo run` pick it up without extra env vars. It
uses `$STEAM_SDK_LOCATION/redistributable_bin/<arch>/` when set; otherwise
it reuses the copy `steamworks-sys` extracted under
`target/.../build/steamworks-sys-*/out/` (crates.io `steamworks-sys`
vendors the SDK headers and redistributables under `lib/steam/`).

On Linux the binary is linked with `-Wl,-rpath,$ORIGIN` so the loader
finds `libsteam_api.so` in the same directory as the executable (e.g. a
future Steam Linux depot layout).

The macOS packaging script ([scripts/package-macos.sh](../scripts/package-macos.sh))
also copies the dylib into `Mahjuro.app/Contents/MacOS/` next to the
binary — required because Steam's dylib has install_name
`@loader_path/libsteam_api.dylib`.

## Achievements

Achievement IDs live in [src/steam/achievement.rs](../src/steam/achievement.rs).
Each variant maps to an "API Name" string that must match an achievement
configured at
[partner.steamgames.com/apps/achievements/4636490](https://partner.steamgames.com/apps/achievements/4636490).

Current set (designed as a funnel — completion rates double as a
retention dashboard). Enter **API Name**, **Display Name**, and
**Description** in Steamworks; icons are separate (locked / unlocked).

| API Name | Display name | Description | Game trigger (not shown in Steam) |
| -------- | ------------ | ----------- | ----------------------------------- |
| `TUTORIAL_COMPLETE` | Tutorial Graduate | Complete the tutorial. | Tutorial finished |
| `FIRST_STRUCTURE` | First Structure | Score your first structure. | First scoring structure resolved |
| `FIRST_BLIND_CLEARED` | First Blind | Clear your first blind. | First non-tutorial round cleared |
| `FIRST_BOSS_DEFEATED` | Boss Down | Defeat your first boss blind. | First boss blind beaten |
| `FIRST_RUN_COMPLETED` | Run Won | Win a full run. | First full-run victory |
| `TEN_RUNS_PLAYED` | Dedicated | Complete 10 runs. | `runs_completed` reaches 10 |
| `STAKE_2_UNLOCKED` | Summer Unlocked | Unlock the Summer stake. | Summer stake unlocked |
| `ALL_BOSSES_SEEN` | Full Roster | Encounter every non-final boss blind at least once. | All non-final bosses seen |
| `SILK_MOTH_EMERGED` | Silk Moth | Carry Silk Thread through to Silk Moth. | Silk Thread → Silk Moth |
| `TAOTIE_AWAKENED` | Taotie | Carry Melting Ice through to Taotie. | Melting Ice → Taotie |
| `GEESE_TAKE_FLIGHT` | Geese | Carry XXXL Egg through to Geese. | XXXL Egg → Geese |
| `THIRTEEN_ORPHANS` | Thirteen Orphans | Score Kokushi Musō (thirteen orphans). | Kokushi Musō scored |
| `HOUSE_DEFEATED` | Beat the House | Defeat The House on the final boss blind. | Final boss **The House** cleared |

When adding an achievement, add the variant + API Name in code, configure
the matching achievement in the partner backend, and wire the trigger
where the underlying state change happens (search for `unlock_achievement`
to see the pattern).

Local testing: launch through Steam (so the SDK can attach the overlay),
play through the trigger condition, and confirm the toast pops in the
corner. To reset progress for re-testing, delete the achievement on the
partner site under "Reset Progress" or call
`steamworks::Client::user_stats().reset_all_stats(true)` from a debug
build.

The `--no-steam` flag on the binary disables Steam init entirely — useful
when iterating on UI without Steam claiming the foreground process slot.

## Steam Cloud (Auto-Cloud)

Saves stay on disk in the user **config** directory. Cloud sync is **Steam
Auto-Cloud** (no `ISteamRemoteStorage` calls in game code). Configure on
[Steam Cloud Settings](https://partner.steamgames.com/apps/cloud/4636490).

### Initial setup (top of the Cloud page)

- **Byte quota per user** — ~50 MB is plenty for small JSON saves.
- **Number of files allowed per user** — raise above the default if needed
  (several profiles and run snapshots multiply files).
- **Save** and **Publish**. For a shipped title you can use **Enable cloud
  support for developers only** while testing.

See [Steam Cloud — Initial Setup](https://partner.steamgames.com/doc/features/cloud#initial_setup).

### Cross-Platform Configuration (partner UI)

The page calls out how **Auto-Cloud** behaves across OSes:

- The only **Root** that is valid on every platform **without** extra wiring
  is **App Install Directory**. Mahjuro does **not** write saves there
  ([`dirs::config_dir().join("Mahjuro")`](../src/persistence.rs)).
- If you add **separate ROOT PATH rows** with a different **Root** per OS,
  saves are **partitioned by platform** (no shared cloud between Windows /
  macOS / Linux).
- To **share one cloud namespace** across OSes while using different folders
  on disk, use **Root Overrides** (same idea as the Unity
  `Application.persistentDataPath` example in the Steam docs): one primary
  root plus overrides per OS. If you use overrides, the primary row’s **OS**
  must be **[All OSes]** per Valve. Details: [Steam Cloud — Cross-Platform Saves](https://partner.steamgames.com/doc/features/cloud#crossplatform_saves)
  and **Root Overrides** on the same page.

### ROOT PATHS — fields per row

Each Auto-Cloud rule is one row. The Steamworks doc describes a root path as
five editable parts plus the UI preview:

| Field | Role |
| ----- | ---- |
| **Root** | Dropdown base directory (e.g. **App Install Directory**, **WinAppDataRoaming**, **MacAppSupport**, **LinuxHome**, …). Full list: [Steam Auto-Cloud — Root](https://partner.steamgames.com/doc/features/cloud#root). |
| **Subdirectory** | Path under that root (use `.` if files sit directly in **Root**). May include `{64BitSteamID}` or `{Steam3AccountID}`. |
| **Pattern** | Glob for files to sync; Mahjuro uses `*.json`. |
| **OS** | **Windows**, **macOS**, **Linux / SteamOS**, or **[All OSes]** (required on the primary path when using **Root Overrides** — see Steam docs). |
| **Recursive** | Include subfolders when matching **Pattern**. Mahjuro’s save directory is flat: leave **off**. |
| **Cross-Platform** | Read-only column in the partner UI (whether that path is treated as cross-platform). |
| **Preview** | Resolved example path — confirm it matches the table below before publishing. |

The client may create **`steam_autocloud.vdf`** in watched locations; ignore
it in game code.

### Root Overrides (cross-platform saves)

Valve describes each override’s fields under [Root Overrides](https://partner.steamgames.com/doc/features/cloud#root_overrides):
**Original Root**, **OS**, **New Root**, **Add/Replace Path**, **Replace Path**.
The Unity example uses **Add/Replace Path** + **Replace Path** when the folder
layout under the root differs per OS. Mahjuro uses the same **Subdirectory**
(`Mahjuro`) on every platform, so you usually only need to pick **New Root**
per override and leave **Add/Replace Path** empty and **Replace Path** off
— then the original **Subdirectory** still applies under the new root. If
**Preview** looks wrong, adjust using Valve’s **Add/Replace Path** /
**Replace Path** semantics (same doc section).

**Steps (partner site):**

1. **Remove** the three separate **ROOT PATHS** rows (Windows-only,
   macOS-only, Linux-only). Each of those owns a different cloud partition.
2. Add **one** root path row (this becomes the path Steam treats as the
   single cross-platform rule):
   - **Root:** `WinAppDataRoaming`
   - **Subdirectory:** `Mahjuro`
   - **Pattern:** `*.json`
   - **OS:** **[All OSes]** (required when using overrides — see Steam doc)
   - **Recursive:** off
3. In the **Root Overrides** section, add one override **per non-Windows OS**
   (Original Root = the root path from step 2; exact control labels match the
   live UI):
   - **macOS** — **New Root:** `MacAppSupport` (expect
     `~/Library/Application Support/Mahjuro` in **Preview**).
   - **Linux + SteamOS** — **New Root:** `LinuxXdgConfigHome` (expect
     `$XDG_CONFIG_HOME/Mahjuro`, falling back to `~/.config/Mahjuro` when the
     variable is unset — confirm against a real machine if needed).
4. **Save**, **Publish**, then verify with Steam’s `testappcloudpaths` flow
   ([Pre-release Testing](https://partner.steamgames.com/doc/features/cloud#pre-release_testing))
   on each OS you ship.

### Folders to match ([`src/persistence.rs`](../src/persistence.rs))

These are the directories Rust resolves via `dirs::config_dir().join("Mahjuro")`
(Steam **Root** names are from the partner dropdown):

| OS | Typical resolved path | Steam **Root** + **Subdirectory** |
| -- | -------------------- | ----------------------------------- |
| Windows | `%APPDATA%\Mahjuro\` (Roaming) | **WinAppDataRoaming** + `Mahjuro` |
| macOS | `~/Library/Application Support/Mahjuro/` | **MacAppSupport** + `Mahjuro` |
| Linux / SteamOS | `~/.config/Mahjuro/` (or `$XDG_CONFIG_HOME/Mahjuro/`) | **LinuxXdgConfigHome** + `Mahjuro` |

**Per-OS rows** (each with **OS** = Windows / macOS / Linux only) are fine for
**cloud backup on that OS**; they do **not** share one save across platforms.
For **one save everywhere**, use **one** row with **OS** **[All OSes]** plus
**Root Overrides** as above.

Synced files include `settings.json`, `profile_*.json`, `run_*.json`,
`tuning_overrides.json`, and any other `*.json` in that directory.

No game code change is required for Auto-Cloud — the game keeps writing to
the local config dir and Steam syncs around launch/exit.

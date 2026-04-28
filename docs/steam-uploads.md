# Steam build uploads

Mahjuro publishes Windows / macOS / Linux builds to Steam (AppID `4636490`)
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
| Linux    | `4636493` | `STEAM_DEPOT_LINUX`     |

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

This downloads `mahjuro-v0.4.2-{windows,macos,linux}-*` from the GitHub
release, stages them under `build-staging/content/<platform>/`, renders the
VDFs, and runs `steamcmd +run_app_build`. The `--branch internal` flag sets
the build live on the `internal` beta branch — leave it off to upload without
promoting (then promote manually in the partner UI).

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

`STEAM_SDK_LOCATION` must point at an unpacked Steamworks SDK before
`cargo build`. The build script ([build.rs](../build.rs)) copies
`libsteam_api.dylib` (or `.so` / `steam_api64.dll` on other platforms)
from `$STEAM_SDK_LOCATION/redistributable_bin/<arch>/` into the cargo
target dir so `cargo run` works without `DYLD_LIBRARY_PATH` games. The
macOS packaging script ([scripts/package-macos.sh](../scripts/package-macos.sh))
copies the same dylib into `Mahjuro.app/Contents/MacOS/` next to the
binary — required because Steam's dylib has install_name
`@loader_path/libsteam_api.dylib`.

If `STEAM_SDK_LOCATION` is unset, `steamworks-sys` will fail with its
own clear error during link.

## Achievements

Achievement IDs live in [src/steam/achievement.rs](../src/steam/achievement.rs).
Each variant maps to an "API Name" string that must match an achievement
configured at
[partner.steamgames.com/apps/achievements/4636490](https://partner.steamgames.com/apps/achievements/4636490).

Current set (designed as a funnel — completion rates double as a
retention dashboard):

| API Name              | Trigger                                          |
| --------------------- | ------------------------------------------------ |
| `TUTORIAL_COMPLETE`   | Finished the tutorial                            |
| `FIRST_HAND`          | First scoring cascade resolved                   |
| `FIRST_BLIND_CLEARED` | First non-tutorial round cleared                 |
| `FIRST_BOSS_DEFEATED` | Beat first boss blind                            |
| `FIRST_RUN_COMPLETED` | Won a full run for the first time                |
| `TEN_RUNS_PLAYED`     | `runs_completed` reached 10                      |
| `STAKE_2_UNLOCKED`    | Unlocked Summer stake                            |
| `ALL_BOSSES_SEEN`     | Encountered every non-final boss at least once   |

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

Saves are local-only in code. Cloud sync is delegated to **Steam
Auto-Cloud** so we don't have to wrap the persistence layer with the
RemoteStorage API. Configure once in the partner backend:

1. Visit
   [partner.steamgames.com/apps/cloud/4636490](https://partner.steamgames.com/apps/cloud/4636490).
2. Set "Cloud Quota" to ~50 MB (saves are small JSON; this is plenty).
3. Add per-platform path roots:

   | Platform | Root                                  | Pattern  |
   | -------- | ------------------------------------- | -------- |
   | macOS    | `%MacAppSupport%/Mahjuro/`            | `*.json` |
   | Windows  | `%WinAppDataLocal%/Mahjuro/`          | `*.json` |
   | Linux    | `%XDGCONFIGHOME%/Mahjuro/`            | `*.json` |

   These match what [src/persistence.rs](../src/persistence.rs) writes to
   via `dirs::config_dir().join("Mahjuro")`.
4. Recursive: leave off (the directory is flat).
5. Save the config. Steam now syncs `settings.json`,
   `profile_*.json`, `run_*.json`, and `tuning_overrides.json` between
   the player's machines automatically when the game launches and quits.

There's no code change required for Auto-Cloud — the game continues to
write to the local config dir, and Steam handles the sync transparently.

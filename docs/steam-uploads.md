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

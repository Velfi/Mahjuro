# Mac App Store release

Step-by-step for shipping the `dist-mas` SKU to App Store Connect. Assumes you already have an app record and certificates partially configured.

Player-facing **Support URL** and **Privacy Policy** copy: [`docs/mas-support.md`](../mas-support.md).

## 1. Align App Store Connect with the repo

| Field | Repo default | Your Connect record |
|-------|--------------|---------------------|
| Bundle ID | `com.zelda-built-this.Mahjuro.store` | Must match exactly |
| Platform | macOS | — |
| Game Center | Enabled on App ID | Required for achievements |

If your Connect bundle ID differs, set `MAS_BUNDLE_ID` when packaging:

```bash
MAS_BUNDLE_ID=com.yourteam.Mahjuro scripts/package-macos-store.sh --sign
```

### Game Center achievements

Create **15 achievements** in App Store Connect → Services → Game Center. IDs must match the table in [distribution.md](distribution.md) (e.g. `TUTORIAL_COMPLETE`, `FIRST_STRUCTURE`, …). Hidden/shown and point values are up to you; IDs are the contract.

Optional later: three **leaderboards** with IDs `RUNS_FINISHED`, `RUNS_WON`, `BEST_ENDING_ROUND_SCORE`.

## 2. Developer portal certificates

You need **two** Mac identities (different from Developer ID / Steam):

1. **Apple Distribution** (or “3rd Party Mac Developer Application”) — signs `Mahjuro-Store.app`
2. **3rd Party Mac Developer Installer** — signs the `.pkg` for Transporter

Download both from [Certificates, Identifiers & Profiles](https://developer.apple.com/account/resources/certificates/list) and install in Keychain.

Enable on the App ID:

- App Sandbox
- Game Center
- (JIT is not a portal checkbox — declare in review notes; see below)

## 3. Build and package locally

```bash
# Unsigned smoke test
scripts/package-macos-store.sh --validate

# Signed .pkg for upload
export MAS_BUILD_NUMBER=1          # increment every upload
export MAS_PROVISIONING_PROFILE="$HOME/Downloads/mahjuroprod.provisionprofile"  # Mac App Store profile
export APPLE_MAS_APP_SIGNING_IDENTITY="Apple Distribution: Your Name (TEAMID)"
export APPLE_MAS_INSTALLER_SIGNING_IDENTITY="3rd Party Mac Developer Installer: Your Name (TEAMID)"
scripts/package-macos-store.sh --universal --sign --validate
```

Outputs:

- `Mahjuro-Store.app` — sandboxed, asset packs in `Contents/Resources/`
- `mahjuro-store-v<short>-b<build>-macos-<arch>.pkg` — upload this

### Version numbers

- **CFBundleShortVersionString** — from `workspace.package.version` before `-` (e.g. `0.6.0`)
- **CFBundleVersion** — suffix after `-` or `MAS_BUILD_NUMBER` (must be **numeric** and increase per build)

## 4. Upload

**Preflight** (checks Keychain for Mac App Store certs):

```bash
scripts/mas-preflight.sh
```

**One-shot** build + sign + upload (after certs are installed):

```bash
export APPLE_API_ISSUER="<uuid from App Store Connect → Users and Access → Integrations>"
export MAS_BUILD_NUMBER=1   # increment every upload
export MAS_PROVISIONING_PROFILE="$HOME/Downloads/mahjuroprod.provisionprofile"
scripts/upload-macos-store.sh --universal
```

Uses `~/.private_keys/AuthKey_33B59YFTBZ.p8` by default. Override with `APPLE_API_KEY` / `APPLE_API_KEY_PATH`.

Or use [Transporter](https://apps.apple.com/app/transporter/id1450874784) with the signed `.pkg` from `scripts/package-macos-store.sh --sign`.

Then in App Store Connect: attach the build to a version, complete metadata, submit for review.

## 5. App Review notes (recommended)

Paste something like:

> Mahjuro uses Metal via wgpu and requires JIT / unsigned executable memory entitlements (`com.apple.security.cs.allow-jit`, `com.apple.security.cs.allow-unsigned-executable-memory`) for shader pipeline creation. No arbitrary native code download. Game Center is used for achievements only. Play-stats export uses NSSavePanel (user-selected path). Saves and mods stay in the app sandbox container.

## 6. Local QA before submit

Run the signed app from `/Applications` (or `spctl --assess --type execute` after install):

- [ ] Launches without `MAHJURO_ASSETS` (bundle packs load from Resources)
- [ ] New profile → save → quit → relaunch (container persistence)
- [ ] Options → Export play stats → NSSavePanel → HTML writes
- [ ] Options → Open tileset mods → Finder reveals container path
- [ ] Signed into Game Center (System Settings) → trigger an achievement → banner / GC app shows progress
- [ ] Crash log under `~/Library/Containers/<bundle-id>/Data/.../Mahjuro/logs/`

Build feature set:

```bash
cargo build --release --no-default-features --features game,dist-mas
```

## 7. CI

`.github/workflows/release-macos-store.yml` builds an unsigned universal `.app`. Add signing secrets on a protected branch before using it for production uploads.

## Troubleshooting

| Symptom | Check |
|---------|--------|
| Certs in Keychain Access but preflight says MISSING | Run `scripts/install-apple-wwdr-intermediates.sh` (needs Apple WWDR **G3** intermediate) |
| Empty version in Connect | Re-run package script (fixed `mas-version.sh` reads workspace version) |
| Game Center never unlocks | User signed into GC in System Settings; `com.apple.security.network.client` in entitlements |
| “Invalid bundle” on upload | Installer cert on `.pkg`, app cert on `.app`, bundle ID match |
| `codesign: unrecognized option --provisioning-profile` | macOS has no such flag — set `MAS_PROVISIONING_PROFILE`; script copies it to `Contents/embedded.provisionprofile` |
| Entitlements vs profile mismatch (90287) | App ID has Game Center in Developer Portal; re-download profile; sign with `MAS_PROVISIONING_PROFILE` |
| Missing application identifier (90886) | `MAS_PROVISIONING_PROFILE` set — packaging merges `com.apple.application-identifier` + team ID from profile into signed entitlements |
| ITMS-91109 quarantine on `embedded.provisionprofile` | Profile copied from Downloads picks up `com.apple.quarantine`; packaging runs `xattr -cr` on the `.app` before signing |
| Achievement ID mismatch | Connect ID must equal `Achievement::game_center_id()` in code |
| Assets not found at launch | `pack_manifest.json` present in `Contents/Resources/` |

# Microsoft Store release

Step-by-step for shipping the `dist-msstore` SKU to Partner Center. Assumes you have a reserved app name and a Partner Center developer account.

## 1. Align Partner Center with the repo

| Field | Repo default | Partner Center |
|-------|--------------|----------------|
| Package identity name | `Mahjuro.Mahjuro` | Must match **Package/Identity/Name** |
| Publisher | `CN=REPLACE_WITH_PARTNER_CENTER_PUBLISHER` | Must match **Package/Identity/Publisher** exactly |
| Capabilities | `internetClient`, `runFullTrust` | Full-trust desktop app |

Find your publisher CN under **Partner Center → Account settings → Developer settings → Publisher ID**. It looks like `CN=XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX`.

Set it when packaging:

```powershell
$env:MSSTORE_PUBLISHER = 'CN=YOUR-PARTNER-CENTER-PUBLISHER-GUID'
$env:MSSTORE_BUILD_NUMBER = '1'   # increment every upload
.\scripts\package-windows-store.ps1 -Sign -Validate
```

Or edit `packaging/msix/AppxManifest.xml` once and commit the real `Publisher` value.

### Xbox achievements

Create **15 achievements** in Partner Center with IDs matching [distribution.md](distribution.md) (e.g. `TUTORIAL_COMPLETE`, `FIRST_STRUCTURE`, …). The Xbox Live shim is stubbed until GDK is linked; achievements log locally but do not sync yet.

## 2. Signing certificate

Partner Center requires a signed MSIX.

1. **Partner Center → Account settings → Developer settings → Manage certificates**
2. Create a certificate (or use an existing `.pfx`)
3. Download the `.pfx` and note the password

```powershell
$env:MSSTORE_SIGNING_PFX = 'C:\path\to\partner-center-cert.pfx'
$env:MSSTORE_SIGNING_PASSWORD = 'your-cert-password'
$env:MSSTORE_BUILD_NUMBER = '1'
.\scripts\package-windows-store.ps1 -Sign -Validate
```

## 3. Build locally (Windows)

Requires Windows 10/11, Rust MSVC target, Windows SDK, Python, and asset bake tools (`ffmpeg`, `pngquant`, `oxipng` — same as Steam release).

```powershell
cargo build --release --no-default-features --features game,dist-msstore --target x86_64-pc-windows-msvc
.\scripts\package-windows-store.ps1 -Configuration Release -Sign -Validate
```

Outputs:

- `mahjuro-store-v<short>-b<build>-windows-x86_64.msix` — upload this
- `Mahjuro-Store.msix` — stable alias for CI

### Version numbers

- **First three quads** — from `workspace.package.version` before `-` (e.g. `0.6.0`)
- **Fourth quad** — suffix after `-` or `MSSTORE_BUILD_NUMBER` (must **increase** per upload)

Example: version `0.6.0-2` → package version `0.6.0.2`.

## 4. Upload

In **Partner Center → Apps and games → Mahjuro → Packages**, drag the signed `.msix` into the upload area (`.msix`, `.msixbundle`, `.msixupload`, etc.).

After processing, assign the package to a submission and push to your sandbox account for QA.

## 5. Build from macOS via GitHub Actions

Packaging requires Windows. Trigger **Build Windows Store MSIX** (`.github/workflows/build-windows-store.yml`) from the Actions tab, then download the `mahjuro-windows-store` artifact.

For a signed upload artifact, add repository secrets:

| Secret | Purpose |
|--------|---------|
| `MSSTORE_PUBLISHER` | Partner Center publisher CN |
| `MSSTORE_SIGNING_PFX_BASE64` | Base64-encoded `.pfx` |
| `MSSTORE_SIGNING_PASSWORD` | Certificate password |
| `MSSTORE_BUILD_NUMBER` | Optional build quad override |

## 6. Local QA before submit

Install from Partner Center sandbox (not sideload of unsigned packages):

- [ ] Launches without `MAHJURO_ASSETS` (bundle packs load from install dir)
- [ ] New profile → save → quit → relaunch (`LOCALAPPDATA` persistence)
- [ ] Options → Export play stats → save dialog → HTML writes
- [ ] Options → Open tileset mods → Explorer selects container path
- [ ] Crash log under package-local app data

Build feature set:

```powershell
cargo build --release --no-default-features --features game,dist-msstore
```

## 7. CI

- `.github/workflows/build-windows-store.yml` — manual MSIX build + artifact
- `.github/workflows/release-windows-store.yml` — called from release automation
- `ci.yml` `store-features-compile` — compile check for `dist-msstore` on `windows-2022`

## Troubleshooting

| Symptom | Check |
|---------|--------|
| `Publisher` rejected on upload | CN must match Partner Center exactly; use `$env:MSSTORE_PUBLISHER` |
| Package identity mismatch | `MSSTORE_PACKAGE_NAME` or manifest `Name` must match reserved name |
| Version already exists | Increment `MSSTORE_BUILD_NUMBER` (4th quad) |
| Unsigned package rejected | Run with `-Sign` and valid Partner Center `.pfx` |
| Assets not found at launch | `pack_manifest.json` and four `.zip` packs next to `mahjuro.exe` |
| `MakeAppx.exe not found` | Install Windows 10/11 SDK |
| Achievement ID mismatch | Partner Center ID must equal `Achievement::xbox_achievement_id()` in code |

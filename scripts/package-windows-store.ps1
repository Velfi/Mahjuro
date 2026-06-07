# Build Mahjuro MSIX for Microsoft Store (dist-msstore).
#
# Usage:
#   .\scripts\package-windows-store.ps1 [-Configuration Release] [-Sign] [-Validate]
#
# Prerequisites:
#   - Windows SDK (MakeAppx.exe, SignTool.exe)
#   - Python + asset bake tools (ffmpeg, pngquant, oxipng) for pack generation
#   - Partner Center publisher identity in packaging/msix/AppxManifest.xml
#     (or set MSSTORE_PUBLISHER to override at pack time)
#
# Signing (optional -Sign):
#   MSSTORE_SIGNING_PFX       path to Partner Center .pfx
#   MSSTORE_SIGNING_PASSWORD  certificate password
#
# Optional:
#   MSSTORE_BUILD_NUMBER      4th version quad (must increase per upload)
#   MSSTORE_PACKAGE_NAME      Identity Name (default from manifest template)
#   MSSTORE_PUBLISHER         Identity Publisher CN
#
# Output:
#   mahjuro-store-v<short>-b<build>-windows-x86_64.msix

param(
    [ValidateSet("Debug", "Release")]
    [string]$Configuration = "Release",
    [switch]$Sign,
    [switch]$Validate
)

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location $RepoRoot

. (Join-Path $RepoRoot "scripts\msstore-version.ps1")
Resolve-MsStoreVersions

$PackageName = if ($env:MSSTORE_PACKAGE_NAME) { $env:MSSTORE_PACKAGE_NAME } else { "Mahjuro.Mahjuro" }
$Publisher = if ($env:MSSTORE_PUBLISHER) { $env:MSSTORE_PUBLISHER } else { $null }

Write-Host "MS Store versions: short=$MSSTORE_SHORT_VERSION build=$MSSTORE_BUILD_NUMBER quad=$MSSTORE_VERSION_QUAD"

Write-Host "Building mahjuro ($Configuration, dist-msstore)..."
cargo build --$Configuration.ToLower() --no-default-features --features "game,dist-msstore" --target x86_64-pc-windows-msvc

$Stage = Join-Path $RepoRoot "target\msix-stage"
$Layout = Join-Path $Stage "layout"
if (Test-Path $Stage) { Remove-Item -Recurse -Force $Stage }
New-Item -ItemType Directory -Path $Layout -Force | Out-Null
New-Item -ItemType Directory -Path (Join-Path $Layout "Assets") -Force | Out-Null

$Bin = Join-Path $RepoRoot "target\x86_64-pc-windows-msvc\$($Configuration.ToLower())\mahjuro.exe"
if (-not (Test-Path $Bin)) {
    throw "Binary not found: $Bin"
}
Copy-Item $Bin $Layout

$BakeOut = Join-Path $RepoRoot "target\mahjuro-bake-packs"
if (Test-Path $BakeOut) { Remove-Item -Recurse -Force $BakeOut }
python (Join-Path $RepoRoot "tools\bake_assets\bake_assets.py") --out $BakeOut
Copy-Item (Join-Path $BakeOut "pack_manifest.json") $Layout
Copy-Item (Join-Path $BakeOut "mahjuro-pack-shared.zip") $Layout
Copy-Item (Join-Path $BakeOut "mahjuro-pack-rooms.zip") $Layout
Copy-Item (Join-Path $BakeOut "mahjuro-pack-gameplay-bulk.zip") $Layout
Copy-Item (Join-Path $BakeOut "mahjuro-pack-music.zip") $Layout

$LogoSrc = Join-Path $RepoRoot "icon.png"
$AssetsDir = Join-Path $Layout "Assets"
if (Get-Command magick -ErrorAction SilentlyContinue) {
    magick $LogoSrc -resize 50x50 (Join-Path $AssetsDir "StoreLogo.png")
    magick $LogoSrc -resize 150x150 (Join-Path $AssetsDir "Square150x150Logo.png")
    magick $LogoSrc -resize 44x44 (Join-Path $AssetsDir "Square44x44Logo.png")
} else {
    Copy-Item $LogoSrc (Join-Path $AssetsDir "StoreLogo.png")
    Copy-Item $LogoSrc (Join-Path $AssetsDir "Square150x150Logo.png")
    Copy-Item $LogoSrc (Join-Path $AssetsDir "Square44x44Logo.png")
}

$ManifestTemplate = Join-Path $RepoRoot "packaging\msix\AppxManifest.xml"
$Manifest = Get-Content $ManifestTemplate -Raw
$Manifest = $Manifest -replace '__VERSION_QUAD__', $MSSTORE_VERSION_QUAD
$Manifest = $Manifest -replace 'Name="Mahjuro\.Mahjuro"', "Name=`"$PackageName`""
if ($Publisher) {
    $Manifest = $Manifest -replace 'Publisher="CN=REPLACE_WITH_PARTNER_CENTER_PUBLISHER"', "Publisher=`"$Publisher`""
}
Set-Content -Path (Join-Path $Layout "AppxManifest.xml") -Value $Manifest -Encoding UTF8

$MakeAppx = "${env:ProgramFiles(x86)}\Windows Kits\10\bin\10.0.22621.0\x64\MakeAppx.exe"
if (-not (Test-Path $MakeAppx)) {
    $MakeAppx = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin" -Recurse -Filter MakeAppx.exe -ErrorAction SilentlyContinue |
        Select-Object -First 1 -ExpandProperty FullName
}
if (-not $MakeAppx) {
    throw "MakeAppx.exe not found. Install the Windows 10/11 SDK."
}

$MsixOut = Join-Path $RepoRoot "mahjuro-store-v${MSSTORE_SHORT_VERSION}-b${MSSTORE_BUILD_NUMBER}-windows-x86_64.msix"
& $MakeAppx pack /d $Layout /p $MsixOut /o
Write-Host "Wrote $MsixOut (version $MSSTORE_VERSION_QUAD)"

if ($Sign) {
    $Pfx = $env:MSSTORE_SIGNING_PFX
    $Password = $env:MSSTORE_SIGNING_PASSWORD
    if (-not $Pfx -or -not (Test-Path $Pfx)) {
        throw "Set MSSTORE_SIGNING_PFX to your Partner Center .pfx path"
    }
    if (-not $Password) {
        throw "Set MSSTORE_SIGNING_PASSWORD for the signing certificate"
    }

    $SignTool = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin" -Recurse -Filter SignTool.exe -ErrorAction SilentlyContinue |
        Select-Object -First 1 -ExpandProperty FullName
    if (-not $SignTool) {
        throw "SignTool.exe not found. Install the Windows 10/11 SDK."
    }

    Write-Host "Signing $MsixOut..."
    & $SignTool sign /fd SHA256 /a /f $Pfx /p $Password $MsixOut
    if ($LASTEXITCODE -ne 0) {
        throw "SignTool failed with exit code $LASTEXITCODE"
    }
    Write-Host "Signed $MsixOut"
}

# Keep a stable alias for CI / docs.
Copy-Item $MsixOut (Join-Path $RepoRoot "Mahjuro-Store.msix") -Force

if ($Validate -or $Sign) {
    $validateArgs = @{ Package = $MsixOut }
    if ($Sign) { $validateArgs.RequireSigned = $true }
    & (Join-Path $RepoRoot "scripts\validate-windows-store.ps1") @validateArgs
}

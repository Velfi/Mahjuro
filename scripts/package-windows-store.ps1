# Build Mahjuro MSIX for Microsoft Store (dist-msstore).
#
# Usage:
#   .\scripts\package-windows-store.ps1 [-Configuration Release]
#
# Prerequisites:
#   - Windows SDK (MakeAppx.exe, SignTool.exe)
#   - Partner Center publisher identity in packaging/msix/AppxManifest.xml
#
# Output:
#   Mahjuro-Store.msix

param(
    [ValidateSet("Debug", "Release")]
    [string]$Configuration = "Release"
)

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location $RepoRoot

$Version = (Select-String -Path Cargo.toml -Pattern '^version' | Select-Object -First 1).Line -replace '.*"(.+)"', '$1'
$VersionQuad = if ($Version -match '^(\d+)\.(\d+)\.(\d+)') {
    "$($Matches[1]).$($Matches[2]).$($Matches[3]).0"
} else {
    "0.0.0.0"
}

Write-Host "Building mahjuro ($Configuration, dist-msstore)..."
cargo build --$Configuration.ToLower() --no-default-features --features "game,dist-msstore" --target x86_64-pc-windows-msvc

$Stage = Join-Path $RepoRoot "target\msix-stage"
$Layout = Join-Path $Stage "layout"
if (Test-Path $Stage) { Remove-Item -Recurse -Force $Stage }
New-Item -ItemType Directory -Path $Layout -Force | Out-Null
New-Item -ItemType Directory -Path (Join-Path $Layout "Assets") -Force | Out-Null

$Bin = Join-Path $RepoRoot "target\x86_64-pc-windows-msvc\$($Configuration.ToLower())\mahjuro.exe"
Copy-Item $Bin $Layout

# Asset packs next to the executable (same layout as Steam ZIP).
$BakeOut = Join-Path $RepoRoot "target\mahjuro-bake-packs"
if (Test-Path $BakeOut) { Remove-Item -Recurse -Force $BakeOut }
python (Join-Path $RepoRoot "tools\bake_assets\bake_assets.py") --out $BakeOut
Copy-Item (Join-Path $BakeOut "pack_manifest.json") $Layout
Copy-Item (Join-Path $BakeOut "mahjuro-pack-shared.zip") $Layout
Copy-Item (Join-Path $BakeOut "mahjuro-pack-rooms.zip") $Layout
Copy-Item (Join-Path $BakeOut "mahjuro-pack-gameplay-bulk.zip") $Layout
Copy-Item (Join-Path $BakeOut "mahjuro-pack-music.zip") $Layout

# Placeholder store logos (replace with marketing assets before submission).
$LogoSrc = Join-Path $RepoRoot "icon.png"
Copy-Item $LogoSrc (Join-Path $Layout "Assets\StoreLogo.png")
Copy-Item $LogoSrc (Join-Path $Layout "Assets\Square150x150Logo.png")
Copy-Item $LogoSrc (Join-Path $Layout "Assets\Square44x44Logo.png")

$ManifestTemplate = Join-Path $RepoRoot "packaging\msix\AppxManifest.xml"
$Manifest = Get-Content $ManifestTemplate -Raw
$Manifest = $Manifest -replace '__VERSION_QUAD__', $VersionQuad
Set-Content -Path (Join-Path $Layout "AppxManifest.xml") -Value $Manifest -Encoding UTF8

$MakeAppx = "${env:ProgramFiles(x86)}\Windows Kits\10\bin\10.0.22621.0\x64\MakeAppx.exe"
if (-not (Test-Path $MakeAppx)) {
    $MakeAppx = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin" -Recurse -Filter MakeAppx.exe -ErrorAction SilentlyContinue |
        Select-Object -First 1 -ExpandProperty FullName
}
if (-not $MakeAppx) {
    throw "MakeAppx.exe not found. Install the Windows 10/11 SDK."
}

$MsixOut = Join-Path $RepoRoot "Mahjuro-Store.msix"
& $MakeAppx pack /d $Layout /p $MsixOut /o
Write-Host "Wrote $MsixOut (version $VersionQuad)"

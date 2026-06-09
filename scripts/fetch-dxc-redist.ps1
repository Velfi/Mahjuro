# Download Microsoft.Direct3D.DXC (dxcompiler.dll + dxil.dll) for Windows x64.
#
# Usage:
#   .\scripts\fetch-dxc-redist.ps1 [-Version 1.8.2505.32] [-OutDir path]
#
# Default output: third_party/dxc-redist/x64/ (gitignored DLLs; VERSION is committed).
# build.rs copies those files next to mahjuro.exe when present.
# Release CI runs this before `cargo build --release`.

param(
    [string]$Version = "",
    [string]$OutDir = ""
)

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location $RepoRoot

$VersionFile = Join-Path $RepoRoot "third_party\dxc-redist\VERSION"
if (-not $Version) {
    if (Test-Path $VersionFile) {
        $Version = (Get-Content $VersionFile -Raw).Trim()
    } else {
        $Version = "1.8.2505.32"
    }
}

if (-not $OutDir) {
    $OutDir = Join-Path $RepoRoot "third_party\dxc-redist\x64"
}

$PackageId = "Microsoft.Direct3D.DXC"
$Url = "https://www.nuget.org/api/v2/package/$PackageId/$Version"
$TmpZip = Join-Path $env:TEMP "mahjuro-dxc-$Version.nupkg"
$TmpExtract = Join-Path $env:TEMP "mahjuro-dxc-$Version"

Write-Host "Fetching $PackageId $Version..."
Invoke-WebRequest -Uri $Url -OutFile $TmpZip
if (Test-Path $TmpExtract) {
    Remove-Item -Recurse -Force $TmpExtract
}
Expand-Archive -Path $TmpZip -DestinationPath $TmpExtract -Force

$BinDir = Join-Path $TmpExtract "build\native\bin\x64"
$DxCompiler = Join-Path $BinDir "dxcompiler.dll"
$Dxil = Join-Path $BinDir "dxil.dll"
if (-not (Test-Path $DxCompiler)) {
    throw "dxcompiler.dll not found in NuGet package at $BinDir"
}
if (-not (Test-Path $Dxil)) {
    throw "dxil.dll not found in NuGet package at $BinDir"
}

New-Item -ItemType Directory -Force $OutDir | Out-Null
Copy-Item $DxCompiler (Join-Path $OutDir "dxcompiler.dll") -Force
Copy-Item $Dxil (Join-Path $OutDir "dxil.dll") -Force
Set-Content -Path $VersionFile -Value $Version -NoNewline

Write-Host "Installed DXC redist $Version -> $OutDir"

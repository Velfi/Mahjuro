# Preflight checks for a Mahjuro MSIX before Partner Center upload.
#
# Usage:
#   .\scripts\validate-windows-store.ps1 [-Package Mahjuro-Store.msix]

param(
    [string]$Package = "Mahjuro-Store.msix",
    [switch]$RequireSigned
)

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location $RepoRoot

if (-not (Test-Path $Package)) {
    throw "Package not found: $Package"
}

$fail = 0
function Test-Check {
    param([scriptblock]$Block, [string]$Label)
    try {
        & $Block
        Write-Host "  ok $Label"
    } catch {
        Write-Host "error: $Label — $_" -ForegroundColor Red
        $script:fail = 1
    }
}

Write-Host "== Package =="
Write-Host "  path:    $Package"
Write-Host "  size:    $((Get-Item $Package).Length) bytes"

$MakeAppx = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin" -Recurse -Filter MakeAppx.exe -ErrorAction SilentlyContinue |
    Select-Object -First 1 -ExpandProperty FullName
if (-not $MakeAppx) {
    throw "MakeAppx.exe not found. Install the Windows 10/11 SDK."
}

$Unpack = Join-Path $env:TEMP "mahjuro-msix-validate"
if (Test-Path $Unpack) { Remove-Item -Recurse -Force $Unpack }
New-Item -ItemType Directory -Path $Unpack -Force | Out-Null
& $MakeAppx unpack /p $Package /d $Unpack /o | Out-Null

$ManifestPath = Join-Path $Unpack "AppxManifest.xml"
if (-not (Test-Path $ManifestPath)) {
    Write-Host "error: AppxManifest.xml missing from package" -ForegroundColor Red
    exit 1
}

[xml]$Manifest = Get-Content $ManifestPath -Raw
$Identity = $Manifest.Package.Identity
$Version = $Identity.Version
$Name = $Identity.Name
$Publisher = $Identity.Publisher

Write-Host "== Identity =="
Write-Host "  Name:      $Name"
Write-Host "  Publisher: $Publisher"
Write-Host "  Version:   $Version"

if ($Publisher -match 'REPLACE_WITH_PARTNER_CENTER_PUBLISHER') {
    if ($RequireSigned) {
        Write-Host "error: Publisher is still the template placeholder" -ForegroundColor Red
        $fail = 1
    } else {
        Write-Host "  warning: Publisher is placeholder — set MSSTORE_PUBLISHER before upload" -ForegroundColor Yellow
    }
}

if ($Version -notmatch '^\d+\.\d+\.\d+\.\d+$') {
    Write-Host "error: Version must be four integers (e.g. 0.6.0.2)" -ForegroundColor Red
    $fail = 1
}

Write-Host "== Payload =="
foreach ($file in @(
    "mahjuro.exe",
    "pack_manifest.json",
    "mahjuro-pack-shared.zip",
    "mahjuro-pack-rooms.zip",
    "mahjuro-pack-gameplay-bulk.zip",
    "mahjuro-pack-music.zip"
)) {
    Test-Check { if (-not (Test-Path (Join-Path $Unpack $file))) { throw "missing $file" } } $file
}

Write-Host "== Store logos =="
foreach ($logo in @("Assets\StoreLogo.png", "Assets\Square150x150Logo.png", "Assets\Square44x44Logo.png")) {
    Test-Check { if (-not (Test-Path (Join-Path $Unpack $logo))) { throw "missing $logo" } } $logo
}

Write-Host "== Signature =="
$SignTool = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin" -Recurse -Filter SignTool.exe -ErrorAction SilentlyContinue |
    Select-Object -First 1 -ExpandProperty FullName
if ($SignTool) {
    $verify = & $SignTool verify /pa /v $Package 2>&1
    if ($LASTEXITCODE -eq 0) {
        Write-Host "  ok SignTool verify"
    } elseif ($RequireSigned) {
        Write-Host "error: package is unsigned or signature invalid (required for Partner Center upload)" -ForegroundColor Red
        $verify | ForEach-Object { Write-Host "    $_" }
        $fail = 1
    } else {
        Write-Host "  warning: unsigned (sign with -Sign before Partner Center upload)" -ForegroundColor Yellow
    }
} else {
    Write-Host "warning: SignTool.exe not found; skipping signature check" -ForegroundColor Yellow
}

Remove-Item -Recurse -Force $Unpack

if ($fail -ne 0) {
    Write-Host "validation FAILED" -ForegroundColor Red
    exit 1
}
Write-Host "validation OK"

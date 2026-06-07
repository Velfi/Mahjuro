# Resolve marketing + package versions for Microsoft Store MSIX.
# Dot-source from package-windows-store.ps1.
#
# Inputs:
#   MSSTORE_BUILD_NUMBER  optional override for the 4th version quad (must increase per upload)
#
# Outputs (sets script-scope variables):
#   MSSTORE_SHORT_VERSION  e.g. 0.6.0
#   MSSTORE_BUILD_NUMBER   e.g. 2
#   MSSTORE_VERSION_QUAD   e.g. 0.6.0.2

function Resolve-MsStoreVersions {
    $raw = (Select-String -Path (Join-Path $RepoRoot "Cargo.toml") -Pattern '^version' |
        Select-Object -First 1).Line -replace '.*"(.+)"', '$1'
    if (-not $raw) {
        throw "Could not determine crate version from Cargo.toml"
    }

    if ($raw -match '^(.+)-(.+)$') {
        $script:MSSTORE_SHORT_VERSION = $Matches[1]
        if (-not $env:MSSTORE_BUILD_NUMBER) {
            $script:MSSTORE_BUILD_NUMBER = $Matches[2]
        }
    } else {
        $script:MSSTORE_SHORT_VERSION = $raw
        if (-not $env:MSSTORE_BUILD_NUMBER) {
            $script:MSSTORE_BUILD_NUMBER = "1"
        }
    }

    if ($env:MSSTORE_BUILD_NUMBER) {
        $script:MSSTORE_BUILD_NUMBER = $env:MSSTORE_BUILD_NUMBER
    }

    if ($script:MSSTORE_BUILD_NUMBER -notmatch '^\d+$') {
        Write-Warning "MSSTORE_BUILD_NUMBER '$($script:MSSTORE_BUILD_NUMBER)' is not numeric; using 1"
        $script:MSSTORE_BUILD_NUMBER = "1"
    }

    $script:MSSTORE_VERSION_QUAD = "$($script:MSSTORE_SHORT_VERSION).$($script:MSSTORE_BUILD_NUMBER)"
}

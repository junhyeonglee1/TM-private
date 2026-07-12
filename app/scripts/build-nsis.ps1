[CmdletBinding()]
param(
    [switch]$Approved
)

$ErrorActionPreference = "Stop"

# NSIS bundling is an explicitly approval-gated operation. Keep this guard
# before environment setup, directory creation, compilation, or any other work.
if (-not $Approved) {
    throw "NSIS bundling is not approved. Re-run only after explicit approval and pass -Approved."
}

# This script only creates bundle artifacts; it never launches the generated
# installer or uninstaller. Fresh install, reinstall, removal, registry checks,
# and external TM\data preservation tests require a separate explicit approval.
#
# WebView2 tradeoff: tauri.conf.json uses webviewInstallMode=skip so this bundle
# does not download or install WebView2. A target PC without a compatible
# WebView2 Runtime must install it separately before TM can run.
#
# Sidecar rule verified against the installed Tauri CLI 2.11.4 local schema
# (`@tauri-apps/cli/config.schema.json`, externalBin): configuration receives a
# base path and Tauri resolves `<base>-<target-triple>.exe` on Windows.

. (Join-Path $PSScriptRoot "env.ps1")

$appRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$tmRoot = (Resolve-Path (Join-Path $appRoot "..")).Path
$target = "x86_64-pc-windows-msvc"
$cargoTargetDir = Join-Path $tmRoot "dist\build\cargo"
$cargoReleaseDir = Join-Path $cargoTargetDir "$target\release"
$bundleDir = Join-Path $cargoReleaseDir "bundle\nsis"
$releaseDir = Join-Path $tmRoot "dist\release"
$stageDir = Join-Path $tmRoot "dist\stage\nsis"
$sidecarSource = Join-Path $cargoReleaseDir "tm-cli.exe"
$sidecarBase = Join-Path $stageDir "tm-cli"
$sidecarStaged = "$sidecarBase-$target.exe"
$overridePath = Join-Path $stageDir "tauri.nsis.config.json"
$localNsisRoot = Join-Path $cargoTargetDir ".tauri\NSIS"
$tauri = Join-Path $appRoot "node_modules\.bin\tauri.cmd"
$tsc = Join-Path $appRoot "node_modules\.bin\tsc.cmd"
$vite = Join-Path $appRoot "node_modules\.bin\vite.cmd"
$cargo = "C:\Users\tkfk0\.cargo\bin\cargo.exe"

$env:VSLANG = "1033"
$env:CARGO_NET_OFFLINE = "true"
$env:npm_config_offline = "true"
$env:COREPACK_ENABLE_NETWORK = "0"

if (-not (Test-Path -LiteralPath (Join-Path $appRoot "node_modules") -PathType Container)) {
    throw "node_modules is missing; NSIS bundling is offline and will not install frontend dependencies"
}

# useLocalToolsDir=true keeps NSIS tooling under TM\dist. Refuse to let Tauri
# provision a missing tool cache implicitly; populating it is a separate,
# approval-gated download/copy operation.
$makensisCandidates = @(
    (Join-Path $localNsisRoot "makensis.exe"),
    (Join-Path $localNsisRoot "Bin\makensis.exe")
)
$makensis = $makensisCandidates |
    Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } |
    Select-Object -First 1
$tauriNsisPlugin = Join-Path $localNsisRoot "Plugins\x86-unicode\additional\nsis_tauri_utils.dll"
if (-not $makensis -or -not (Test-Path -LiteralPath $tauriNsisPlugin -PathType Leaf)) {
    throw "local NSIS cache is incomplete at $localNsisRoot; no download was attempted"
}

Push-Location $appRoot
try {
    & $cargo build --locked --offline --release --target $target -p tm-cli --bin tm-cli
    if ($LASTEXITCODE -ne 0) { throw "offline CLI sidecar build failed: $LASTEXITCODE" }

    New-Item -ItemType Directory -Force -Path $stageDir | Out-Null
    Copy-Item -LiteralPath $sidecarSource -Destination $sidecarStaged -Force

    $override = [ordered]@{
        build = [ordered]@{
            beforeBuildCommand = $null
        }
        bundle = [ordered]@{
            externalBin = @($sidecarBase)
        }
    }
    $overrideJson = $override | ConvertTo-Json -Depth 4
    [System.IO.File]::WriteAllText(
        $overridePath,
        $overrideJson,
        [System.Text.UTF8Encoding]::new($false)
    )

    # Cargo and the local Tauri CLI are forced offline. The preflight above also prevents a
    # missing local NSIS tool cache from reaching Tauri's downloader.
    & $tsc -b --pretty false
    if ($LASTEXITCODE -ne 0) { throw "frontend typecheck failed: $LASTEXITCODE" }

    & $vite build
    if ($LASTEXITCODE -ne 0) { throw "frontend release build failed: $LASTEXITCODE" }

    $bundleBuildStartedAt = [DateTime]::UtcNow
    & $tauri build --target $target --bundles nsis --config $overridePath -- --locked --offline
    if ($LASTEXITCODE -ne 0) { throw "offline Tauri NSIS bundle failed: $LASTEXITCODE" }
}
finally {
    Pop-Location
}

$desktopSource = Join-Path $cargoReleaseDir "tm.exe"
$installers = @(
    Get-ChildItem -LiteralPath $bundleDir -Filter "*.exe" -File -ErrorAction SilentlyContinue |
        Where-Object { $_.LastWriteTimeUtc -ge $bundleBuildStartedAt.AddSeconds(-2) }
)
if (-not (Test-Path -LiteralPath $desktopSource -PathType Leaf)) {
    throw "desktop release executable was not produced: $desktopSource"
}
if ($installers.Count -eq 0) {
    throw "NSIS installer was not produced in $bundleDir"
}

New-Item -ItemType Directory -Force -Path $releaseDir | Out-Null
$releaseArtifacts = @(
    @{ Source = $desktopSource; Destination = (Join-Path $releaseDir "tm.exe") },
    @{ Source = $sidecarSource; Destination = (Join-Path $releaseDir "tm-cli.exe") }
)
foreach ($installer in $installers) {
    $releaseArtifacts += @{
        Source = $installer.FullName
        Destination = (Join-Path $releaseDir $installer.Name)
    }
}

$copiedPaths = foreach ($artifact in $releaseArtifacts) {
    try {
        Copy-Item -LiteralPath $artifact.Source -Destination $artifact.Destination -Force
        $artifact.Destination
    }
    catch [System.IO.IOException] {
        if ((Split-Path -Leaf $artifact.Destination) -ne "tm.exe") { throw }
        $appVersion = (Get-Content -Raw -LiteralPath (Join-Path $appRoot "src-tauri\tauri.conf.json") | ConvertFrom-Json).version
        $fallbackDestination = Join-Path $releaseDir "tm-$appVersion.exe"
        Copy-Item -LiteralPath $artifact.Source -Destination $fallbackDestination -Force
        Write-Warning "dist\release\tm.exe is running; wrote the new build to $(Split-Path -Leaf $fallbackDestination)"
        $fallbackDestination
    }
}

Write-Output "NSIS artifacts were built but not installed or executed."
Get-FileHash -Algorithm SHA256 -LiteralPath $copiedPaths

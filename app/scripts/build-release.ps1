$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot "env.ps1")

# This script creates raw Windows x64 executables only. It never invokes the
# Tauri bundler, NSIS, an installer, or an uninstaller. NSIS is handled by the
# separately approval-gated build-nsis.ps1 script.
$appRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$tmRoot = (Resolve-Path (Join-Path $appRoot "..")).Path
$target = "x86_64-pc-windows-msvc"
$releaseDir = Join-Path $tmRoot "dist\release"
$cargoReleaseDir = Join-Path $tmRoot "dist\build\cargo\$target\release"
$stageDir = Join-Path $tmRoot "dist\stage\raw-release"
$overridePath = Join-Path $stageDir "tauri.raw.config.json"
$tauri = Join-Path $appRoot "node_modules\.bin\tauri.cmd"
$tsc = Join-Path $appRoot "node_modules\.bin\tsc.cmd"
$vite = Join-Path $appRoot "node_modules\.bin\vite.cmd"
$cargo = "C:\Users\tkfk0\.cargo\bin\cargo.exe"
$env:VSLANG = "1033"
$env:CARGO_NET_OFFLINE = "true"
$env:npm_config_offline = "true"
$env:COREPACK_ENABLE_NETWORK = "0"

if (-not (Test-Path -LiteralPath (Join-Path $appRoot "node_modules") -PathType Container)) {
    throw "node_modules is missing; run the separately approved dependency installation before the raw release build"
}

Push-Location $appRoot
try {
    & $tsc -b --pretty false
    if ($LASTEXITCODE -ne 0) { throw "frontend typecheck failed: $LASTEXITCODE" }

    & $vite build
    if ($LASTEXITCODE -ne 0) { throw "frontend release build failed: $LASTEXITCODE" }

    New-Item -ItemType Directory -Force -Path $stageDir | Out-Null
    $override = [ordered]@{ build = [ordered]@{ beforeBuildCommand = $null } }
    [System.IO.File]::WriteAllText(
        $overridePath,
        ($override | ConvertTo-Json -Depth 3),
        [System.Text.UTF8Encoding]::new($false)
    )

    # Use Tauri's production build path while explicitly skipping all bundles.
    # custom-protocol is repeated here and guarded in Rust so localhost builds
    # cannot be published by this raw-EXE workflow again.
    & $tauri build --target $target --no-bundle --features custom-protocol --config $overridePath -- --locked --offline
    if ($LASTEXITCODE -ne 0) { throw "desktop release build failed: $LASTEXITCODE" }

    & $cargo build --locked --offline --release --target $target -p tm-cli --bin tm-cli
    if ($LASTEXITCODE -ne 0) { throw "CLI release build failed: $LASTEXITCODE" }
}
finally {
    Pop-Location
}

New-Item -ItemType Directory -Force -Path $releaseDir | Out-Null
$appVersion = (Get-Content -Raw -LiteralPath (Join-Path $appRoot "src-tauri\tauri.conf.json") | ConvertFrom-Json).version
$desktopDestination = Join-Path $releaseDir "tm.exe"
try {
    Copy-Item -LiteralPath (Join-Path $cargoReleaseDir "tm.exe") -Destination $desktopDestination -Force
}
catch [System.IO.IOException] {
    $desktopDestination = Join-Path $releaseDir "tm-$appVersion.exe"
    Copy-Item -LiteralPath (Join-Path $cargoReleaseDir "tm.exe") -Destination $desktopDestination -Force
    Write-Warning "dist\release\tm.exe is running; wrote the new build to $(Split-Path -Leaf $desktopDestination)"
}
Copy-Item -LiteralPath (Join-Path $cargoReleaseDir "tm-cli.exe") -Destination (Join-Path $releaseDir "tm-cli.exe") -Force

Write-Output "Raw Windows x64 executables created. No NSIS installer was built or run."
Get-FileHash -Algorithm SHA256 -LiteralPath $desktopDestination, (Join-Path $releaseDir "tm-cli.exe")

$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot "env.ps1")

$appRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$eslint = Join-Path $appRoot "node_modules\.bin\eslint.cmd"
$tsc = Join-Path $appRoot "node_modules\.bin\tsc.cmd"
$vitest = Join-Path $appRoot "node_modules\.bin\vitest.cmd"
$cargo = "C:\Users\tkfk0\.cargo\bin\cargo.exe"
$env:VSLANG = "1033"
$env:CARGO_NET_OFFLINE = "true"
$env:npm_config_offline = "true"
$env:COREPACK_ENABLE_NETWORK = "0"

# Dependency provisioning is a separate, approval-gated operation. Verification
# must use the existing lockfiles, node_modules and Cargo cache without downloads.
if (-not (Test-Path -LiteralPath (Join-Path $appRoot "node_modules") -PathType Container)) {
    throw "node_modules is missing; run the separately approved dependency installation before verification"
}

Push-Location $appRoot
try {
    & $eslint . --max-warnings 0
    if ($LASTEXITCODE -ne 0) { throw "frontend lint failed: $LASTEXITCODE" }

    & $tsc -b --pretty false
    if ($LASTEXITCODE -ne 0) { throw "frontend typecheck failed: $LASTEXITCODE" }

    & $vitest run
    if ($LASTEXITCODE -ne 0) { throw "frontend tests failed: $LASTEXITCODE" }

    & $cargo fmt --all -- --check
    if ($LASTEXITCODE -ne 0) { throw "cargo fmt check failed: $LASTEXITCODE" }

    & $cargo clippy --locked --offline --workspace --all-targets -- -D warnings
    if ($LASTEXITCODE -ne 0) { throw "cargo clippy failed: $LASTEXITCODE" }

    & $cargo test --locked --offline --workspace
    if ($LASTEXITCODE -ne 0) { throw "cargo tests failed: $LASTEXITCODE" }
}
finally {
    Pop-Location
}

Write-Output "TM verification completed successfully."

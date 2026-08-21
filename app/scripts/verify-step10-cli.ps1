[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$appRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$outputDirectory = Join-Path $appRoot '..\dist\manual-step10-cli-verification'
$outputPath = Join-Path $outputDirectory 'result.json'
New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null
Remove-Item -LiteralPath $outputPath -Force -ErrorAction SilentlyContinue

$toolchain = 'C:\Users\tkfk0\.rustup\toolchains\stable-x86_64-pc-windows-msvc\bin'
$cargo = Join-Path $toolchain 'cargo.exe'
if (-not (Test-Path -LiteralPath $cargo)) {
    throw 'Cargo was not found in the stable Rust toolchain.'
}
$env:PATH = "$toolchain;$env:PATH"

$status = 'failed'
$stage = 'start'
$message = ''
try {
    Set-Location -LiteralPath $appRoot

    $stage = 'format'
    & $cargo fmt --all
    if ($LASTEXITCODE -ne 0) {
        throw "cargo fmt failed with exit code $LASTEXITCODE"
    }

    $stage = 'test'
    & $cargo test --locked --offline -p tm-cli
    if ($LASTEXITCODE -ne 0) {
        throw "cargo test failed with exit code $LASTEXITCODE"
    }

    $stage = 'clippy'
    & $cargo clippy --locked --offline -p tm-cli --all-targets -- -D warnings
    if ($LASTEXITCODE -ne 0) {
        throw "cargo clippy failed with exit code $LASTEXITCODE"
    }

    $status = 'passed'
    $stage = 'complete'
}
catch {
    $message = $_.Exception.Message
}
finally {
    [pscustomobject]@{
        status = $status
        stage = $stage
        message = $message
        checkedAt = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ssZ')
    } | ConvertTo-Json | Set-Content -LiteralPath $outputPath -Encoding UTF8
}

if ($status -eq 'passed') {
    Write-Host 'STEP 10 migration CLI verification passed.' -ForegroundColor Green
    exit 0
}

Write-Host "STEP 10 migration CLI verification failed at $stage`: $message" -ForegroundColor Red
exit 1

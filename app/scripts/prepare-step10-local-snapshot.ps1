[CmdletBinding()]
param(
    [string]$TmRoot = (Split-Path -Parent (Split-Path -Parent $PSScriptRoot)),
    [string]$TmCliPath
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$tmRootPath = [System.IO.Path]::GetFullPath($TmRoot)
$appPath = Join-Path $tmRootPath 'app'
$databasePath = Join-Path $tmRootPath 'data\tm.sqlite3'
$attachmentsPath = Join-Path $tmRootPath 'data\attachments'
if ([string]::IsNullOrWhiteSpace($TmCliPath)) {
    $TmCliPath = Join-Path $tmRootPath 'dist\release\tm-cli.exe'
}
$tmCli = [System.IO.Path]::GetFullPath($TmCliPath)
if (-not (Test-Path -LiteralPath $tmCli -PathType Leaf)) {
    throw "Verified STEP 10 tm-cli was not found: $tmCli"
}
$timestamp = [DateTimeOffset]::UtcNow.ToString('yyyyMMddTHHmmssZ')
$cutoverPath = Join-Path $tmRootPath "backups\cutover\$timestamp"
$rawBackupPath = Join-Path $cutoverPath 'pre-migration-schema3.sqlite3'
$snapshotPath = Join-Path $cutoverPath 'tm-cutover-schema5.sqlite3'
$resultPath = Join-Path $cutoverPath 'cutover-result.json'

$runningTm = Get-Process -Name 'tm' -ErrorAction SilentlyContinue
if ($null -ne $runningTm) {
    throw 'TM desktop is running. Close it before preparing the cutover snapshot.'
}
if (-not (Test-Path -LiteralPath $databasePath -PathType Leaf)) {
    throw "TM database was not found: $databasePath"
}
$walPath = "$databasePath-wal"
if ((Test-Path -LiteralPath $walPath -PathType Leaf) -and (Get-Item -LiteralPath $walPath).Length -gt 0) {
    throw "SQLite WAL still contains uncheckpointed data after TM closed: $walPath"
}

# A zero-byte WAL and a non-empty SHM can remain after a clean SQLite close.
# Prove that no process holds the database before removing those stale sidecars.
$exclusiveDatabase = [System.IO.File]::Open(
    $databasePath,
    [System.IO.FileMode]::Open,
    [System.IO.FileAccess]::ReadWrite,
    [System.IO.FileShare]::None
)
$exclusiveDatabase.Dispose()
foreach ($sidecar in @($walPath, "$databasePath-shm")) {
    Remove-Item -LiteralPath $sidecar -Force -ErrorAction SilentlyContinue
}

[System.IO.Directory]::CreateDirectory($cutoverPath) | Out-Null
Copy-Item -LiteralPath $databasePath -Destination $rawBackupPath
if (Test-Path -LiteralPath $attachmentsPath -PathType Container) {
    Copy-Item -LiteralPath $attachmentsPath -Destination (Join-Path $cutoverPath 'attachments') -Recurse
}

$env:TM_HOME = $tmRootPath
Push-Location $appPath
try {
    $dryRunJson = & $tmCli migration dry-run --json
    if ($LASTEXITCODE -ne 0) {
        throw "tm-cli migration dry-run failed with exit code $LASTEXITCODE"
    }
    $dryRun = $dryRunJson | ConvertFrom-Json
    if (-not $dryRun.verified -or $dryRun.source.schemaVersion -ne 5 -or $dryRun.source.logicalSha256 -ne $dryRun.snapshot.logicalSha256) {
        throw 'tm-cli migration dry-run did not produce a verified schema 5 snapshot.'
    }
    $generatedSnapshotPath = [string]$dryRun.snapshotArtifact.path
    if (-not (Test-Path -LiteralPath $generatedSnapshotPath -PathType Leaf)) {
        throw 'tm-cli reported a snapshot that does not exist.'
    }
    Copy-Item -LiteralPath $generatedSnapshotPath -Destination $snapshotPath

    $inspectJson = & $tmCli migration inspect --path $snapshotPath --json
    if ($LASTEXITCODE -ne 0) {
        throw "tm-cli migration inspect failed with exit code $LASTEXITCODE"
    }
    $inspect = $inspectJson | ConvertFrom-Json
    if ($inspect.logicalSha256 -ne $dryRun.source.logicalSha256) {
        throw 'The stable cutover snapshot does not match the migrated local database.'
    }
}
finally {
    Pop-Location
    Remove-Item Env:TM_HOME -ErrorAction SilentlyContinue
}

$result = [ordered]@{
    preparedAt = [DateTimeOffset]::UtcNow.ToString('o')
    tmWasStopped = $true
    preMigrationBackup = [ordered]@{
        path = $rawBackupPath
        sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $rawBackupPath).Hash.ToLowerInvariant()
    }
    snapshot = [ordered]@{
        path = $snapshotPath
        byteSize = (Get-Item -LiteralPath $snapshotPath).Length
        sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $snapshotPath).Hash.ToLowerInvariant()
        manifest = $inspect
    }
    localArchiveRetentionDays = 90
}
$json = $result | ConvertTo-Json -Depth 20
[System.IO.File]::WriteAllText($resultPath, $json, [System.Text.UTF8Encoding]::new($false))

Write-Host 'STEP 10 local snapshot prepared.' -ForegroundColor Green
Write-Host "Snapshot: $snapshotPath"
Write-Host "Logical SHA-256: $($inspect.logicalSha256)"
Write-Host "Result: $resultPath"

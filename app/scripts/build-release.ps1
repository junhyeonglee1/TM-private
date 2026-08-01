[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateRange(1, [long]::MaxValue)]
    [long]$RunId,

    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[0-9a-fA-F]{40}$')]
    [string]$ExpectedHeadSha,

    [switch]$Approved
)

$ErrorActionPreference = 'Stop'

# Windows Application Control blocks Cargo-generated build scripts on this PC.
# A release is therefore compiled and tested only by the pinned GitHub Actions
# workflow. This script never invokes Cargo, rustc, Tauri, NSIS, an installer,
# or an uninstaller; it only verifies and copies an already-successful artifact.
if (-not $Approved) {
    throw 'GitHub Actions artifact retrieval is not approved. Re-run with -Approved after reviewing the run ID.'
}

$repository = 'junhyeonglee1/TM-private'
$expectedHeadBranch = 'agent/step10-cloud-cutover'
$ExpectedHeadSha = $ExpectedHeadSha.ToLowerInvariant()

$gh = Get-Command gh -ErrorAction SilentlyContinue
if (-not $gh) {
    throw 'GitHub CLI is required to retrieve the verified Windows artifact.'
}

$appRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$tmRoot = (Resolve-Path (Join-Path $appRoot '..')).Path
$releaseDir = Join-Path $tmRoot 'dist\release'
$downloadRoot = Join-Path $tmRoot ("dist\stage\github-actions\{0}-{1}" -f $RunId, [Guid]::NewGuid().ToString('N'))

$runJson = & $gh.Source run view $RunId --repo $Repository --json databaseId,workflowName,conclusion,headBranch,headSha 2>&1
if ($LASTEXITCODE -ne 0) {
    throw "Unable to inspect GitHub Actions run $RunId. Re-authenticate GitHub CLI and retry."
}
$run = $runJson | ConvertFrom-Json
if ($run.workflowName -ne 'STEP 10 Windows build') {
    throw "Run $RunId belongs to an unexpected workflow: $($run.workflowName)"
}
if ($run.conclusion -ne 'success') {
    throw "Run $RunId is not successful: $($run.conclusion)"
}
if ($run.headBranch -ne $expectedHeadBranch) {
    throw "Run $RunId belongs to an unexpected branch: $($run.headBranch)"
}
if ([string]$run.headSha -ne $ExpectedHeadSha) {
    throw "Run $RunId does not match the expected source commit."
}

New-Item -ItemType Directory -Force -Path $downloadRoot | Out-Null
& $gh.Source run download $RunId --repo $Repository --name 'tm-step10-windows-x64' --dir $downloadRoot
if ($LASTEXITCODE -ne 0) {
    throw "Unable to download tm-step10-windows-x64 from run $RunId."
}

$requiredFiles = @(
    'tm.exe',
    'tm-cli.exe',
    'tm-office-decryptor.exe'
)
$manifestPath = Join-Path $downloadRoot 'SHA256SUMS.txt'
if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
    throw 'The artifact does not contain SHA256SUMS.txt.'
}

$manifest = @{}
foreach ($line in Get-Content -LiteralPath $manifestPath) {
    if ($line -notmatch '^([0-9a-fA-F]{64})\s{2}([^\\/:*?"<>|]+)$') {
        throw 'SHA256SUMS.txt contains an invalid entry.'
    }
    $manifest[$Matches[2]] = $Matches[1].ToLowerInvariant()
}

foreach ($name in $requiredFiles) {
    $path = Join-Path $downloadRoot $name
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "The artifact does not contain $name."
    }
    if (-not $manifest.ContainsKey($name)) {
        throw "SHA256SUMS.txt does not contain $name."
    }
    $actual = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $manifest[$name]) {
        throw "SHA-256 verification failed for $name."
    }
}

if ($manifest.Count -ne $requiredFiles.Count) {
    throw 'SHA256SUMS.txt contains an unexpected file entry.'
}

New-Item -ItemType Directory -Force -Path $releaseDir | Out-Null
$destinations = @{}
foreach ($name in $requiredFiles) {
    $destination = Join-Path $releaseDir $name
    $destinations[$name] = $destination
    if (-not (Test-Path -LiteralPath $destination -PathType Leaf)) {
        continue
    }
    try {
        $lockProbe = [System.IO.File]::Open(
            $destination,
            [System.IO.FileMode]::Open,
            [System.IO.FileAccess]::ReadWrite,
            [System.IO.FileShare]::None
        )
        $lockProbe.Dispose()
    }
    catch {
        throw "Close the running TM process before replacing $name. No release files were changed."
    }
}

$copied = @()
foreach ($name in $requiredFiles) {
    $source = Join-Path $downloadRoot $name
    $destination = $destinations[$name]
    Copy-Item -LiteralPath $source -Destination $destination -Force
    $actual = (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $manifest[$name]) {
        throw "The copied release file failed SHA-256 verification: $name"
    }
    $copied += $destination
}

$releaseManifestLines = [string[]]@(
    $requiredFiles | ForEach-Object { '{0}  {1}' -f $manifest[$_], $_ }
)
[System.IO.File]::WriteAllLines(
    (Join-Path $releaseDir 'SHA256SUMS.txt'),
    $releaseManifestLines,
    [System.Text.Encoding]::ASCII
)
$provenance = [ordered]@{
    runId = $RunId
    repository = $Repository
    workflow = $run.workflowName
    conclusion = $run.conclusion
    headBranch = $run.headBranch
    headSha = $run.headSha
    retrievedAtUtc = [DateTime]::UtcNow.ToString('o')
}
[System.IO.File]::WriteAllText(
    (Join-Path $releaseDir 'STEP10_PROVENANCE.json'),
    ($provenance | ConvertTo-Json -Depth 3),
    [System.Text.UTF8Encoding]::new($false)
)

Write-Output "Verified GitHub Actions artifact from run $RunId was copied to $releaseDir."
Get-FileHash -Algorithm SHA256 -LiteralPath $copied

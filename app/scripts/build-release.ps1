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

$appRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$tmRoot = (Resolve-Path (Join-Path $appRoot '..')).Path
. (Join-Path $PSScriptRoot 'expense-release-evidence.ps1')
$releaseDir = Join-Path $tmRoot 'dist\release'
$downloadRoot = Join-Path $tmRoot ("dist\stage\github-actions\{0}-{1}" -f $RunId, [Guid]::NewGuid().ToString('N'))
$gitEvidence = Resolve-TmVerifiedGit
$null = Assert-TmCanonicalGitState -GitPath ([string]$gitEvidence.path) `
    -RepositoryRoot $tmRoot -Repository $repository -Branch $expectedHeadBranch `
    -ExpectedHeadSha $ExpectedHeadSha
$ghEvidence = Resolve-TmVerifiedGh
$evidence = Get-TmVerifiedActionsArtifactEvidence -GhPath ([string]$ghEvidence.path) `
    -Repository $repository -RunId $RunId -ExpectedHeadSha $ExpectedHeadSha `
    -ExpectedBranch $expectedHeadBranch -ExpectedWorkflowName 'STEP 10 Windows build' `
    -ExpectedWorkflowPath '.github/workflows/windows-step10-build.yml' `
    -ArtifactKind step10 -ArtifactName 'tm-step10-windows-x64' `
    -PayloadDestination $downloadRoot

$requiredFiles = @(
    'tm.exe',
    'tm-cli.exe',
    'tm-office-decryptor.exe'
)
$manifest = $evidence.verifiedFileHashes

foreach ($name in $requiredFiles) {
    $path = Join-Path $downloadRoot $name
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "The artifact does not contain $name."
    }
    if (-not $manifest.Contains($name)) {
        throw "SHA256SUMS.txt does not contain $name."
    }
    $actual = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $manifest[$name]) {
        throw "SHA-256 verification failed for $name."
    }
}

if ($manifest.Count -ne $requiredFiles.Count) {
    throw 'The verified STEP 10 manifest contains an unexpected file entry.'
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
    runAttempt = [long]$evidence.runAttempt
    repository = $repository
    workflow = [string]$evidence.workflowName
    workflowPath = [string]$evidence.workflowPath
    event = [string]$evidence.event
    conclusion = 'success'
    headBranch = $expectedHeadBranch
    headSha = [string]$evidence.headSha
    artifactId = [long]$evidence.artifactId
    artifactDigest = [string]$evidence.artifactDigest
    downloadedZipSha256 = [string]$evidence.downloadedZipSha256
    manifestSha256 = [string]$evidence.manifestSha256
    gitSha256 = [string]$gitEvidence.sha256
    githubCliSha256 = [string]$ghEvidence.sha256
    retrievedAtUtc = [DateTime]::UtcNow.ToString('o')
}
[System.IO.File]::WriteAllText(
    (Join-Path $releaseDir 'STEP10_PROVENANCE.json'),
    ($provenance | ConvertTo-Json -Depth 3),
    [System.Text.UTF8Encoding]::new($false)
)

Write-Output "Verified GitHub Actions artifact from run $RunId was copied to $releaseDir."
Get-FileHash -Algorithm SHA256 -LiteralPath $copied

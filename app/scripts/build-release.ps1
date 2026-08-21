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
$releaseManifestLines = [string[]]@(
    $requiredFiles | ForEach-Object { '{0}  {1}' -f $manifest[$_], $_ }
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
$metadataRoot = Join-Path $downloadRoot '.tm-release-metadata'
New-Item -ItemType Directory -Force -Path $metadataRoot | Out-Null
$manifestSource = Join-Path $metadataRoot 'SHA256SUMS.txt'
$provenanceSource = Join-Path $metadataRoot 'STEP10_PROVENANCE.json'
[System.IO.File]::WriteAllLines($manifestSource, $releaseManifestLines, [System.Text.Encoding]::ASCII)
[System.IO.File]::WriteAllText(
    $provenanceSource,
    ($provenance | ConvertTo-Json -Depth 3),
    [System.Text.UTF8Encoding]::new($false)
)

# dist\release remains the complete release bundle. The repository root is the
# canonical launch location, so every verified patch also refreshes the files
# needed beside TM\tm.exe. All destinations are lock-checked and staged before
# any existing client file is changed.
$installItems = [System.Collections.Generic.List[object]]::new()
foreach ($name in @('tm-office-decryptor.exe', 'tm-cli.exe', 'tm.exe')) {
    $installItems.Add([pscustomobject]@{
        Label = 'release'
        Name = $name
        Source = Join-Path $downloadRoot $name
        Destination = Join-Path $releaseDir $name
        ExpectedSha256 = [string]$manifest[$name]
        Existed = $false
        Backup = $null
        Staged = $null
    })
}
foreach ($metadata in @(
    [pscustomobject]@{ Name = 'SHA256SUMS.txt'; Source = $manifestSource },
    [pscustomobject]@{ Name = 'STEP10_PROVENANCE.json'; Source = $provenanceSource }
)) {
    $installItems.Add([pscustomobject]@{
        Label = 'release'
        Name = $metadata.Name
        Source = $metadata.Source
        Destination = Join-Path $releaseDir $metadata.Name
        ExpectedSha256 = (Get-FileHash -LiteralPath $metadata.Source -Algorithm SHA256).Hash.ToLowerInvariant()
        Existed = $false
        Backup = $null
        Staged = $null
    })
}
foreach ($name in @('tm-office-decryptor.exe', 'tm-cli.exe')) {
    $installItems.Add([pscustomobject]@{
        Label = 'canonical'
        Name = $name
        Source = Join-Path $downloadRoot $name
        Destination = Join-Path $tmRoot $name
        ExpectedSha256 = [string]$manifest[$name]
        Existed = $false
        Backup = $null
        Staged = $null
    })
}
foreach ($metadata in @(
    [pscustomobject]@{ Name = 'SHA256SUMS.txt'; Source = $manifestSource },
    [pscustomobject]@{ Name = 'STEP10_PROVENANCE.json'; Source = $provenanceSource }
)) {
    $installItems.Add([pscustomobject]@{
        Label = 'canonical'
        Name = $metadata.Name
        Source = $metadata.Source
        Destination = Join-Path $tmRoot $metadata.Name
        ExpectedSha256 = (Get-FileHash -LiteralPath $metadata.Source -Algorithm SHA256).Hash.ToLowerInvariant()
        Existed = $false
        Backup = $null
        Staged = $null
    })
}
# Publish the canonical GUI executable last. If Windows loses power during the
# tiny commit window, the launcher is therefore either the previous verified
# binary or the new verified binary, never a partially copied file.
$installItems.Add([pscustomobject]@{
    Label = 'canonical'
    Name = 'tm.exe'
    Source = Join-Path $downloadRoot 'tm.exe'
    Destination = Join-Path $tmRoot 'tm.exe'
    ExpectedSha256 = [string]$manifest['tm.exe']
    Existed = $false
    Backup = $null
    Staged = $null
})

foreach ($item in $installItems) {
    $item.Existed = Test-Path -LiteralPath $item.Destination -PathType Leaf
    if (-not $item.Existed) {
        continue
    }
    try {
        $lockProbe = [System.IO.File]::Open(
            $item.Destination,
            [System.IO.FileMode]::Open,
            [System.IO.FileAccess]::ReadWrite,
            [System.IO.FileShare]::None
        )
        $lockProbe.Dispose()
    }
    catch {
        throw "Close the running TM process before replacing $($item.Label)\$($item.Name). No installed files were changed."
    }
}

$installId = [Guid]::NewGuid().ToString('N')
$backupRoot = Join-Path $tmRoot ("backups\windows-client-before-{0}-{1}" -f [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssZ'), $installId)
$installed = [System.Collections.Generic.List[string]]::new()
try {
    foreach ($item in $installItems) {
        $item.Staged = Join-Path (Split-Path -Parent $item.Destination) (".{0}.tm-install-{1}" -f $item.Name, $installId)
        Copy-Item -LiteralPath $item.Source -Destination $item.Staged -Force
        $stagedHash = (Get-FileHash -LiteralPath $item.Staged -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($stagedHash -ne $item.ExpectedSha256) {
            throw "The staged release file failed SHA-256 verification: $($item.Label)\$($item.Name)"
        }
    }

    foreach ($item in $installItems | Where-Object Existed) {
        $backupDirectory = Join-Path $backupRoot $item.Label
        New-Item -ItemType Directory -Force -Path $backupDirectory | Out-Null
        $item.Backup = Join-Path $backupDirectory $item.Name
        Copy-Item -LiteralPath $item.Destination -Destination $item.Backup -Force
    }

    foreach ($item in $installItems) {
        Move-Item -LiteralPath $item.Staged -Destination $item.Destination -Force
        $item.Staged = $null
        $installed.Add([string]$item.Destination)
    }

    foreach ($item in $installItems) {
        $actual = (Get-FileHash -LiteralPath $item.Destination -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($actual -ne $item.ExpectedSha256) {
            throw "The installed release file failed SHA-256 verification: $($item.Label)\$($item.Name)"
        }
    }
}
catch {
    $installError = $_
    $rollbackItems = @($installItems)
    [array]::Reverse($rollbackItems)
    foreach ($item in $rollbackItems) {
        if ($item.Existed -and $null -ne $item.Backup -and (Test-Path -LiteralPath $item.Backup -PathType Leaf)) {
            Copy-Item -LiteralPath $item.Backup -Destination $item.Destination -Force
        }
        elseif (-not $item.Existed -and (Test-Path -LiteralPath $item.Destination -PathType Leaf)) {
            Remove-Item -LiteralPath $item.Destination -Force
        }
        if ($null -ne $item.Staged -and (Test-Path -LiteralPath $item.Staged -PathType Leaf)) {
            Remove-Item -LiteralPath $item.Staged -Force
        }
    }
    throw $installError
}

Write-Output "Verified GitHub Actions artifact from run $RunId was installed to $releaseDir and canonical launcher $tmRoot\tm.exe."
Get-FileHash -Algorithm SHA256 -LiteralPath ($installed | Where-Object { [System.IO.Path]::GetFileName($_) -in $requiredFiles })

[CmdletBinding(DefaultParameterSetName = 'Validate')]
param(
    [string]$DockerfilePath,
    [string]$PolicyPath,

    [Parameter(Mandatory = $true, ParameterSetName = 'Provenance')]
    [ValidatePattern('^[A-Za-z0-9._/:@-]+$')]
    [string]$ImageReference,

    [Parameter(Mandatory = $true, ParameterSetName = 'Provenance')]
    [ValidatePattern('^[0-9a-fA-F]{40}$')]
    [string]$CommitSha,

    [Parameter(Mandatory = $true, ParameterSetName = 'Provenance')]
    [ValidatePattern('^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$')]
    [string]$Repository,

    [Parameter(Mandatory = $true, ParameterSetName = 'Provenance')]
    [ValidatePattern('^[0-9a-fA-F]{64}$')]
    [string]$SourceArchiveSha256,

    [Parameter(Mandatory = $true, ParameterSetName = 'Provenance')]
    [ValidatePattern('^[0-9]+$')]
    [string]$WorkflowRunId,

    [Parameter(Mandatory = $true, ParameterSetName = 'Provenance')]
    [ValidateRange(1, [int]::MaxValue)]
    [int]$WorkflowRunAttempt,

    [Parameter(Mandatory = $true, ParameterSetName = 'Provenance')]
    [string]$GitRef,

    [Parameter(Mandatory = $true, ParameterSetName = 'Provenance')]
    [string]$EventName,

    [Parameter(Mandatory = $true, ParameterSetName = 'Provenance')]
    [string]$OutputDirectory
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$appRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if ([string]::IsNullOrWhiteSpace($DockerfilePath)) {
    $DockerfilePath = Join-Path $appRoot 'Dockerfile'
}
if ([string]::IsNullOrWhiteSpace($PolicyPath)) {
    $PolicyPath = Join-Path $appRoot 'container-supply-chain.lock.json'
}
$DockerfilePath = [System.IO.Path]::GetFullPath($DockerfilePath)
$PolicyPath = [System.IO.Path]::GetFullPath($PolicyPath)

function Get-Sha256 {
    param([Parameter(Mandatory = $true)][string]$Path)
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Assert-MatchCount {
    param(
        [Parameter(Mandatory = $true)][string]$Text,
        [Parameter(Mandatory = $true)][string]$Pattern,
        [Parameter(Mandatory = $true)][int]$Expected,
        [Parameter(Mandatory = $true)][string]$Message
    )
    $count = [System.Text.RegularExpressions.Regex]::Matches(
        $Text,
        $Pattern,
        [System.Text.RegularExpressions.RegexOptions]::Multiline
    ).Count
    if ($count -ne $Expected) {
        throw "$Message Expected $Expected occurrence(s), found $count."
    }
}

if (-not (Test-Path -LiteralPath $DockerfilePath -PathType Leaf) -or
    -not (Test-Path -LiteralPath $PolicyPath -PathType Leaf)) {
    throw 'The reviewed Dockerfile or container supply-chain lock is missing.'
}

$dockerfile = [System.IO.File]::ReadAllText($DockerfilePath, [System.Text.Encoding]::UTF8)
$policy = [System.IO.File]::ReadAllText($PolicyPath, [System.Text.Encoding]::UTF8) | ConvertFrom-Json
if ([int]$policy.schemaVersion -ne 1 -or
    [string]$policy.platform -ne 'linux/amd64' -or
    [string]$policy.rustToolchain -ne '1.97.0' -or
    [string]$policy.debianSnapshot -notmatch '^20[0-9]{6}T[0-9]{6}Z$') {
    throw 'The container supply-chain lock has an unsupported schema or platform policy.'
}

$stages = @($policy.builder, $policy.runtime)
foreach ($stage in $stages) {
    if ([string]$stage.stage -notin @('builder', 'runtime') -or
        [string]$stage.image -notmatch '^[a-z0-9][a-z0-9._/:\-]+$' -or
        [string]$stage.manifestDigest -notmatch '^sha256:[0-9a-f]{64}$' -or
        [string]$stage.platformManifestDigest -notmatch '^sha256:[0-9a-f]{64}$') {
        throw 'The container supply-chain lock contains an invalid base image reference.'
    }
}
if ([string]$policy.builder.stage -eq [string]$policy.runtime.stage) {
    throw 'The container supply-chain lock contains duplicate stages.'
}

$allFrom = [System.Text.RegularExpressions.Regex]::Matches($dockerfile, '(?im)^\s*FROM\s+')
$pinnedFrom = [System.Text.RegularExpressions.Regex]::Matches(
    $dockerfile,
    '(?im)^FROM\s+--platform=linux/amd64\s+(?<image>[a-z0-9][a-z0-9._/:\-]+)@(?<digest>sha256:[0-9a-f]{64})\s+AS\s+(?<stage>builder|runtime)\s*$'
)
if ($allFrom.Count -ne 2 -or $pinnedFrom.Count -ne 2) {
    throw 'Dockerfile must contain exactly two linux/amd64 base images pinned by sha256 manifest digest.'
}
foreach ($stage in $stages) {
    $match = @($pinnedFrom | Where-Object { $_.Groups['stage'].Value -eq [string]$stage.stage })
    if ($match.Count -ne 1 -or
        $match[0].Groups['image'].Value -cne [string]$stage.image -or
        $match[0].Groups['digest'].Value -cne [string]$stage.manifestDigest) {
        throw "Dockerfile base image for stage '$($stage.stage)' does not match the reviewed lock."
    }
}

if ($dockerfile -match '(?im)^\s*ARG\s+DEBIAN_SNAPSHOT(?:=|\s|$)' -or
    $dockerfile -match '\$\{?DEBIAN_SNAPSHOT\}?') {
    throw 'The Debian snapshot must not be overridable through a Docker build argument.'
}

$snapshot = [string]$policy.debianSnapshot
$sources = @(
    "deb [check-valid-until=no] https://snapshot.debian.org/archive/debian/$snapshot/ bookworm main",
    "deb [check-valid-until=no] https://snapshot.debian.org/archive/debian/$snapshot/ bookworm-updates main",
    "deb [check-valid-until=no] https://snapshot.debian.org/archive/debian-security/$snapshot/ bookworm-security main"
)
$allDebLines = [System.Text.RegularExpressions.Regex]::Matches(
    $dockerfile,
    '(?m)^\s*"deb \[check-valid-until=no\] https://[^"\r\n]+"\s*\\\s*$'
)
$anyDebLines = [System.Text.RegularExpressions.Regex]::Matches(
    $dockerfile,
    '(?im)^\s*["'']?deb\s+'
)
if ($allDebLines.Count -ne 6 -or $anyDebLines.Count -ne 6) {
    throw 'Dockerfile must define only the three reviewed frozen Debian sources in both stages.'
}
foreach ($source in $sources) {
    Assert-MatchCount -Text $dockerfile -Pattern ([regex]::Escape('"' + $source + '"') + '\s*\\\s*$') `
        -Expected 2 -Message "Dockerfile is missing the frozen source '$source'."
}
if ($dockerfile -match '(?i)deb\.debian\.org|security\.debian\.org') {
    throw 'Dockerfile contains a mutable Debian mirror.'
}

Assert-MatchCount -Text $dockerfile -Pattern 'rm -rf /etc/apt/sources\.list\.d/\*' -Expected 2 `
    -Message 'Dockerfile must remove all inherited apt source fragments.'
Assert-MatchCount -Text $dockerfile -Pattern 'apt-get upgrade --yes --no-install-recommends' -Expected 2 `
    -Message 'Dockerfile must align installed base packages with the frozen snapshot.'
Assert-MatchCount -Text $dockerfile -Pattern ([regex]::Escape('COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt')) `
    -Expected 1 -Message 'Runtime HTTPS bootstrap must copy the CA bundle from the pinned builder.'
Assert-MatchCount -Text $dockerfile -Pattern ([regex]::Escape('RUN rustup component add --toolchain 1.97.0 rustfmt clippy')) `
    -Expected 1 -Message 'Rust components must target the locked toolchain explicitly.'

Write-Host "STEP 16 container inputs match the reviewed lock: $($policy.platform), snapshot $snapshot." -ForegroundColor Green

if ($PSCmdlet.ParameterSetName -ne 'Provenance') {
    return
}

$docker = Get-Command docker -ErrorAction SilentlyContinue
if (-not $docker) {
    throw 'Docker is required to write container provenance.'
}
$CommitSha = $CommitSha.ToLowerInvariant()
$SourceArchiveSha256 = $SourceArchiveSha256.ToLowerInvariant()
$OutputDirectory = [System.IO.Path]::GetFullPath($OutputDirectory)
New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null

$inspectJson = & $docker.Source image inspect $ImageReference
if ($LASTEXITCODE -ne 0) {
    throw 'The reviewed CI image could not be inspected.'
}
$images = @($inspectJson | ConvertFrom-Json)
if ($images.Count -ne 1 -or
    [string]$images[0].Id -notmatch '^sha256:[0-9a-f]{64}$' -or
    [string]$images[0].Os -ne 'linux' -or
    [string]$images[0].Architecture -ne 'amd64') {
    throw 'The reviewed CI image has an invalid digest or platform.'
}
$labels = $images[0].Config.Labels
$revisionProperty = if ($null -eq $labels) { $null } else { $labels.PSObject.Properties['org.opencontainers.image.revision'] }
$sourceProperty = if ($null -eq $labels) { $null } else { $labels.PSObject.Properties['org.opencontainers.image.source'] }
$revision = if ($null -eq $revisionProperty) { $null } else { $revisionProperty.Value }
$source = if ($null -eq $sourceProperty) { $null } else { $sourceProperty.Value }
if ([string]$revision -cne $CommitSha -or [string]$source -cne "https://github.com/$Repository") {
    throw 'The reviewed CI image is not labeled with the exact source commit and repository.'
}

$resolvedBases = foreach ($stage in $stages) {
    $reference = "$($stage.image)@$($stage.manifestDigest)"
    $rawIndex = & $docker.Source buildx imagetools inspect --raw $reference
    if ($LASTEXITCODE -ne 0) {
        throw "Unable to resolve the reviewed base image '$reference'."
    }
    $index = $rawIndex | ConvertFrom-Json
    $resolvedDigest = [string]$stage.manifestDigest
    $manifestsProperty = $index.PSObject.Properties['manifests']
    if ($null -ne $manifestsProperty) {
        $candidates = @($index.manifests | Where-Object {
            $variantProperty = $_.platform.PSObject.Properties['variant']
            [string]$_.platform.os -eq 'linux' -and
            [string]$_.platform.architecture -eq 'amd64' -and
            ($null -eq $variantProperty -or [string]$variantProperty.Value -eq '')
        })
        if ($candidates.Count -ne 1 -or [string]$candidates[0].digest -notmatch '^sha256:[0-9a-f]{64}$') {
            throw "The reviewed base image '$reference' has no unique linux/amd64 manifest."
        }
        $resolvedDigest = [string]$candidates[0].digest
    }
    if ($resolvedDigest -cne [string]$stage.platformManifestDigest) {
        throw "The resolved linux/amd64 manifest for '$reference' does not match the reviewed lock."
    }
    [ordered]@{
        stage = [string]$stage.stage
        image = [string]$stage.image
        indexDigest = [string]$stage.manifestDigest
        platform = [string]$policy.platform
        platformManifestDigest = [string]$stage.platformManifestDigest
    }
}

$contextRoot = Split-Path -Parent $DockerfilePath
$cargoLockPath = Join-Path $contextRoot 'Cargo.lock'
$dockerIgnorePath = Join-Path $contextRoot '.dockerignore'
if (-not (Test-Path -LiteralPath $cargoLockPath -PathType Leaf) -or
    -not (Test-Path -LiteralPath $dockerIgnorePath -PathType Leaf)) {
    throw 'The exact build context is missing Cargo.lock or .dockerignore.'
}

$dockerVersion = (& $docker.Source version --format '{{.Client.Version}}').Trim()
if ($LASTEXITCODE -ne 0) { throw 'Docker client version could not be recorded.' }
$buildxVersion = (& $docker.Source buildx version).Trim()
if ($LASTEXITCODE -ne 0) { throw 'Docker Buildx version could not be recorded.' }

$provenance = [ordered]@{
    schemaVersion = 1
    kind = 'tm-step16-container-provenance'
    generatedAtUtc = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ss.fffZ')
    attestationScope = 'ci-built-image-only'
    railwayProductionImageRelationship = 'independent-rebuild-requires-separate-deployment-receipt'
    source = [ordered]@{
        repository = $Repository
        commitSha = $CommitSha
        gitRef = $GitRef
        eventName = $EventName
        workflowRunId = $WorkflowRunId
        workflowRunAttempt = $WorkflowRunAttempt
        sourceArchiveSha256 = $SourceArchiveSha256
    }
    inputs = [ordered]@{
        platform = [string]$policy.platform
        rustToolchain = [string]$policy.rustToolchain
        debianSnapshot = $snapshot
        dockerfileSha256 = Get-Sha256 $DockerfilePath
        dockerIgnoreSha256 = Get-Sha256 $dockerIgnorePath
        cargoLockSha256 = Get-Sha256 $cargoLockPath
        supplyChainLockSha256 = Get-Sha256 $PolicyPath
        baseImages = @($resolvedBases)
    }
    ciImage = [ordered]@{
        reference = $ImageReference
        configDigest = [string]$images[0].Id
        digestType = 'docker-image-config'
        os = [string]$images[0].Os
        architecture = [string]$images[0].Architecture
        revisionLabel = [string]$revision
        sourceLabel = [string]$source
    }
    tooling = [ordered]@{
        dockerVersion = $dockerVersion
        buildxVersion = $buildxVersion
    }
    productionDeployment = [ordered]@{
        imageDigest = $null
        deploymentId = $null
        receiptRequired = $true
        equalityWithCiConfigDigestExpected = $false
    }
}

$provenancePath = Join-Path $OutputDirectory 'container-provenance.json'
$provenance | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $provenancePath -Encoding utf8
$inspectJson | Set-Content -LiteralPath (Join-Path $OutputDirectory 'image-inspect.json') -Encoding utf8
Write-Host "STEP 16 container provenance written: $provenancePath" -ForegroundColor Green

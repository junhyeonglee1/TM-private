[CmdletBinding(SupportsShouldProcess = $true, ConfirmImpact = 'High')]
param(
    [ValidateSet('Phase1', 'Phase2')]
    [string]$Phase = 'Phase1',
    [string]$ExpectedHeadSha,
    [long]$Step10RunId,
    [long]$Step16RunId,
    [string]$PhaseOneReceiptPath,
    [string]$PhaseTwoResultPath,
    [switch]$ApproveActivation,
    [switch]$RunGuardSelfTest,
    [switch]$Apply,
    [switch]$Force
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
Add-Type -AssemblyName System.Net.Http
. (Join-Path $PSScriptRoot 'expense-release-evidence.ps1')

$repository = 'junhyeonglee1/TM-private'
$expectedBranch = 'agent/step10-cloud-cutover'
$step10WorkflowName = 'STEP 10 Windows build'
$step10WorkflowPath = '.github/workflows/windows-step10-build.yml'
$step10ArtifactName = 'tm-step10-windows-x64'
$step16WorkflowName = 'STEP 16 security'
$step16WorkflowPath = '.github/workflows/step16-security.yml'
$step16ArtifactName = 'tm-step16-container-provenance'
$projectId = '7fcb22b5-db34-4e2b-a12a-cbc60391ff5f'
$environment = 'production'
$service = 'tm-server'
$baseUri = 'https://tm-server-production-5573.up.railway.app'
$tokenCredentialResource = 'TM Cloud Production'
$tokenCredentialUser = 'single-user'
$mutexName = 'Local\TMExpenseProductionKeyConfiguration'
$phaseOneReceiptKind = 'tm-expense-classification-phase1'
$phaseTwoReceiptKind = 'tm-expense-classification-phase2'
$receiptVersion = 1

$appRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$tmRoot = [System.IO.Path]::GetFullPath((Join-Path $appRoot '..'))
$distRoot = [System.IO.Path]::GetFullPath((Join-Path $tmRoot 'dist'))
$rolloutRoot = Join-Path $distRoot 'manual-expense-classification-deployment'
if ([string]::IsNullOrWhiteSpace($PhaseOneReceiptPath)) {
    $PhaseOneReceiptPath = Join-Path $rolloutRoot 'phase1-receipt.json'
}
if ([string]::IsNullOrWhiteSpace($PhaseTwoResultPath)) {
    $PhaseTwoResultPath = Join-Path $rolloutRoot 'phase2-result.json'
}
$PhaseOneReceiptPath = [System.IO.Path]::GetFullPath($PhaseOneReceiptPath)
$PhaseTwoResultPath = [System.IO.Path]::GetFullPath($PhaseTwoResultPath)
$failureResultPath = Join-Path $rolloutRoot ("{0}-failure.json" -f $Phase.ToLowerInvariant())
$verifierPath = Join-Path $PSScriptRoot 'verify-expense-classification-production.ps1'

function Assert-PathUnderDist {
    param([Parameter(Mandatory = $true)][string]$Path)

    $root = $distRoot.TrimEnd('\') + '\'
    $full = [System.IO.Path]::GetFullPath($Path)
    if (-not $full.StartsWith($root, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw 'Classification rollout output paths must remain under the TM dist directory.'
    }
}

function Assert-BooleanValue {
    param(
        [AllowNull()]$Value,
        [Parameter(Mandatory = $true)][bool]$Expected,
        [Parameter(Mandatory = $true)][string]$Name
    )

    if ($Value -isnot [bool] -or $Value -ne $Expected) {
        throw "$Name must be the Boolean value $Expected."
    }
}

function Get-ProductionToken {
    $vault = [Windows.Security.Credentials.PasswordVault,Windows.Security.Credentials,ContentType=WindowsRuntime]::new()
    $credential = $null
    try {
        $credential = $vault.Retrieve($tokenCredentialResource, $tokenCredentialUser)
        $credential.RetrievePassword()
        $token = [string]$credential.Password
        if ($token -notmatch '^tm_pat_v1_[A-Za-z0-9_-]{43}$') {
            throw 'The Credential Locker TM production token has an invalid format.'
        }
        return $token
    }
    finally {
        $credential = $null
        $vault = $null
    }
}

function Invoke-ProductionOperationsStatus {
    $token = $null
    $handler = $null
    $client = $null
    $request = $null
    $response = $null
    try {
        $token = Get-ProductionToken
        $handler = [System.Net.Http.HttpClientHandler]::new()
        $handler.AllowAutoRedirect = $false
        $handler.UseCookies = $false
        $client = [System.Net.Http.HttpClient]::new($handler)
        $client.Timeout = [TimeSpan]::FromSeconds(30)
        $request = [System.Net.Http.HttpRequestMessage]::new(
            [System.Net.Http.HttpMethod]::Get,
            "$baseUri/api/v1/ops/status"
        )
        $request.Headers.Authorization =
            [System.Net.Http.Headers.AuthenticationHeaderValue]::new('Bearer', $token)
        $response = $client.SendAsync($request).GetAwaiter().GetResult()
        if ([int]$response.StatusCode -ne 200) {
            throw "Production operations status returned HTTP $([int]$response.StatusCode)."
        }
        $cacheControl = ''
        if ($response.Headers.Contains('Cache-Control')) {
            $cacheControl = [string]::Join(',', $response.Headers.GetValues('Cache-Control'))
        }
        if ($cacheControl -notmatch '(?i)(^|,|\s)no-store($|,|\s)') {
            throw 'Production operations status did not return Cache-Control: no-store.'
        }
        $contentLength = $response.Content.Headers.ContentLength
        if ($null -ne $contentLength -and [long]$contentLength -gt 1048576) {
            throw 'Production operations status exceeded the fixed response limit.'
        }
        $bytes = $response.Content.ReadAsByteArrayAsync().GetAwaiter().GetResult()
        if ($bytes.Length -gt 1048576) {
            throw 'Production operations status exceeded the fixed response limit.'
        }
        try {
            $envelope = [System.Text.Encoding]::UTF8.GetString($bytes) | ConvertFrom-TmJson
        }
        catch {
            throw 'Production operations status returned invalid JSON.'
        }
        if ($null -eq $envelope -or $null -eq $envelope.data) {
            throw 'Production operations status returned an invalid envelope.'
        }
        return $envelope.data
    }
    finally {
        if ($null -ne $response) { $response.Dispose() }
        if ($null -ne $request) { $request.Dispose() }
        if ($null -ne $client) { $client.Dispose() }
        if ($null -ne $handler) { $handler.Dispose() }
        $token = $null
        Remove-Variable token -ErrorAction SilentlyContinue
    }
}

function Assert-PhaseOneProductionPreflight {
    param([Parameter(Mandatory = $true)]$Operations)

    if ($null -eq $Operations.database -or
        $null -eq $Operations.controls -or
        $null -eq $Operations.remoteBackup -or
        $null -eq $Operations.deploymentProvenance -or
        $null -eq $Operations.objectives) {
        throw 'Production did not return the complete schema 15 rollout proof.'
    }
    Assert-BooleanValue $Operations.database.ok $true 'database.ok'
    Assert-BooleanValue $Operations.controls.aiEnabled $true 'controls.aiEnabled'
    Assert-BooleanValue $Operations.controls.expenseAiEnabled $false 'controls.expenseAiEnabled'
    Assert-BooleanValue $Operations.controls.expenseCryptoReady $true 'controls.expenseCryptoReady'
    Assert-BooleanValue $Operations.controls.expenseExpectedKeyFingerprintMatch $true 'controls.expenseExpectedKeyFingerprintMatch'
    Assert-BooleanValue $Operations.controls.expenseActivationFingerprintMatch $true 'controls.expenseActivationFingerprintMatch'
    foreach ($name in @(
        'migrationLedgerComplete',
        'requiredTablesComplete',
        'schemaSemanticsValidated',
        'expenseCryptoProbePresent'
    )) {
        Assert-BooleanValue $Operations.remoteBackup.$name $true "remoteBackup.$name"
    }
    $previousDeploymentId = [Guid]::Empty
    if ([int]$Operations.database.schemaVersion -ne 15 -or
        [string]$Operations.controls.incidentMode -cne 'normal' -or
        [string]$Operations.controls.expenseRolloutMode -cne 'enabled' -or
        [string]$Operations.remoteBackup.status -cne 'succeeded' -or
        [int]$Operations.remoteBackup.schemaVersion -ne 15 -or
        [string]$Operations.remoteBackup.integrityCheck -cne 'ok' -or
        [string]$Operations.remoteBackup.expenseCryptoProbeSha256 -notmatch '^[0-9a-f]{64}$' -or
        [string]$Operations.remoteBackup.expenseCryptoProbeSha256 -cne
            [string]$Operations.controls.expenseCryptoProbeSha256 -or
        [string]$Operations.deploymentProvenance.buildCommitSha -notmatch '^[0-9a-fA-F]{40}$' -or
        -not [Guid]::TryParse(
            [string]$Operations.deploymentProvenance.railwayDeploymentId,
            [ref]$previousDeploymentId
        )) {
        throw 'Production schema 15, expense controls, backup, or deployment provenance is not ready.'
    }
    $checkedAt = [DateTimeOffset]::MinValue
    if (-not [DateTimeOffset]::TryParse(
            [string]$Operations.remoteBackup.checkedAt,
            [System.Globalization.CultureInfo]::InvariantCulture,
            [System.Globalization.DateTimeStyles]::RoundtripKind,
            [ref]$checkedAt
        )) {
        throw 'The schema 15 remote backup timestamp is invalid.'
    }
    $freshnessHours = [double]$Operations.objectives.backupFreshnessTargetHours
    $backupAge = [DateTimeOffset]::UtcNow - $checkedAt.ToUniversalTime()
    if ($freshnessHours -le 0 -or $freshnessHours -gt 24 -or
        $backupAge.TotalMinutes -lt -5 -or $backupAge.TotalHours -gt $freshnessHours -or
        [string]$Operations.remoteBackup.snapshotId -notmatch '^[A-Za-z0-9][A-Za-z0-9._-]{0,255}$' -or
        [string]$Operations.remoteBackup.databaseSha256 -notmatch '^[0-9a-fA-F]{64}$' -or
        [uint64]$Operations.remoteBackup.databaseByteSize -lt 1) {
        throw 'The schema 15 remote backup is stale or lacks immutable evidence.'
    }
    $criticalAlerts = @($Operations.alerts | Where-Object {
        [string]$_.severity -ceq 'critical'
    })
    if ([string]$Operations.overallStatus -ceq 'critical' -or $criticalAlerts.Count -ne 0) {
        throw 'Production has a critical alert and cannot begin schema 16 migration.'
    }
    return [pscustomobject]@{
        schemaVersion = 15
        deploymentId = $previousDeploymentId.ToString('D')
        buildCommitSha = ([string]$Operations.deploymentProvenance.buildCommitSha).ToLowerInvariant()
        backupCheckedAt = [string]$Operations.remoteBackup.checkedAt
        backupSnapshotId = [string]$Operations.remoteBackup.snapshotId
        backupDatabaseSha256 = ([string]$Operations.remoteBackup.databaseSha256).ToLowerInvariant()
        backupDatabaseByteSize = [uint64]$Operations.remoteBackup.databaseByteSize
    }
}

function Assert-PhaseOnePreflightUnchanged {
    param(
        [Parameter(Mandatory = $true)]$Expected,
        [Parameter(Mandatory = $true)]$Actual
    )

    if ([string]$Expected.deploymentId -cne [string]$Actual.deploymentId -or
        [string]$Expected.buildCommitSha -cne [string]$Actual.buildCommitSha -or
        [string]$Expected.backupSnapshotId -cne [string]$Actual.backupSnapshotId -or
        [string]$Expected.backupDatabaseSha256 -cne [string]$Actual.backupDatabaseSha256 -or
        [uint64]$Expected.backupDatabaseByteSize -ne [uint64]$Actual.backupDatabaseByteSize) {
        throw 'Production changed while the exact schema 16 source was prepared.'
    }
}

function Get-RailwayDeployments {
    param([Parameter(Mandatory = $true)][string]$RailwayPath)

    $arguments = [string[]]@(
        'deployment', 'list', '--json', '--limit', '20',
        '--project', $projectId,
        '--environment', $environment,
        '--service', $service
    )
    $processParameters = @{
        FilePath = $RailwayPath
        Arguments = $arguments
        TimeoutSeconds = 60
        MaximumCapturedCharacters = 1048576
    }
    $result = Invoke-TmBoundedProcess @processParameters
    if ($result.ExitCode -ne 0) {
        throw 'Unable to list Railway production deployments.'
    }
    try {
        return @($result.StandardOutput | ConvertFrom-TmJsonArrayItems)
    }
    catch {
        throw 'Railway returned invalid deployment metadata JSON.'
    }
}

function Add-DeploymentIdsFromJsonValue {
    param(
        [AllowNull()]$Value,
        [Parameter(Mandatory = $true)]
        [AllowEmptyCollection()]
        [System.Collections.Generic.HashSet[string]]$Ids
    )

    if ($null -eq $Value -or $Value -is [string] -or $Value -is [ValueType]) {
        return
    }
    if ($Value -is [System.Collections.IEnumerable] -and
        $Value -isnot [System.Management.Automation.PSCustomObject]) {
        foreach ($item in $Value) {
            Add-DeploymentIdsFromJsonValue -Value $item -Ids $Ids
        }
        return
    }
    foreach ($property in $Value.PSObject.Properties) {
        if ([string]$property.Name -ceq 'deploymentId') {
            $parsedId = [Guid]::Empty
            if ([Guid]::TryParse([string]$property.Value, [ref]$parsedId)) {
                $null = $Ids.Add($parsedId.ToString('D'))
            }
        }
        Add-DeploymentIdsFromJsonValue -Value $property.Value -Ids $Ids
    }
}

function Get-DeploymentIdFromUpload {
    param([Parameter(Mandatory = $true)][object[]]$OutputLines)

    $ids = [System.Collections.Generic.HashSet[string]]::new(
        [System.StringComparer]::OrdinalIgnoreCase
    )
    foreach ($line in $OutputLines) {
        $text = [string]$line
        if ([string]::IsNullOrWhiteSpace($text) -or $text.Length -gt 1048576) {
            continue
        }
        try {
            $value = $text | ConvertFrom-TmJson
        }
        catch {
            continue
        }
        Add-DeploymentIdsFromJsonValue -Value $value -Ids $ids
    }
    if ($ids.Count -ne 1) {
        throw "Railway upload output did not prove exactly one deploymentId (found $($ids.Count))."
    }
    return @($ids)[0]
}

function Wait-VerifiedRailwayDeployment {
    param(
        [Parameter(Mandatory = $true)][string]$RailwayPath,
        [Parameter(Mandatory = $true)][string]$DeploymentId,
        [Parameter(Mandatory = $true)][string]$Message
    )

    $deadline = [DateTimeOffset]::UtcNow.AddMinutes(20)
    while ([DateTimeOffset]::UtcNow -lt $deadline) {
        $deployments = @(Get-RailwayDeployments -RailwayPath $RailwayPath)
        $matching = @($deployments | Where-Object {
            ([string]$_.id).ToLowerInvariant() -ceq $DeploymentId.ToLowerInvariant()
        })
        if ($matching.Count -eq 1) {
            $deployment = $matching[0]
            if ([string]$deployment.meta.cliMessage -cne $Message) {
                throw 'The Railway deployment message does not match the exact verified commit.'
            }
            $status = ([string]$deployment.status).ToUpperInvariant()
            if ($status -in @('FAILED', 'CRASHED', 'REMOVED')) {
                throw "Railway deployment $DeploymentId ended with $status."
            }
            if ($status -ceq 'SUCCESS') {
                $latest = @(
                    $deployments |
                        Sort-Object { [DateTimeOffset]$_.createdAt } -Descending |
                        Select-Object -First 1
                )
                if ($latest.Count -ne 1 -or
                    ([string]$latest[0].id).ToLowerInvariant() -cne
                        $DeploymentId.ToLowerInvariant()) {
                    throw 'The successful deployment is not the latest active Railway deployment.'
                }
                if ([string]$deployment.meta.imageDigest -notmatch '^sha256:[0-9a-fA-F]{64}$') {
                    throw 'The successful Railway deployment has no immutable image digest.'
                }
                return $deployment
            }
        }
        elseif ($matching.Count -gt 1) {
            throw 'Railway returned duplicate deployment IDs.'
        }
        Start-Sleep -Seconds 5
    }
    throw "Railway deployment $DeploymentId did not reach SUCCESS within 20 minutes."
}

function Get-LatestDeployment {
    param([Parameter(Mandatory = $true)][string]$RailwayPath)

    $deployments = @(Get-RailwayDeployments -RailwayPath $RailwayPath)
    $seen = [System.Collections.Generic.HashSet[string]]::new(
        [System.StringComparer]::OrdinalIgnoreCase
    )
    $valid = [System.Collections.Generic.List[object]]::new()
    foreach ($deployment in $deployments) {
        $id = [Guid]::Empty
        $createdAt = [DateTimeOffset]::MinValue
        if (-not [Guid]::TryParse([string]$deployment.id, [ref]$id) -or
            -not [DateTimeOffset]::TryParse(
                [string]$deployment.createdAt,
                [System.Globalization.CultureInfo]::InvariantCulture,
                [System.Globalization.DateTimeStyles]::RoundtripKind,
                [ref]$createdAt
            ) -or -not $seen.Add($id.ToString('D'))) {
            throw 'Railway returned an invalid or duplicate deployment timeline entry.'
        }
        $valid.Add([pscustomobject]@{
            deployment = $deployment
            createdAt = $createdAt.ToUniversalTime()
        })
    }
    $latest = @(
        $valid |
            Sort-Object createdAt -Descending |
            Select-Object -First 1
    )
    if ($latest.Count -ne 1) {
        throw 'Railway did not return one latest production deployment.'
    }
    return $latest[0].deployment
}

function Assert-ExactDeployment {
    param(
        [Parameter(Mandatory = $true)]$Deployment,
        [Parameter(Mandatory = $true)][string]$DeploymentId,
        [Parameter(Mandatory = $true)][string]$Message,
        [string]$ExpectedImageDigest
    )

    $parsed = [Guid]::Empty
    if (-not [Guid]::TryParse([string]$Deployment.id, [ref]$parsed) -or
        $parsed.ToString('D') -cne $DeploymentId -or
        [string]$Deployment.status -cne 'SUCCESS' -or
        [string]$Deployment.meta.cliMessage -cne $Message -or
        [string]$Deployment.meta.imageDigest -notmatch '^sha256:[0-9a-fA-F]{64}$') {
        throw 'The Railway deployment is not the exact successful rollout deployment.'
    }
    if (-not [string]::IsNullOrWhiteSpace($ExpectedImageDigest) -and
        ([string]$Deployment.meta.imageDigest).ToLowerInvariant() -cne
            $ExpectedImageDigest.ToLowerInvariant()) {
        throw 'The Railway deployment image digest changed from the protected receipt.'
    }
}

function New-VerifiedSourceStage {
    param(
        [Parameter(Mandatory = $true)][string]$GitPath,
        [Parameter(Mandatory = $true)][string]$HeadSha
    )

    New-Item -ItemType Directory -Force -Path $rolloutRoot | Out-Null
    $id = [Guid]::NewGuid().ToString('N')
    $archivePath = Join-Path $rolloutRoot ("source-{0}-{1}.zip" -f $HeadSha, $id)
    $stageRoot = Join-Path $rolloutRoot ("staging-{0}" -f $id)
    $arguments = @(
        '-C', $tmRoot,
        'archive', '--format=zip', "--output=$archivePath",
        "$HeadSha^{commit}", 'app'
    )
    $archiveParameters = @{
        GitPath = $GitPath
        Arguments = $arguments
        FailureMessage = 'Git could not create the exact schema 16 source archive.'
        AllowEmpty = $true
    }
    $null = Invoke-TmVerifiedGitText @archiveParameters
    if (-not (Test-Path -LiteralPath $archivePath -PathType Leaf)) {
        throw 'Git did not create the exact schema 16 source archive.'
    }
    $archiveSha256 = (
        Get-FileHash -LiteralPath $archivePath -Algorithm SHA256
    ).Hash.ToLowerInvariant()
    Expand-TmSafeSourceArchive -ZipPath $archivePath -DestinationRoot $stageRoot
    $stagedAppRoot = [System.IO.Path]::GetFullPath((Join-Path $stageRoot 'app'))
    if (-not (Test-Path -LiteralPath (Join-Path $stagedAppRoot 'Dockerfile') -PathType Leaf) -or
        -not (Test-Path -LiteralPath (Join-Path $stagedAppRoot 'railway.json') -PathType Leaf)) {
        throw 'The exact source archive lacks its reviewed Railway build inputs.'
    }
    return [pscustomobject]@{
        archivePath = $archivePath
        archiveSha256 = $archiveSha256
        stageRoot = $stageRoot
        stagedAppRoot = $stagedAppRoot
        stagedManifestSha256 = Get-TmDirectoryManifestSha256 -Root $stageRoot
    }
}

function Assert-SourceStageUnchanged {
    param([Parameter(Mandatory = $true)]$Source)

    $archiveHash = (
        Get-FileHash -LiteralPath $Source.archivePath -Algorithm SHA256
    ).Hash.ToLowerInvariant()
    $manifestHash = Get-TmDirectoryManifestSha256 -Root $Source.stageRoot
    if ($archiveHash -cne [string]$Source.archiveSha256 -or
        $manifestHash -cne [string]$Source.stagedManifestSha256) {
        throw 'The exact source archive or staging tree changed before upload.'
    }
}

function Set-ClassificationControl {
    param(
        [Parameter(Mandatory = $true)][string]$RailwayPath,
        [Parameter(Mandatory = $true)]
        [ValidateSet('true', 'false')]
        [string]$Enabled
    )

    $arguments = [string[]]@(
        'variable', 'set',
        "TM_EXPENSE_CLASSIFICATION_AI_ENABLED=$Enabled",
        'TM_EXPENSE_AI_ENABLED=false',
        "TM_BUILD_COMMIT_SHA=$ExpectedHeadSha",
        '--skip-deploys',
        '--project', $projectId,
        '--environment', $environment,
        '--service', $service
    )
    $parameters = @{
        FilePath = $RailwayPath
        Arguments = $arguments
        TimeoutSeconds = 120
        DiscardOutput = $true
    }
    $result = Invoke-TmBoundedProcess @parameters
    if ($result.ExitCode -ne 0) {
        throw "Railway rejected the fail-closed expense classification control ($Enabled)."
    }
}

function Start-ExactSourceDeployment {
    param(
        [Parameter(Mandatory = $true)][string]$RailwayPath,
        [Parameter(Mandatory = $true)]$Source,
        [Parameter(Mandatory = $true)][string]$Message
    )

    Assert-SourceStageUnchanged -Source $Source
    $arguments = [string[]]@(
        'up', '--detach', '--json', '--yes', '--message', $Message,
        '--project', $projectId,
        '--environment', $environment,
        '--service', $service
    )
    $parameters = @{
        FilePath = $RailwayPath
        WorkingDirectory = $Source.stagedAppRoot
        Arguments = $arguments
        TimeoutSeconds = 300
        MaximumCapturedCharacters = 1048576
    }
    $result = Invoke-TmBoundedProcess @parameters
    if ($result.ExitCode -ne 0) {
        throw 'Railway rejected the exact Actions-verified schema 16 source upload.'
    }
    $lines = @(
        @($result.StandardOutput -split '\r?\n') +
        @($result.StandardError -split '\r?\n')
    )
    $deploymentId = Get-DeploymentIdFromUpload -OutputLines $lines
    Assert-SourceStageUnchanged -Source $Source
    return $deploymentId
}

function Read-ClassificationVerification {
    param([Parameter(Mandatory = $true)][string]$Path)

    $result = Read-TmBoundedJsonFile -Path $Path -MaximumBytes 1048576
    Assert-TmExactJsonProperties $result @(
        'success', 'verifiedAtUtc', 'headSha', 'deploymentId', 'schemaVersion',
        'classificationEnabled', 'expenseCommentaryEnabled',
        'remoteBackupStatus', 'remoteBackupSchemaVersion',
        'remoteBackupIntegrityCheck', 'remoteBackupSchemaSemanticsValidated',
        'preMigrationSchemaVersion', 'classificationOpen',
        'classificationLatestStatus', 'classificationBudget',
        'secretValuesWritten', 'readOnlyVerification'
    ) 'expense classification verification'
    return $result
}

function Assert-ClassificationVerification {
    param(
        [Parameter(Mandatory = $true)]$Verification,
        [Parameter(Mandatory = $true)][string]$DeploymentId,
        [Parameter(Mandatory = $true)][bool]$ExpectedEnabled
    )

    Assert-BooleanValue $Verification.success $true 'verification.success'
    Assert-BooleanValue $Verification.classificationEnabled $ExpectedEnabled 'verification.classificationEnabled'
    Assert-BooleanValue $Verification.expenseCommentaryEnabled $false 'verification.expenseCommentaryEnabled'
    Assert-BooleanValue $Verification.remoteBackupSchemaSemanticsValidated $true 'verification.remoteBackupSchemaSemanticsValidated'
    Assert-BooleanValue $Verification.secretValuesWritten $false 'verification.secretValuesWritten'
    Assert-BooleanValue $Verification.readOnlyVerification $true 'verification.readOnlyVerification'
    $parsedDeploymentId = [Guid]::Empty
    if (([string]$Verification.headSha).ToLowerInvariant() -cne $ExpectedHeadSha -or
        -not [Guid]::TryParse([string]$Verification.deploymentId, [ref]$parsedDeploymentId) -or
        $parsedDeploymentId.ToString('D') -cne $DeploymentId -or
        [int]$Verification.schemaVersion -ne 16 -or
        [int]$Verification.preMigrationSchemaVersion -ne 15 -or
        [string]$Verification.remoteBackupStatus -cne 'succeeded' -or
        [int]$Verification.remoteBackupSchemaVersion -ne 16 -or
        [string]$Verification.remoteBackupIntegrityCheck -cne 'ok' -or
        [int64]$Verification.classificationOpen.claimedCount -ne 0 -or
        [int64]$Verification.classificationOpen.stagedCount -ne 0 -or
        $null -ne $Verification.classificationOpen.oldestOpenCreatedAt) {
        throw 'The read-only schema 16 verification is not bound to the exact safe deployment.'
    }
}

function Invoke-ClassificationVerifier {
    param(
        [Parameter(Mandatory = $true)][string]$DeploymentId,
        [Parameter(Mandatory = $true)]
        [ValidateSet('true', 'false')]
        [string]$ExpectedEnabled,
        [Parameter(Mandatory = $true)][string]$OutputPath
    )

    if (Test-Path -LiteralPath $OutputPath -PathType Leaf) {
        Remove-Item -LiteralPath $OutputPath -Force
    }
    $parameters = @{
        ExpectedHeadSha = $ExpectedHeadSha
        ExpectedDeploymentId = $DeploymentId
        ExpectedClassificationEnabled = $ExpectedEnabled
        OutputPath = $OutputPath
    }
    & $verifierPath @parameters
    $verification = Read-ClassificationVerification -Path $OutputPath
    $assertion = @{
        Verification = $verification
        DeploymentId = $DeploymentId
        ExpectedEnabled = [bool]::Parse($ExpectedEnabled)
    }
    Assert-ClassificationVerification @assertion
    return $verification
}

function Wait-ClassificationVerifier {
    param(
        [Parameter(Mandatory = $true)][string]$DeploymentId,
        [Parameter(Mandatory = $true)]
        [ValidateSet('true', 'false')]
        [string]$ExpectedEnabled,
        [Parameter(Mandatory = $true)][string]$OutputPath,
        [int]$TimeoutMinutes = 20
    )

    $deadline = [DateTimeOffset]::UtcNow.AddMinutes($TimeoutMinutes)
    $lastError = 'verification not attempted'
    while ([DateTimeOffset]::UtcNow -lt $deadline) {
        try {
            $parameters = @{
                DeploymentId = $DeploymentId
                ExpectedEnabled = $ExpectedEnabled
                OutputPath = $OutputPath
            }
            return Invoke-ClassificationVerifier @parameters
        }
        catch {
            $lastError = $_.Exception.Message
            Start-Sleep -Seconds 15
        }
    }
    throw "Schema 16 read-only verification did not pass before timeout: $lastError"
}

function Get-OperatorToolReceipt {
    param(
        [Parameter(Mandatory = $true)]$GitEvidence,
        [Parameter(Mandatory = $true)]$GhEvidence
    )

    return [ordered]@{
        git = [ordered]@{
            version = [string]$GitEvidence.version
            sha256 = [string]$GitEvidence.sha256
            authenticodeStatus = [string]$GitEvidence.authenticodeStatus
            signerSubject = [string]$GitEvidence.signerSubject
            installKind = [string]$GitEvidence.installKind
        }
        githubCli = [ordered]@{
            version = [string]$GhEvidence.version
            sha256 = [string]$GhEvidence.sha256
            authenticodeStatus = [string]$GhEvidence.authenticodeStatus
            signerSubject = [string]$GhEvidence.signerSubject
            installKind = [string]$GhEvidence.installKind
        }
    }
}

function Get-RailwayToolReceipt {
    param([Parameter(Mandatory = $true)]$RailwayEvidence)

    return [ordered]@{
        version = [string]$RailwayEvidence.version
        sha256 = [string]$RailwayEvidence.sha256
        authenticodeStatus = [string]$RailwayEvidence.authenticodeStatus
        signerThumbprint = [string]$RailwayEvidence.signerThumbprint
        installKind = [string]$RailwayEvidence.installKind
    }
}

function New-PhaseOneReceipt {
    param(
        [Parameter(Mandatory = $true)]$Preflight,
        [Parameter(Mandatory = $true)]$Step10Evidence,
        [Parameter(Mandatory = $true)]$Step16Evidence,
        [Parameter(Mandatory = $true)]$GitEvidence,
        [Parameter(Mandatory = $true)]$GhEvidence,
        [Parameter(Mandatory = $true)]$RailwayEvidence,
        [Parameter(Mandatory = $true)]$Source,
        [Parameter(Mandatory = $true)]$Deployment,
        [Parameter(Mandatory = $true)]$Verification,
        [Parameter(Mandatory = $true)][string]$VerificationPath,
        [Parameter(Mandatory = $true)][string]$Message
    )

    $now = [DateTimeOffset]::UtcNow
    return [ordered]@{
        success = $true
        receiptVersion = $receiptVersion
        kind = $phaseOneReceiptKind
        receiptId = [Guid]::NewGuid().ToString('D')
        createdAtUtc = $now.ToString('o')
        expiresAtUtc = $now.AddHours(24).ToString('o')
        repository = $repository
        branch = $expectedBranch
        headSha = $ExpectedHeadSha
        step10RunId = $Step10RunId
        step16RunId = $Step16RunId
        actionsEvidence = [ordered]@{
            step10 = $Step10Evidence
            step16 = $Step16Evidence
        }
        operatorTools = Get-OperatorToolReceipt -GitEvidence $GitEvidence -GhEvidence $GhEvidence
        railwayCli = Get-RailwayToolReceipt -RailwayEvidence $RailwayEvidence
        projectId = $projectId
        environment = $environment
        service = $service
        preDeployment = $Preflight
        sourceArchiveSha256 = [string]$Source.archiveSha256
        stagedSourceManifestSha256 = [string]$Source.stagedManifestSha256
        deploymentId = [string]$Deployment.id
        deploymentStatus = [string]$Deployment.status
        deploymentCreatedAt = [string]$Deployment.createdAt
        deploymentMessage = $Message
        productionImageDigest = ([string]$Deployment.meta.imageDigest).ToLowerInvariant()
        verification = $Verification
        verificationSha256 = (
            Get-FileHash -LiteralPath $VerificationPath -Algorithm SHA256
        ).Hash.ToLowerInvariant()
        classificationEnabled = $false
        expenseCommentaryEnabled = $false
        activationRequiresExplicitApproval = $true
        sameSourceActivationRequired = $true
        secretValuesWritten = $false
        integrityProofKind = 'dpapi-current-user-v1'
    }
}

function Read-PhaseOneReceipt {
    param([Parameter(Mandatory = $true)][string]$Path)

    $receipt = Read-TmBoundedJsonFile -Path $Path -MaximumBytes 4194304
    Assert-TmReceiptIntegrityProof $receipt
    Assert-TmExactJsonProperties $receipt @(
        'success', 'receiptVersion', 'kind', 'receiptId', 'createdAtUtc',
        'expiresAtUtc', 'repository', 'branch', 'headSha', 'step10RunId',
        'step16RunId', 'actionsEvidence', 'operatorTools', 'railwayCli',
        'projectId', 'environment', 'service', 'preDeployment',
        'sourceArchiveSha256', 'stagedSourceManifestSha256', 'deploymentId',
        'deploymentStatus', 'deploymentCreatedAt', 'deploymentMessage',
        'productionImageDigest', 'verification', 'verificationSha256',
        'classificationEnabled', 'expenseCommentaryEnabled',
        'activationRequiresExplicitApproval', 'sameSourceActivationRequired',
        'secretValuesWritten', 'integrityProofKind', 'integrityProof'
    ) 'phase 1 classification receipt'
    foreach ($name in @(
        'success', 'activationRequiresExplicitApproval',
        'sameSourceActivationRequired'
    )) {
        Assert-BooleanValue $receipt.$name $true "phaseOneReceipt.$name"
    }
    foreach ($name in @(
        'classificationEnabled', 'expenseCommentaryEnabled', 'secretValuesWritten'
    )) {
        Assert-BooleanValue $receipt.$name $false "phaseOneReceipt.$name"
    }
    $receiptId = [Guid]::Empty
    $deploymentId = [Guid]::Empty
    $createdAt = [DateTimeOffset]::MinValue
    $expiresAt = [DateTimeOffset]::MinValue
    if ([int]$receipt.receiptVersion -ne $receiptVersion -or
        [string]$receipt.kind -cne $phaseOneReceiptKind -or
        [string]$receipt.repository -cne $repository -or
        [string]$receipt.branch -cne $expectedBranch -or
        ([string]$receipt.headSha).ToLowerInvariant() -cne $ExpectedHeadSha -or
        [long]$receipt.step10RunId -ne $Step10RunId -or
        [long]$receipt.step16RunId -ne $Step16RunId -or
        [string]$receipt.projectId -cne $projectId -or
        [string]$receipt.environment -cne $environment -or
        [string]$receipt.service -cne $service -or
        -not [Guid]::TryParse([string]$receipt.receiptId, [ref]$receiptId) -or
        -not [Guid]::TryParse([string]$receipt.deploymentId, [ref]$deploymentId) -or
        [string]$receipt.deploymentStatus -cne 'SUCCESS' -or
        [string]$receipt.sourceArchiveSha256 -notmatch '^[0-9a-f]{64}$' -or
        [string]$receipt.stagedSourceManifestSha256 -notmatch '^[0-9a-f]{64}$' -or
        [string]$receipt.productionImageDigest -notmatch '^sha256:[0-9a-f]{64}$' -or
        [string]$receipt.verificationSha256 -notmatch '^[0-9a-f]{64}$' -or
        -not [DateTimeOffset]::TryParse([string]$receipt.createdAtUtc, [ref]$createdAt) -or
        -not [DateTimeOffset]::TryParse([string]$receipt.expiresAtUtc, [ref]$expiresAt) -or
        $expiresAt.ToUniversalTime() -le $createdAt.ToUniversalTime() -or
        $expiresAt.ToUniversalTime() -gt $createdAt.ToUniversalTime().AddHours(24.1)) {
        throw 'The phase 1 receipt is not bound to this exact schema 16 rollout.'
    }
    $receiptVerificationParameters = @{
        Verification = $receipt.verification
        DeploymentId = $deploymentId.ToString('D')
        ExpectedEnabled = $false
    }
    Assert-ClassificationVerification @receiptVerificationParameters
    return $receipt
}

function Assert-PhaseOneReceiptEvidence {
    param(
        [Parameter(Mandatory = $true)]$Receipt,
        [Parameter(Mandatory = $true)]$Step10Evidence,
        [Parameter(Mandatory = $true)]$Step16Evidence,
        [Parameter(Mandatory = $true)]$GitEvidence,
        [Parameter(Mandatory = $true)]$GhEvidence,
        [Parameter(Mandatory = $true)]$RailwayEvidence
    )

    $step10Parameters = @{
        Expected = $Receipt.actionsEvidence.step10
        Actual = $Step10Evidence
        Name = 'STEP 10'
    }
    Assert-TmReleaseEvidenceMatches @step10Parameters
    $step16Parameters = @{
        Expected = $Receipt.actionsEvidence.step16
        Actual = $Step16Evidence
        Name = 'STEP 16'
    }
    Assert-TmReleaseEvidenceMatches @step16Parameters
    Assert-TmSignedToolMatches -Expected $Receipt.operatorTools.git -Actual $GitEvidence -Name 'Git'
    Assert-TmSignedToolMatches -Expected $Receipt.operatorTools.githubCli -Actual $GhEvidence -Name 'GitHub CLI'
    Assert-TmRailwayCliMatches -Expected $Receipt.railwayCli -Actual $RailwayEvidence
}

function Write-PhaseTwoReceipt {
    param(
        [Parameter(Mandatory = $true)]$PhaseOneReceipt,
        [Parameter(Mandatory = $true)]$Source,
        [Parameter(Mandatory = $true)]$Deployment,
        [Parameter(Mandatory = $true)]$Verification,
        [Parameter(Mandatory = $true)][string]$Message
    )

    $receipt = [ordered]@{
        success = $true
        receiptVersion = $receiptVersion
        kind = $phaseTwoReceiptKind
        completedAtUtc = [DateTimeOffset]::UtcNow.ToString('o')
        repository = $repository
        branch = $expectedBranch
        headSha = $ExpectedHeadSha
        step10RunId = $Step10RunId
        step16RunId = $Step16RunId
        phaseOneReceiptId = [string]$PhaseOneReceipt.receiptId
        phaseOneReceiptSha256 = (
            Get-FileHash -LiteralPath $PhaseOneReceiptPath -Algorithm SHA256
        ).Hash.ToLowerInvariant()
        phaseOneDeploymentId = [string]$PhaseOneReceipt.deploymentId
        sourceArchiveSha256 = [string]$Source.archiveSha256
        stagedSourceManifestSha256 = [string]$Source.stagedManifestSha256
        deploymentId = [string]$Deployment.id
        deploymentStatus = [string]$Deployment.status
        deploymentCreatedAt = [string]$Deployment.createdAt
        deploymentMessage = $Message
        productionImageDigest = ([string]$Deployment.meta.imageDigest).ToLowerInvariant()
        verification = $Verification
        classificationEnabled = $true
        expenseCommentaryEnabled = $false
        phaseOneReceiptConsumed = $true
        sameSourceVerified = $true
        secretValuesWritten = $false
        integrityProofKind = 'dpapi-current-user-v1'
    }
    Add-TmReceiptIntegrityProof $receipt
    Write-TmJsonNoBom -Value $receipt -Path $PhaseTwoResultPath
}

function Invoke-ClassificationDeploymentGuardSelfTest {
    $root = Join-Path ([System.IO.Path]::GetTempPath()) (
        "tm-expense-classification-rollout-{0}" -f [Guid]::NewGuid().ToString('N')
    )
    $previousHeadSha = $ExpectedHeadSha
    $previousStep10 = $Step10RunId
    $previousStep16 = $Step16RunId
    $previousPhaseOneReceiptPath = $PhaseOneReceiptPath
    try {
        New-Item -ItemType Directory -Force -Path $root | Out-Null
        $firstId = '11111111-1111-4111-8111-111111111111'
        $secondId = '22222222-2222-4222-8222-222222222222'
        $singleJson = '{"result":{"deploymentId":"' + $firstId + '"}}'
        $single = Get-DeploymentIdFromUpload -OutputLines @(
            '{"type":"diagnostic","message":"queued"}',
            $singleJson
        )
        if ($single -cne $firstId) {
            throw 'Classification rollout self-test did not extract one deployment ID.'
        }
        $ambiguousRejected = $false
        try {
            $firstJson = '{"deploymentId":"' + $firstId + '"}'
            $secondJson = '{"deploymentId":"' + $secondId + '"}'
            Get-DeploymentIdFromUpload -OutputLines @($firstJson, $secondJson) | Out-Null
        }
        catch {
            $ambiguousRejected = $_.Exception.Message -like '*exactly one deploymentId*'
        }
        if (-not $ambiguousRejected) {
            throw 'Classification rollout self-test accepted ambiguous deployment IDs.'
        }

        $script:ExpectedHeadSha = 'a' * 40
        $script:Step10RunId = 10
        $script:Step16RunId = 16
        $script:PhaseOneReceiptPath = Join-Path $root 'phase1.json'
        $verification = [pscustomobject]@{
            success = $true
            verifiedAtUtc = [DateTimeOffset]::UtcNow.ToString('o')
            headSha = $script:ExpectedHeadSha
            deploymentId = $firstId
            schemaVersion = 16
            classificationEnabled = $false
            expenseCommentaryEnabled = $false
            remoteBackupStatus = 'succeeded'
            remoteBackupSchemaVersion = 16
            remoteBackupIntegrityCheck = 'ok'
            remoteBackupSchemaSemanticsValidated = $true
            preMigrationSchemaVersion = 15
            classificationOpen = [pscustomobject]@{
                claimedCount = 0
                stagedCount = 0
                oldestOpenCreatedAt = $null
            }
            classificationLatestStatus = $null
            classificationBudget = [pscustomobject]@{
                budgetMonth = '2026-08'
                committedMicrousd = 0
                remainingMicrousd = 250000
                hardLimitMicrousd = 250000
            }
            secretValuesWritten = $false
            readOnlyVerification = $true
        }
        Assert-ClassificationVerification -Verification $verification -DeploymentId $firstId -ExpectedEnabled $false
        $verification.classificationEnabled = $true
        $wrongGateRejected = $false
        try {
            Assert-ClassificationVerification -Verification $verification -DeploymentId $firstId -ExpectedEnabled $false
        }
        catch {
            $wrongGateRejected = $true
        }
        if (-not $wrongGateRejected) {
            throw 'Classification rollout self-test accepted the wrong phase gate.'
        }
        $verification.classificationEnabled = $false

        $createdAt = [DateTimeOffset]::UtcNow
        $receipt = [ordered]@{
            success = $true
            receiptVersion = $receiptVersion
            kind = $phaseOneReceiptKind
            receiptId = [Guid]::NewGuid().ToString('D')
            createdAtUtc = $createdAt.ToString('o')
            expiresAtUtc = $createdAt.AddHours(1).ToString('o')
            repository = $repository
            branch = $expectedBranch
            headSha = $script:ExpectedHeadSha
            step10RunId = 10
            step16RunId = 16
            actionsEvidence = [ordered]@{ kind = 'self-test' }
            operatorTools = [ordered]@{ kind = 'self-test' }
            railwayCli = [ordered]@{ kind = 'self-test' }
            projectId = $projectId
            environment = $environment
            service = $service
            preDeployment = [ordered]@{ kind = 'self-test' }
            sourceArchiveSha256 = 'b' * 64
            stagedSourceManifestSha256 = 'c' * 64
            deploymentId = $firstId
            deploymentStatus = 'SUCCESS'
            deploymentCreatedAt = $createdAt.ToString('o')
            deploymentMessage = 'self-test'
            productionImageDigest = 'sha256:' + ('d' * 64)
            verification = $verification
            verificationSha256 = 'e' * 64
            classificationEnabled = $false
            expenseCommentaryEnabled = $false
            activationRequiresExplicitApproval = $true
            sameSourceActivationRequired = $true
            secretValuesWritten = $false
            integrityProofKind = 'dpapi-current-user-v1'
        }
        Add-TmReceiptIntegrityProof $receipt
        Write-TmJsonNoBom -Value $receipt -Path $script:PhaseOneReceiptPath
        $validated = Read-PhaseOneReceipt -Path $script:PhaseOneReceiptPath
        $stateParameters = @{
            StateKind = 'approval'
            ReceiptId = [string]$validated.receiptId
            ReceiptPath = $script:PhaseOneReceiptPath
            ExpiresAtUtc = [string]$validated.expiresAtUtc
            StateRoot = $root
        }
        $state = New-TmPendingReceiptState @stateParameters
        $consumeParameters = @{
            StateKind = 'approval'
            ReceiptId = [string]$validated.receiptId
            ReceiptPath = $script:PhaseOneReceiptPath
            StateRoot = $root
            Consume = $true
        }
        $null = Assert-TmPendingReceiptState @consumeParameters
        $replayRejected = $false
        try {
            Assert-TmPendingReceiptState @consumeParameters | Out-Null
        }
        catch {
            $replayRejected = $true
        }
        if (-not $replayRejected -or [string]::IsNullOrWhiteSpace([string]$state.Path)) {
            throw 'Classification rollout self-test did not reject receipt replay.'
        }
    }
    finally {
        $script:ExpectedHeadSha = $previousHeadSha
        $script:Step10RunId = $previousStep10
        $script:Step16RunId = $previousStep16
        $script:PhaseOneReceiptPath = $previousPhaseOneReceiptPath
        if (Test-Path -LiteralPath $root -PathType Container) {
            Remove-Item -LiteralPath $root -Recurse -Force
        }
    }
    Write-Host 'Expense classification deployment guard self-test: PASS' -ForegroundColor Green
}

if ($RunGuardSelfTest) {
    Invoke-ClassificationDeploymentGuardSelfTest
    Invoke-TmReleaseEvidenceGuardSelfTest
    exit 0
}

Assert-PathUnderDist -Path $PhaseOneReceiptPath
Assert-PathUnderDist -Path $PhaseTwoResultPath
Assert-PathUnderDist -Path $failureResultPath
if ($ExpectedHeadSha -notmatch '^[0-9a-fA-F]{40}$' -or
    $Step10RunId -lt 1 -or $Step16RunId -lt 1) {
    throw 'An exact 40-character commit SHA and successful STEP 10/STEP 16 run IDs are required.'
}
$ExpectedHeadSha = $ExpectedHeadSha.ToLowerInvariant()
if ($Phase -ceq 'Phase1' -and $ApproveActivation) {
    throw 'ApproveActivation is valid only for Phase2.'
}
if ($Phase -ceq 'Phase2' -and -not $ApproveActivation) {
    throw 'Phase2 requires the explicit -ApproveActivation switch.'
}
$confirmBypassRequested = $PSBoundParameters.ContainsKey('Confirm') -and
    -not [bool]$PSBoundParameters['Confirm']
if ($Force -or $confirmBypassRequested) {
    throw 'Production classification rollout refuses -Force and -Confirm:$false.'
}

$gitEvidence = $null
$ghEvidence = $null
$railwayEvidence = $null
$step10Evidence = $null
$step16Evidence = $null
$source = $null
$mutex = $null
$mutexAcquired = $false
$abandonedMutexRecovered = $false
$stage = 'exclusive-lock'
$deploymentId = $null
$classificationEnableAttempted = $false
$activationUploadAttempted = $false
$activationDeploymentSucceeded = $false
$failureRollbackAttempted = $false
$failureRollbackSucceeded = $false
$phaseOneReceipt = $null

try {
    New-Item -ItemType Directory -Force -Path $rolloutRoot | Out-Null
    $mutex = [System.Threading.Mutex]::new($false, $mutexName)
    try {
        $mutexAcquired = $mutex.WaitOne(0)
    }
    catch [System.Threading.AbandonedMutexException] {
        $mutexAcquired = $true
        $abandonedMutexRecovered = $true
    }
    if (-not $mutexAcquired) {
        throw 'Another TM expense production configuration or deployment is already running.'
    }

    $stage = 'canonical-release-evidence'
    $gitEvidence = Resolve-TmVerifiedGit
    $git = [string]$gitEvidence.path
    $gitStateParameters = @{
        GitPath = $git
        RepositoryRoot = $tmRoot
        Repository = $repository
        Branch = $expectedBranch
        ExpectedHeadSha = $ExpectedHeadSha
    }
    $null = Assert-TmCanonicalGitState @gitStateParameters
    $ghEvidence = Resolve-TmVerifiedGh
    $gh = [string]$ghEvidence.path
    $step10Arguments = @{
        GhPath = $gh
        Repository = $repository
        RunId = $Step10RunId
        ExpectedHeadSha = $ExpectedHeadSha
        ExpectedBranch = $expectedBranch
        ExpectedWorkflowName = $step10WorkflowName
        ExpectedWorkflowPath = $step10WorkflowPath
        ArtifactKind = 'step10'
        ArtifactName = $step10ArtifactName
    }
    $step10Evidence = Get-TmVerifiedActionsArtifactEvidence @step10Arguments
    $step16Arguments = @{
        GhPath = $gh
        Repository = $repository
        RunId = $Step16RunId
        ExpectedHeadSha = $ExpectedHeadSha
        ExpectedBranch = $expectedBranch
        ExpectedWorkflowName = $step16WorkflowName
        ExpectedWorkflowPath = $step16WorkflowPath
        ArtifactKind = 'step16'
        ArtifactName = $step16ArtifactName
    }
    $step16Evidence = Get-TmVerifiedActionsArtifactEvidence @step16Arguments
    $railwayEvidence = Resolve-TmVerifiedRailwayCli
    $railway = [string]$railwayEvidence.path
    $source = New-VerifiedSourceStage -GitPath $git -HeadSha $ExpectedHeadSha

    $phaseOneMessage = "schema16-expense-classification-disabled-$($ExpectedHeadSha.Substring(0, 12))"
    $phaseTwoMessage = "schema16-expense-classification-enabled-$($ExpectedHeadSha.Substring(0, 12))"

    if ($Phase -ceq 'Phase1') {
        $stage = 'schema15-read-only-preflight'
        $preflight = Assert-PhaseOneProductionPreflight (Invoke-ProductionOperationsStatus)
        if (-not $Apply) {
            Write-Host "Phase1 dry run passed: $repository $expectedBranch $ExpectedHeadSha" -ForegroundColor Green
            Write-Host "Actions: STEP 10=$Step10RunId STEP 16=$Step16RunId"
            Write-Host 'No Railway variable was changed and no deployment was triggered.'
            return
        }
        if (Test-Path -LiteralPath $PhaseOneReceiptPath) {
            throw 'A protected Phase1 receipt already exists. Refusing to overwrite rollout evidence.'
        }
        $action = "Deploy exact schema 16 commit $ExpectedHeadSha with classification disabled"
        if (-not $PSCmdlet.ShouldProcess('Railway production/tm-server', $action)) {
            return
        }

        $stage = 'schema15-final-read-only-preflight'
        $finalPreflight = Assert-PhaseOneProductionPreflight (Invoke-ProductionOperationsStatus)
        Assert-PhaseOnePreflightUnchanged -Expected $preflight -Actual $finalPreflight
        $stage = 'phase1-fail-closed-variable'
        Set-ClassificationControl -RailwayPath $railway -Enabled false
        $stage = 'phase1-source-upload'
        $deploymentId = Start-ExactSourceDeployment -RailwayPath $railway -Source $source -Message $phaseOneMessage
        $stage = 'phase1-deployment-wait'
        $deployment = Wait-VerifiedRailwayDeployment -RailwayPath $railway -DeploymentId $deploymentId -Message $phaseOneMessage
        $stage = 'phase1-schema16-backup-verification'
        $phaseOneVerificationPath = Join-Path $rolloutRoot 'phase1-verification.json'
        $verification = Wait-ClassificationVerifier -DeploymentId $deploymentId -ExpectedEnabled false -OutputPath $phaseOneVerificationPath
        $stage = 'phase1-receipt'
        $receiptParameters = @{
            Preflight = $preflight
            Step10Evidence = $step10Evidence
            Step16Evidence = $step16Evidence
            GitEvidence = $gitEvidence
            GhEvidence = $ghEvidence
            RailwayEvidence = $railwayEvidence
            Source = $source
            Deployment = $deployment
            Verification = $verification
            VerificationPath = $phaseOneVerificationPath
            Message = $phaseOneMessage
        }
        $receipt = New-PhaseOneReceipt @receiptParameters
        Add-TmReceiptIntegrityProof $receipt
        Write-TmJsonNoBom -Value $receipt -Path $PhaseOneReceiptPath
        $stateParameters = @{
            StateKind = 'approval'
            ReceiptId = [string]$receipt.receiptId
            ReceiptPath = $PhaseOneReceiptPath
            ExpiresAtUtc = [string]$receipt.expiresAtUtc
        }
        $null = New-TmPendingReceiptState @stateParameters
        Write-Host "Phase1 passed and stopped with classification disabled: $deploymentId" -ForegroundColor Green
        Write-Host "Protected Phase1 receipt: $PhaseOneReceiptPath"
        Write-Host 'Review the receipt, then run Phase2 separately with -ApproveActivation -Apply.'
        return
    }

    $stage = 'phase1-receipt-validation'
    $phaseOneReceipt = Read-PhaseOneReceipt -Path $PhaseOneReceiptPath
    $evidenceParameters = @{
        Receipt = $phaseOneReceipt
        Step10Evidence = $step10Evidence
        Step16Evidence = $step16Evidence
        GitEvidence = $gitEvidence
        GhEvidence = $ghEvidence
        RailwayEvidence = $railwayEvidence
    }
    Assert-PhaseOneReceiptEvidence @evidenceParameters
    $pendingParameters = @{
        StateKind = 'approval'
        ReceiptId = [string]$phaseOneReceipt.receiptId
        ReceiptPath = $PhaseOneReceiptPath
    }
    $null = Assert-TmPendingReceiptState @pendingParameters
    if ([string]$source.archiveSha256 -cne [string]$phaseOneReceipt.sourceArchiveSha256 -or
        [string]$source.stagedManifestSha256 -cne
            [string]$phaseOneReceipt.stagedSourceManifestSha256) {
        throw 'Phase2 source is not byte-for-byte bound to the Phase1 exact commit archive.'
    }
    $latest = Get-LatestDeployment -RailwayPath $railway
    $exactPhaseOne = @{
        Deployment = $latest
        DeploymentId = [string]$phaseOneReceipt.deploymentId
        Message = [string]$phaseOneReceipt.deploymentMessage
        ExpectedImageDigest = [string]$phaseOneReceipt.productionImageDigest
    }
    Assert-ExactDeployment @exactPhaseOne
    $stage = 'phase2-live-disabled-verification'
    $phaseTwoPreflightPath = Join-Path $rolloutRoot 'phase2-preflight-verification.json'
    $preflightVerifier = @{
        DeploymentId = [string]$phaseOneReceipt.deploymentId
        ExpectedEnabled = 'false'
        OutputPath = $phaseTwoPreflightPath
    }
    $null = Invoke-ClassificationVerifier @preflightVerifier

    if (-not $Apply) {
        Write-Host "Phase2 dry run passed: $repository $expectedBranch $ExpectedHeadSha" -ForegroundColor Green
        Write-Host "Phase1 deployment: $($phaseOneReceipt.deploymentId)"
        Write-Host 'Classification remains disabled. Re-run with -Apply and approve the prompt.'
        return
    }
    if (Test-Path -LiteralPath $PhaseTwoResultPath) {
        throw 'A protected Phase2 result already exists. Refusing to overwrite rollout evidence.'
    }
    $action = "Activate classification on exact commit $ExpectedHeadSha using Phase1 receipt $($phaseOneReceipt.receiptId)"
    if (-not $PSCmdlet.ShouldProcess('Railway production/tm-server', $action)) {
        return
    }

    $stage = 'phase2-final-evidence-check'
    $finalGitEvidence = Resolve-TmVerifiedGit
    Assert-TmSignedToolMatches -Expected $phaseOneReceipt.operatorTools.git -Actual $finalGitEvidence -Name 'Git'
    $git = [string]$finalGitEvidence.path
    $gitStateParameters.GitPath = $git
    $null = Assert-TmCanonicalGitState @gitStateParameters
    $finalRailwayEvidence = Resolve-TmVerifiedRailwayCli
    Assert-TmRailwayCliMatches -Expected $phaseOneReceipt.railwayCli -Actual $finalRailwayEvidence
    $railway = [string]$finalRailwayEvidence.path
    Assert-SourceStageUnchanged -Source $source
    $latest = Get-LatestDeployment -RailwayPath $railway
    $exactPhaseOne.Deployment = $latest
    Assert-ExactDeployment @exactPhaseOne
    $null = Invoke-ClassificationVerifier @preflightVerifier

    $stage = 'phase2-enable-variable'
    $classificationEnableAttempted = $true
    Set-ClassificationControl -RailwayPath $railway -Enabled true
    $stage = 'phase2-source-upload'
    $activationUploadAttempted = $true
    $deploymentId = Start-ExactSourceDeployment -RailwayPath $railway -Source $source -Message $phaseTwoMessage
    $stage = 'phase2-deployment-wait'
    $deployment = Wait-VerifiedRailwayDeployment -RailwayPath $railway -DeploymentId $deploymentId -Message $phaseTwoMessage
    $activationDeploymentSucceeded = $true
    $stage = 'phase2-final-read-only-verification'
    $phaseTwoVerificationPath = Join-Path $rolloutRoot 'phase2-verification.json'
    $finalVerifier = @{
        DeploymentId = $deploymentId
        ExpectedEnabled = 'true'
        OutputPath = $phaseTwoVerificationPath
        TimeoutMinutes = 10
    }
    $verification = Wait-ClassificationVerifier @finalVerifier
    $stage = 'phase1-receipt-consumption'
    $pendingParameters.Consume = $true
    $null = Assert-TmPendingReceiptState @pendingParameters
    $stage = 'phase2-receipt'
    Write-PhaseTwoReceipt -PhaseOneReceipt $phaseOneReceipt -Source $source -Deployment $deployment -Verification $verification -Message $phaseTwoMessage
    Write-Host "Phase2 classification activation passed: $deploymentId" -ForegroundColor Green
    Write-Host "Protected Phase2 result: $PhaseTwoResultPath"
}
catch {
    $originalError = $_.Exception.Message
    if ($Phase -ceq 'Phase2' -and $classificationEnableAttempted -and
        $null -ne $railwayEvidence) {
        $failureRollbackAttempted = $true
        try {
            $railway = [string](Resolve-TmVerifiedRailwayCli).path
            Set-ClassificationControl -RailwayPath $railway -Enabled false
            if ($activationUploadAttempted -and $null -ne $source) {
                $rollbackMessage =
                    "schema16-expense-classification-rollback-$($ExpectedHeadSha.Substring(0, 12))"
                $rollbackId = Start-ExactSourceDeployment -RailwayPath $railway -Source $source -Message $rollbackMessage
                $rollbackDeployment = Wait-VerifiedRailwayDeployment -RailwayPath $railway -DeploymentId $rollbackId -Message $rollbackMessage
                $rollbackVerificationPath = Join-Path $rolloutRoot 'rollback-verification.json'
                $rollbackVerifier = @{
                    DeploymentId = $rollbackId
                    ExpectedEnabled = 'false'
                    OutputPath = $rollbackVerificationPath
                    TimeoutMinutes = 10
                }
                $null = Wait-ClassificationVerifier @rollbackVerifier
                $null = $rollbackDeployment
            }
            $failureRollbackSucceeded = $true
        }
        catch {
            $failureRollbackSucceeded = $false
        }
    }
    New-Item -ItemType Directory -Force -Path $rolloutRoot | Out-Null
    $failure = [ordered]@{
        success = $false
        failedAtUtc = [DateTimeOffset]::UtcNow.ToString('o')
        phase = $Phase
        stage = $stage
        reason = $originalError
        headSha = $ExpectedHeadSha
        step10RunId = $Step10RunId
        step16RunId = $Step16RunId
        deploymentId = $deploymentId
        classificationEnableAttempted = $classificationEnableAttempted
        activationUploadAttempted = $activationUploadAttempted
        activationDeploymentSucceeded = $activationDeploymentSucceeded
        failClosedRollbackAttempted = $failureRollbackAttempted
        failClosedRollbackSucceeded = $failureRollbackSucceeded
        secretValuesWritten = $false
    }
    Write-TmJsonNoBom -Value $failure -Path $failureResultPath
    throw ("Verified schema 16 classification rollout failed at {0}: {1}" -f $stage, $originalError)
}
finally {
    if ($null -ne $source -and
        -not [string]::IsNullOrWhiteSpace([string]$source.stageRoot) -and
        (Test-Path -LiteralPath $source.stageRoot -PathType Container)) {
        $resolvedStage = [System.IO.Path]::GetFullPath([string]$source.stageRoot)
        $resolvedRollout = [System.IO.Path]::GetFullPath($rolloutRoot).TrimEnd('\') + '\'
        if ($resolvedStage.StartsWith(
                $resolvedRollout,
                [System.StringComparison]::OrdinalIgnoreCase
            ) -and (Split-Path -Leaf $resolvedStage) -like 'staging-*') {
            Remove-Item -LiteralPath $resolvedStage -Recurse -Force
        }
    }
    if ($mutexAcquired -and $null -ne $mutex) {
        $mutex.ReleaseMutex()
    }
    if ($null -ne $mutex) {
        $mutex.Dispose()
    }
}

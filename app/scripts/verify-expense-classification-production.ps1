[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[0-9a-fA-F]{40}$')]
    [string]$ExpectedHeadSha,

    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[0-9a-fA-F-]{36}$')]
    [string]$ExpectedDeploymentId,

    [ValidateSet('true', 'false')]
    [string]$ExpectedClassificationEnabled = 'true',

    [string]$OutputPath
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
Add-Type -AssemblyName System.Net.Http

$baseUri = 'https://tm-server-production-5573.up.railway.app'
$credentialResource = 'TM Cloud Production'
$credentialUser = 'single-user'
$maximumResponseBytes = 1MB
$expectedOperationLimitMicrousd = 250000
$ExpectedHeadSha = $ExpectedHeadSha.ToLowerInvariant()
$parsedDeploymentId = [Guid]::Empty
if (-not [Guid]::TryParse($ExpectedDeploymentId, [ref]$parsedDeploymentId)) {
    throw 'ExpectedDeploymentId must be a valid Railway deployment UUID.'
}
$ExpectedDeploymentId = $parsedDeploymentId.ToString('D')
$expectedEnabled = $ExpectedClassificationEnabled -ceq 'true'

$tmRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
if ([string]::IsNullOrWhiteSpace($OutputPath)) {
    $OutputPath = Join-Path $tmRoot 'dist\manual-expense-classification-production-verification\result.json'
}
$OutputPath = [System.IO.Path]::GetFullPath($OutputPath)
$outputDirectory = Split-Path -Parent $OutputPath
if (-not $OutputPath.StartsWith(
        ([System.IO.Path]::GetFullPath((Join-Path $tmRoot 'dist')).TrimEnd('\') + '\'),
        [System.StringComparison]::OrdinalIgnoreCase
    )) {
    throw 'OutputPath must remain under the TM dist directory.'
}

function Assert-Boolean {
    param(
        [AllowNull()]$Value,
        [Parameter(Mandatory = $true)][bool]$Expected,
        [Parameter(Mandatory = $true)][string]$Name
    )
    if ($Value -isnot [bool] -or $Value -ne $Expected) {
        throw "$Name must be the Boolean value $Expected."
    }
}

function Invoke-TmReadOnlyGet {
    param(
        [Parameter(Mandatory = $true)][System.Net.Http.HttpClient]$Client,
        [Parameter(Mandatory = $true)][string]$Uri,
        [AllowNull()][string]$Token
    )
    $request = $null
    $response = $null
    try {
        $request = [System.Net.Http.HttpRequestMessage]::new(
            [System.Net.Http.HttpMethod]::Get,
            $Uri
        )
        if (-not [string]::IsNullOrWhiteSpace($Token)) {
            $request.Headers.Authorization =
                [System.Net.Http.Headers.AuthenticationHeaderValue]::new('Bearer', $Token)
        }
        $response = $Client.SendAsync($request).GetAwaiter().GetResult()
        if ($response.Content.Headers.ContentLength.HasValue -and
            $response.Content.Headers.ContentLength.Value -gt $maximumResponseBytes) {
            throw 'TM production returned an oversized response.'
        }
        $bytes = $response.Content.ReadAsByteArrayAsync().GetAwaiter().GetResult()
        if ($bytes.Length -gt $maximumResponseBytes) {
            throw 'TM production returned an oversized response.'
        }
        $cacheControl = if ($response.Headers.Contains('Cache-Control')) {
            [string]::Join(',', $response.Headers.GetValues('Cache-Control'))
        } else {
            ''
        }
        [pscustomobject]@{
            StatusCode = [int]$response.StatusCode
            Body = [System.Text.Encoding]::UTF8.GetString($bytes)
            CacheControl = $cacheControl
        }
    }
    finally {
        if ($null -ne $response) { $response.Dispose() }
        if ($null -ne $request) { $request.Dispose() }
    }
}

function ConvertFrom-TmEnvelope {
    param(
        [Parameter(Mandatory = $true)]$Response,
        [Parameter(Mandatory = $true)][string]$Name,
        [switch]$RequireNoStore
    )
    if ($Response.StatusCode -ne 200) {
        throw "$Name returned HTTP $($Response.StatusCode). The response body was suppressed."
    }
    if ($RequireNoStore -and $Response.CacheControl -notmatch '(?i)(?:^|,)\s*no-store\s*(?:,|$)') {
        throw "$Name did not return Cache-Control: no-store."
    }
    try {
        $envelope = $Response.Body | ConvertFrom-Json
    }
    catch {
        throw "$Name returned invalid JSON. The response body was suppressed."
    }
    if ($null -eq $envelope -or $envelope.PSObject.Properties.Name -notcontains 'data') {
        throw "$Name did not contain a data envelope."
    }
    return $envelope.data
}

$vault = $null
$credential = $null
$token = $null
$handler = $null
$client = $null
try {
    $vault = [Windows.Security.Credentials.PasswordVault,Windows.Security.Credentials,ContentType=WindowsRuntime]::new()
    $credential = $vault.Retrieve($credentialResource, $credentialUser)
    $credential.RetrievePassword()
    $token = [string]$credential.Password
    if ($token -notmatch '^tm_pat_v1_[A-Za-z0-9_-]{43}$') {
        throw 'The Credential Locker TM production token has an invalid format.'
    }

    $handler = [System.Net.Http.HttpClientHandler]::new()
    $handler.AllowAutoRedirect = $false
    $handler.UseCookies = $false
    $client = [System.Net.Http.HttpClient]::new($handler)
    $client.Timeout = [TimeSpan]::FromSeconds(60)

    $ready = Invoke-TmReadOnlyGet -Client $client -Uri "$baseUri/readyz" -Token $null
    if ($ready.StatusCode -ne 200) {
        throw "readiness returned HTTP $($ready.StatusCode). The response body was suppressed."
    }
    $opsResponse = Invoke-TmReadOnlyGet `
        -Client $client `
        -Uri "$baseUri/api/v1/ops/status" `
        -Token $token
    $ops = ConvertFrom-TmEnvelope -Response $opsResponse -Name 'operations status' -RequireNoStore

    Assert-Boolean $ops.database.ok $true 'database.ok'
    Assert-Boolean $ops.controls.aiEnabled $true 'controls.aiEnabled'
    Assert-Boolean $ops.controls.expenseAiEnabled $false 'controls.expenseAiEnabled'
    Assert-Boolean `
        $ops.controls.expenseClassificationAiEnabled `
        $expectedEnabled `
        'controls.expenseClassificationAiEnabled'
    Assert-Boolean $ops.controls.expenseCryptoReady $true 'controls.expenseCryptoReady'
    Assert-Boolean $ops.controls.expenseKeyInitialized $true 'controls.expenseKeyInitialized'
    Assert-Boolean `
        $ops.controls.expenseExpectedKeyFingerprintMatch `
        $true `
        'controls.expenseExpectedKeyFingerprintMatch'
    Assert-Boolean `
        $ops.controls.expenseActivationFingerprintMatch `
        $true `
        'controls.expenseActivationFingerprintMatch'
    if ([int]$ops.database.schemaVersion -ne 16 -or
        [string]$ops.database.currentSchemaMigrationName -cne 'expense-ai-hybrid-classification' -or
        [string]$ops.controls.incidentMode -cne 'normal' -or
        [string]$ops.controls.expenseRolloutMode -cne 'enabled' -or
        ([string]$ops.deploymentProvenance.buildCommitSha).ToLowerInvariant() -cne
            $ExpectedHeadSha -or
        ([string]$ops.deploymentProvenance.railwayDeploymentId).ToLowerInvariant() -cne
            $ExpectedDeploymentId) {
        throw 'Production schema, controls, or exact deployment provenance are not ready.'
    }

    $schemaAppliedAt = [DateTimeOffset]::MinValue
    $preMigrationCreatedAt = [DateTimeOffset]::MinValue
    if (-not [DateTimeOffset]::TryParse([string]$ops.database.currentSchemaAppliedAt, [ref]$schemaAppliedAt) -or
        -not [DateTimeOffset]::TryParse(
            [string]$ops.localBackup.latestPreMigrationCreatedAt,
            [ref]$preMigrationCreatedAt
        ) -or
        [int]$ops.localBackup.latestPreMigrationSchemaVersion -ne 15 -or
        [string]$ops.localBackup.latestPreMigrationIntegrityCheck -cne 'ok' -or
        $ops.localBackup.latestPreMigrationSchemaSemanticsValidated -ne $true -or
        $preMigrationCreatedAt.ToUniversalTime() -lt $schemaAppliedAt.ToUniversalTime().AddMinutes(-10) -or
        $preMigrationCreatedAt.ToUniversalTime() -gt $schemaAppliedAt.ToUniversalTime().AddMinutes(1)) {
        throw 'The schema 16 migration is not backed by a valid schema 15 pre-migration snapshot.'
    }

    foreach ($name in @(
        'migrationLedgerComplete',
        'requiredTablesComplete',
        'schemaSemanticsValidated',
        'expenseCryptoProbePresent'
    )) {
        Assert-Boolean $ops.remoteBackup.$name $true "remoteBackup.$name"
    }
    if ([string]$ops.remoteBackup.status -cne 'succeeded' -or
        [int]$ops.remoteBackup.schemaVersion -ne 16 -or
        [string]$ops.remoteBackup.integrityCheck -cne 'ok' -or
        [string]$ops.remoteBackup.expenseCryptoProbeSha256 -notmatch '^[0-9a-f]{64}$' -or
        [string]$ops.remoteBackup.expenseCryptoProbeSha256 -cne
            [string]$ops.controls.expenseCryptoProbeSha256) {
        throw 'The latest remote backup has not completed schema 16 verification.'
    }

    $budget = $ops.expenseClassificationBudget
    Assert-Boolean `
        $budget.hardStopReached `
        ([uint64]$budget.committedMicrousd -ge $expectedOperationLimitMicrousd) `
        'expenseClassificationBudget.hardStopReached'
    if ([string]$budget.operation -cne 'expense_classification' -or
        [uint64]$budget.hardLimitMicrousd -ne $expectedOperationLimitMicrousd -or
        [uint64]$budget.committedMicrousd -gt $expectedOperationLimitMicrousd -or
        [uint64]$budget.remainingMicrousd -ne
            ($expectedOperationLimitMicrousd - [uint64]$budget.committedMicrousd)) {
        throw 'The expense classification budget status is inconsistent.'
    }

    $classification = $ops.expenseClassification
    if ($null -eq $classification -or
        [int64]$classification.claimedCount -ne 0 -or
        [int64]$classification.stagedCount -ne 0 -or
        $null -ne $classification.oldestOpenCreatedAt) {
        throw 'Expense classification has an unexpected open run during verification.'
    }
    $latest = $classification.latest
    if ($null -ne $latest) {
        $allowedLatestFields = @(
            'status', 'targetMonth', 'itemGroupCount', 'reviewCount', 'resultCount',
            'confirmedCount', 'provisionalCount', 'reviewRequiredCount',
            'privacySkippedCount', 'versionConflictCount', 'actualCostMicrousd',
            'failureCode', 'createdAt', 'completedAt'
        )
        $unexpectedLatestFields = @($latest.PSObject.Properties.Name | Where-Object {
            $allowedLatestFields -notcontains $_
        })
        if ($unexpectedLatestFields.Count -ne 0 -or
            [string]$latest.status -notin @('claimed', 'staged', 'applied', 'failed', 'stale', 'no_candidates') -or
            [string]$latest.targetMonth -notmatch '^\d{4}-(0[1-9]|1[0-2])$') {
            throw 'The latest expense classification safe status is invalid.'
        }
        foreach ($field in @(
            'itemGroupCount', 'reviewCount', 'resultCount', 'confirmedCount',
            'provisionalCount', 'reviewRequiredCount', 'privacySkippedCount',
            'versionConflictCount', 'actualCostMicrousd'
        )) {
            if ([int64]$latest.$field -lt 0) {
                throw "expenseClassification.latest.$field must be nonnegative."
            }
        }
        $serializedLatest = $latest | ConvertTo-Json -Depth 4 -Compress
        if ($serializedLatest -match '(?i)requestId|inputSha256|merchant|counterparty|eventId|reviewId') {
            throw 'The expense classification operations status exposed a forbidden identifier.'
        }
    }

    New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null
    $result = [ordered]@{
        success = $true
        verifiedAtUtc = [DateTimeOffset]::UtcNow.ToString('o')
        headSha = $ExpectedHeadSha
        deploymentId = $ExpectedDeploymentId
        schemaVersion = 16
        classificationEnabled = $expectedEnabled
        expenseCommentaryEnabled = $false
        remoteBackupStatus = [string]$ops.remoteBackup.status
        remoteBackupSchemaVersion = [int]$ops.remoteBackup.schemaVersion
        remoteBackupIntegrityCheck = [string]$ops.remoteBackup.integrityCheck
        remoteBackupSchemaSemanticsValidated =
            [bool]$ops.remoteBackup.schemaSemanticsValidated
        preMigrationSchemaVersion =
            [int]$ops.localBackup.latestPreMigrationSchemaVersion
        classificationOpen = [ordered]@{
            claimedCount = [int64]$classification.claimedCount
            stagedCount = [int64]$classification.stagedCount
            oldestOpenCreatedAt = $classification.oldestOpenCreatedAt
        }
        classificationLatestStatus = if ($null -eq $latest) { $null } else { [string]$latest.status }
        classificationBudget = [ordered]@{
            budgetMonth = [string]$budget.budgetMonth
            committedMicrousd = [uint64]$budget.committedMicrousd
            remainingMicrousd = [uint64]$budget.remainingMicrousd
            hardLimitMicrousd = [uint64]$budget.hardLimitMicrousd
        }
        secretValuesWritten = $false
        readOnlyVerification = $true
    }
    $json = $result | ConvertTo-Json -Depth 8
    [System.IO.File]::WriteAllText(
        $OutputPath,
        $json,
        [System.Text.UTF8Encoding]::new($false)
    )
    Write-Host 'TM schema 16 expense classification production verification passed.' -ForegroundColor Green
    Write-Host "Schema: $($result.schemaVersion)"
    Write-Host "Classification enabled: $($result.classificationEnabled)"
    Write-Host "Remote backup: $($result.remoteBackupStatus)"
    Write-Host "Result: $OutputPath"
}
finally {
    if ($null -ne $client) { $client.Dispose() }
    if ($null -ne $handler) { $handler.Dispose() }
    Remove-Variable token, credential, vault -ErrorAction SilentlyContinue
}

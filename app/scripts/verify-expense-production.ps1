[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[0-9a-fA-F-]{36}$')]
    [string]$ExpectedDeploymentId,

    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[0-9a-fA-F]{40}$')]
    [string]$ExpectedHeadSha,

    [string]$ConfigurationResultPath,
    [string]$DeploymentResultPath,
    [string]$OutputPath
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
Add-Type -AssemblyName System.Net.Http

$projectId = '7fcb22b5-db34-4e2b-a12a-cbc60391ff5f'
$environment = 'production'
$service = 'tm-server'
$repository = 'junhyeonglee1/TM-private'
$expectedBranch = 'agent/step10-cloud-cutover'
$resource = 'TM Cloud Production'
$userName = 'single-user'
$BaseUri = 'https://tm-server-production-5573.up.railway.app'
$parsedExpectedDeploymentId = [Guid]::Empty
if (-not [Guid]::TryParse($ExpectedDeploymentId, [ref]$parsedExpectedDeploymentId)) {
    throw 'ExpectedDeploymentId must be a valid Railway deployment UUID.'
}
$ExpectedDeploymentId = $parsedExpectedDeploymentId.ToString('D')
$ExpectedHeadSha = $ExpectedHeadSha.ToLowerInvariant()

$tmRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
if ([string]::IsNullOrWhiteSpace($ConfigurationResultPath)) {
    $ConfigurationResultPath = Join-Path $tmRoot 'dist\manual-expense-railway-configuration\result.json'
}
if ([string]::IsNullOrWhiteSpace($DeploymentResultPath)) {
    $DeploymentResultPath = Join-Path $tmRoot 'dist\manual-expense-railway-deployment\result.json'
}
if ([string]::IsNullOrWhiteSpace($OutputPath)) {
    $OutputPath = Join-Path $tmRoot 'dist\manual-expense-production-verification\result.json'
}
$ConfigurationResultPath = [System.IO.Path]::GetFullPath($ConfigurationResultPath)
$DeploymentResultPath = [System.IO.Path]::GetFullPath($DeploymentResultPath)
$OutputPath = [System.IO.Path]::GetFullPath($OutputPath)
$resultDirectory = Split-Path -Parent $OutputPath

function Read-BoundedJson {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][uint64]$MaximumBytes
    )
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Required deployment evidence is missing: $Path"
    }
    $item = Get-Item -LiteralPath $Path
    if ([uint64]$item.Length -lt 2 -or [uint64]$item.Length -gt $MaximumBytes) {
        throw "Required deployment evidence has an invalid size: $Path"
    }
    try {
        return [System.IO.File]::ReadAllText($Path, [System.Text.Encoding]::UTF8) | ConvertFrom-Json
    }
    catch {
        throw "Required deployment evidence is not valid JSON: $Path"
    }
}

function Assert-StrictBoolean {
    param(
        [AllowNull()]$Value,
        [Parameter(Mandatory = $true)][bool]$Expected,
        [Parameter(Mandatory = $true)][string]$Name
    )
    if ($Value -isnot [bool] -or $Value -ne $Expected) {
        throw "$Name must be the Boolean value $Expected."
    }
}

function Get-RailwayDeployments {
    param([Parameter(Mandatory = $true)][string]$RailwayPath)
    $json = & $RailwayPath deployment list `
        --json `
        --limit 20 `
        --project $projectId `
        --environment $environment `
        --service $service 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw 'Unable to list Railway production deployments.'
    }
    return @($json | ConvertFrom-Json)
}

function Assert-ExpectedRailwayDeployment {
    param(
        [Parameter(Mandatory = $true)][string]$RailwayPath,
        [Parameter(Mandatory = $true)]$DeploymentResult
    )
    $deployments = @(Get-RailwayDeployments -RailwayPath $RailwayPath)
    $matching = @($deployments | Where-Object {
        ([string]$_.id).ToLowerInvariant() -eq $ExpectedDeploymentId
    })
    if ($matching.Count -ne 1) {
        throw 'The expected Railway deployment could not be proven uniquely.'
    }
    $deployment = $matching[0]
    $latest = @($deployments | Sort-Object { [DateTimeOffset]$_.createdAt } -Descending | Select-Object -First 1)
    if ($latest.Count -ne 1 -or
        ([string]$latest[0].id).ToLowerInvariant() -ne $ExpectedDeploymentId -or
        [string]$deployment.status -ne 'SUCCESS' -or
        [string]$deployment.meta.cliMessage -cne [string]$DeploymentResult.deploymentMessage -or
        ([string]$deployment.meta.imageDigest).ToLowerInvariant() -cne
            ([string]$DeploymentResult.productionImageDigest).ToLowerInvariant()) {
        throw 'The expected deployment is not the latest successful immutable Railway deployment.'
    }
    return $deployment
}

function Invoke-TmRequest {
    param(
        [Parameter(Mandatory = $true)][System.Net.Http.HttpClient]$Client,
        [Parameter(Mandatory = $true)][string]$Uri,
        [string]$Token
    )
    $request = $null
    $response = $null
    try {
        $request = [System.Net.Http.HttpRequestMessage]::new(
            [System.Net.Http.HttpMethod]::Get,
            $Uri
        )
        if (-not [string]::IsNullOrWhiteSpace($Token)) {
            $request.Headers.Authorization = [System.Net.Http.Headers.AuthenticationHeaderValue]::new('Bearer', $Token)
        }
        $response = $Client.SendAsync($request).GetAwaiter().GetResult()
        $cacheControl = ''
        if ($response.Headers.Contains('Cache-Control')) {
            $cacheControl = [string]::Join(',', $response.Headers.GetValues('Cache-Control'))
        }
        [pscustomobject]@{
            StatusCode = [int]$response.StatusCode
            Body = $response.Content.ReadAsStringAsync().GetAwaiter().GetResult()
            CacheControl = $cacheControl
            ETag = if ($null -ne $response.Headers.ETag) { [string]$response.Headers.ETag.Tag } else { '' }
        }
    }
    finally {
        if ($null -ne $response) { $response.Dispose() }
        if ($null -ne $request) { $request.Dispose() }
    }
}

function Assert-Status {
    param($Response, [int]$Expected, [string]$Stage)
    if ($Response.StatusCode -ne $Expected) {
        throw "$Stage returned HTTP $($Response.StatusCode), expected $Expected. $($Response.Body)"
    }
}

function Assert-NoStore {
    param($Response, [string]$Stage)
    if ([string]$Response.CacheControl -notmatch '(^|,|\s)no-store($|,|\s)') {
        throw "$Stage did not return Cache-Control: no-store."
    }
}

function Invoke-ExpenseRead {
    param(
        [System.Net.Http.HttpClient]$Client,
        [string]$Base,
        [string]$Token,
        [string]$Path,
        [string]$Stage
    )
    $response = Invoke-TmRequest -Client $Client -Uri "$Base$Path" -Token $Token
    Assert-Status $response 200 $Stage
    Assert-NoStore $response $Stage
    return ($response.Body | ConvertFrom-Json).data
}

$uri = [System.Uri]$BaseUri
if (-not $uri.IsAbsoluteUri -or $uri.Scheme -ne 'https' -or
    -not [string]::IsNullOrEmpty($uri.UserInfo) -or
    -not [string]::IsNullOrEmpty($uri.Query) -or
    -not [string]::IsNullOrEmpty($uri.Fragment)) {
    throw 'BaseUri must be a credential-free HTTPS origin.'
}
$base = $uri.GetLeftPart([System.UriPartial]::Authority)
$seoul = [System.TimeZoneInfo]::FindSystemTimeZoneById('Korea Standard Time')
$today = [System.TimeZoneInfo]::ConvertTimeFromUtc([DateTime]::UtcNow, $seoul)
$month = $today.ToString('yyyy-MM')

$deploymentResult = Read-BoundedJson -Path $DeploymentResultPath -MaximumBytes 65536
$configurationResult = Read-BoundedJson -Path $ConfigurationResultPath -MaximumBytes 65536
Assert-StrictBoolean $deploymentResult.success $true 'deploymentResult.success'
Assert-StrictBoolean $deploymentResult.deploymentWaitRequired $false 'deploymentResult.deploymentWaitRequired'
if ($deploymentResult.expectedExpenseAiEnabled -isnot [bool]) {
    throw 'deploymentResult.expectedExpenseAiEnabled must be a Boolean.'
}
Assert-StrictBoolean $configurationResult.success $true 'configurationResult.success'
Assert-StrictBoolean $configurationResult.deploymentTriggered $false 'configurationResult.deploymentTriggered'
Assert-StrictBoolean $configurationResult.sourceDeploymentRequired $true 'configurationResult.sourceDeploymentRequired'
$configurationSha256 = (Get-FileHash -LiteralPath $ConfigurationResultPath -Algorithm SHA256).Hash.ToLowerInvariant()
if ([string]$configurationResult.receiptId -cne [string]$deploymentResult.configurationReceiptId -or
    $configurationSha256 -ne ([string]$deploymentResult.configurationReceiptSha256).ToLowerInvariant() -or
    ([string]$configurationResult.expectedHeadSha).ToLowerInvariant() -ne $ExpectedHeadSha -or
    [long]$configurationResult.step10RunId -ne [long]$deploymentResult.step10RunId -or
    [long]$configurationResult.step16RunId -ne [long]$deploymentResult.step16RunId -or
    $configurationResult.expenseAiEnabled -isnot [bool] -or
    $configurationResult.expenseAiEnabled -ne $deploymentResult.expectedExpenseAiEnabled) {
    throw 'The deployment evidence no longer matches its exact configuration receipt.'
}
if ([string]$deploymentResult.repository -ne $repository -or
    [string]$deploymentResult.branch -ne $expectedBranch -or
    ([string]$deploymentResult.headSha).ToLowerInvariant() -ne $ExpectedHeadSha -or
    ([string]$deploymentResult.deploymentId).ToLowerInvariant() -ne $ExpectedDeploymentId -or
    [string]$deploymentResult.projectId -ne $projectId -or
    [string]$deploymentResult.environment -ne $environment -or
    [string]$deploymentResult.service -ne $service -or
    [string]$deploymentResult.deploymentStatus -ne 'SUCCESS' -or
    [string]$deploymentResult.deploymentMessage -cne "schema15 expense $($ExpectedHeadSha.Substring(0, 12))" -or
    [string]$deploymentResult.productionImageDigest -notmatch '^sha256:[0-9a-fA-F]{64}$' -or
    ([string]$deploymentResult.sourceArchiveHeadSha).ToLowerInvariant() -ne $ExpectedHeadSha -or
    [string]$deploymentResult.sourceArchiveSha256 -notmatch '^[0-9a-fA-F]{64}$' -or
    [string]$deploymentResult.configurationReceiptId -notmatch '^[0-9a-fA-F-]{36}$' -or
    [string]$deploymentResult.configurationReceiptSha256 -notmatch '^[0-9a-fA-F]{64}$' -or
    [long]$deploymentResult.step10RunId -lt 1 -or
    [long]$deploymentResult.step16RunId -lt 1) {
    throw 'The deployment result is not bound to the expected schema 15 production release.'
}
if (-not (Test-Path -LiteralPath ([string]$deploymentResult.sourceArchivePath) -PathType Leaf) -or
    (Get-FileHash -LiteralPath ([string]$deploymentResult.sourceArchivePath) -Algorithm SHA256).Hash.ToLowerInvariant() -ne
        ([string]$deploymentResult.sourceArchiveSha256).ToLowerInvariant()) {
    throw 'The exact source archive no longer matches its deployment evidence.'
}

$railway = Get-ChildItem -LiteralPath (Join-Path $env:LOCALAPPDATA 'pnpm\store\v11\links\@railway\cli') `
    -Filter railway.exe -File -Recurse |
    Sort-Object FullName -Descending |
    Select-Object -First 1 -ExpandProperty FullName
if ([string]::IsNullOrWhiteSpace($railway) -or -not (Test-Path -LiteralPath $railway -PathType Leaf)) {
    throw 'Railway CLI was not found.'
}
$deploymentEvidence = Assert-ExpectedRailwayDeployment -RailwayPath $railway -DeploymentResult $deploymentResult

$vault = $null
$credential = $null
$token = $null
$handler = $null
$client = $null
$stage = 'credential-locker'

try {
    New-Item -ItemType Directory -Force -Path $resultDirectory | Out-Null
    $vault = [Windows.Security.Credentials.PasswordVault,Windows.Security.Credentials,ContentType=WindowsRuntime]::new()
    $credential = $vault.Retrieve($resource, $userName)
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

    $stage = 'readiness'
    $ready = Invoke-TmRequest -Client $client -Uri "$base/readyz"
    Assert-Status $ready 200 $stage

    $stage = 'operations-status'
    $ops = Invoke-TmRequest -Client $client -Uri "$base/api/v1/ops/status" -Token $token
    Assert-Status $ops 200 $stage
    Assert-NoStore $ops $stage
    $opsData = ($ops.Body | ConvertFrom-Json).data
    Assert-StrictBoolean $opsData.database.ok $true 'database.ok'
    Assert-StrictBoolean $opsData.controls.expenseCryptoReady $true 'controls.expenseCryptoReady'
    Assert-StrictBoolean $opsData.controls.expenseKeyInitialized $true 'controls.expenseKeyInitialized'
    Assert-StrictBoolean `
        $opsData.controls.expenseAiEnabled `
        ([bool]$deploymentResult.expectedExpenseAiEnabled) `
        'controls.expenseAiEnabled'
    if ([int]$opsData.database.schemaVersion -ne 15 -or
        [string]$opsData.controls.incidentMode -ne 'normal' -or
        [int]$opsData.controls.maximumExpenseImportBodyBytes -ne (8 * 1024 * 1024) -or
        [int]$opsData.controls.maximumExpensePreviewBodyBytes -ne (8 * 1024 * 1024) -or
        ([string]$opsData.deploymentProvenance.buildCommitSha).ToLowerInvariant() -ne $ExpectedHeadSha -or
        ([string]$opsData.deploymentProvenance.railwayDeploymentId).ToLowerInvariant() -ne $ExpectedDeploymentId) {
        throw 'Production schema 15 controls or runtime deployment provenance are not ready.'
    }
    $schemaAppliedAt = [DateTimeOffset]::MinValue
    $preMigrationCreatedAt = [DateTimeOffset]::MinValue
    if (-not [DateTimeOffset]::TryParse(
        [string]$opsData.database.currentSchemaAppliedAt,
        [System.Globalization.CultureInfo]::InvariantCulture,
        [System.Globalization.DateTimeStyles]::RoundtripKind,
        [ref]$schemaAppliedAt
    ) -or -not [DateTimeOffset]::TryParse(
        [string]$opsData.localBackup.latestPreMigrationCreatedAt,
        [System.Globalization.CultureInfo]::InvariantCulture,
        [System.Globalization.DateTimeStyles]::RoundtripKind,
        [ref]$preMigrationCreatedAt
    ) -or [int]$opsData.localBackup.preMigrationCount -lt 1 -or
        [uint64]$opsData.localBackup.latestPreMigrationByteSize -lt 1) {
        throw 'Production does not expose the schema 15 pre-migration backup proof.'
    }
    $migrationBackupGap = $schemaAppliedAt.ToUniversalTime() - $preMigrationCreatedAt.ToUniversalTime()
    if ($migrationBackupGap.TotalMinutes -lt -1 -or $migrationBackupGap.TotalMinutes -gt 30) {
        throw 'The latest pre-migration backup does not correspond to the schema 15 migration.'
    }
    Assert-StrictBoolean $opsData.remoteBackup.migrationLedgerComplete $true 'remoteBackup.migrationLedgerComplete'
    Assert-StrictBoolean $opsData.remoteBackup.requiredTablesComplete $true 'remoteBackup.requiredTablesComplete'
    Assert-StrictBoolean $opsData.remoteBackup.schemaSemanticsValidated $true 'remoteBackup.schemaSemanticsValidated'
    if ([string]$opsData.remoteBackup.status -ne 'succeeded' -or
        [int]$opsData.remoteBackup.schemaVersion -ne 15 -or
        [string]$opsData.remoteBackup.integrityCheck -ne 'ok') {
        throw 'The latest production backup has not completed schema 15 verification.'
    }
    $backupCheckedAt = [DateTimeOffset]::MinValue
    if (-not [DateTimeOffset]::TryParse(
        [string]$opsData.remoteBackup.checkedAt,
        [System.Globalization.CultureInfo]::InvariantCulture,
        [System.Globalization.DateTimeStyles]::RoundtripKind,
        [ref]$backupCheckedAt
    )) {
        throw 'The latest production backup has no valid checkedAt timestamp.'
    }
    $backupAge = [DateTimeOffset]::UtcNow - $backupCheckedAt.ToUniversalTime()
    $backupFreshnessHours = [double]$opsData.objectives.backupFreshnessTargetHours
    if ($backupFreshnessHours -le 0 -or $backupFreshnessHours -gt 24 -or
        $backupAge.TotalMinutes -lt -5 -or $backupAge.TotalHours -gt $backupFreshnessHours -or
        [string]$opsData.remoteBackup.snapshotId -notmatch '^[A-Za-z0-9][A-Za-z0-9._-]{0,255}$' -or
        [string]$opsData.remoteBackup.databaseSha256 -notmatch '^[0-9a-fA-F]{64}$' -or
        [uint64]$opsData.remoteBackup.databaseByteSize -lt 1) {
        throw 'The latest production backup is stale or has incomplete provenance.'
    }
    $criticalAlerts = @($opsData.alerts | Where-Object { [string]$_.severity -eq 'critical' })
    if ([string]$opsData.overallStatus -eq 'critical' -or $criticalAlerts.Count -gt 0) {
        $criticalCodes = @($criticalAlerts | ForEach-Object { [string]$_.code }) -join ','
        throw "Production operations status is critical: $criticalCodes"
    }

    $stage = 'expense-read-apis'
    $summary = Invoke-ExpenseRead -Client $client -Base $base -Token $token -Path "/api/v1/expenses/summary?month=$month" -Stage 'expense-summary'
    $transactions = Invoke-ExpenseRead -Client $client -Base $base -Token $token -Path "/api/v1/expenses/transactions?month=$month&limit=1" -Stage 'expense-transactions'
    $reviews = Invoke-ExpenseRead -Client $client -Base $base -Token $token -Path "/api/v1/expenses/reviews?status=pending&limit=1" -Stage 'expense-reviews'
    $recurring = @(Invoke-ExpenseRead -Client $client -Base $base -Token $token -Path '/api/v1/expenses/recurring' -Stage 'expense-recurring')
    $occurrences = @(Invoke-ExpenseRead -Client $client -Base $base -Token $token -Path "/api/v1/expenses/recurring/occurrences?month=$month" -Stage 'expense-occurrences')
    $latestReport = Invoke-ExpenseRead -Client $client -Base $base -Token $token -Path "/api/v1/expenses/reports/latest?month=$month" -Stage 'expense-report-latest'
    if ([string]$summary.month -ne $month -or $null -eq $summary.currencies -or
        $null -eq $transactions.items -or $null -eq $reviews.items) {
        throw 'An expense read API response does not match the schema 15 contract.'
    }

    $stage = 'mobile-shell'
    $mobile = Invoke-TmRequest -Client $client -Uri "$base/mobile/"
    Assert-Status $mobile 200 $stage
    $mobileScript = Invoke-TmRequest -Client $client -Uri "$base/mobile/app.js"
    Assert-Status $mobileScript 200 "$stage-script"
    if ($mobile.Body -notmatch 'id="tab-expenses"' -or
        $mobileScript.Body -notmatch '/api/v1/expenses/summary' -or
        $mobileScript.Body -notmatch '/api/v1/expenses/recurring' -or
        $mobileScript.Body -match '/api/v1/expenses/imports' -or
        $mobileScript.Body -match 'preview_expense_import') {
        throw 'The mobile expense shell is missing or exposes the Windows-only import surface.'
    }

    $stage = 'deployment-final-proof'
    $deploymentEvidence = Assert-ExpectedRailwayDeployment -RailwayPath $railway -DeploymentResult $deploymentResult

    $stage = 'complete'
    [ordered]@{
        success = $true
        verifiedAtUtc = [DateTimeOffset]::UtcNow.ToString('o')
        baseUri = $base
        headSha = $ExpectedHeadSha
        deploymentId = $ExpectedDeploymentId
        deploymentStatus = [string]$deploymentEvidence.status
        productionImageDigest = ([string]$deploymentEvidence.meta.imageDigest).ToLowerInvariant()
        configurationReceiptId = [string]$deploymentResult.configurationReceiptId
        configurationReceiptSha256 = $configurationSha256
        sourceArchiveSha256 = ([string]$deploymentResult.sourceArchiveSha256).ToLowerInvariant()
        schemaVersion = [int]$opsData.database.schemaVersion
        schemaAppliedAt = [string]$opsData.database.currentSchemaAppliedAt
        preMigrationBackupCreatedAt = [string]$opsData.localBackup.latestPreMigrationCreatedAt
        expenseCryptoReady = [bool]$opsData.controls.expenseCryptoReady
        expenseAiEnabled = [bool]$opsData.controls.expenseAiEnabled
        remoteBackupStatus = [string]$opsData.remoteBackup.status
        remoteBackupSchemaVersion = [int]$opsData.remoteBackup.schemaVersion
        remoteBackupCheckedAt = [string]$opsData.remoteBackup.checkedAt
        remoteBackupSnapshotId = [string]$opsData.remoteBackup.snapshotId
        remoteBackupDatabaseSha256 = ([string]$opsData.remoteBackup.databaseSha256).ToLowerInvariant()
        remoteBackupDatabaseByteSize = [uint64]$opsData.remoteBackup.databaseByteSize
        operationsStatus = [string]$opsData.overallStatus
        month = $month
        currencyCount = @($summary.currencies).Count
        transactionSampleCount = @($transactions.items).Count
        pendingReviewSampleCount = @($reviews.items).Count
        recurringCount = $recurring.Count
        occurrenceCount = $occurrences.Count
        cachedAiReportPresent = $null -ne $latestReport
        syntheticRecurringCreatedAndRemoved = $false
        aiCallPerformed = $false
        productionTransactionImported = $false
        secretsPersistedByScript = $false
    } | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $OutputPath -Encoding utf8

    Write-Host 'TM schema 15 production expense verification passed.' -ForegroundColor Green
    Write-Host "Deployment: $ExpectedDeploymentId"
    Write-Host "Schema: $($opsData.database.schemaVersion)"
    Write-Host "Remote backup: $($opsData.remoteBackup.status) / schema $($opsData.remoteBackup.schemaVersion)"
    Write-Host "Expense crypto ready: $($opsData.controls.expenseCryptoReady)"
    Write-Host 'Synthetic recurring mutation: skipped (read-only verifier)'
    Write-Host 'No AI request or transaction import was performed.'
}
catch {
    New-Item -ItemType Directory -Force -Path $resultDirectory | Out-Null
    [ordered]@{
        success = $false
        failedAtUtc = [DateTimeOffset]::UtcNow.ToString('o')
        stage = $stage
        reason = $_.Exception.Message
        expectedHeadSha = $ExpectedHeadSha
        expectedDeploymentId = $ExpectedDeploymentId
        syntheticRecurringCleanupSucceeded = $false
        secretsPersistedByScript = $false
    } | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $OutputPath -Encoding utf8
    throw "TM schema 15 production expense verification failed at ${stage}: $($_.Exception.Message)"
}
finally {
    if ($null -ne $client) { $client.Dispose() }
    if ($null -ne $handler) { $handler.Dispose() }
    $token = $null
    Remove-Variable token, credential, vault -ErrorAction SilentlyContinue
}

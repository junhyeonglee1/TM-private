[CmdletBinding(SupportsShouldProcess = $true, ConfirmImpact = 'High')]
param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[0-9a-fA-F]{40}$')]
    [string]$ExpectedHeadSha,

    [Parameter(Mandatory = $true)]
    [ValidateRange(1, [long]::MaxValue)]
    [long]$Step10RunId,

    [Parameter(Mandatory = $true)]
    [ValidateRange(1, [long]::MaxValue)]
    [long]$Step16RunId,

    [string]$ConfigurationResultPath,
    [string]$ResultPath,
    [switch]$RunGuardSelfTest,
    [switch]$Apply,
    [switch]$Force
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
Add-Type -AssemblyName System.Net.Http

$repository = 'junhyeonglee1/TM-private'
$expectedBranch = 'agent/step10-cloud-cutover'
$projectId = '7fcb22b5-db34-4e2b-a12a-cbc60391ff5f'
$environment = 'production'
$service = 'tm-server'
$baseUri = 'https://tm-server-production-5573.up.railway.app'
$tokenCredentialResource = 'TM Cloud Production'
$tokenCredentialUser = 'single-user'
$ExpectedHeadSha = $ExpectedHeadSha.ToLowerInvariant()

$appRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$tmRoot = [System.IO.Path]::GetFullPath((Join-Path $appRoot '..'))
if ([string]::IsNullOrWhiteSpace($ConfigurationResultPath)) {
    $ConfigurationResultPath = Join-Path $tmRoot 'dist\manual-expense-railway-configuration\result.json'
}
if ([string]::IsNullOrWhiteSpace($ResultPath)) {
    $ResultPath = Join-Path $tmRoot 'dist\manual-expense-railway-deployment\result.json'
}
$ConfigurationResultPath = [System.IO.Path]::GetFullPath($ConfigurationResultPath)
$ResultPath = [System.IO.Path]::GetFullPath($ResultPath)
$resultDirectory = Split-Path -Parent $ResultPath
$base = ([System.Uri]$baseUri).GetLeftPart([System.UriPartial]::Authority)

function Resolve-GitExecutable {
    $command = Get-Command git -ErrorAction SilentlyContinue
    if ($command) { return $command.Source }
    $bundled = Join-Path $env:USERPROFILE '.cache\codex-runtimes\codex-primary-runtime\dependencies\native\git\cmd\git.exe'
    if (Test-Path -LiteralPath $bundled -PathType Leaf) { return $bundled }
    throw 'Git executable was not found.'
}

function Assert-VerifiedActionsRun {
    param(
        [Parameter(Mandatory = $true)][string]$GhPath,
        [Parameter(Mandatory = $true)][long]$RunId,
        [Parameter(Mandatory = $true)][string]$WorkflowName
    )
    $json = & $GhPath run view $RunId --repo $repository --json workflowName,conclusion,headBranch,headSha 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw "Unable to inspect GitHub Actions run $RunId."
    }
    $run = $json | ConvertFrom-Json
    if ([string]$run.workflowName -ne $WorkflowName -or
        [string]$run.conclusion -ne 'success' -or
        [string]$run.headBranch -ne $expectedBranch -or
        ([string]$run.headSha).ToLowerInvariant() -ne $ExpectedHeadSha) {
        throw "GitHub Actions run $RunId does not prove $WorkflowName for the expected commit."
    }
}

function Assert-BooleanProperty {
    param(
        [Parameter(Mandatory = $true)]$Object,
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][bool]$Expected
    )
    if ($null -eq $Object -or
        $Object.PSObject.Properties.Name -notcontains $Name -or
        $Object.$Name -isnot [bool] -or
        $Object.$Name -ne $Expected) {
        throw "The configuration proof has an invalid Boolean $Name value."
    }
}

function Read-BoundedJson {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][uint64]$MaximumBytes
    )
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Required JSON proof is missing: $Path"
    }
    $item = Get-Item -LiteralPath $Path
    if ([uint64]$item.Length -lt 2 -or [uint64]$item.Length -gt $MaximumBytes) {
        throw "Required JSON proof has an invalid size: $Path"
    }
    try {
        return [System.IO.File]::ReadAllText($Path, [System.Text.Encoding]::UTF8) | ConvertFrom-Json
    }
    catch {
        throw "Required JSON proof is not valid JSON: $Path"
    }
}

function Read-ConfigurationReceipt {
    param([Parameter(Mandatory = $true)][string]$Path)

    $receipt = Read-BoundedJson -Path $Path -MaximumBytes 65536
    Assert-BooleanProperty $receipt 'success' $true
    Assert-BooleanProperty $receipt 'deploymentTriggered' $false
    Assert-BooleanProperty $receipt 'sourceDeploymentRequired' $true
    Assert-BooleanProperty $receipt 'secretValueWrittenToResult' $false
    Assert-BooleanProperty $receipt 'keyWriteAmbiguous' $false
    if ($receipt.expenseAiEnabled -isnot [bool]) {
        throw 'The configuration receipt has an invalid expenseAiEnabled value.'
    }
    if ([int]$receipt.receiptVersion -ne 1 -or
        [string]$receipt.receiptId -notmatch '^[0-9a-fA-F-]{36}$' -or
        ([string]$receipt.expectedHeadSha).ToLowerInvariant() -ne $ExpectedHeadSha -or
        [long]$receipt.step10RunId -ne $Step10RunId -or
        [long]$receipt.step16RunId -ne $Step16RunId -or
        [string]$receipt.projectId -ne $projectId -or
        [string]$receipt.environment -ne $environment -or
        [string]$receipt.service -ne $service) {
        throw 'The configuration receipt is not bound to this exact target, commit, and Actions evidence.'
    }
    $receiptId = [Guid]::Empty
    if (-not [Guid]::TryParse([string]$receipt.receiptId, [ref]$receiptId)) {
        throw 'The configuration receipt ID is invalid.'
    }
    $configuredAt = [DateTimeOffset]::MinValue
    $expiresAt = [DateTimeOffset]::MinValue
    if (-not [DateTimeOffset]::TryParse(
        [string]$receipt.configuredAtUtc,
        [System.Globalization.CultureInfo]::InvariantCulture,
        [System.Globalization.DateTimeStyles]::RoundtripKind,
        [ref]$configuredAt
    ) -or -not [DateTimeOffset]::TryParse(
        [string]$receipt.expiresAtUtc,
        [System.Globalization.CultureInfo]::InvariantCulture,
        [System.Globalization.DateTimeStyles]::RoundtripKind,
        [ref]$expiresAt
    )) {
        throw 'The configuration receipt timestamps are invalid.'
    }
    $now = [DateTimeOffset]::UtcNow
    $validity = $expiresAt.ToUniversalTime() - $configuredAt.ToUniversalTime()
    if ($validity.TotalMinutes -lt 1 -or $validity.TotalMinutes -gt 30.1 -or
        $configuredAt.ToUniversalTime() -gt $now.AddMinutes(5) -or
        $expiresAt.ToUniversalTime() -lt $now) {
        throw 'The configuration receipt has expired or has an invalid lifetime.'
    }
    if ([int]$receipt.proofSchemaVersion -notin @(14, 15) -or
        [string]$receipt.remoteBackupSnapshotId -notmatch '^[A-Za-z0-9][A-Za-z0-9._-]{0,255}$' -or
        [string]$receipt.remoteBackupDatabaseSha256 -notmatch '^[0-9a-fA-F]{64}$' -or
        [uint64]$receipt.remoteBackupDatabaseByteSize -lt 1) {
        throw 'The configuration receipt is missing its schema or remote-backup provenance.'
    }
    $backupCheckedAt = [DateTimeOffset]::MinValue
    if (-not [DateTimeOffset]::TryParse(
        [string]$receipt.remoteBackupCheckedAt,
        [System.Globalization.CultureInfo]::InvariantCulture,
        [System.Globalization.DateTimeStyles]::RoundtripKind,
        [ref]$backupCheckedAt
    )) {
        throw 'The configuration receipt has an invalid backup timestamp.'
    }

    foreach ($name in @(
        'proofLedgerEmpty',
        'proofKeyInitialized',
        'proofCryptoReady',
        'proofInitializationAllowed',
        'recoveryCredentialPresent',
        'recoveryCredentialMatchVerified',
        'recoveryCredentialSubmittedToRailway',
        'keyWriteRequired',
        'keyWriteAttempted',
        'keyWriteConfirmed'
    )) {
        if ($receipt.$name -isnot [bool]) {
            throw "The configuration receipt has a non-Boolean $name proof."
        }
    }
    if ([int]$receipt.proofSchemaVersion -eq 14) {
        if ([string]$receipt.proofMode -ne 'schema14' -or
            $receipt.proofLedgerEmpty -ne $true -or
            $receipt.proofKeyInitialized -ne $false -or
            $receipt.proofCryptoReady -ne $false -or
            $receipt.proofInitializationAllowed -ne $true -or
            $receipt.recoveryCredentialPresent -ne $true -or
            $receipt.recoveryCredentialMatchVerified -ne $true -or
            $receipt.recoveryCredentialSubmittedToRailway -ne $true -or
            $receipt.keyWriteRequired -ne $true -or
            $receipt.keyWriteAttempted -ne $true -or
            $receipt.keyWriteConfirmed -ne $true) {
            throw 'The schema 14 receipt does not prove a staged first-time expense key.'
        }
    }
    else {
        if ([string]$receipt.proofMode -ne 'schema15' -or
            $receipt.recoveryCredentialPresent -ne $true) {
            throw 'The schema 15 receipt is incomplete.'
        }
        if ($receipt.keyWriteRequired) {
            if ($receipt.proofLedgerEmpty -ne $true -or
                $receipt.proofKeyInitialized -ne $false -or
                $receipt.proofCryptoReady -ne $false -or
                $receipt.proofInitializationAllowed -ne $true -or
                $receipt.recoveryCredentialMatchVerified -ne $true -or
                $receipt.recoveryCredentialSubmittedToRailway -ne $true -or
                $receipt.keyWriteAttempted -ne $true -or
                $receipt.keyWriteConfirmed -ne $true) {
                throw 'The schema 15 receipt does not prove a safe staged first-time key.'
            }
        }
        elseif ($receipt.proofKeyInitialized -ne $true -or
            $receipt.proofCryptoReady -ne $true -or
            $receipt.proofInitializationAllowed -ne $false -or
            $receipt.keyWriteAttempted -ne $false -or
            $receipt.keyWriteConfirmed -ne $false) {
            throw 'The schema 15 receipt does not prove an already healthy expense key.'
        }
    }
    return $receipt
}

function Invoke-TmOperationsStatus {
    param(
        [Parameter(Mandatory = $true)][string]$Origin,
        [Parameter(Mandatory = $true)][string]$Token
    )
    $handler = $null
    $client = $null
    $request = $null
    $response = $null
    try {
        $handler = [System.Net.Http.HttpClientHandler]::new()
        $handler.AllowAutoRedirect = $false
        $handler.UseCookies = $false
        $client = [System.Net.Http.HttpClient]::new($handler)
        $client.Timeout = [TimeSpan]::FromSeconds(30)
        $request = [System.Net.Http.HttpRequestMessage]::new(
            [System.Net.Http.HttpMethod]::Get,
            "$Origin/api/v1/ops/status"
        )
        $request.Headers.Authorization =
            [System.Net.Http.Headers.AuthenticationHeaderValue]::new('Bearer', $Token)
        $response = $client.SendAsync($request).GetAwaiter().GetResult()
        if ([int]$response.StatusCode -ne 200) {
            throw "Production operations status returned HTTP $([int]$response.StatusCode)."
        }
        $cacheControl = ''
        if ($response.Headers.Contains('Cache-Control')) {
            $cacheControl = [string]::Join(',', $response.Headers.GetValues('Cache-Control'))
        }
        if ($cacheControl -notmatch '(^|,|\s)no-store($|,|\s)') {
            throw 'Production operations status did not return Cache-Control: no-store.'
        }
        $body = $response.Content.ReadAsStringAsync().GetAwaiter().GetResult() | ConvertFrom-Json
        if ($null -eq $body.data) {
            throw 'Production operations status returned an invalid envelope.'
        }
        return $body.data
    }
    finally {
        if ($null -ne $response) { $response.Dispose() }
        if ($null -ne $request) { $request.Dispose() }
        if ($null -ne $client) { $client.Dispose() }
        if ($null -ne $handler) { $handler.Dispose() }
    }
}

function Assert-LiveDeploymentPreflight {
    param(
        [Parameter(Mandatory = $true)]$Operations,
        [Parameter(Mandatory = $true)]$Receipt
    )
    if ($null -eq $Operations.database -or $null -eq $Operations.controls -or
        $Operations.database.ok -isnot [bool] -or
        $Operations.database.ok -ne $true -or
        [string]$Operations.controls.incidentMode -ne 'normal' -or
        [int]$Operations.database.schemaVersion -ne [int]$Receipt.proofSchemaVersion) {
        throw 'The live production database no longer matches the configuration proof.'
    }
    $criticalAlerts = @($Operations.alerts | Where-Object { [string]$_.severity -eq 'critical' })
    $backupAlerts = @($Operations.alerts | Where-Object { [string]$_.code -like 'REMOTE_BACKUP_*' })
    $schema15Bootstrap = [int]$Receipt.proofSchemaVersion -eq 15 -and
        $Receipt.keyWriteRequired -eq $true
    if ($schema15Bootstrap) {
        $unexpectedCritical = @($criticalAlerts | Where-Object {
            [string]$_.code -ne 'EXPENSE_CRYPTO_NOT_READY'
        })
        if ([string]$Operations.overallStatus -ne 'critical' -or
            $criticalAlerts.Count -ne 1 -or
            $unexpectedCritical.Count -ne 0) {
            throw 'Schema 15 first-time key recovery has unexpected critical alerts.'
        }
    }
    elseif ([string]$Operations.overallStatus -notin @('healthy', 'warning') -or
        $criticalAlerts.Count -gt 0) {
        throw 'Production has a critical or invalid operations status.'
    }
    if ($backupAlerts.Count -gt 0) {
        throw 'Production has a critical or remote-backup alert.'
    }
    if ($null -eq $Operations.remoteBackup -or $null -eq $Operations.objectives -or
        [string]$Operations.remoteBackup.status -ne 'succeeded' -or
        [int]$Operations.remoteBackup.schemaVersion -ne [int]$Receipt.proofSchemaVersion -or
        [string]$Operations.remoteBackup.integrityCheck -ne 'ok' -or
        $Operations.remoteBackup.migrationLedgerComplete -isnot [bool] -or
        $Operations.remoteBackup.requiredTablesComplete -isnot [bool] -or
        $Operations.remoteBackup.schemaSemanticsValidated -isnot [bool] -or
        $Operations.remoteBackup.migrationLedgerComplete -ne $true -or
        $Operations.remoteBackup.requiredTablesComplete -ne $true -or
        $Operations.remoteBackup.schemaSemanticsValidated -ne $true) {
        throw 'The live remote backup proof is no longer valid.'
    }
    $checkedAt = [DateTimeOffset]::MinValue
    if (-not [DateTimeOffset]::TryParse(
        [string]$Operations.remoteBackup.checkedAt,
        [System.Globalization.CultureInfo]::InvariantCulture,
        [System.Globalization.DateTimeStyles]::RoundtripKind,
        [ref]$checkedAt
    )) {
        throw 'The live remote backup timestamp is invalid.'
    }
    $freshnessHours = [double]$Operations.objectives.backupFreshnessTargetHours
    $age = [DateTimeOffset]::UtcNow - $checkedAt.ToUniversalTime()
    if ($freshnessHours -le 0 -or $freshnessHours -gt 24 -or
        $age.TotalMinutes -lt -5 -or $age.TotalHours -gt $freshnessHours -or
        [string]$Operations.remoteBackup.snapshotId -cne [string]$Receipt.remoteBackupSnapshotId -or
        ([string]$Operations.remoteBackup.databaseSha256).ToLowerInvariant() -cne
            ([string]$Receipt.remoteBackupDatabaseSha256).ToLowerInvariant() -or
        [uint64]$Operations.remoteBackup.databaseByteSize -ne
            [uint64]$Receipt.remoteBackupDatabaseByteSize -or
        [string]$Operations.remoteBackup.checkedAt -cne [string]$Receipt.remoteBackupCheckedAt) {
        throw 'The live backup is stale or no longer matches the approved configuration receipt.'
    }
    if ([int]$Receipt.proofSchemaVersion -eq 15) {
        $expectedInitialized = -not $schema15Bootstrap
        foreach ($name in @('expenseLedgerEmpty', 'expenseKeyInitialized', 'expenseCryptoReady', 'expenseKeyInitializationAllowed')) {
            if ($Operations.controls.PSObject.Properties.Name -notcontains $name -or
                $Operations.controls.$name -isnot [bool]) {
                throw "The live schema 15 service returned an invalid $name proof."
            }
        }
        if ($schema15Bootstrap) {
            if ($Operations.controls.expenseLedgerEmpty -ne $true -or
                $Operations.controls.expenseKeyInitialized -ne $false -or
                $Operations.controls.expenseCryptoReady -ne $false -or
                $Operations.controls.expenseKeyInitializationAllowed -ne $true) {
                throw 'The live schema 15 first-time key proof changed after configuration.'
            }
        }
        elseif ($Operations.controls.expenseKeyInitialized -ne $expectedInitialized -or
            $Operations.controls.expenseCryptoReady -ne $true -or
            $Operations.controls.expenseKeyInitializationAllowed -ne $false) {
            throw 'The live schema 15 service no longer proves its healthy expense key.'
        }
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

function Add-DeploymentIdsFromJsonValue {
    param(
        [AllowNull()]$Value,
        [Parameter(Mandatory = $true)]
        [AllowEmptyCollection()]
        [System.Collections.Generic.HashSet[string]]$Ids
    )
    if ($null -eq $Value -or $Value -is [string] -or
        $Value -is [ValueType]) {
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
        if ([string]$property.Name -eq 'deploymentId' -and
            [string]$property.Value -match '^[0-9a-fA-F-]{36}$') {
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
    $ids = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
    foreach ($line in $OutputLines) {
        $text = [string]$line
        if ([string]::IsNullOrWhiteSpace($text) -or $text.Length -gt 1048576) { continue }
        try {
            $value = $text | ConvertFrom-Json
        }
        catch {
            # NDJSON may include a small number of non-result diagnostic lines. Only JSON deploymentId fields count.
            continue
        }
        Add-DeploymentIdsFromJsonValue -Value $value -Ids $ids
    }
    if ($ids.Count -ne 1) {
        throw "Railway upload output did not prove exactly one deploymentId (found $($ids.Count))."
    }
    return @($ids)[0]
}

function Invoke-DeploymentGuardSelfTest {
    $firstId = '11111111-1111-4111-8111-111111111111'
    $secondId = '22222222-2222-4222-8222-222222222222'
    $single = Get-DeploymentIdFromUpload -OutputLines @(
        '{"type":"diagnostic","message":"queued"}',
        "{`"type`":`"result`",`"deploymentId`":`"$firstId`"}"
    )
    if ($single -ne $firstId) {
        throw 'Deployment guard self-test did not extract the exact result deployment ID.'
    }
    $duplicate = Get-DeploymentIdFromUpload -OutputLines @(
        "{`"deploymentId`":`"$firstId`"}",
        "{`"result`":{`"deploymentId`":`"$firstId`"}}"
    )
    if ($duplicate -ne $firstId) {
        throw 'Deployment guard self-test did not de-duplicate the same deployment ID.'
    }
    $threw = $false
    try {
        Get-DeploymentIdFromUpload -OutputLines @(
            "{`"deploymentId`":`"$firstId`"}",
            "{`"deploymentId`":`"$secondId`"}"
        ) | Out-Null
    }
    catch {
        $threw = $_.Exception.Message -like '*exactly one deploymentId*'
    }
    if (-not $threw) {
        throw 'Deployment guard self-test accepted ambiguous deployment IDs.'
    }

    $receiptPath = Join-Path ([System.IO.Path]::GetTempPath()) "tm-expense-receipt-$([Guid]::NewGuid().ToString('N')).json"
    try {
        $now = [DateTimeOffset]::UtcNow
        $receipt = [ordered]@{
            success = $true
            receiptVersion = 1
            receiptId = [Guid]::NewGuid().ToString('D')
            configuredAtUtc = $now.ToString('o')
            expiresAtUtc = $now.AddMinutes(30).ToString('o')
            expectedHeadSha = $ExpectedHeadSha
            step10RunId = $Step10RunId
            step16RunId = $Step16RunId
            projectId = $projectId
            environment = $environment
            service = $service
            proofSchemaVersion = 14
            proofMode = 'schema14'
            proofLedgerEmpty = $true
            proofKeyInitialized = $false
            proofCryptoReady = $false
            proofInitializationAllowed = $true
            remoteBackupCheckedAt = $now.ToString('o')
            remoteBackupSnapshotId = 'snapshot-14'
            remoteBackupDatabaseSha256 = 'a' * 64
            remoteBackupDatabaseByteSize = 1
            expenseAiEnabled = $true
            recoveryCredentialPresent = $true
            recoveryCredentialMatchVerified = $true
            recoveryCredentialSubmittedToRailway = $true
            keyWriteRequired = $true
            keyWriteAttempted = $true
            keyWriteConfirmed = $true
            keyWriteAmbiguous = $false
            secretValueWrittenToResult = $false
            deploymentTriggered = $false
            sourceDeploymentRequired = $true
        }
        $receipt | ConvertTo-Json | Set-Content -LiteralPath $receiptPath -Encoding utf8
        $validated = Read-ConfigurationReceipt -Path $receiptPath
        if ([string]$validated.expectedHeadSha -ne $ExpectedHeadSha) {
            throw 'Deployment guard self-test did not preserve the receipt commit binding.'
        }
        $receipt.success = 'true'
        $receipt | ConvertTo-Json | Set-Content -LiteralPath $receiptPath -Encoding utf8
        $threw = $false
        try {
            Read-ConfigurationReceipt -Path $receiptPath | Out-Null
        }
        catch {
            $threw = $_.Exception.Message -like '*invalid Boolean success*'
        }
        if (-not $threw) {
            throw 'Deployment guard self-test accepted a string receipt Boolean.'
        }
    }
    finally {
        if (Test-Path -LiteralPath $receiptPath -PathType Leaf) {
            Remove-Item -LiteralPath $receiptPath -Force
        }
    }
    Write-Host 'Verified Railway deployment guard self-test: PASS' -ForegroundColor Green
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
            ([string]$_.id).ToLowerInvariant() -eq $DeploymentId.ToLowerInvariant()
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
            if ($status -eq 'SUCCESS') {
                $latest = @($deployments | Sort-Object { [DateTimeOffset]$_.createdAt } -Descending | Select-Object -First 1)
                if ($latest.Count -ne 1 -or
                    ([string]$latest[0].id).ToLowerInvariant() -ne $DeploymentId.ToLowerInvariant()) {
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

if ($RunGuardSelfTest) {
    Invoke-DeploymentGuardSelfTest
    exit 0
}

$git = Resolve-GitExecutable
$gh = Get-Command gh -ErrorAction SilentlyContinue
if (-not $gh) { throw 'GitHub CLI was not found.' }
$railway = Get-ChildItem -LiteralPath (Join-Path $env:LOCALAPPDATA 'pnpm\store\v11\links\@railway\cli') `
    -Filter railway.exe -File -Recurse |
    Sort-Object FullName -Descending |
    Select-Object -First 1 -ExpandProperty FullName
if ([string]::IsNullOrWhiteSpace($railway) -or -not (Test-Path -LiteralPath $railway -PathType Leaf)) {
    throw 'Railway CLI was not found.'
}

$origin = (& $git -C $tmRoot config --get remote.origin.url).Trim()
$branch = (& $git -C $tmRoot branch --show-current).Trim()
$headSha = (& $git -C $tmRoot rev-parse HEAD).Trim().ToLowerInvariant()
$workingTree = @(& $git -C $tmRoot status --porcelain=v1 --untracked-files=all)
if ($LASTEXITCODE -ne 0) { throw 'Unable to inspect the local Git worktree.' }
$normalizedOrigin = $origin -replace '\.git$', ''
if ($normalizedOrigin -ne "https://github.com/$repository" -or
    $branch -ne $expectedBranch -or
    $headSha -ne $ExpectedHeadSha -or
    $workingTree.Count -ne 0) {
    throw 'Railway deployment requires the clean canonical branch at the exact expected commit.'
}

Assert-VerifiedActionsRun -GhPath $gh.Source -RunId $Step10RunId -WorkflowName 'STEP 10 Windows build'
Assert-VerifiedActionsRun -GhPath $gh.Source -RunId $Step16RunId -WorkflowName 'STEP 16 security'
$configurationReceipt = Read-ConfigurationReceipt -Path $ConfigurationResultPath
$configurationReceiptSha256 = (Get-FileHash -LiteralPath $ConfigurationResultPath -Algorithm SHA256).Hash.ToLowerInvariant()

$vault = $null
$tokenCredential = $null
$token = $null
$stageRoot = $null
$sourceArchivePath = $null
$sourceArchiveSha256 = $null
$deploymentStartAttempted = $false
$deploymentId = $null
$stage = 'credential-locker'
$message = "schema15 expense $($ExpectedHeadSha.Substring(0, 12))"
$startedAt = [DateTimeOffset]::UtcNow

try {
    $vault = [Windows.Security.Credentials.PasswordVault,Windows.Security.Credentials,ContentType=WindowsRuntime]::new()
    $tokenCredential = $vault.Retrieve($tokenCredentialResource, $tokenCredentialUser)
    $tokenCredential.RetrievePassword()
    $token = [string]$tokenCredential.Password
    if ($token -notmatch '^tm_pat_v1_[A-Za-z0-9_-]{43}$') {
        throw 'The Credential Locker TM production token has an invalid format.'
    }

    $stage = 'live-read-only-preflight'
    $operations = Invoke-TmOperationsStatus -Origin $base -Token $token
    Assert-LiveDeploymentPreflight -Operations $operations -Receipt $configurationReceipt

    if (-not $Apply) {
        Write-Host "Dry run passed: $repository $expectedBranch $ExpectedHeadSha" -ForegroundColor Green
        Write-Host "Configuration receipt: $($configurationReceipt.receiptId)"
        Write-Host "Actions: STEP 10=$Step10RunId STEP 16=$Step16RunId"
        Write-Host 'No source was archived or uploaded.'
        return
    }
    if ($Force) { $ConfirmPreference = 'None' }
    if (-not $PSCmdlet.ShouldProcess(
        'Railway production/tm-server',
        "Upload exact commit $ExpectedHeadSha using configuration receipt $($configurationReceipt.receiptId)"
    )) {
        return
    }

    $stage = 'exact-source-archive'
    New-Item -ItemType Directory -Force -Path $resultDirectory | Out-Null
    $archiveId = [Guid]::NewGuid().ToString('N')
    $sourceArchivePath = Join-Path $resultDirectory "source-$ExpectedHeadSha-$archiveId.zip"
    $stageRoot = Join-Path $resultDirectory "staging-$archiveId"
    $archiveOutput = & $git -C $tmRoot archive `
        --format=zip `
        "--output=$sourceArchivePath" `
        $ExpectedHeadSha `
        app 2>&1
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $sourceArchivePath -PathType Leaf)) {
        throw "Git could not create the exact source archive: $archiveOutput"
    }
    $sourceArchiveSha256 = (Get-FileHash -LiteralPath $sourceArchivePath -Algorithm SHA256).Hash.ToLowerInvariant()
    Expand-Archive -LiteralPath $sourceArchivePath -DestinationPath $stageRoot
    $stagedAppRoot = [System.IO.Path]::GetFullPath((Join-Path $stageRoot 'app'))
    if (-not (Test-Path -LiteralPath (Join-Path $stagedAppRoot 'Dockerfile') -PathType Leaf)) {
        throw 'The exact source archive does not contain app/Dockerfile.'
    }

    $stage = 'source-upload'
    $deploymentStartAttempted = $true
    Push-Location $stagedAppRoot
    try {
        $uploadOutput = @(& $railway up `
            --detach `
            --json `
            --yes `
            --message $message `
            --project $projectId `
            --environment $environment `
            --service $service 2>&1)
        if ($LASTEXITCODE -ne 0) {
            throw 'Railway rejected the exact verified source upload.'
        }
    }
    finally {
        Pop-Location
    }
    $deploymentId = Get-DeploymentIdFromUpload -OutputLines $uploadOutput

    $stage = 'deployment-terminal-status'
    $deployment = Wait-VerifiedRailwayDeployment `
        -RailwayPath $railway `
        -DeploymentId $deploymentId `
        -Message $message

    $stage = 'complete'
    [ordered]@{
        success = $true
        startedAtUtc = $startedAt.ToString('o')
        completedAtUtc = [DateTimeOffset]::UtcNow.ToString('o')
        repository = $repository
        branch = $expectedBranch
        headSha = $ExpectedHeadSha
        step10RunId = $Step10RunId
        step16RunId = $Step16RunId
        configurationReceiptId = [string]$configurationReceipt.receiptId
        configurationReceiptSha256 = $configurationReceiptSha256
        configurationProofSchemaVersion = [int]$configurationReceipt.proofSchemaVersion
        expectedExpenseAiEnabled = [bool]$configurationReceipt.expenseAiEnabled
        projectId = $projectId
        environment = $environment
        service = $service
        deploymentId = $deploymentId
        deploymentStatus = [string]$deployment.status
        deploymentCreatedAt = [string]$deployment.createdAt
        deploymentMessage = $message
        productionImageDigest = ([string]$deployment.meta.imageDigest).ToLowerInvariant()
        sourceArchivePath = $sourceArchivePath
        sourceArchiveSha256 = $sourceArchiveSha256
        sourceArchiveHeadSha = $ExpectedHeadSha
        deploymentWaitRequired = $false
    } | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $ResultPath -Encoding utf8
    Write-Host "Railway deployment succeeded: $deploymentId" -ForegroundColor Green
    Write-Host "Production image digest: $(([string]$deployment.meta.imageDigest).ToLowerInvariant())"
    Write-Host 'Run the read-only production verifier after the schema 15 remote backup succeeds.'
}
catch {
    New-Item -ItemType Directory -Force -Path $resultDirectory | Out-Null
    [ordered]@{
        success = $false
        failedAtUtc = [DateTimeOffset]::UtcNow.ToString('o')
        stage = $stage
        reason = $_.Exception.Message
        headSha = $ExpectedHeadSha
        configurationReceiptId = if ($null -ne $configurationReceipt) {
            [string]$configurationReceipt.receiptId
        } else {
            $null
        }
        deploymentStartAttempted = $deploymentStartAttempted
        deploymentId = $deploymentId
        sourceArchivePath = $sourceArchivePath
        sourceArchiveSha256 = $sourceArchiveSha256
    } | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $ResultPath -Encoding utf8
    throw "Verified Railway source deployment failed at ${stage}: $($_.Exception.Message)"
}
finally {
    if (-not [string]::IsNullOrWhiteSpace($stageRoot) -and
        (Test-Path -LiteralPath $stageRoot -PathType Container)) {
        $resolvedStage = [System.IO.Path]::GetFullPath($stageRoot)
        $resolvedResult = [System.IO.Path]::GetFullPath($resultDirectory).TrimEnd('\') + '\'
        if ($resolvedStage.StartsWith($resolvedResult, [System.StringComparison]::OrdinalIgnoreCase) -and
            (Split-Path -Leaf $resolvedStage) -like 'staging-*') {
            Remove-Item -LiteralPath $resolvedStage -Recurse -Force
        }
    }
    $token = $null
    $tokenCredential = $null
    $vault = $null
    Remove-Variable token, tokenCredential, vault -ErrorAction SilentlyContinue
}

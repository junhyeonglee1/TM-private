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
$mutexName = 'Local\TMExpenseProductionKeyConfiguration'
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
        return [System.IO.File]::ReadAllText($Path, [System.Text.Encoding]::UTF8) | ConvertFrom-TmJson
    }
    catch {
        throw "Required JSON proof is not valid JSON: $Path"
    }
}

function Read-ConfigurationReceipt {
    param([Parameter(Mandatory = $true)][string]$Path)

    $receipt = Read-BoundedJson -Path $Path -MaximumBytes 1048576
    Assert-TmReceiptIntegrityProof $receipt
    Assert-BooleanProperty $receipt 'success' $true
    Assert-BooleanProperty $receipt 'deploymentTriggered' $false
    Assert-BooleanProperty $receipt 'sourceDeploymentRequired' $true
    Assert-BooleanProperty $receipt 'secretValueWrittenToResult' $false
    Assert-BooleanProperty $receipt 'keyWriteAmbiguous' $false
    Assert-BooleanProperty $receipt 'singleUseStateRequired' $true
    Assert-BooleanProperty $receipt 'featureControlAttempted' $true
    Assert-BooleanProperty $receipt 'featureControlAmbiguous' $false
    if ($receipt.expenseAiEnabled -isnot [bool]) {
        throw 'The configuration receipt has an invalid expenseAiEnabled value.'
    }
    if ([int]$receipt.receiptVersion -ne 4 -or
        [string]$receipt.kind -cne 'tm-expense-production-configuration' -or
        [string]$receipt.receiptId -notmatch '^[0-9a-fA-F-]{36}$' -or
        [string]$receipt.repository -cne $repository -or
        [string]$receipt.branch -cne $expectedBranch -or
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
    if ($validity.TotalMinutes -lt 1 -or $validity.TotalMinutes -gt 120.1 -or
        $configuredAt.ToUniversalTime() -gt $now.AddMinutes(5)) {
        throw 'The configuration receipt has an invalid lifetime.'
    }
    $receipt | Add-Member -NotePropertyName runtimeExpired `
        -NotePropertyValue ($expiresAt.ToUniversalTime() -lt $now)
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
        'recoveryCredentialVaultRoundTripVerified',
        'productionFingerprintComparisonDeferred',
        'recoveryCredentialSubmittedToRailway',
        'keyWriteRequired',
        'keyWriteAttempted',
        'keyWriteConfirmed',
        'keyApprovalStateConsumed'
    )) {
        if ($receipt.$name -isnot [bool]) {
            throw "The configuration receipt has a non-Boolean $name proof."
        }
    }
    if ([string]$receipt.expenseKeyFingerprint -notmatch '^tm_exp_kfp_v1_[0-9a-f]{64}$') {
        throw 'The configuration receipt has an invalid expense key fingerprint.'
    }
    if ([string]$receipt.expenseExpectedKeyFingerprint -cne
            [string]$receipt.expenseKeyFingerprint -or
        [string]$receipt.expenseRolloutMode -cne 'locked') {
        throw 'The configuration receipt does not bind the staged key to a locked rollout.'
    }
    $requiredVariableNames = @(
        'TM_EXPENSE_AI_ENABLED',
        'TM_BUILD_COMMIT_SHA',
        'TM_EXPENSE_EXPECTED_KEY_FINGERPRINT',
        'TM_EXPENSE_ROLLOUT_MODE'
    )
    $receiptVariableNames = @($receipt.variableNames | ForEach-Object { [string]$_ })
    foreach ($name in $requiredVariableNames) {
        if ($receiptVariableNames -cnotcontains $name) {
            throw 'The configuration receipt is missing a required locked-rollout variable.'
        }
    }
    if ($receipt.keyWriteRequired -eq $true -and
        $receiptVariableNames -cnotcontains 'TM_EXPENSE_DATA_KEY_V1') {
        throw 'The configuration receipt is missing the staged expense key variable.'
    }
    if ([int]$receipt.proofSchemaVersion -eq 14) {
        if ([string]$receipt.proofMode -ne 'schema14' -or
            $receipt.proofLedgerEmpty -ne $true -or
            $receipt.proofKeyInitialized -ne $false -or
            $receipt.proofCryptoReady -ne $false -or
            $receipt.proofInitializationAllowed -ne $true -or
            $receipt.recoveryCredentialPresent -ne $true -or
            $receipt.recoveryCredentialMatchVerified -ne $false -or
            $receipt.recoveryCredentialVaultRoundTripVerified -ne $true -or
            $receipt.productionFingerprintComparisonDeferred -ne $true -or
            $receipt.recoveryCredentialSubmittedToRailway -ne $true -or
            $receipt.keyWriteRequired -ne $true -or
            $receipt.keyWriteAttempted -ne $true -or
            $receipt.keyWriteConfirmed -ne $true -or
            $receipt.keyApprovalStateConsumed -ne $true -or
            [string]$receipt.keyApprovalId -notmatch '^[0-9a-fA-F-]{36}$' -or
            [string]$receipt.keyApprovalReceiptSha256 -notmatch '^[0-9a-fA-F]{64}$') {
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
                $receipt.recoveryCredentialMatchVerified -ne $false -or
                $receipt.recoveryCredentialVaultRoundTripVerified -ne $true -or
                $receipt.productionFingerprintComparisonDeferred -ne $true -or
                $receipt.recoveryCredentialSubmittedToRailway -ne $true -or
                $receipt.keyWriteAttempted -ne $true -or
                $receipt.keyWriteConfirmed -ne $true -or
                $receipt.keyApprovalStateConsumed -ne $true -or
                [string]$receipt.keyApprovalId -notmatch '^[0-9a-fA-F-]{36}$' -or
                [string]$receipt.keyApprovalReceiptSha256 -notmatch '^[0-9a-fA-F]{64}$') {
                throw 'The schema 15 receipt does not prove a safe staged first-time key.'
            }
        }
        elseif ($receipt.proofKeyInitialized -ne $true -or
            $receipt.proofCryptoReady -ne $true -or
            $receipt.proofInitializationAllowed -ne $false -or
            $receipt.recoveryCredentialMatchVerified -ne $true -or
            $receipt.recoveryCredentialVaultRoundTripVerified -ne $true -or
            $receipt.productionFingerprintComparisonDeferred -ne $false -or
            $receipt.keyWriteAttempted -ne $false -or
            $receipt.keyWriteConfirmed -ne $false -or
            $receipt.keyApprovalStateConsumed -ne $false) {
            throw 'The schema 15 receipt does not prove an already healthy expense key.'
        }
    }
    if ($null -eq $receipt.actionsEvidence -or
        $null -eq $receipt.actionsEvidence.step10 -or
        $null -eq $receipt.actionsEvidence.step16 -or
        $null -eq $receipt.operatorTools -or
        $null -eq $receipt.operatorTools.git -or
        $null -eq $receipt.operatorTools.githubCli -or
        $null -eq $receipt.railwayCli) {
        throw 'The configuration receipt is missing immutable Actions artifact or operator-tool evidence.'
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
        try {
            $body = $response.Content.ReadAsStringAsync().GetAwaiter().GetResult() | ConvertFrom-TmJson
        }
        catch {
            throw 'Production operations status returned invalid JSON.'
        }
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
        if ($Operations.controls.PSObject.Properties.Name -notcontains 'expenseKeyFingerprint') {
            throw 'The live schema 15 service is missing its expense key fingerprint proof.'
        }
        if ($schema15Bootstrap) {
            if ($Operations.controls.expenseLedgerEmpty -ne $true -or
                $Operations.controls.expenseKeyInitialized -ne $false -or
                $Operations.controls.expenseCryptoReady -ne $false -or
                $Operations.controls.expenseKeyInitializationAllowed -ne $true -or
                $null -ne $Operations.controls.expenseKeyFingerprint -or
                $Receipt.productionFingerprintComparisonDeferred -ne $true) {
                throw 'The live schema 15 first-time key proof changed after configuration.'
            }
        }
        elseif ($Operations.controls.expenseKeyInitialized -ne $expectedInitialized -or
            $Operations.controls.expenseCryptoReady -ne $true -or
            $Operations.controls.expenseKeyInitializationAllowed -ne $false -or
            [string]$Operations.controls.expenseKeyFingerprint -cne
                [string]$Receipt.expenseKeyFingerprint -or
            $Receipt.productionFingerprintComparisonDeferred -ne $false) {
            throw 'The live schema 15 service no longer proves its healthy expense key.'
        }
    }
}

function Assert-ExpenseRolloutProof {
    param(
        [Parameter(Mandatory = $true)]$Operations,
        [Parameter(Mandatory = $true)]$Receipt,
        [Parameter(Mandatory = $true)][ValidateSet('locked', 'enabled')][string]$Mode,
        [Parameter(Mandatory = $true)][string]$DeploymentId,
        [switch]$AllowEnabledLedgerChange
    )

    if ($null -eq $Operations.database -or $null -eq $Operations.controls -or
        $null -eq $Operations.deploymentProvenance -or
        $Operations.database.ok -isnot [bool] -or $Operations.database.ok -ne $true -or
        [int]$Operations.database.schemaVersion -ne 15 -or
        [string]$Operations.controls.incidentMode -cne 'normal' -or
        [string]$Operations.controls.expenseRolloutMode -cne $Mode -or
        ([string]$Operations.deploymentProvenance.buildCommitSha).ToLowerInvariant() -cne
            $ExpectedHeadSha -or
        ([string]$Operations.deploymentProvenance.railwayDeploymentId).ToLowerInvariant() -cne
            $DeploymentId.ToLowerInvariant()) {
        throw 'The schema 15 rollout proof is not bound to the exact deployment.'
    }
    foreach ($name in @(
        'expenseCryptoReady', 'expenseLedgerEmpty', 'expenseKeyInitialized',
        'expenseKeyInitializationAllowed', 'expenseExpectedKeyFingerprintMatch'
    )) {
        if ($Operations.controls.PSObject.Properties.Name -notcontains $name -or
            $Operations.controls.$name -isnot [bool]) {
            throw 'The schema 15 rollout proof is missing a required Boolean control.'
        }
    }
    if ($Operations.controls.PSObject.Properties.Name -notcontains
            'expenseActivationFingerprintMatch' -or
        ($null -ne $Operations.controls.expenseActivationFingerprintMatch -and
            $Operations.controls.expenseActivationFingerprintMatch -isnot [bool])) {
        throw 'The schema 15 rollout proof has an invalid activation binding.'
    }
    if ([string]$Operations.controls.expenseKeyFingerprint -cne
            [string]$Receipt.expenseKeyFingerprint -or
        $Operations.controls.expenseExpectedKeyFingerprintMatch -ne $true -or
        ($Mode -eq 'locked' -and $Receipt.keyWriteRequired -eq $true -and
            $Operations.controls.expenseLedgerEmpty -ne [bool]$Receipt.proofLedgerEmpty) -or
        ($Mode -eq 'enabled' -and -not $AllowEnabledLedgerChange -and
            $Operations.controls.expenseLedgerEmpty -ne [bool]$Receipt.proofLedgerEmpty) -or
        $Operations.controls.expenseAiEnabled -isnot [bool] -or
        $Operations.controls.expenseAiEnabled -ne [bool]$Receipt.expenseAiEnabled) {
        throw 'The schema 15 rollout key does not match the approved configuration receipt.'
    }
    $criticalAlerts = @($Operations.alerts | Where-Object { [string]$_.severity -eq 'critical' })
    if ($Mode -eq 'locked') {
        $expectedInitialized = -not [bool]$Receipt.keyWriteRequired
        $expectedInitializationAllowed = [bool]$Receipt.keyWriteRequired
        $unexpectedCritical = @($criticalAlerts | Where-Object {
            [string]$_.code -cne 'EXPENSE_ROLLOUT_LOCKED'
        })
        if ([string]$Operations.overallStatus -cne 'critical' -or
            $criticalAlerts.Count -ne 1 -or $unexpectedCritical.Count -ne 0 -or
            $Operations.controls.expenseCryptoReady -ne $false -or
            $Operations.controls.expenseKeyInitialized -ne $expectedInitialized -or
            $Operations.controls.expenseKeyInitializationAllowed -ne
                $expectedInitializationAllowed) {
            throw 'The first schema 15 deployment is not safely locked before key initialization.'
        }
        $schemaAppliedAt = [DateTimeOffset]::MinValue
        $preMigrationCreatedAt = [DateTimeOffset]::MinValue
        if ($null -eq $Operations.localBackup -or
            -not [DateTimeOffset]::TryParse(
                [string]$Operations.database.currentSchemaAppliedAt,
                [System.Globalization.CultureInfo]::InvariantCulture,
                [System.Globalization.DateTimeStyles]::RoundtripKind,
                [ref]$schemaAppliedAt
            ) -or
            -not [DateTimeOffset]::TryParse(
                [string]$Operations.localBackup.latestPreMigrationCreatedAt,
                [System.Globalization.CultureInfo]::InvariantCulture,
                [System.Globalization.DateTimeStyles]::RoundtripKind,
                [ref]$preMigrationCreatedAt
            ) -or
            [int]$Operations.localBackup.preMigrationCount -lt 1 -or
            [uint64]$Operations.localBackup.latestPreMigrationByteSize -lt 1 -or
            [string]$Operations.localBackup.latestPreMigrationSha256 -notmatch '^[0-9a-f]{64}$' -or
            [int]$Operations.localBackup.latestPreMigrationSchemaVersion -ne 14 -or
            [string]$Operations.localBackup.latestPreMigrationIntegrityCheck -cne 'ok' -or
            $Operations.localBackup.latestPreMigrationSchemaSemanticsValidated -isnot [bool] -or
            $Operations.localBackup.latestPreMigrationSchemaSemanticsValidated -ne $true) {
            throw 'The locked rollout has no verified schema 14 pre-migration rollback backup.'
        }
        $migrationGap = $schemaAppliedAt.ToUniversalTime() -
            $preMigrationCreatedAt.ToUniversalTime()
        if ($migrationGap.TotalMinutes -lt -1 -or $migrationGap.TotalMinutes -gt 30) {
            throw 'The locked rollout pre-migration backup is not bound to the schema 15 migration.'
        }
        return
    }
    if ([string]$Operations.overallStatus -notin @('healthy', 'warning') -or
        $criticalAlerts.Count -ne 0 -or
        $Operations.controls.expenseCryptoReady -ne $true -or
        $Operations.controls.expenseKeyInitialized -ne $true -or
        $Operations.controls.expenseKeyInitializationAllowed -ne $false -or
        $Operations.controls.expenseActivationFingerprintMatch -ne $true) {
        throw 'The activated schema 15 deployment did not prove the approved expense key.'
    }
}

function Wait-ExpenseRolloutProof {
    param(
        [Parameter(Mandatory = $true)][string]$Origin,
        [Parameter(Mandatory = $true)][string]$Token,
        [Parameter(Mandatory = $true)]$Receipt,
        [Parameter(Mandatory = $true)][ValidateSet('locked', 'enabled')][string]$Mode,
        [Parameter(Mandatory = $true)][string]$DeploymentId
    )
    $deadline = [DateTimeOffset]::UtcNow.AddMinutes(5)
    while ([DateTimeOffset]::UtcNow -lt $deadline) {
        try {
            $operations = Invoke-TmOperationsStatus -Origin $Origin -Token $Token
            Assert-ExpenseRolloutProof -Operations $operations -Receipt $Receipt `
                -Mode $Mode -DeploymentId $DeploymentId
            return $operations
        }
        catch {
            Start-Sleep -Seconds 5
        }
    }
    throw 'The schema 15 deployment did not produce the required receipt-bound rollout proof.'
}

function Set-ExpenseRolloutActivation {
    param(
        [Parameter(Mandatory = $true)][string]$RailwayPath,
        [Parameter(Mandatory = $true)][string]$Fingerprint
    )
    $result = Invoke-TmBoundedProcess -FilePath $RailwayPath `
        -Arguments ([string[]]@(
            'variable', 'set',
            "TM_EXPENSE_ACTIVATION_FINGERPRINT=$Fingerprint",
            'TM_EXPENSE_ROLLOUT_MODE=enabled', '--skip-deploys',
            '--project', $projectId, '--environment', $environment, '--service', $service
        )) -TimeoutSeconds 120 -DiscardOutput
    if ($result.ExitCode -ne 0) {
        throw 'Railway rejected the receipt-bound expense rollout activation.'
    }
}

function Get-RailwayDeployments {
    param([Parameter(Mandatory = $true)][string]$RailwayPath)
    $result = Invoke-TmBoundedProcess -FilePath $RailwayPath `
        -Arguments ([string[]]@(
            'deployment', 'list', '--json', '--limit', '20', '--project', $projectId,
            '--environment', $environment, '--service', $service
        )) -TimeoutSeconds 60 -MaximumCapturedCharacters 1048576
    if ($result.ExitCode -ne 0) {
        throw 'Unable to list Railway production deployments.'
    }
    try {
        $result.StandardOutput | ConvertFrom-TmJsonArrayItems
    }
    catch {
        throw 'Railway returned invalid deployment metadata JSON.'
    }
}

function Resolve-ExpenseDeploymentState {
    param(
        [Parameter(Mandatory = $true)]$Operations,
        [Parameter(Mandatory = $true)]$Receipt,
        [Parameter(Mandatory = $true)][string]$RailwayPath,
        [AllowNull()][object[]]$DeploymentSnapshot
    )

    if ($null -eq $Operations.database -or $null -eq $Operations.controls -or
        $null -eq $Operations.deploymentProvenance -or
        $Operations.database.PSObject.Properties.Name -notcontains 'schemaVersion' -or
        $Operations.deploymentProvenance.PSObject.Properties.Name -notcontains
            'buildCommitSha') {
        throw 'Production does not positively prove a complete pre-rollout deployment.'
    }
    $observedHeadSha = ([string]$Operations.deploymentProvenance.buildCommitSha).ToLowerInvariant()
    $hasDeploymentId = $Operations.deploymentProvenance.PSObject.Properties.Name -contains
        'railwayDeploymentId'
    $parsedDeploymentId = [Guid]::Empty
    $deploymentIdValid = $hasDeploymentId -and [Guid]::TryParse(
        [string]$Operations.deploymentProvenance.railwayDeploymentId,
        [ref]$parsedDeploymentId
    )
    $hasRolloutMode = $Operations.controls.PSObject.Properties.Name -contains
        'expenseRolloutMode'
    $rolloutMode = if ($hasRolloutMode) {
        [string]$Operations.controls.expenseRolloutMode
    } else {
        ''
    }
    $expectedShaObserved = $observedHeadSha -ceq $ExpectedHeadSha
    if ($expectedShaObserved -and
        ([int]$Operations.database.schemaVersion -ne 15 -or
            -not $deploymentIdValid -or
            $rolloutMode -notin @('locked', 'enabled'))) {
        throw 'Production exposes a partial exact rollout and cannot be treated as fresh.'
    }
    $exactRolloutCandidate = $expectedShaObserved
    if (-not $exactRolloutCandidate) {
        $positivePreRollout = $observedHeadSha -match '^[0-9a-f]{40}$' -and
            $deploymentIdValid -and
            ([int]$Operations.database.schemaVersion -eq 14 -or
                ([int]$Operations.database.schemaVersion -eq 15 -and
                    $rolloutMode -ceq 'enabled'))
        if (-not $positivePreRollout) {
            throw 'Production does not positively prove a complete pre-rollout deployment.'
        }
        Assert-LiveDeploymentPreflight -Operations $Operations -Receipt $Receipt
        return [pscustomobject]@{ Mode = 'fresh'; Deployment = $null }
    }

    # An exact live locked/enabled rollout must be resolved before the broader
    # pre-deployment proof. Otherwise an enabled deployment can satisfy the broad
    # schema/key proof and be mistaken for a fresh rollout after an activation timeout.
    $deploymentId = $parsedDeploymentId.ToString('D')
    $mode = $rolloutMode
    Assert-ExpenseRolloutProof -Operations $Operations -Receipt $Receipt `
        -Mode $mode -DeploymentId $deploymentId -AllowEnabledLedgerChange
    $expectedMessage = if ($mode -eq 'locked') {
        "schema15-expense-lock-$($ExpectedHeadSha.Substring(0, 12))"
    } else {
        "schema15-expense-activate-$($ExpectedHeadSha.Substring(0, 12))"
    }
    $deployments = if ($PSBoundParameters.ContainsKey('DeploymentSnapshot')) {
        @($DeploymentSnapshot)
    } else {
        @(Get-RailwayDeployments -RailwayPath $RailwayPath)
    }
    $matching = @($deployments | Where-Object {
        ([string]$_.id).ToLowerInvariant() -eq $deploymentId
    })
    $deploymentCreatedAt = [DateTimeOffset]::MinValue
    if ($matching.Count -ne 1 -or
        [string]$matching[0].status -cne 'SUCCESS' -or
        [string]$matching[0].meta.cliMessage -cne $expectedMessage -or
        [string]$matching[0].meta.imageDigest -notmatch '^sha256:[0-9a-fA-F]{64}$' -or
        -not [DateTimeOffset]::TryParse(
            [string]$matching[0].createdAt,
            [System.Globalization.CultureInfo]::InvariantCulture,
            [System.Globalization.DateTimeStyles]::RoundtripKind,
            [ref]$deploymentCreatedAt
        )) {
        throw 'The resumable rollout is not bound to an exact successful Railway deployment.'
    }
    return [pscustomobject]@{ Mode = $mode; Deployment = $matching[0] }
}

function Find-LockedRolloutDeployment {
    param(
        [Parameter(Mandatory = $true)][string]$RailwayPath,
        [Parameter(Mandatory = $true)]$Operations,
        [Parameter(Mandatory = $true)]$Receipt,
        [AllowNull()][object[]]$DeploymentSnapshot
    )

    $parsedEnabledId = [Guid]::Empty
    if ($null -eq $Operations.database -or $null -eq $Operations.controls -or
        $null -eq $Operations.deploymentProvenance -or
        [int]$Operations.database.schemaVersion -ne 15 -or
        [string]$Operations.controls.expenseRolloutMode -cne 'enabled' -or
        ([string]$Operations.deploymentProvenance.buildCommitSha).ToLowerInvariant() -cne
            $ExpectedHeadSha -or
        -not [Guid]::TryParse(
            [string]$Operations.deploymentProvenance.railwayDeploymentId,
            [ref]$parsedEnabledId
        )) {
        throw 'The locked rollout recovery has no exact live enabled deployment proof.'
    }
    $enabledId = $parsedEnabledId.ToString('D')
    Assert-ExpenseRolloutProof -Operations $Operations -Receipt $Receipt `
        -Mode enabled -DeploymentId $enabledId -AllowEnabledLedgerChange

    $deployments = if ($PSBoundParameters.ContainsKey('DeploymentSnapshot')) {
        @($DeploymentSnapshot)
    } else {
        @(Get-RailwayDeployments -RailwayPath $RailwayPath)
    }
    $timeline = [System.Collections.Generic.List[object]]::new()
    $deploymentIds = [System.Collections.Generic.HashSet[string]]::new(
        [System.StringComparer]::OrdinalIgnoreCase
    )
    foreach ($item in $deployments) {
        $parsedId = [Guid]::Empty
        $createdAt = [DateTimeOffset]::MinValue
        if (-not [Guid]::TryParse([string]$item.id, [ref]$parsedId) -or
            -not [DateTimeOffset]::TryParse(
                [string]$item.createdAt,
                [System.Globalization.CultureInfo]::InvariantCulture,
                [System.Globalization.DateTimeStyles]::RoundtripKind,
                [ref]$createdAt
            ) -or -not $deploymentIds.Add($parsedId.ToString('D'))) {
            throw 'Railway returned an ambiguous deployment ID or creation timestamp.'
        }
        $timeline.Add([pscustomobject]@{
            Id = $parsedId.ToString('D')
            CreatedAt = $createdAt.ToUniversalTime()
            Deployment = $item
        })
    }
    $ordered = @($timeline | Sort-Object CreatedAt -Descending)
    if ($ordered.Count -lt 2 -or $ordered[0].Id -cne $enabledId -or
        $ordered[0].CreatedAt -le $ordered[1].CreatedAt) {
        throw 'The live enabled deployment is not uniquely followed by an exact predecessor.'
    }

    $enabled = $ordered[0].Deployment
    $locked = $ordered[1].Deployment
    $enabledMessage = "schema15-expense-activate-$($ExpectedHeadSha.Substring(0, 12))"
    $lockedMessage = "schema15-expense-lock-$($ExpectedHeadSha.Substring(0, 12))"
    $enabledDigest = ([string]$enabled.meta.imageDigest).ToLowerInvariant()
    $lockedDigest = ([string]$locked.meta.imageDigest).ToLowerInvariant()
    $configuredAt = [DateTimeOffset]::MinValue
    if ([string]$enabled.status -cne 'SUCCESS' -or
        [string]$enabled.meta.cliMessage -cne $enabledMessage -or
        $enabledDigest -notmatch '^sha256:[0-9a-f]{64}$' -or
        [string]$locked.status -notin @('SUCCESS', 'REMOVED') -or
        [string]$locked.meta.cliMessage -cne $lockedMessage -or
        $lockedDigest -notmatch '^sha256:[0-9a-f]{64}$' -or
        $lockedDigest -cne $enabledDigest -or
        -not [DateTimeOffset]::TryParse(
            [string]$Receipt.configuredAtUtc,
            [System.Globalization.CultureInfo]::InvariantCulture,
            [System.Globalization.DateTimeStyles]::RoundtripKind,
            [ref]$configuredAt
        ) -or $ordered[1].CreatedAt -lt $configuredAt.ToUniversalTime().AddMinutes(-5)) {
        throw 'The enabled rollout has no exact digest-bound locked predecessor proof.'
    }
    return [pscustomobject]@{
        EnabledDeployment = $enabled
        LockedDeployment = $locked
        LockedDeploymentRecoveredAfterActivation =
            ([string]$locked.status -ceq 'REMOVED')
        RecoveryVerifiedAtUtc = [DateTimeOffset]::UtcNow.ToString('o')
    }
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
            $value = $text | ConvertFrom-TmJson
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

    # Regression: after an activation timeout, the broad schema/key preflight is
    # still true. The exact enabled deployment must win, and Railway's REMOVED
    # status for its immediately preceding locked deployment is accepted only as
    # an exact, digest-identical timeline pair.
    $recoveryNow = [DateTimeOffset]::UtcNow
    $lockedId = '33333333-3333-4333-8333-333333333333'
    $enabledId = '44444444-4444-4444-8444-444444444444'
    $digest = 'sha256:' + ('b' * 64)
    $recoveryReceipt = [pscustomobject]@{
        configuredAtUtc = $recoveryNow.AddMinutes(-10).ToString('o')
        proofLedgerEmpty = $false
        keyWriteRequired = $false
        expenseKeyFingerprint = 'tm_exp_kfp_v1_' + ('c' * 64)
        expenseAiEnabled = $false
    }
    $recoveryOperations = [pscustomobject]@{
        database = [pscustomobject]@{ schemaVersion = 15; ok = $true }
        controls = [pscustomobject]@{
            incidentMode = 'normal'; expenseRolloutMode = 'enabled'
            expenseCryptoReady = $true; expenseLedgerEmpty = $false
            expenseKeyInitialized = $true; expenseKeyInitializationAllowed = $false
            expenseExpectedKeyFingerprintMatch = $true
            expenseActivationFingerprintMatch = $true
            expenseKeyFingerprint = $recoveryReceipt.expenseKeyFingerprint
            expenseAiEnabled = $false
        }
        deploymentProvenance = [pscustomobject]@{
            buildCommitSha = $ExpectedHeadSha; railwayDeploymentId = $enabledId
        }
        overallStatus = 'healthy'
        alerts = @()
    }
    $lockedDeployment = [pscustomobject]@{
        id = $lockedId; status = 'REMOVED'; createdAt = $recoveryNow.AddMinutes(-5).ToString('o')
        meta = [pscustomobject]@{
            cliMessage = "schema15-expense-lock-$($ExpectedHeadSha.Substring(0, 12))"
            imageDigest = $digest
        }
    }
    $enabledDeployment = [pscustomobject]@{
        id = $enabledId; status = 'SUCCESS'; createdAt = $recoveryNow.AddMinutes(-4).ToString('o')
        meta = [pscustomobject]@{
            cliMessage = "schema15-expense-activate-$($ExpectedHeadSha.Substring(0, 12))"
            imageDigest = $digest
        }
    }
    $snapshot = @($enabledDeployment, $lockedDeployment)
    $resolvedAfterTimeout = Resolve-ExpenseDeploymentState `
        -Operations $recoveryOperations -Receipt $recoveryReceipt `
        -RailwayPath 'self-test' -DeploymentSnapshot $snapshot
    if ([string]$resolvedAfterTimeout.Mode -cne 'enabled') {
        throw 'Deployment guard self-test treated an exact enabled timeout recovery as fresh.'
    }
    $invalidModeOperations = ($recoveryOperations | ConvertTo-Json -Depth 8) |
        ConvertFrom-TmJson
    $invalidModeOperations.controls.expenseRolloutMode = 'invalid'
    $threw = $false
    try {
        Resolve-ExpenseDeploymentState -Operations $invalidModeOperations `
            -Receipt $recoveryReceipt -RailwayPath 'self-test' `
            -DeploymentSnapshot $snapshot | Out-Null
    }
    catch {
        $threw = $_.Exception.Message -like '*partial exact rollout*'
    }
    if (-not $threw) {
        throw 'Deployment guard self-test treated an expected-SHA invalid mode as fresh.'
    }
    $missingIdOperations = ($recoveryOperations | ConvertTo-Json -Depth 8) |
        ConvertFrom-TmJson
    $missingIdOperations.deploymentProvenance = [pscustomobject]@{
        buildCommitSha = $ExpectedHeadSha
    }
    $threw = $false
    try {
        Resolve-ExpenseDeploymentState -Operations $missingIdOperations `
            -Receipt $recoveryReceipt -RailwayPath 'self-test' `
            -DeploymentSnapshot $snapshot | Out-Null
    }
    catch {
        $threw = $_.Exception.Message -like '*partial exact rollout*'
    }
    if (-not $threw) {
        throw 'Deployment guard self-test treated missing exact deployment provenance as fresh.'
    }
    $recoveredPair = Find-LockedRolloutDeployment -RailwayPath 'self-test' `
        -Operations $recoveryOperations -Receipt $recoveryReceipt `
        -DeploymentSnapshot $snapshot
    if (-not [bool]$recoveredPair.LockedDeploymentRecoveredAfterActivation -or
        [string]$recoveredPair.LockedDeployment.id -cne $lockedId) {
        throw 'Deployment guard self-test rejected the exact removed locked predecessor.'
    }
    $lockedDeployment.meta.imageDigest = 'sha256:' + ('d' * 64)
    $threw = $false
    try {
        Find-LockedRolloutDeployment -RailwayPath 'self-test' `
            -Operations $recoveryOperations -Receipt $recoveryReceipt `
            -DeploymentSnapshot $snapshot | Out-Null
    }
    catch {
        $threw = $_.Exception.Message -like '*digest-bound locked predecessor*'
    }
    if (-not $threw) {
        throw 'Deployment guard self-test accepted a removed locked deployment with a different digest.'
    }

    $receiptPath = Join-Path ([System.IO.Path]::GetTempPath()) "tm-expense-receipt-$([Guid]::NewGuid().ToString('N')).json"
    try {
        $now = [DateTimeOffset]::UtcNow
        $step10Evidence = [pscustomobject]@{
            workflowName = $step10WorkflowName; workflowPath = $step10WorkflowPath
            runId = $Step10RunId; runAttempt = 1; event = 'push'; headSha = $ExpectedHeadSha
            artifactName = $step10ArtifactName; artifactId = 10
            artifactDigest = 'sha256:' + ('1' * 64); artifactSizeBytes = 1
            manifestSha256 = '2' * 64; manifestVerified = $true; provenanceVerified = $false
            verifiedFileHashes = [ordered]@{ 'tm.exe' = '3' * 64 }
        }
        $step16Evidence = [pscustomobject]@{
            workflowName = $step16WorkflowName; workflowPath = $step16WorkflowPath
            runId = $Step16RunId; runAttempt = 1; event = 'push'; headSha = $ExpectedHeadSha
            artifactName = $step16ArtifactName; artifactId = 16
            artifactDigest = 'sha256:' + ('4' * 64); artifactSizeBytes = 1
            manifestSha256 = '5' * 64; manifestVerified = $true; provenanceVerified = $true
            verifiedFileHashes = [ordered]@{ 'container-provenance.json' = '6' * 64 }
        }
        $receipt = [ordered]@{
            success = $true
            receiptVersion = 4
            kind = 'tm-expense-production-configuration'
            receiptId = [Guid]::NewGuid().ToString('D')
            configuredAtUtc = $now.ToString('o')
            expiresAtUtc = $now.AddMinutes(120).ToString('o')
            repository = $repository
            branch = $expectedBranch
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
            expenseExpectedKeyFingerprint = 'tm_exp_kfp_v1_' + ('a' * 64)
            expenseRolloutMode = 'locked'
            variableNames = @(
                'TM_EXPENSE_DATA_KEY_V1', 'TM_EXPENSE_AI_ENABLED', 'TM_BUILD_COMMIT_SHA',
                'TM_EXPENSE_EXPECTED_KEY_FINGERPRINT', 'TM_EXPENSE_ROLLOUT_MODE'
            )
            recoveryCredentialPresent = $true
            recoveryCredentialMatchVerified = $false
            recoveryCredentialVaultRoundTripVerified = $true
            expenseKeyFingerprint = 'tm_exp_kfp_v1_' + ('a' * 64)
            productionFingerprintComparisonDeferred = $true
            recoveryCredentialSubmittedToRailway = $true
            keyWriteRequired = $true
            keyWriteAttempted = $true
            keyWriteConfirmed = $true
            keyWriteAmbiguous = $false
            keyApprovalId = [Guid]::NewGuid().ToString('D')
            keyApprovalReceiptSha256 = '7' * 64
            keyApprovalStateConsumed = $true
            actionsEvidence = [ordered]@{ step10 = $step10Evidence; step16 = $step16Evidence }
            operatorTools = [ordered]@{
                git = [ordered]@{
                    version = '2.51.0'; sha256 = '9' * 64; authenticodeStatus = 'Valid'
                    signerSubject = 'CN=Johannes Schindelin'; installKind = 'codex-bundled-runtime'
                }
                githubCli = [ordered]@{
                    version = '2.80.0'; sha256 = 'a' * 64; authenticodeStatus = 'Valid'
                    signerSubject = 'CN=GitHub, Inc.'; installKind = 'official-program-files'
                }
            }
            railwayCli = [ordered]@{
                version = '5.28.1'; sha256 = '8' * 64; authenticodeStatus = 'NotSigned'
                signerThumbprint = ''; installKind = 'official-pnpm-store'
            }
            secretValueWrittenToResult = $false
            deploymentTriggered = $false
            sourceDeploymentRequired = $true
            featureControlAttempted = $true
            featureControlAmbiguous = $false
            singleUseStateRequired = $true
            integrityProofKind = 'dpapi-current-user-v1'
        }
        Add-TmReceiptIntegrityProof $receipt
        Write-TmJsonNoBom -Value $receipt -Path $receiptPath
        $validated = Read-ConfigurationReceipt -Path $receiptPath
        if ([string]$validated.expectedHeadSha -ne $ExpectedHeadSha) {
            throw 'Deployment guard self-test did not preserve the receipt commit binding.'
        }
        $receipt.success = 'true'
        Write-TmJsonNoBom -Value $receipt -Path $receiptPath
        $threw = $false
        try {
            Read-ConfigurationReceipt -Path $receiptPath | Out-Null
        }
        catch {
            $threw = $_.Exception.Message -like '*changed after it was approved*'
        }
        if (-not $threw) {
            throw 'Deployment guard self-test accepted a tampered configuration receipt.'
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
    Invoke-TmReleaseEvidenceGuardSelfTest
    exit 0
}

$gitToolEvidence = Resolve-TmVerifiedGit
$git = [string]$gitToolEvidence.path
$null = Assert-TmCanonicalGitState -GitPath $git -RepositoryRoot $tmRoot `
    -Repository $repository -Branch $expectedBranch -ExpectedHeadSha $ExpectedHeadSha
$ghToolEvidence = Resolve-TmVerifiedGh
$gh = [string]$ghToolEvidence.path
$step10Evidence = Get-TmVerifiedActionsArtifactEvidence -GhPath $gh `
    -Repository $repository -RunId $Step10RunId -ExpectedHeadSha $ExpectedHeadSha `
    -ExpectedBranch $expectedBranch -ExpectedWorkflowName $step10WorkflowName `
    -ExpectedWorkflowPath $step10WorkflowPath -ArtifactKind step10 `
    -ArtifactName $step10ArtifactName
$step16Evidence = Get-TmVerifiedActionsArtifactEvidence -GhPath $gh `
    -Repository $repository -RunId $Step16RunId -ExpectedHeadSha $ExpectedHeadSha `
    -ExpectedBranch $expectedBranch -ExpectedWorkflowName $step16WorkflowName `
    -ExpectedWorkflowPath $step16WorkflowPath -ArtifactKind step16 `
    -ArtifactName $step16ArtifactName
$configurationReceipt = Read-ConfigurationReceipt -Path $ConfigurationResultPath
Assert-TmReleaseEvidenceMatches -Expected $configurationReceipt.actionsEvidence.step10 `
    -Actual $step10Evidence -Name 'STEP 10'
Assert-TmReleaseEvidenceMatches -Expected $configurationReceipt.actionsEvidence.step16 `
    -Actual $step16Evidence -Name 'STEP 16'
Assert-TmSignedToolMatches -Expected $configurationReceipt.operatorTools.git `
    -Actual $gitToolEvidence -Name 'Git'
Assert-TmSignedToolMatches -Expected $configurationReceipt.operatorTools.githubCli `
    -Actual $ghToolEvidence -Name 'GitHub CLI'
$railwayEvidence = Resolve-TmVerifiedRailwayCli
Assert-TmRailwayCliMatches -Expected $configurationReceipt.railwayCli -Actual $railwayEvidence
$railway = [string]$railwayEvidence.path
$configurationReceiptAlreadyConsumed = $false
try {
    $null = Assert-TmPendingReceiptState -StateKind configuration `
        -ReceiptId ([string]$configurationReceipt.receiptId) `
        -ReceiptPath $ConfigurationResultPath -AllowExpired
}
catch {
    $null = Assert-TmConsumedReceiptState -StateKind configuration `
        -ReceiptId ([string]$configurationReceipt.receiptId) `
        -ReceiptPath $ConfigurationResultPath
    $configurationReceiptAlreadyConsumed = $true
}
$configurationReceiptSha256 = (Get-FileHash -LiteralPath $ConfigurationResultPath -Algorithm SHA256).Hash.ToLowerInvariant()

$vault = $null
$tokenCredential = $null
$token = $null
$stageRoot = $null
$sourceArchivePath = $null
$sourceArchiveSha256 = $null
$stagedSourceManifestSha256 = $null
$deploymentStartAttempted = $false
$deploymentStartAmbiguous = $false
$deploymentId = $null
$lockedDeploymentId = $null
$lockedDeployment = $null
$lockedDeploymentRecoveredAfterActivation = $false
$lockedDeploymentRecoveryVerifiedAt = $null
$enabledOperations = $null
$activationVariableAttempted = $false
$activationVariableAmbiguous = $false
$activationDeploymentAttempted = $false
$activationDeploymentAmbiguous = $false
$expenseActivationVerifiedAt = $null
$configurationReceiptConsumed = $false
$deploymentState = $null
$mutex = $null
$mutexAcquired = $false
$abandonedMutexRecovered = $false
$stage = 'credential-locker'
$lockedMessage = "schema15-expense-lock-$($ExpectedHeadSha.Substring(0, 12))"
$message = "schema15-expense-activate-$($ExpectedHeadSha.Substring(0, 12))"
$startedAt = [DateTimeOffset]::UtcNow

try {
    $stage = 'deployment-mutex'
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

    $stage = 'credential-locker'
    $vault = [Windows.Security.Credentials.PasswordVault,Windows.Security.Credentials,ContentType=WindowsRuntime]::new()
    $tokenCredential = $vault.Retrieve($tokenCredentialResource, $tokenCredentialUser)
    $tokenCredential.RetrievePassword()
    $token = [string]$tokenCredential.Password
    if ($token -notmatch '^tm_pat_v1_[A-Za-z0-9_-]{43}$') {
        throw 'The Credential Locker TM production token has an invalid format.'
    }

    $stage = 'live-read-only-preflight'
    $operations = Invoke-TmOperationsStatus -Origin $base -Token $token
    $deploymentState = Resolve-ExpenseDeploymentState -Operations $operations `
        -Receipt $configurationReceipt -RailwayPath $railway
    if ($configurationReceipt.runtimeExpired -eq $true -and
        [string]$deploymentState.Mode -eq 'fresh') {
        throw 'An expired configuration receipt may resume an exact locked/enabled rollout but cannot start a fresh deployment.'
    }
    if ($configurationReceiptAlreadyConsumed -and
        [string]$deploymentState.Mode -ne 'enabled') {
        throw 'A consumed configuration receipt can resume only its exact enabled deployment receipt write.'
    }

    if (-not $Apply) {
        Write-Host "Dry run passed: $repository $expectedBranch $ExpectedHeadSha" -ForegroundColor Green
        Write-Host "Configuration receipt: $($configurationReceipt.receiptId)"
        Write-Host "Actions: STEP 10=$Step10RunId STEP 16=$Step16RunId"
        Write-Host "Rollout state: $($deploymentState.Mode)"
        Write-Host 'No source was archived or uploaded.'
        return
    }
    $confirmBypassRequested = $PSBoundParameters.ContainsKey('Confirm') -and
        -not [bool]$PSBoundParameters['Confirm']
    if ($Force -or $confirmBypassRequested) {
        throw 'Production source deployment refuses -Force and -Confirm:$false.'
    }
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
    Assert-TmSafeGitEnvironment
    $null = Invoke-TmVerifiedGitText -GitPath $git `
        -Arguments @(
            '-c', 'core.attributesFile=NUL', '-C', $tmRoot,
            'archive', '--format=zip', "--output=$sourceArchivePath",
            "$ExpectedHeadSha^{commit}", 'app'
        ) `
        -FailureMessage 'Git could not create the exact source archive.' -AllowEmpty
    if (-not (Test-Path -LiteralPath $sourceArchivePath -PathType Leaf)) {
        throw 'Git did not create the exact source archive.'
    }
    $sourceArchiveSha256 = (Get-FileHash -LiteralPath $sourceArchivePath -Algorithm SHA256).Hash.ToLowerInvariant()
    Expand-TmSafeSourceArchive -ZipPath $sourceArchivePath -DestinationRoot $stageRoot
    $stagedAppRoot = [System.IO.Path]::GetFullPath((Join-Path $stageRoot 'app'))
    if (-not (Test-Path -LiteralPath (Join-Path $stagedAppRoot 'Dockerfile') -PathType Leaf)) {
        throw 'The exact source archive does not contain app/Dockerfile.'
    }
    $stagedSourceManifestSha256 = Get-TmDirectoryManifestSha256 -Root $stageRoot

    $stage = 'final-live-read-only-preflight'
    $finalOperations = Invoke-TmOperationsStatus -Origin $base -Token $token
    $finalDeploymentState = Resolve-ExpenseDeploymentState -Operations $finalOperations `
        -Receipt $configurationReceipt -RailwayPath $railway
    $initialStateDeploymentId = if ($null -eq $deploymentState.Deployment) {
        ''
    } else {
        [string]$deploymentState.Deployment.id
    }
    $finalStateDeploymentId = if ($null -eq $finalDeploymentState.Deployment) {
        ''
    } else {
        [string]$finalDeploymentState.Deployment.id
    }
    if ([string]$finalDeploymentState.Mode -cne [string]$deploymentState.Mode -or
        $finalStateDeploymentId -cne $initialStateDeploymentId) {
        throw 'The production rollout state changed while exact source evidence was prepared.'
    }
    $finalRailwayEvidence = Resolve-TmVerifiedRailwayCli
    Assert-TmRailwayCliMatches -Expected $configurationReceipt.railwayCli -Actual $finalRailwayEvidence
    $finalGitToolEvidence = Resolve-TmVerifiedGit
    Assert-TmSignedToolMatches -Expected $configurationReceipt.operatorTools.git `
        -Actual $finalGitToolEvidence -Name 'Git'
    $git = [string]$finalGitToolEvidence.path
    $null = Assert-TmCanonicalGitState -GitPath $git -RepositoryRoot $tmRoot `
        -Repository $repository -Branch $expectedBranch -ExpectedHeadSha $ExpectedHeadSha
    $finalGhToolEvidence = Resolve-TmVerifiedGh
    Assert-TmSignedToolMatches -Expected $configurationReceipt.operatorTools.githubCli `
        -Actual $finalGhToolEvidence -Name 'GitHub CLI'
    $railway = [string]$finalRailwayEvidence.path
    $finalArchiveSha256 = (Get-FileHash -LiteralPath $sourceArchivePath -Algorithm SHA256).Hash.ToLowerInvariant()
    $finalStagedSourceManifestSha256 = Get-TmDirectoryManifestSha256 -Root $stageRoot
    if ($finalArchiveSha256 -cne $sourceArchiveSha256 -or
        $finalStagedSourceManifestSha256 -cne $stagedSourceManifestSha256) {
        throw 'The exact source archive or staged source changed before upload.'
    }

    if ([string]$deploymentState.Mode -eq 'fresh') {
        $deploymentStartAttempted = $true
        $deploymentStartAmbiguous = $true
        $stage = 'source-upload'
        $uploadResult = Invoke-TmBoundedProcess -FilePath $railway -WorkingDirectory $stagedAppRoot `
            -Arguments ([string[]]@(
                'up', '--detach', '--json', '--yes', '--message', $lockedMessage,
                '--project', $projectId, '--environment', $environment, '--service', $service
            )) -TimeoutSeconds 300 -MaximumCapturedCharacters 1048576
        if ($uploadResult.ExitCode -ne 0) {
            throw 'Railway rejected the exact verified source upload.'
        }
        $uploadOutput = @(
            @($uploadResult.StandardOutput -split "`r?`n") +
            @($uploadResult.StandardError -split "`r?`n")
        )
        $lockedDeploymentId = Get-DeploymentIdFromUpload -OutputLines $uploadOutput
        $deploymentStartAmbiguous = $false

        $stage = 'locked-deployment-terminal-status'
        $lockedDeployment = Wait-VerifiedRailwayDeployment `
            -RailwayPath $railway `
            -DeploymentId $lockedDeploymentId `
            -Message $lockedMessage

        $postLockedArchiveSha256 = (Get-FileHash -LiteralPath $sourceArchivePath -Algorithm SHA256).Hash.ToLowerInvariant()
        $postLockedManifestSha256 = Get-TmDirectoryManifestSha256 -Root $stageRoot
        if ($postLockedArchiveSha256 -cne $sourceArchiveSha256 -or
            $postLockedManifestSha256 -cne $stagedSourceManifestSha256) {
            throw 'The exact source archive or staging tree changed during the locked upload.'
        }

        $stage = 'locked-rollout-proof'
        $null = Wait-ExpenseRolloutProof -Origin $base -Token $token `
            -Receipt $configurationReceipt -Mode locked -DeploymentId $lockedDeploymentId
    }
    elseif ([string]$deploymentState.Mode -eq 'locked') {
        $lockedDeployment = $deploymentState.Deployment
        $lockedDeploymentId = [string]$lockedDeployment.id
    }
    else {
        $deployment = $finalDeploymentState.Deployment
        $deploymentId = [string]$deployment.id
        $enabledOperations = $finalOperations
        $expenseActivationVerifiedAt = [DateTimeOffset]::UtcNow
    }

    if ([string]$deploymentState.Mode -ne 'enabled') {
        $stage = 'receipt-bound-rollout-activation'
        $activationVariableAttempted = $true
        $activationVariableAmbiguous = $true
        Set-ExpenseRolloutActivation -RailwayPath $railway `
            -Fingerprint ([string]$configurationReceipt.expenseKeyFingerprint)
        $activationVariableAmbiguous = $false

        $preActivationArchiveSha256 = (Get-FileHash -LiteralPath $sourceArchivePath -Algorithm SHA256).Hash.ToLowerInvariant()
        $preActivationManifestSha256 = Get-TmDirectoryManifestSha256 -Root $stageRoot
        if ($preActivationArchiveSha256 -cne $sourceArchiveSha256 -or
            $preActivationManifestSha256 -cne $stagedSourceManifestSha256) {
            throw 'The exact source archive or staging tree changed before the activation upload.'
        }

        $stage = 'activation-source-upload'
        $activationDeploymentAttempted = $true
        $activationDeploymentAmbiguous = $true
        $activationUploadResult = Invoke-TmBoundedProcess -FilePath $railway `
            -WorkingDirectory $stagedAppRoot -Arguments ([string[]]@(
                'up', '--detach', '--json', '--yes', '--message', $message,
                '--project', $projectId, '--environment', $environment, '--service', $service
            )) -TimeoutSeconds 300 -MaximumCapturedCharacters 1048576
        if ($activationUploadResult.ExitCode -ne 0) {
            throw 'Railway rejected the receipt-bound activation source upload.'
        }
        $activationUploadOutput = @(
            @($activationUploadResult.StandardOutput -split "`r?`n") +
            @($activationUploadResult.StandardError -split "`r?`n")
        )
        $deploymentId = Get-DeploymentIdFromUpload -OutputLines $activationUploadOutput
        $activationDeploymentAmbiguous = $false

        $stage = 'activation-deployment-terminal-status'
        $deployment = Wait-VerifiedRailwayDeployment -RailwayPath $railway `
            -DeploymentId $deploymentId -Message $message

        $postActivationArchiveSha256 = (Get-FileHash -LiteralPath $sourceArchivePath -Algorithm SHA256).Hash.ToLowerInvariant()
        $postActivationManifestSha256 = Get-TmDirectoryManifestSha256 -Root $stageRoot
        if ($postActivationArchiveSha256 -cne $sourceArchiveSha256 -or
            $postActivationManifestSha256 -cne $stagedSourceManifestSha256) {
            throw 'The exact source archive or staging tree changed during the activation upload.'
        }

        $stage = 'enabled-rollout-proof'
        $enabledOperations = Wait-ExpenseRolloutProof -Origin $base -Token $token `
            -Receipt $configurationReceipt -Mode enabled -DeploymentId $deploymentId
        $expenseActivationVerifiedAt = [DateTimeOffset]::UtcNow
    }

    $stage = 'activated-rollout-pair-proof'
    if ($null -eq $enabledOperations) {
        throw 'The activation completed without an exact live enabled proof.'
    }
    $rolloutPair = Find-LockedRolloutDeployment -RailwayPath $railway `
        -Operations $enabledOperations -Receipt $configurationReceipt
    $deployment = $rolloutPair.EnabledDeployment
    $deploymentId = [string]$deployment.id
    $lockedDeployment = $rolloutPair.LockedDeployment
    $lockedDeploymentId = [string]$lockedDeployment.id
    $lockedDeploymentRecoveredAfterActivation =
        [bool]$rolloutPair.LockedDeploymentRecoveredAfterActivation
    $lockedDeploymentRecoveryVerifiedAt =
        [string]$rolloutPair.RecoveryVerifiedAtUtc

    $stage = 'configuration-receipt-final-consumption'
    if (-not $configurationReceiptAlreadyConsumed) {
        $null = Assert-TmPendingReceiptState -StateKind configuration `
            -ReceiptId ([string]$configurationReceipt.receiptId) `
            -ReceiptPath $ConfigurationResultPath -Consume -AllowExpired
    }
    $configurationReceiptConsumed = $true

    $stage = 'complete'
    $deploymentReceipt = [ordered]@{
        success = $true
        receiptVersion = 3
        kind = 'tm-expense-production-deployment'
        receiptId = [Guid]::NewGuid().ToString('D')
        startedAtUtc = $startedAt.ToString('o')
        completedAtUtc = [DateTimeOffset]::UtcNow.ToString('o')
        repository = $repository
        branch = $expectedBranch
        headSha = $ExpectedHeadSha
        step10RunId = $Step10RunId
        step16RunId = $Step16RunId
        step10RunAttempt = [long]$step10Evidence.runAttempt
        step16RunAttempt = [long]$step16Evidence.runAttempt
        actionsEvidence = [ordered]@{
            step10 = $step10Evidence
            step16 = $step16Evidence
        }
        operatorTools = [ordered]@{
            git = [ordered]@{
                version = [string]$finalGitToolEvidence.version
                sha256 = [string]$finalGitToolEvidence.sha256
                authenticodeStatus = [string]$finalGitToolEvidence.authenticodeStatus
                signerSubject = [string]$finalGitToolEvidence.signerSubject
                installKind = [string]$finalGitToolEvidence.installKind
            }
            githubCli = [ordered]@{
                version = [string]$finalGhToolEvidence.version
                sha256 = [string]$finalGhToolEvidence.sha256
                authenticodeStatus = [string]$finalGhToolEvidence.authenticodeStatus
                signerSubject = [string]$finalGhToolEvidence.signerSubject
                installKind = [string]$finalGhToolEvidence.installKind
            }
        }
        railwayCli = [ordered]@{
            version = [string]$finalRailwayEvidence.version
            sha256 = [string]$finalRailwayEvidence.sha256
            authenticodeStatus = [string]$finalRailwayEvidence.authenticodeStatus
            signerThumbprint = [string]$finalRailwayEvidence.signerThumbprint
            installKind = [string]$finalRailwayEvidence.installKind
        }
        configurationReceiptId = [string]$configurationReceipt.receiptId
        configurationReceiptSha256 = $configurationReceiptSha256
        configurationReceiptVersion = [int]$configurationReceipt.receiptVersion
        configurationReceiptIntegrityProofKind = [string]$configurationReceipt.integrityProofKind
        configurationReceiptStateConsumed = $true
        configurationProofSchemaVersion = [int]$configurationReceipt.proofSchemaVersion
        expectedExpenseAiEnabled = [bool]$configurationReceipt.expenseAiEnabled
        expectedExpenseKeyFingerprint = [string]$configurationReceipt.expenseKeyFingerprint
        expenseRolloutActivated = $true
        expenseActivationVerifiedAtUtc = $expenseActivationVerifiedAt.ToString('o')
        projectId = $projectId
        environment = $environment
        service = $service
        deploymentId = $deploymentId
        deploymentStatus = [string]$deployment.status
        deploymentCreatedAt = [string]$deployment.createdAt
        deploymentMessage = $message
        productionImageDigest = ([string]$deployment.meta.imageDigest).ToLowerInvariant()
        lockedDeploymentId = $lockedDeploymentId
        lockedDeploymentStatus = [string]$lockedDeployment.status
        lockedDeploymentCreatedAt = [string]$lockedDeployment.createdAt
        lockedDeploymentMessage = $lockedMessage
        lockedProductionImageDigest = ([string]$lockedDeployment.meta.imageDigest).ToLowerInvariant()
        lockedDeploymentRecoveredAfterActivation =
            $lockedDeploymentRecoveredAfterActivation
        lockedDeploymentRemovalEligibleAfterActivation = $true
        lockedDeploymentRecoveryVerifiedAtUtc =
            $lockedDeploymentRecoveryVerifiedAt
        sourceArchivePath = $sourceArchivePath
        sourceArchiveSha256 = $sourceArchiveSha256
        stagedSourceManifestSha256 = $stagedSourceManifestSha256
        sourceArchiveHeadSha = $ExpectedHeadSha
        deploymentWaitRequired = $false
        deploymentMutexAcquired = $true
        abandonedMutexRecovered = $abandonedMutexRecovered
        integrityProofKind = 'dpapi-current-user-v1'
    }
    Add-TmReceiptIntegrityProof $deploymentReceipt
    Write-TmJsonNoBom -Value $deploymentReceipt -Path $ResultPath
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
        deploymentStartAmbiguous = $deploymentStartAmbiguous
        lockedDeploymentId = $lockedDeploymentId
        lockedDeploymentRecoveredAfterActivation =
            $lockedDeploymentRecoveredAfterActivation
        activationVariableAttempted = $activationVariableAttempted
        activationVariableAmbiguous = $activationVariableAmbiguous
        activationDeploymentAttempted = $activationDeploymentAttempted
        activationDeploymentAmbiguous = $activationDeploymentAmbiguous
        configurationReceiptConsumed = $configurationReceiptConsumed
        deploymentMutexAcquired = $mutexAcquired
        abandonedMutexRecovered = $abandonedMutexRecovered
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
    if ($mutexAcquired -and $null -ne $mutex) { $mutex.ReleaseMutex() }
    if ($null -ne $mutex) { $mutex.Dispose() }
}

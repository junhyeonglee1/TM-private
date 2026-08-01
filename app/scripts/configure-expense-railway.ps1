[CmdletBinding(SupportsShouldProcess = $true, ConfirmImpact = 'High')]
param(
    [ValidateSet('true', 'false')]
    [string]$ExpenseAiEnabled = 'true',
    [string]$ExpectedHeadSha,
    [long]$Step10RunId,
    [long]$Step16RunId,
    [string]$ResultPath,
    [switch]$InitializeNewKey,
    [switch]$RunGuardSelfTest,
    [switch]$Apply,
    [switch]$Force
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
Add-Type -AssemblyName System.Net.Http

# This is intentionally production-only. The read-only proof and Railway target
# must not be independently overridable because that could authorize a key write
# to one service using another service's empty-ledger proof.
$ProjectId = '7fcb22b5-db34-4e2b-a12a-cbc60391ff5f'
$Environment = 'production'
$Service = 'tm-server'
$BaseUri = 'https://tm-server-production-5573.up.railway.app'
$credentialResource = 'TM Expense Production Data Key'
$credentialUser = "$ProjectId/$Environment/$Service"
$tokenCredentialResource = 'TM Cloud Production'
$tokenCredentialUser = 'single-user'
$mutexName = 'Local\TMExpenseProductionKeyConfiguration'

$tmRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
if ([string]::IsNullOrWhiteSpace($ResultPath)) {
    $ResultPath = Join-Path $tmRoot 'dist\manual-expense-railway-configuration\result.json'
}
$ResultPath = [System.IO.Path]::GetFullPath($ResultPath)
$resultDirectory = Split-Path -Parent $ResultPath
$base = ([System.Uri]$BaseUri).GetLeftPart([System.UriPartial]::Authority)

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

function Assert-VerifiedRemoteBackup {
    param(
        [Parameter(Mandatory = $true)]$Operations,
        [Parameter(Mandatory = $true)][int]$ExpectedSchemaVersion
    )

    if ($null -eq $Operations.remoteBackup -or $null -eq $Operations.objectives -or
        [string]$Operations.remoteBackup.status -ne 'succeeded' -or
        [int]$Operations.remoteBackup.schemaVersion -ne $ExpectedSchemaVersion -or
        [string]$Operations.remoteBackup.integrityCheck -ne 'ok' -or
        $Operations.remoteBackup.migrationLedgerComplete -isnot [bool] -or
        $Operations.remoteBackup.requiredTablesComplete -isnot [bool] -or
        $Operations.remoteBackup.schemaSemanticsValidated -isnot [bool] -or
        $Operations.remoteBackup.migrationLedgerComplete -ne $true -or
        $Operations.remoteBackup.requiredTablesComplete -ne $true -or
        $Operations.remoteBackup.schemaSemanticsValidated -ne $true) {
        throw "Schema $ExpectedSchemaVersion requires a verified matching remote backup."
    }

    $checkedAt = [DateTimeOffset]::MinValue
    if (-not [DateTimeOffset]::TryParse(
        [string]$Operations.remoteBackup.checkedAt,
        [System.Globalization.CultureInfo]::InvariantCulture,
        [System.Globalization.DateTimeStyles]::RoundtripKind,
        [ref]$checkedAt
    )) {
        throw 'The verified remote backup has no valid checkedAt timestamp.'
    }
    $freshnessHours = [double]$Operations.objectives.backupFreshnessTargetHours
    $age = [DateTimeOffset]::UtcNow - $checkedAt.ToUniversalTime()
    if ($freshnessHours -le 0 -or $freshnessHours -gt 24 -or
        $age.TotalMinutes -lt -5 -or $age.TotalHours -gt $freshnessHours -or
        [string]$Operations.remoteBackup.snapshotId -notmatch '^[A-Za-z0-9][A-Za-z0-9._-]{0,255}$' -or
        [string]$Operations.remoteBackup.databaseSha256 -notmatch '^[0-9a-fA-F]{64}$' -or
        [uint64]$Operations.remoteBackup.databaseByteSize -lt 1) {
        throw 'The verified remote backup is stale or has incomplete provenance.'
    }

    if ([string]$Operations.overallStatus -notin @('healthy', 'warning', 'critical')) {
        throw 'Production returned an invalid overall operations status.'
    }
    $backupAlerts = @($Operations.alerts | Where-Object { [string]$_.code -like 'REMOTE_BACKUP_*' })
    if ($backupAlerts.Count -gt 0) {
        $codes = @($backupAlerts | ForEach-Object { [string]$_.code }) -join ','
        throw "Production reports a remote backup alert: $codes"
    }
}

function Get-ExpenseKeyProof {
    param([Parameter(Mandatory = $true)]$Operations)

    if ($null -eq $Operations.database -or $null -eq $Operations.controls -or
        $Operations.database.ok -isnot [bool] -or
        $Operations.database.ok -ne $true -or
        [string]$Operations.controls.incidentMode -ne 'normal') {
        throw 'Production database or incident controls are not healthy.'
    }

    $rawSchemaVersion = $Operations.database.schemaVersion
    if ($rawSchemaVersion -isnot [int] -and $rawSchemaVersion -isnot [long]) {
        throw 'Production returned a non-integer schema version.'
    }
    $schemaVersion = [int]$rawSchemaVersion
    Assert-VerifiedRemoteBackup `
        -Operations $Operations `
        -ExpectedSchemaVersion $schemaVersion
    if ($schemaVersion -eq 14) {
        $criticalAlerts = @($Operations.alerts | Where-Object { [string]$_.severity -eq 'critical' })
        if ([string]$Operations.overallStatus -eq 'critical' -or $criticalAlerts.Count -gt 0) {
            throw 'Schema 14 key bootstrap is blocked while production has a critical alert.'
        }
        if ($null -eq $Operations.remoteBackup -or
            $Operations.remoteBackup.migrationLedgerComplete -isnot [bool] -or
            $Operations.remoteBackup.requiredTablesComplete -isnot [bool] -or
            $Operations.remoteBackup.schemaSemanticsValidated -isnot [bool] -or
            [string]$Operations.remoteBackup.status -ne 'succeeded' -or
            [int]$Operations.remoteBackup.schemaVersion -ne 14 -or
            [string]$Operations.remoteBackup.integrityCheck -ne 'ok' -or
            $Operations.remoteBackup.migrationLedgerComplete -ne $true -or
            $Operations.remoteBackup.requiredTablesComplete -ne $true -or
            $Operations.remoteBackup.schemaSemanticsValidated -ne $true) {
            throw 'Schema 14 bootstrap requires a current verified schema 14 remote backup.'
        }
        return [pscustomobject]@{
            SchemaVersion = 14
            BootstrapMode = 'schema14'
            LedgerEmpty = $true
            KeyInitialized = $false
            CryptoReady = $false
            InitializationAllowed = $true
            RemoteBackupCheckedAt = [string]$Operations.remoteBackup.checkedAt
            RemoteBackupSnapshotId = [string]$Operations.remoteBackup.snapshotId
            RemoteBackupDatabaseSha256 = ([string]$Operations.remoteBackup.databaseSha256).ToLowerInvariant()
            RemoteBackupDatabaseByteSize = [uint64]$Operations.remoteBackup.databaseByteSize
            OverallStatus = [string]$Operations.overallStatus
        }
    }

    if ($schemaVersion -ne 15) {
        throw "Production schema $schemaVersion is not eligible for expense key configuration."
    }
    $requiredProperties = @(
        'expenseLedgerEmpty',
        'expenseKeyInitialized',
        'expenseKeyInitializationAllowed',
        'expenseCryptoReady'
    )
    foreach ($property in $requiredProperties) {
        if ($Operations.controls.PSObject.Properties.Name -notcontains $property) {
            throw "Production schema 15 is missing the required $property proof."
        }
        if ($Operations.controls.$property -isnot [bool]) {
            throw "Production schema 15 returned a non-Boolean $property proof."
        }
    }
    $ledgerEmpty = [bool]$Operations.controls.expenseLedgerEmpty
    $keyInitialized = [bool]$Operations.controls.expenseKeyInitialized
    $cryptoReady = [bool]$Operations.controls.expenseCryptoReady
    $initializationAllowed = [bool]$Operations.controls.expenseKeyInitializationAllowed
    if ($initializationAllowed -ne ($ledgerEmpty -and -not $keyInitialized)) {
        throw 'Production returned an inconsistent expense key initialization proof.'
    }
    if (($cryptoReady -and -not $keyInitialized) -or
        ($initializationAllowed -and $cryptoReady)) {
        throw 'Production returned an impossible expense crypto readiness state.'
    }
    $criticalAlerts = @($Operations.alerts | Where-Object { [string]$_.severity -eq 'critical' })
    if ([string]$Operations.overallStatus -eq 'critical') {
        $unexpectedCritical = @($criticalAlerts | Where-Object {
            [string]$_.code -ne 'EXPENSE_CRYPTO_NOT_READY'
        })
        if (-not $initializationAllowed -or
            $criticalAlerts.Count -ne 1 -or
            $unexpectedCritical.Count -ne 0) {
            throw 'Schema 15 key initialization has unexpected critical alerts.'
        }
    }
    elseif ($criticalAlerts.Count -gt 0) {
        throw 'Production returned critical alerts without a critical overall status.'
    }
    return [pscustomobject]@{
        SchemaVersion = 15
        BootstrapMode = 'schema15'
        LedgerEmpty = $ledgerEmpty
        KeyInitialized = $keyInitialized
        CryptoReady = $cryptoReady
        InitializationAllowed = $initializationAllowed
        RemoteBackupCheckedAt = [string]$Operations.remoteBackup.checkedAt
        RemoteBackupSnapshotId = [string]$Operations.remoteBackup.snapshotId
        RemoteBackupDatabaseSha256 = ([string]$Operations.remoteBackup.databaseSha256).ToLowerInvariant()
        RemoteBackupDatabaseByteSize = [uint64]$Operations.remoteBackup.databaseByteSize
        OverallStatus = [string]$Operations.overallStatus
    }
}

function Resolve-ExpenseKeyAction {
    param(
        [Parameter(Mandatory = $true)]$Proof,
        [Parameter(Mandatory = $true)][bool]$InitializeRequested,
        [Parameter(Mandatory = $true)][bool]$CredentialExists
    )

    if ($Proof.KeyInitialized) {
        if (-not $Proof.CryptoReady) {
            throw 'Production has an initialized expense key probe but crypto is not ready. Use the separate recovery procedure; this script will not overwrite the key.'
        }
        if ($InitializeRequested) {
            throw 'Production already has a healthy initialized expense key. -InitializeNewKey was refused.'
        }
        if (-not $CredentialExists) {
            throw 'Production crypto is healthy, but this PC has no stored recovery credential. No secret was changed.'
        }
        return 'skip'
    }
    if (-not $Proof.InitializationAllowed -or -not $Proof.LedgerEmpty) {
        throw 'Production did not prove that first-time expense key initialization is safe.'
    }
    if (-not $InitializeRequested) {
        throw 'First-time production key initialization requires the explicit -InitializeNewKey switch.'
    }
    return 'write'
}

function Assert-ExpenseKeyFinalProof {
    param([Parameter(Mandatory = $true)]$Proof)

    if (-not $Proof.InitializationAllowed -or
        -not $Proof.LedgerEmpty -or
        $Proof.KeyInitialized -or
        $Proof.CryptoReady) {
        throw 'Production changed after preflight; the expense key was not sent.'
    }
}

function Get-CredentialFailureAction {
    param(
        [Parameter(Mandatory = $true)][bool]$CreatedCredential,
        [Parameter(Mandatory = $true)][bool]$KeyWriteAttempted
    )
    if ($CreatedCredential -and -not $KeyWriteAttempted) { return 'remove' }
    return 'retain'
}

function Assert-GuardThrows {
    param(
        [Parameter(Mandatory = $true)][scriptblock]$Action,
        [Parameter(Mandatory = $true)][string]$ExpectedMessage
    )
    $threw = $false
    try {
        & $Action | Out-Null
    }
    catch {
        $threw = $true
        if ($_.Exception.Message -notlike "*$ExpectedMessage*") {
            throw "Guard self-test expected '$ExpectedMessage' but received '$($_.Exception.Message)'."
        }
    }
    if (-not $threw) { throw "Guard self-test expected failure containing '$ExpectedMessage'." }
}

function Invoke-ExpenseKeyGuardSelfTest {
    $schema14Json = @'
{"overallStatus":"healthy","alerts":[],"objectives":{"backupFreshnessTargetHours":24},"database":{"ok":true,"schemaVersion":14},"controls":{"incidentMode":"normal"},"remoteBackup":{"status":"succeeded","checkedAt":"","snapshotId":"snapshot-14","databaseSha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","databaseByteSize":1,"schemaVersion":14,"integrityCheck":"ok","migrationLedgerComplete":true,"requiredTablesComplete":true,"schemaSemanticsValidated":true}}
'@
    $schema15Json = @'
{"overallStatus":"critical","alerts":[{"severity":"critical","code":"EXPENSE_CRYPTO_NOT_READY"}],"objectives":{"backupFreshnessTargetHours":24},"database":{"ok":true,"schemaVersion":15},"controls":{"incidentMode":"normal","expenseLedgerEmpty":true,"expenseKeyInitialized":false,"expenseKeyInitializationAllowed":true,"expenseCryptoReady":false},"remoteBackup":{"status":"succeeded","checkedAt":"","snapshotId":"snapshot-15","databaseSha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","databaseByteSize":1,"schemaVersion":15,"integrityCheck":"ok","migrationLedgerComplete":true,"requiredTablesComplete":true,"schemaSemanticsValidated":true}}
'@
    $schema14 = $schema14Json | ConvertFrom-Json
    $schema15 = $schema15Json | ConvertFrom-Json
    $now = [DateTimeOffset]::UtcNow.ToString('o')
    $schema14.remoteBackup.checkedAt = $now
    $schema15.remoteBackup.checkedAt = $now
    $proof14 = Get-ExpenseKeyProof $schema14
    $proof15 = Get-ExpenseKeyProof $schema15
    if (-not $proof14.InitializationAllowed -or -not $proof15.InitializationAllowed) {
        throw 'Guard self-test did not accept valid first-time bootstrap proofs.'
    }

    $badBackup = $schema14Json | ConvertFrom-Json
    $badBackup.remoteBackup.checkedAt = $now
    $badBackup.remoteBackup.status = 'pending'
    Assert-GuardThrows { Get-ExpenseKeyProof $badBackup } 'verified matching remote backup'
    $stringBoolean = $schema15Json | ConvertFrom-Json
    $stringBoolean.remoteBackup.checkedAt = $now
    $stringBoolean.controls.expenseLedgerEmpty = 'false'
    Assert-GuardThrows { Get-ExpenseKeyProof $stringBoolean } 'non-Boolean expenseLedgerEmpty'
    $impossible = $schema15Json | ConvertFrom-Json
    $impossible.remoteBackup.checkedAt = $now
    $impossible.controls.expenseCryptoReady = $true
    Assert-GuardThrows { Get-ExpenseKeyProof $impossible } 'impossible expense crypto readiness state'

    Assert-GuardThrows {
        Resolve-ExpenseKeyAction $proof15 $false $false
    } 'explicit -InitializeNewKey'
    if ((Resolve-ExpenseKeyAction $proof15 $true $false) -ne 'write') {
        throw 'Guard self-test did not select the first-time key write action.'
    }

    $healthy = [pscustomobject]@{
        LedgerEmpty = $false
        KeyInitialized = $true
        CryptoReady = $true
        InitializationAllowed = $false
    }
    if ((Resolve-ExpenseKeyAction $healthy $false $true) -ne 'skip') {
        throw 'Guard self-test did not protect a healthy initialized key.'
    }
    Assert-GuardThrows {
        Resolve-ExpenseKeyAction $healthy $true $true
    } 'already has a healthy initialized expense key'
    $unavailable = [pscustomobject]@{
        LedgerEmpty = $false
        KeyInitialized = $true
        CryptoReady = $false
        InitializationAllowed = $false
    }
    Assert-GuardThrows {
        Resolve-ExpenseKeyAction $unavailable $false $true
    } 'separate recovery procedure'
    Assert-GuardThrows {
        Assert-ExpenseKeyFinalProof $healthy
    } 'changed after preflight'

    if ((Get-CredentialFailureAction $true $false) -ne 'remove' -or
        (Get-CredentialFailureAction $true $true) -ne 'retain') {
        throw 'Guard self-test failed the credential retention policy.'
    }
    Write-Host 'Expense Railway key guard self-test: PASS' -ForegroundColor Green
}

function New-ExpenseDataKey {
    $keyBytes = [byte[]]::new(32)
    $random = [System.Security.Cryptography.RandomNumberGenerator]::Create()
    try {
        $random.GetBytes($keyBytes)
        $base64Url = [Convert]::ToBase64String($keyBytes).TrimEnd('=').Replace('+', '-').Replace('/', '_')
        return "tm_exp_v1_$base64Url"
    }
    finally {
        [Array]::Clear($keyBytes, 0, $keyBytes.Length)
        $random.Dispose()
        $keyBytes = $null
        $base64Url = $null
    }
}

function Set-RailwayExpenseKey {
    param(
        [Parameter(Mandatory = $true)][string]$RailwayPath,
        [Parameter(Mandatory = $true)][string]$EncodedKey
    )

    $arguments = @(
        'variable', 'set', 'TM_EXPENSE_DATA_KEY_V1', '--stdin', '--skip-deploys',
        '--project', $ProjectId,
        '--environment', $Environment,
        '--service', $Service
    )
    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $RailwayPath
    $startInfo.Arguments = ($arguments -join ' ')
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardInput = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $process = [System.Diagnostics.Process]::new()
    try {
        $process.StartInfo = $startInfo
        if (-not $process.Start()) {
            throw 'Railway CLI could not be started.'
        }
        $process.StandardInput.Write($EncodedKey)
        $process.StandardInput.Close()
        $null = $process.StandardOutput.ReadToEnd()
        $null = $process.StandardError.ReadToEnd()
        $process.WaitForExit()
        if ($process.ExitCode -ne 0) {
            throw "Railway rejected the sealed expense key (exit $($process.ExitCode))."
        }
    }
    finally {
        $process.Dispose()
        $startInfo = $null
    }
}

if ($RunGuardSelfTest) {
    Invoke-ExpenseKeyGuardSelfTest
    exit 0
}

if ($ExpectedHeadSha -notmatch '^[0-9a-fA-F]{40}$' -or
    $Step10RunId -lt 1 -or
    $Step16RunId -lt 1) {
    throw 'An exact 40-character commit SHA and successful STEP 10/STEP 16 run IDs are required.'
}
$ExpectedHeadSha = $ExpectedHeadSha.ToLowerInvariant()

$railway = Get-ChildItem -LiteralPath (Join-Path $env:LOCALAPPDATA 'pnpm\store\v11\links\@railway\cli') `
    -Filter railway.exe -File -Recurse |
    Sort-Object FullName -Descending |
    Select-Object -First 1 -ExpandProperty FullName
if ([string]::IsNullOrWhiteSpace($railway) -or -not (Test-Path -LiteralPath $railway -PathType Leaf)) {
    throw 'Railway CLI was not found.'
}

$summary = "environment=$Environment service=$Service expenseAiEnabled=$ExpenseAiEnabled head=$ExpectedHeadSha"

$vault = $null
$tokenCredential = $null
$expenseCredential = $null
$verifiedCredential = $null
$token = $null
$encodedKey = $null
$mutex = $null
$mutexAcquired = $false
$abandonedMutexRecovered = $false
$credentialExists = $false
$createdCredential = $false
$keyWriteRequired = $false
$keyWriteAttempted = $false
$keyWriteConfirmed = $false
$keyWriteAmbiguous = $false
$recoveryCredentialMatchVerified = $false
$stage = 'exclusive-lock'
$proof = $null
$keyAction = $null

try {
    New-Item -ItemType Directory -Force -Path $resultDirectory | Out-Null
    $mutex = [System.Threading.Mutex]::new($false, $mutexName)
    try {
        $mutexAcquired = $mutex.WaitOne(0)
    }
    catch [System.Threading.AbandonedMutexException] {
        $mutexAcquired = $true
        $abandonedMutexRecovered = $true
    }
    if (-not $mutexAcquired) {
        throw 'Another production expense key configuration is already running on this PC.'
    }

    $stage = 'credential-locker'
    $vault = [Windows.Security.Credentials.PasswordVault,Windows.Security.Credentials,ContentType=WindowsRuntime]::new()
    $tokenCredential = $vault.Retrieve($tokenCredentialResource, $tokenCredentialUser)
    $tokenCredential.RetrievePassword()
    $token = [string]$tokenCredential.Password
    if ($token -notmatch '^tm_pat_v1_[A-Za-z0-9_-]{43}$') {
        throw 'The Credential Locker TM production token has an invalid format.'
    }

    try {
        $expenseCredential = $vault.Retrieve($credentialResource, $credentialUser)
        $expenseCredential.RetrievePassword()
        $encodedKey = [string]$expenseCredential.Password
        $credentialExists = $true
    }
    catch [System.Exception] {
        if ($_.Exception.HResult -ne -2147023728) { throw }
    }
    if ($credentialExists -and $encodedKey -notmatch '^tm_exp_v1_[A-Za-z0-9_-]{43}$') {
        throw 'The saved expense encryption key has an invalid format. It was not replaced.'
    }

    $stage = 'read-only-preflight'
    $proof = Get-ExpenseKeyProof (Invoke-TmOperationsStatus -Origin $base -Token $token)
    $keyAction = Resolve-ExpenseKeyAction `
        -Proof $proof `
        -InitializeRequested ([bool]$InitializeNewKey) `
        -CredentialExists $credentialExists
    $keyWriteRequired = $keyAction -eq 'write'

    $approvalSummary = "schema=$($proof.SchemaVersion) backup=$($proof.RemoteBackupSnapshotId) keyAction=$keyAction expenseAiEnabled=$ExpenseAiEnabled head=$ExpectedHeadSha"
    if (-not $Apply) {
        Write-Host "Dry run passed: $approvalSummary" -ForegroundColor Green
        Write-Host 'No Railway variable was changed and no deployment was triggered.'
        Write-Host 'Re-run with -Apply after reviewing this live read-only proof.'
        return
    }
    if ($Force) { $ConfirmPreference = 'None' }
    if (-not $PSCmdlet.ShouldProcess(
        'Railway production/tm-server',
        "Configure schema 15 expense controls after live proof: $approvalSummary"
    )) {
        return
    }

    if ($keyWriteRequired) {
        if (-not $credentialExists) {
            $encodedKey = New-ExpenseDataKey
            $expenseCredential = [Windows.Security.Credentials.PasswordCredential,Windows.Security.Credentials,ContentType=WindowsRuntime]::new(
                $credentialResource,
                $credentialUser,
                $encodedKey
            )
            $vault.Add($expenseCredential)
            $createdCredential = $true
            $credentialExists = $true
        }
        $verifiedCredential = $vault.Retrieve($credentialResource, $credentialUser)
        $verifiedCredential.RetrievePassword()
        if ([string]$verifiedCredential.Password -cne $encodedKey) {
            throw 'Windows Credential Locker verification failed.'
        }
        $recoveryCredentialMatchVerified = $true
    }

    if ($keyWriteRequired) {
        $stage = 'read-only-final-proof'
        $finalProof = Get-ExpenseKeyProof (Invoke-TmOperationsStatus -Origin $base -Token $token)
        Assert-ExpenseKeyFinalProof $finalProof

        $stage = 'sealed-expense-key'
        $keyWriteAttempted = $true
        try {
            Set-RailwayExpenseKey -RailwayPath $railway -EncodedKey $encodedKey
            $keyWriteConfirmed = $true
        }
        catch {
            $keyWriteAmbiguous = $true
            throw
        }
    }

    $stage = 'feature-control'
    & $railway variable set `
        "TM_EXPENSE_AI_ENABLED=$ExpenseAiEnabled" `
        "TM_BUILD_COMMIT_SHA=$ExpectedHeadSha" `
        --skip-deploys `
        --project $ProjectId `
        --environment $Environment `
        --service $Service | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw 'Railway rejected the expense feature control or deployment.'
    }

    $stage = 'complete'
    $configuredAt = [DateTimeOffset]::UtcNow
    $receiptId = [Guid]::NewGuid().ToString('D')
    [ordered]@{
        success = $true
        receiptVersion = 1
        receiptId = $receiptId
        configuredAtUtc = $configuredAt.ToString('o')
        expiresAtUtc = $configuredAt.AddMinutes(30).ToString('o')
        expectedHeadSha = $ExpectedHeadSha
        step10RunId = $Step10RunId
        step16RunId = $Step16RunId
        projectId = $ProjectId
        environment = $Environment
        service = $Service
        proofSchemaVersion = $proof.SchemaVersion
        proofMode = $proof.BootstrapMode
        proofLedgerEmpty = [bool]$proof.LedgerEmpty
        proofKeyInitialized = [bool]$proof.KeyInitialized
        proofCryptoReady = [bool]$proof.CryptoReady
        proofInitializationAllowed = [bool]$proof.InitializationAllowed
        proofOverallStatus = [string]$proof.OverallStatus
        remoteBackupCheckedAt = [string]$proof.RemoteBackupCheckedAt
        remoteBackupSnapshotId = [string]$proof.RemoteBackupSnapshotId
        remoteBackupDatabaseSha256 = [string]$proof.RemoteBackupDatabaseSha256
        remoteBackupDatabaseByteSize = [uint64]$proof.RemoteBackupDatabaseByteSize
        variableNames = if ($keyWriteRequired) {
            @('TM_EXPENSE_DATA_KEY_V1', 'TM_EXPENSE_AI_ENABLED', 'TM_BUILD_COMMIT_SHA')
        } else {
            @('TM_EXPENSE_AI_ENABLED', 'TM_BUILD_COMMIT_SHA')
        }
        expenseAiEnabled = [bool]::Parse($ExpenseAiEnabled)
        recoveryCredentialResource = $credentialResource
        recoveryCredentialUser = $credentialUser
        recoveryCredentialCreated = $createdCredential
        recoveryCredentialPresent = $credentialExists
        recoveryCredentialMatchVerified = $recoveryCredentialMatchVerified
        recoveryCredentialSubmittedToRailway = $keyWriteConfirmed
        keyWriteRequired = $keyWriteRequired
        keyWriteAttempted = $keyWriteAttempted
        keyWriteConfirmed = $keyWriteConfirmed
        keyWriteAmbiguous = $false
        abandonedMutexRecovered = $abandonedMutexRecovered
        secretValueWrittenToResult = $false
        deploymentTriggered = $false
        sourceDeploymentRequired = $true
    } | ConvertTo-Json | Set-Content -LiteralPath $ResultPath -Encoding utf8

    Write-Host ''
    Write-Host 'TM schema 15 expense controls were configured.' -ForegroundColor Green
    if ($keyWriteRequired) {
        Write-Host 'A first-time expense key was sealed to Railway and retained in Windows Credential Locker.'
    }
    else {
        Write-Host 'The healthy production expense key was not rewritten.'
        Write-Host 'The stored local recovery credential was not cryptographically matched to production by this run.' -ForegroundColor Yellow
    }
    Write-Host 'No deployment was triggered. Deploy the exact Actions-verified source commit, then verify readiness and backup health.'
}
catch {
    $failureMessage = $_.Exception.Message
    $credentialFailureAction = Get-CredentialFailureAction `
        -CreatedCredential $createdCredential `
        -KeyWriteAttempted $keyWriteAttempted
    if ($credentialFailureAction -eq 'remove' -and $null -ne $vault -and $null -ne $expenseCredential) {
        try {
            $vault.Remove($expenseCredential)
            $createdCredential = $false
        }
        catch {
            # Fail closed: a local-only credential is harmless, and the result records that no key write was attempted.
        }
    }
    [ordered]@{
        success = $false
        failedAtUtc = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ss.fffZ')
        stage = $stage
        reason = $failureMessage
        keyWriteAttempted = $keyWriteAttempted
        keyWriteConfirmed = $keyWriteConfirmed
        keyWriteAmbiguous = $keyWriteAmbiguous
        secretValueWrittenToResult = $false
    } | ConvertTo-Json | Set-Content -LiteralPath $ResultPath -Encoding utf8
    throw "Expense Railway configuration failed at ${stage}: $failureMessage"
}
finally {
    if ($mutexAcquired -and $null -ne $mutex) { $mutex.ReleaseMutex() }
    if ($null -ne $mutex) { $mutex.Dispose() }
    $encodedKey = $null
    $token = $null
    $expenseCredential = $null
    $verifiedCredential = $null
    $tokenCredential = $null
    $vault = $null
    Remove-Variable encodedKey, token, expenseCredential, verifiedCredential, tokenCredential, vault -ErrorAction SilentlyContinue
}

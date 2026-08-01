[CmdletBinding()]
param(
    [ValidatePattern('^https://')]
    [string]$BaseUri = 'https://tm-server-production-5573.up.railway.app',

    [string]$OutputPath
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
Add-Type -AssemblyName System.Net.Http

$expectedSchemaVersion = 15
$expectedFeatureHardLimitMicrousd = 2000000
$resource = 'TM Cloud Production'
$userName = 'single-user'
$tmRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
if ([string]::IsNullOrWhiteSpace($OutputPath)) {
    $OutputPath = Join-Path $tmRoot 'dist\manual-stock-screen-production-verification\result.json'
}
$OutputPath = [System.IO.Path]::GetFullPath($OutputPath)

function Invoke-TmRequest {
    param(
        [Parameter(Mandatory = $true)][System.Net.Http.HttpClient]$Client,
        [Parameter(Mandatory = $true)][System.Net.Http.HttpMethod]$Method,
        [Parameter(Mandatory = $true)][string]$Uri,
        [Parameter(Mandatory = $true)][string]$Token,
        [string]$JsonBody
    )

    $request = $null
    $response = $null
    try {
        $request = [System.Net.Http.HttpRequestMessage]::new($Method, $Uri)
        $request.Headers.Authorization =
            [System.Net.Http.Headers.AuthenticationHeaderValue]::new('Bearer', $Token)
        if (-not [string]::IsNullOrWhiteSpace($JsonBody)) {
            $request.Content = [System.Net.Http.StringContent]::new(
                $JsonBody,
                [System.Text.Encoding]::UTF8,
                'application/json'
            )
        }
        $response = $Client.SendAsync($request).GetAwaiter().GetResult()
        [pscustomobject]@{
            StatusCode = [int]$response.StatusCode
            Body = $response.Content.ReadAsStringAsync().GetAwaiter().GetResult()
        }
    }
    finally {
        if ($null -ne $response) { $response.Dispose() }
        if ($null -ne $request) { $request.Dispose() }
    }
}

function Require-Status {
    param($Response, [int]$Expected, [string]$Stage)
    if ($Response.StatusCode -ne $Expected) {
        throw "$Stage returned HTTP $($Response.StatusCode), expected $Expected. $($Response.Body)"
    }
}

function Require-JsonProperty {
    param(
        [Parameter(Mandatory = $true)]$Object,
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$Stage
    )

    if ($null -eq $Object -or
        -not @($Object.PSObject.Properties.Name).Contains($Name)) {
        throw "$Stage is missing required property '$Name'."
    }
}

$vault = $null
$credential = $null
$token = $null
$handler = $null
$client = $null
$base = $null
$stage = 'credential-locker'
try {
    $uri = [System.Uri]$BaseUri
    if (-not $uri.IsAbsoluteUri -or $uri.Scheme -ne 'https' -or
        -not [string]::IsNullOrEmpty($uri.UserInfo) -or
        -not [string]::IsNullOrEmpty($uri.Query) -or
        -not [string]::IsNullOrEmpty($uri.Fragment)) {
        throw 'BaseUri must be a credential-free HTTPS origin.'
    }
    $base = $uri.GetLeftPart([System.UriPartial]::Authority)

    $vault =
        [Windows.Security.Credentials.PasswordVault,Windows.Security.Credentials,ContentType=WindowsRuntime]::new()
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
    $ready = Invoke-TmRequest `
        -Client $client `
        -Method ([System.Net.Http.HttpMethod]::Get) `
        -Uri "$base/readyz" `
        -Token $token
    Require-Status $ready 200 $stage

    $stage = 'operations-status-before'
    $opsBeforeResponse = Invoke-TmRequest `
        -Client $client `
        -Method ([System.Net.Http.HttpMethod]::Get) `
        -Uri "$base/api/v1/ops/status" `
        -Token $token
    Require-Status $opsBeforeResponse 200 $stage
    $opsBefore = ($opsBeforeResponse.Body | ConvertFrom-Json).data
    if ([int]$opsBefore.database.schemaVersion -ne $expectedSchemaVersion -or
        [string]$opsBefore.remoteBackup.status -ne 'succeeded' -or
        [int]$opsBefore.remoteBackup.schemaVersion -ne $expectedSchemaVersion -or
        [string]$opsBefore.remoteBackup.integrityCheck -ne 'ok' -or
        [bool]$opsBefore.remoteBackup.migrationLedgerComplete -ne $true -or
        [bool]$opsBefore.remoteBackup.requiredTablesComplete -ne $true -or
        [bool]$opsBefore.remoteBackup.schemaSemanticsValidated -ne $true) {
        throw "Production DB and remote backup must both be healthy schema $expectedSchemaVersion."
    }

    Require-JsonProperty $opsBefore 'stock' $stage
    $stockBefore = $opsBefore.stock
    foreach ($requiredName in @(
        'licenseGate',
        'screenEnabled',
        'aiEnabled',
        'latestExpectedTradingDate',
        'latestSuccess',
        'latestAttempt',
        'coverage',
        'universeAgeDays',
        'nextRunAt',
        'monthlyFeatureCostMicrousd',
        'hardLimitMicrousd',
        'lastErrorCode'
    )) {
        Require-JsonProperty $stockBefore $requiredName $stage
    }
    if ([bool]$stockBefore.licenseGate -or
        [bool]$stockBefore.screenEnabled -or
        [bool]$stockBefore.aiEnabled) {
        throw 'The initial production verification requires all stock feature gates to be false.'
    }
    if ([int64]$stockBefore.hardLimitMicrousd -ne $expectedFeatureHardLimitMicrousd) {
        throw 'The stock feature hard limit must be exactly 2000000 microUSD.'
    }
    $costBefore = [int64]$stockBefore.monthlyFeatureCostMicrousd
    if ($costBefore -lt 0 -or $costBefore -gt $expectedFeatureHardLimitMicrousd) {
        throw 'The stock monthly feature cost is outside its hard-limit boundary.'
    }

    $stage = 'mobile-stock-screen-shell'
    $mobileResponse = Invoke-TmRequest `
        -Client $client `
        -Method ([System.Net.Http.HttpMethod]::Get) `
        -Uri "$base/mobile/" `
        -Token $token
    Require-Status $mobileResponse 200 $stage
    if ($mobileResponse.Body -notmatch 'id="stock-screen-panel"' -or
        $mobileResponse.Body -notmatch 'id="stock-screen-results"') {
        throw 'The mobile stock screen panel or result region is missing.'
    }
    $mobileScriptResponse = Invoke-TmRequest `
        -Client $client `
        -Method ([System.Net.Http.HttpMethod]::Get) `
        -Uri "$base/mobile/app.js" `
        -Token $token
    Require-Status $mobileScriptResponse 200 "$stage-script"
    if ($mobileScriptResponse.Body -notmatch 'get_latest_stock_screen' -or
        $mobileScriptResponse.Body -notmatch 'list_stock_screen_results') {
        throw 'The mobile stock screen read commands are missing.'
    }

    $stage = 'latest-stock-screen-read'
    $latestResponse = Invoke-TmRequest `
        -Client $client `
        -Method ([System.Net.Http.HttpMethod]::Post) `
        -Uri "$base/api/v1/desktop/commands/get_latest_stock_screen" `
        -Token $token `
        -JsonBody '{"args":{}}'
    Require-Status $latestResponse 200 $stage
    $latestEnvelope = $latestResponse.Body | ConvertFrom-Json
    Require-JsonProperty $latestEnvelope 'data' $stage

    $stage = 'operations-status-after'
    $opsAfterResponse = Invoke-TmRequest `
        -Client $client `
        -Method ([System.Net.Http.HttpMethod]::Get) `
        -Uri "$base/api/v1/ops/status" `
        -Token $token
    Require-Status $opsAfterResponse 200 $stage
    $opsAfter = ($opsAfterResponse.Body | ConvertFrom-Json).data
    Require-JsonProperty $opsAfter 'stock' $stage
    $stockAfter = $opsAfter.stock
    $costAfter = [int64]$stockAfter.monthlyFeatureCostMicrousd
    if ($costAfter -ne $costBefore) {
        throw "A read-only flags-off verification changed stock AI cost: $costBefore -> $costAfter."
    }
    if ([bool]$stockAfter.licenseGate -or
        [bool]$stockAfter.screenEnabled -or
        [bool]$stockAfter.aiEnabled) {
        throw 'A stock feature gate changed during verification.'
    }

    $result = [ordered]@{
        verifiedAtUtc = [DateTime]::UtcNow.ToString('o')
        baseUri = $base
        schemaVersion = $expectedSchemaVersion
        remoteBackupStatus = [string]$opsAfter.remoteBackup.status
        remoteBackupIntegrity = [string]$opsAfter.remoteBackup.integrityCheck
        remoteBackupMigrationLedgerComplete = [bool]$opsAfter.remoteBackup.migrationLedgerComplete
        remoteBackupRequiredTablesComplete = [bool]$opsAfter.remoteBackup.requiredTablesComplete
        remoteBackupSchemaSemanticsValidated = [bool]$opsAfter.remoteBackup.schemaSemanticsValidated
        licenseGate = $false
        screenEnabled = $false
        aiEnabled = $false
        hardLimitMicrousd = $expectedFeatureHardLimitMicrousd
        monthlyFeatureCostBeforeMicrousd = $costBefore
        monthlyFeatureCostAfterMicrousd = $costAfter
        latestScreenReadOnlyCommand = $true
        mobileStockScreenShell = $true
        providerCallAuthorized = $false
        openAiCallAuthorized = $false
    }
    $directory = Split-Path -Parent $OutputPath
    [System.IO.Directory]::CreateDirectory($directory) | Out-Null
    [System.IO.File]::WriteAllText(
        $OutputPath,
        ($result | ConvertTo-Json -Depth 10),
        [System.Text.UTF8Encoding]::new($false)
    )

    Write-Host 'TM stock screen flags-off production verification passed.' -ForegroundColor Green
    Write-Host "Schema: $expectedSchemaVersion; stock AI cost change: 0 microUSD"
    Write-Host 'No Alpaca or OpenAI call was authorized.'
    Write-Host "Result: $OutputPath"
}
catch {
    throw "Stock screen production verification failed at ${stage}: $($_.Exception.Message)"
}
finally {
    $token = $null
    if ($null -ne $client) { $client.Dispose() }
    if ($null -ne $handler) { $handler.Dispose() }
    $credential = $null
    $vault = $null
}

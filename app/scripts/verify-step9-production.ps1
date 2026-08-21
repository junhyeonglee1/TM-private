[CmdletBinding()]
param(
    [ValidatePattern('^https://')]
    [string]$BaseUri = 'https://tm-server-production-5573.up.railway.app',

    [string]$OutputPath,

    [Security.SecureString]$ProvidedSecureToken
)

$ErrorActionPreference = 'Stop'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
Add-Type -AssemblyName System.Net.Http

if ([string]::IsNullOrWhiteSpace($OutputPath)) {
    $OutputPath = Join-Path $PSScriptRoot '..\dist\manual-step9-production-verification\result.json'
}

function Invoke-TmGet {
    param(
        [Parameter(Mandatory = $true)]
        [System.Net.Http.HttpClient]$Client,

        [Parameter(Mandatory = $true)]
        [string]$Uri,

        [Parameter(Mandatory = $true)]
        [string]$Token
    )

    $request = New-Object System.Net.Http.HttpRequestMessage(
        [System.Net.Http.HttpMethod]::Get,
        $Uri
    )
    $response = $null
    try {
        $request.Headers.Authorization = New-Object System.Net.Http.Headers.AuthenticationHeaderValue('Bearer', $Token)
        $response = $Client.SendAsync($request).GetAwaiter().GetResult()
        $body = $response.Content.ReadAsStringAsync().GetAwaiter().GetResult()
        return [pscustomobject]@{
            StatusCode = [int]$response.StatusCode
            Body = $body
        }
    }
    finally {
        if ($null -ne $response) { $response.Dispose() }
        $request.Dispose()
    }
}

$secureToken = if ($null -ne $ProvidedSecureToken) {
    $ProvidedSecureToken
}
else {
    Read-Host 'TM production auth token' -AsSecureString
}
$token = [System.Net.NetworkCredential]::new('', $secureToken).Password
$client = New-Object System.Net.Http.HttpClient
$client.Timeout = [TimeSpan]::FromSeconds(20)
$stage = 'token-input'

try {
    if ([string]::IsNullOrWhiteSpace($token)) {
        throw 'The authentication token was empty.'
    }

    $base = $BaseUri.TrimEnd('/')

    $stage = 'authentication'
    $auth = Invoke-TmGet -Client $client -Uri "$base/api/v1/auth/status" -Token $token
    if ($auth.StatusCode -ne 200) {
        throw "Authentication failed with HTTP $($auth.StatusCode)."
    }
    $authJson = $auth.Body | ConvertFrom-Json
    if ($authJson.data.authenticated -ne $true) {
        throw 'The server did not report an authenticated session.'
    }

    $stage = 'operations-status'
    $ops = Invoke-TmGet -Client $client -Uri "$base/api/v1/ops/status" -Token $token
    if ($ops.StatusCode -ne 200) {
        throw "Operations status failed with HTTP $($ops.StatusCode)."
    }
    $opsJson = $ops.Body | ConvertFrom-Json
    if ($opsJson.data.database.ok -ne $true) {
        throw 'The production database health check failed.'
    }
    if ([int]$opsJson.data.database.schemaVersion -ne 5) {
        throw "Expected schema 5, received $($opsJson.data.database.schemaVersion)."
    }

    $stage = 'ai-route-isolation'
    $ai = Invoke-TmGet -Client $client -Uri "$base/api/v1/ai/status" -Token $token
    $aiRouteAvailable = $ai.StatusCode -eq 200
    $aiJson = $null
    if ($aiRouteAvailable) {
        $aiJson = $ai.Body | ConvertFrom-Json
        if ([int64]$aiJson.data.budget.warningLimitMicrousd -ne 10000000) {
            throw 'The AI monthly warning limit is not USD 10.'
        }
        if ([int64]$aiJson.data.budget.hardLimitMicrousd -ne 20000000) {
            throw 'The AI monthly hard stop is not USD 20.'
        }
    }
    elseif ($ai.StatusCode -ne 404) {
        throw "AI isolation check returned unexpected HTTP $($ai.StatusCode)."
    }

    $stage = 'complete'
    $result = [ordered]@{
        success = $true
        verifiedAtUtc = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ss.fffZ')
        baseUri = $base
        authenticated = $true
        schemaVersion = [int]$opsJson.data.database.schemaVersion
        databaseOk = [bool]$opsJson.data.database.ok
        journalMode = [string]$opsJson.data.database.journalMode
        localBackupCount = [int]$opsJson.data.localBackup.count
        remoteBackupStatus = [string]$opsJson.data.remoteBackup.status
        aiRouteAvailable = $aiRouteAvailable
        aiRouteStatus = $ai.StatusCode
        aiModel = if ($aiRouteAvailable) { [string]$aiJson.data.model } else { $null }
        aiConfigured = if ($aiRouteAvailable) { [bool]$aiJson.data.configured } else { $false }
        aiWarningLimitMicrousd = if ($aiRouteAvailable) { [int64]$aiJson.data.budget.warningLimitMicrousd } else { $null }
        aiHardLimitMicrousd = if ($aiRouteAvailable) { [int64]$aiJson.data.budget.hardLimitMicrousd } else { $null }
        aiCommittedMicrousd = if ($aiRouteAvailable) { [int64]$aiJson.data.budget.committedMicrousd } else { $null }
        aiBudgetRuntimeVerificationDeferred = -not $aiRouteAvailable
        billableAiCallPerformed = $false
        productionMutationPerformed = $false
        tokenPersisted = $false
    }

    $outputDirectory = Split-Path -Parent $OutputPath
    New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null
    $result | ConvertTo-Json | Set-Content -LiteralPath $OutputPath -Encoding utf8

    Write-Host ''
    Write-Host 'STEP 9 production read-only verification passed.' -ForegroundColor Green
    Write-Host "Schema: $($result.schemaVersion)"
    Write-Host "Remote backup status: $($result.remoteBackupStatus)"
    if ($aiRouteAvailable) {
        Write-Host 'AI budget: USD 10 warning / USD 20 hard stop'
    }
    else {
        Write-Host 'AI route isolation: 404 (expected until STEP 11)'
    }
}
catch {
    $failure = [ordered]@{
        success = $false
        verifiedAtUtc = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ss.fffZ')
        baseUri = $BaseUri.TrimEnd('/')
        failedStage = $stage
        message = $_.Exception.Message
        billableAiCallPerformed = $false
        productionMutationPerformed = $false
        tokenPersisted = $false
    }
    $outputDirectory = Split-Path -Parent $OutputPath
    New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null
    $failure | ConvertTo-Json | Set-Content -LiteralPath $OutputPath -Encoding utf8

    Write-Host ''
    Write-Host 'STEP 9 production read-only verification failed.' -ForegroundColor Red
    Write-Host "Failed stage: $stage"
    Write-Host $_.Exception.Message
}
finally {
    $client.Dispose()
    $token = $null
    $secureToken = $null
    $ProvidedSecureToken = $null
    Remove-Variable token, secureToken, ProvidedSecureToken, auth, authJson, ops, opsJson, ai, aiJson, aiRouteAvailable -ErrorAction SilentlyContinue
}

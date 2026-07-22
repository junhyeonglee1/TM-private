[CmdletBinding()]
param(
    [ValidatePattern('^https://')]
    [string]$BaseUri = 'https://tm-server-production-5573.up.railway.app',

    [ValidateRange(60, 900)]
    [int]$TimeoutSeconds = 420,

    [string]$OutputPath
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
Add-Type -AssemblyName System.Net.Http

$resource = 'TM Cloud Production'
$userName = 'single-user'
$tmRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
if ([string]::IsNullOrWhiteSpace($OutputPath)) {
    $OutputPath = Join-Path $tmRoot 'dist\manual-step16-production-verification\result.json'
}
$OutputPath = [System.IO.Path]::GetFullPath($OutputPath)

function Invoke-TmRequest {
    param(
        [Parameter(Mandatory = $true)][System.Net.Http.HttpClient]$Client,
        [Parameter(Mandatory = $true)][System.Net.Http.HttpMethod]$Method,
        [Parameter(Mandatory = $true)][string]$Uri,
        [string]$Token,
        [hashtable]$Headers,
        [string]$JsonBody
    )
    $request = $null
    $response = $null
    try {
        $request = [System.Net.Http.HttpRequestMessage]::new($Method, $Uri)
        if (-not [string]::IsNullOrWhiteSpace($Token)) {
            $request.Headers.Authorization = [System.Net.Http.Headers.AuthenticationHeaderValue]::new('Bearer', $Token)
        }
        if ($null -ne $Headers) {
            foreach ($entry in $Headers.GetEnumerator()) {
                if (-not $request.Headers.TryAddWithoutValidation([string]$entry.Key, [string]$entry.Value)) {
                    throw "Request header was rejected: $($entry.Key)"
                }
            }
        }
        if (-not [string]::IsNullOrWhiteSpace($JsonBody)) {
            $request.Content = [System.Net.Http.StringContent]::new($JsonBody, [System.Text.Encoding]::UTF8, 'application/json')
        }
        $response = $Client.SendAsync($request).GetAwaiter().GetResult()
        $header = @{}
        foreach ($name in @(
            'Cache-Control', 'Content-Security-Policy', 'Strict-Transport-Security',
            'Permissions-Policy', 'Cross-Origin-Opener-Policy', 'Cross-Origin-Resource-Policy',
            'X-Permitted-Cross-Domain-Policies', 'Access-Control-Allow-Origin'
        )) {
            $values = $null
            if ($response.Headers.TryGetValues($name, [ref]$values)) {
                $header[$name] = [string]::Join(',', $values)
            }
        }
        [pscustomobject]@{
            StatusCode = [int]$response.StatusCode
            Body = $response.Content.ReadAsStringAsync().GetAwaiter().GetResult()
            Headers = $header
        }
    }
    finally {
        if ($null -ne $response) { $response.Dispose() }
        if ($null -ne $request) { $request.Dispose() }
    }
}

function Assert-Status {
    param([Parameter(Mandatory = $true)]$Response, [int]$Expected, [string]$Stage)
    if ($Response.StatusCode -ne $Expected) {
        throw "$Stage returned HTTP $($Response.StatusCode), expected $Expected."
    }
}

$vault = $null
$credential = $null
$token = $null
$handler = $null
$client = $null
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
    $client.Timeout = [TimeSpan]::FromSeconds(45)

    $stage = 'public-readiness'
    $ready = Invoke-TmRequest -Client $client -Method ([System.Net.Http.HttpMethod]::Get) -Uri "$base/readyz"
    Assert-Status -Response $ready -Expected 200 -Stage $stage

    $stage = 'security-headers'
    $shell = Invoke-TmRequest -Client $client -Method ([System.Net.Http.HttpMethod]::Get) -Uri "$base/mobile/"
    Assert-Status -Response $shell -Expected 200 -Stage $stage
    $expectedHeaders = @{
        'Strict-Transport-Security' = 'max-age=31536000; includeSubDomains'
        'Cross-Origin-Opener-Policy' = 'same-origin'
        'Cross-Origin-Resource-Policy' = 'same-origin'
        'X-Permitted-Cross-Domain-Policies' = 'none'
    }
    foreach ($entry in $expectedHeaders.GetEnumerator()) {
        if ([string]$shell.Headers[$entry.Key] -ne [string]$entry.Value) {
            throw "Security header mismatch: $($entry.Key)"
        }
    }
    if ([string]$shell.Headers['Permissions-Policy'] -notmatch 'camera=\(\)' -or
        [string]$shell.Headers['Content-Security-Policy'] -notmatch "default-src 'self'" -or
        $shell.Headers.ContainsKey('Access-Control-Allow-Origin')) {
        throw 'PWA permission, CSP, or no-CORS policy did not match STEP 16.'
    }

    $stage = 'request-target-limit'
    $longTarget = "$base/healthz?value=$('a' * 2100)"
    $limited = Invoke-TmRequest -Client $client -Method ([System.Net.Http.HttpMethod]::Get) -Uri $longTarget
    Assert-Status -Response $limited -Expected 414 -Stage $stage

    $stage = 'authentication-boundary'
    $unauthenticated = Invoke-TmRequest -Client $client -Method ([System.Net.Http.HttpMethod]::Get) -Uri "$base/api/v1/tasks"
    Assert-Status -Response $unauthenticated -Expected 401 -Stage $stage
    $auth = Invoke-TmRequest -Client $client -Method ([System.Net.Http.HttpMethod]::Get) -Uri "$base/api/v1/auth/status" -Token $token
    Assert-Status -Response $auth -Expected 200 -Stage 'primary-admin-authentication'
    $authJson = $auth.Body | ConvertFrom-Json
    if ([string]$authJson.data.subject -ne 'primary-admin') {
        throw 'Production credential was not identified as primary admin.'
    }

    $stage = 'ai-preflight-no-billing'
    $ai = Invoke-TmRequest -Client $client -Method ([System.Net.Http.HttpMethod]::Post) `
        -Uri "$base/api/v1/assistant/query" -Token $token `
        -JsonBody (@{ message = 'STEP 16 preflight must not call OpenAI' } | ConvertTo-Json -Compress)
    Assert-Status -Response $ai -Expected 428 -Stage $stage
    $aiJson = $ai.Body | ConvertFrom-Json
    if ([string]$aiJson.error.code -ne 'AI_CALL_CONFIRMATION_REQUIRED') {
        throw 'AI preflight was not rejected before an OpenAI call.'
    }

    $stage = 'operations-status'
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        $ops = Invoke-TmRequest -Client $client -Method ([System.Net.Http.HttpMethod]::Get) -Uri "$base/api/v1/ops/status" -Token $token
        Assert-Status -Response $ops -Expected 200 -Stage $stage
        $opsJson = $ops.Body | ConvertFrom-Json
        $backupReady = [string]$opsJson.data.remoteBackup.status -eq 'succeeded' -and
            [int]$opsJson.data.remoteBackup.schemaVersion -ge 9 -and
            [string]$opsJson.data.remoteBackup.integrityCheck -eq 'ok'
        if (-not $backupReady) { Start-Sleep -Seconds 5 }
    } while (-not $backupReady -and [DateTime]::UtcNow -lt $deadline)
    if (-not $backupReady) { throw 'A fresh verified schema 9 remote backup was not observed.' }
    if ([string]$opsJson.data.controls.incidentMode -ne 'normal' -or
        [bool]$opsJson.data.controls.aiEnabled -ne $true -or
        [int]$opsJson.data.objectives.rpoHours -ne 24 -or
        [int]$opsJson.data.objectives.rtoHours -ne 2 -or
        [int]$opsJson.data.objectives.rollbackTargetMinutes -ne 15 -or
        [int64]$opsJson.data.aiBudget.warningLimitMicrousd -ne 10000000 -or
        [int64]$opsJson.data.aiBudget.hardLimitMicrousd -ne 20000000 -or
        [int]$opsJson.data.scheduler.deadLetterCount -ne 0) {
        throw 'Production runtime controls, SLO, budget ceiling, or scheduler state did not match STEP 16.'
    }

    $result = [ordered]@{
        success = $true
        verifiedAtUtc = [DateTime]::UtcNow.ToString('o')
        baseUri = $base
        schemaVersion = [int]$opsJson.data.database.schemaVersion
        overallStatus = [string]$opsJson.data.overallStatus
        incidentMode = [string]$opsJson.data.controls.incidentMode
        aiEnabled = [bool]$opsJson.data.controls.aiEnabled
        rpoHours = [int]$opsJson.data.objectives.rpoHours
        rtoHours = [int]$opsJson.data.objectives.rtoHours
        rollbackTargetMinutes = [int]$opsJson.data.objectives.rollbackTargetMinutes
        remoteBackupStatus = [string]$opsJson.data.remoteBackup.status
        remoteBackupIntegrityCheck = [string]$opsJson.data.remoteBackup.integrityCheck
        securityHeadersVerified = $true
        requestLimitsVerified = $true
        authenticationBoundaryVerified = $true
        aiPreflightBlockedBeforeBilling = $true
        billableAiCallPerformed = $false
        productionBusinessMutationPerformed = $false
        secretsPersistedByScript = $false
        tokenReadFromCredentialLocker = $true
    }
    $outputDirectory = Split-Path -Parent $OutputPath
    New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null
    $result | ConvertTo-Json | Set-Content -LiteralPath $OutputPath -Encoding utf8
    Write-Host 'STEP 16 production security and operations verification passed.' -ForegroundColor Green
    Write-Host "Schema: $($result.schemaVersion)"
    Write-Host "Operations: $($result.overallStatus)"
    Write-Host 'No OpenAI call or production business mutation was performed.'
}
catch {
    $failure = [ordered]@{
        success = $false
        verifiedAtUtc = [DateTime]::UtcNow.ToString('o')
        stage = $stage
        reason = $_.Exception.Message
        secretsPersistedByScript = $false
    }
    try {
        $outputDirectory = Split-Path -Parent $OutputPath
        New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null
        $failure | ConvertTo-Json | Set-Content -LiteralPath $OutputPath -Encoding utf8
    }
    catch { }
    throw
}
finally {
    if ($null -ne $client) { $client.Dispose() }
    if ($null -ne $handler) { $handler.Dispose() }
    Remove-Variable token, credential, vault, auth, authJson, ai, aiJson -ErrorAction SilentlyContinue
}

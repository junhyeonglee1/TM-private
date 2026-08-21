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
    $OutputPath = Join-Path $tmRoot 'dist\manual-step15-production-verification\result.json'
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
            $request.Content = [System.Net.Http.StringContent]::new(
                $JsonBody,
                [System.Text.Encoding]::UTF8,
                'application/json'
            )
        }
        $response = $Client.SendAsync($request).GetAwaiter().GetResult()
        $setCookies = [System.Collections.Generic.List[string]]::new()
        $values = $null
        if ($response.Headers.TryGetValues('Set-Cookie', [ref]$values)) {
            foreach ($value in $values) { $setCookies.Add([string]$value) }
        }
        [pscustomobject]@{
            StatusCode = [int]$response.StatusCode
            Body = $response.Content.ReadAsStringAsync().GetAwaiter().GetResult()
            SetCookies = @($setCookies)
            CacheControl = if ($null -ne $response.Headers.CacheControl) { [string]$response.Headers.CacheControl } else { $null }
            ContentSecurityPolicy = if ($response.Headers.Contains('Content-Security-Policy')) {
                [string]::Join(';', $response.Headers.GetValues('Content-Security-Policy'))
            } else { $null }
            AccessControlAllowOrigin = if ($response.Headers.Contains('Access-Control-Allow-Origin')) {
                [string]::Join(',', $response.Headers.GetValues('Access-Control-Allow-Origin'))
            } else { $null }
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
$temporaryDeviceId = $null
$pairingCode = $null
$pollingSecret = $null
$deviceCookie = $null
$csrfToken = $null
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

    $stage = 'pwa-shell'
    $shell = Invoke-TmRequest -Client $client -Method ([System.Net.Http.HttpMethod]::Get) -Uri "$base/mobile/"
    Assert-Status -Response $shell -Expected 200 -Stage $stage
    if ($shell.Body -notmatch '<title>TM Assistant</title>' -or
        $shell.ContentSecurityPolicy -notmatch "default-src 'self'" -or
        -not [string]::IsNullOrWhiteSpace($shell.AccessControlAllowOrigin)) {
        throw 'PWA shell security or same-origin policy did not match STEP 15.'
    }
    $serviceWorker = Invoke-TmRequest -Client $client -Method ([System.Net.Http.HttpMethod]::Get) -Uri "$base/mobile/sw.js"
    Assert-Status -Response $serviceWorker -Expected 200 -Stage 'pwa-service-worker'
    if ($serviceWorker.Body -notmatch 'url.pathname.startsWith\("/api/"\)' -or
        $serviceWorker.Body -notmatch 'request.method !== "GET"') {
        throw 'PWA service worker does not enforce the app-shell-only cache boundary.'
    }

    $stage = 'primary-admin-authentication'
    $auth = Invoke-TmRequest -Client $client -Method ([System.Net.Http.HttpMethod]::Get) -Uri "$base/api/v1/auth/status" -Token $token
    Assert-Status -Response $auth -Expected 200 -Stage $stage
    $authJson = $auth.Body | ConvertFrom-Json
    if ([string]$authJson.data.subject -ne 'primary-admin') {
        throw 'Production bearer credential was not identified as primary admin.'
    }

    $stage = 'pairing-start'
    $label = "STEP15 verification $([DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss'))"
    $started = Invoke-TmRequest -Client $client -Method ([System.Net.Http.HttpMethod]::Post) `
        -Uri "$base/api/v1/device-pairings" -Headers @{ Origin = $base } `
        -JsonBody (@{ deviceLabel = $label } | ConvertTo-Json -Compress)
    Assert-Status -Response $started -Expected 200 -Stage $stage
    $startedJson = $started.Body | ConvertFrom-Json
    $pairingId = [string]$startedJson.data.pairing.id
    $pairingCode = [string]$startedJson.data.code
    $pollingSecret = [string]$startedJson.data.pollingSecret
    if ($pairingId -notmatch '^[A-Za-z0-9-]{1,64}$' -or $pairingCode -notmatch '^\d{6}$' -or
        [string]::IsNullOrWhiteSpace($pollingSecret)) {
        throw 'Pairing start response did not match the bounded contract.'
    }

    $stage = 'pairing-admin-approval'
    $approved = Invoke-TmRequest -Client $client -Method ([System.Net.Http.HttpMethod]::Post) `
        -Uri "$base/api/v1/admin/device-pairings/$pairingId/approve" -Token $token `
        -Headers @{ 'x-tm-confirm-device-admin' = 'approve' } `
        -JsonBody (@{ code = $pairingCode } | ConvertTo-Json -Compress)
    Assert-Status -Response $approved -Expected 200 -Stage $stage

    $stage = 'pairing-complete'
    $completed = Invoke-TmRequest -Client $client -Method ([System.Net.Http.HttpMethod]::Post) `
        -Uri "$base/api/v1/device-pairings/$pairingId/complete" -Headers @{ Origin = $base } `
        -JsonBody (@{ pollingSecret = $pollingSecret } | ConvertTo-Json -Compress)
    Assert-Status -Response $completed -Expected 200 -Stage $stage
    $completedJson = $completed.Body | ConvertFrom-Json
    $temporaryDeviceId = [string]$completedJson.data.id
    $deviceSetCookie = @($completed.SetCookies | Where-Object { $_ -like '__Host-tm_device=*' })
    $csrfSetCookie = @($completed.SetCookies | Where-Object { $_ -like '__Host-tm_csrf=*' })
    if ($deviceSetCookie.Count -ne 1 -or $csrfSetCookie.Count -ne 1 -or
        $deviceSetCookie[0] -notmatch '; Secure; HttpOnly; SameSite=Strict' -or
        $csrfSetCookie[0] -notmatch '; Secure; SameSite=Strict' -or
        $csrfSetCookie[0] -match 'HttpOnly') {
        throw 'Device or CSRF cookie security attributes did not match STEP 15.'
    }
    $deviceCookie = ($deviceSetCookie[0] -split ';', 2)[0]
    $csrfCookie = ($csrfSetCookie[0] -split ';', 2)[0]
    $csrfToken = ($csrfCookie -split '=', 2)[1]
    $cookieHeader = "$deviceCookie; $csrfCookie"

    $stage = 'device-self-and-scope'
    $self = Invoke-TmRequest -Client $client -Method ([System.Net.Http.HttpMethod]::Get) `
        -Uri "$base/api/v1/device/self" -Headers @{ Cookie = $cookieHeader }
    Assert-Status -Response $self -Expected 200 -Stage 'device-self'
    $forbiddenOps = Invoke-TmRequest -Client $client -Method ([System.Net.Http.HttpMethod]::Get) `
        -Uri "$base/api/v1/ops/status" -Headers @{ Cookie = $cookieHeader }
    Assert-Status -Response $forbiddenOps -Expected 403 -Stage 'device-ops-scope'
    $forbiddenAdmin = Invoke-TmRequest -Client $client -Method ([System.Net.Http.HttpMethod]::Get) `
        -Uri "$base/api/v1/admin/devices" -Headers @{ Cookie = $cookieHeader }
    Assert-Status -Response $forbiddenAdmin -Expected 403 -Stage 'device-admin-scope'

    $stage = 'device-revocation'
    $revoked = Invoke-TmRequest -Client $client -Method ([System.Net.Http.HttpMethod]::Post) `
        -Uri "$base/api/v1/admin/devices/$temporaryDeviceId/revoke" -Token $token `
        -Headers @{ 'x-tm-confirm-device-admin' = 'revoke' } -JsonBody '{}'
    Assert-Status -Response $revoked -Expected 200 -Stage $stage
    $temporaryDeviceId = $null
    $afterRevoke = Invoke-TmRequest -Client $client -Method ([System.Net.Http.HttpMethod]::Get) `
        -Uri "$base/api/v1/device/self" -Headers @{ Cookie = $cookieHeader }
    Assert-Status -Response $afterRevoke -Expected 401 -Stage 'post-revocation-authentication'

    $stage = 'schema9-backup-readiness'
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    $opsJson = $null
    $ready = $false
    do {
        $ops = Invoke-TmRequest -Client $client -Method ([System.Net.Http.HttpMethod]::Get) `
            -Uri "$base/api/v1/ops/status" -Token $token
        if ($ops.StatusCode -eq 200) {
            $opsJson = $ops.Body | ConvertFrom-Json
            $ready = (
                [bool]$opsJson.data.database.ok -eq $true -and
                [int]$opsJson.data.database.schemaVersion -eq 9 -and
                [string]$opsJson.data.scheduler.status -eq 'healthy' -and
                [int]$opsJson.data.scheduler.deadLetterCount -eq 0 -and
                [string]$opsJson.data.remoteBackup.status -eq 'succeeded' -and
                [int]$opsJson.data.remoteBackup.schemaVersion -eq 9 -and
                [string]$opsJson.data.remoteBackup.integrityCheck -eq 'ok'
            )
            if ($ready) { break }
        }
        Start-Sleep -Seconds 5
    } while ([DateTime]::UtcNow -lt $deadline)
    if (-not $ready) {
        throw 'Schema 9 service and verified schema 9 remote backup did not become ready before timeout.'
    }

    $result = [ordered]@{
        success = $true
        verifiedAtUtc = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ss.fffZ')
        baseUri = $base
        schemaVersion = 9
        pwaShellAvailable = $true
        appShellOnlyOfflineCache = $true
        primaryAdminSeparated = $true
        pairingAndCookieVerified = $true
        deviceScopeVerified = $true
        immediateRevocationVerified = $true
        remoteBackupStatus = [string]$opsJson.data.remoteBackup.status
        remoteBackupSchemaVersion = [int]$opsJson.data.remoteBackup.schemaVersion
        remoteBackupIntegrityCheck = [string]$opsJson.data.remoteBackup.integrityCheck
        localTmServerRequired = $false
        billableAiCallPerformed = $false
        productionBusinessMutationPerformed = $false
        temporaryAuthLedgerMutationPerformed = $true
        temporaryDeviceRevoked = $true
        secretsPersistedByScript = $false
        tokenReadFromCredentialLocker = $true
    }
    $outputDirectory = Split-Path -Parent $OutputPath
    New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null
    $result | ConvertTo-Json | Set-Content -LiteralPath $OutputPath -Encoding utf8

    Write-Host ''
    Write-Host 'STEP 15 production PWA and device-auth verification passed.' -ForegroundColor Green
    Write-Host "Schema: $($result.schemaVersion)"
    Write-Host "PWA: $base/mobile/"
    Write-Host 'Temporary verification device was registered, scope-tested, and revoked.'
    Write-Host 'No OpenAI call or production Task/Note mutation was performed.'
}
catch {
    $failure = [ordered]@{
        success = $false
        failedAtUtc = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ss.fffZ')
        stage = $stage
        reason = $_.Exception.Message
        billableAiCallPerformed = $false
        productionBusinessMutationPerformed = $false
        secretsPersistedByScript = $false
    }
    try {
        $outputDirectory = Split-Path -Parent $OutputPath
        New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null
        $failure | ConvertTo-Json | Set-Content -LiteralPath $OutputPath -Encoding utf8
    }
    catch { Write-Warning 'Could not write the STEP 15 failure result.' }
    throw "STEP 15 production verification failed at ${stage}: $($_.Exception.Message)"
}
finally {
    if ($null -ne $temporaryDeviceId -and $null -ne $client -and
        -not [string]::IsNullOrWhiteSpace($token)) {
        try {
            $cleanup = Invoke-TmRequest -Client $client -Method ([System.Net.Http.HttpMethod]::Post) `
                -Uri "$base/api/v1/admin/devices/$temporaryDeviceId/revoke" -Token $token `
                -Headers @{ 'x-tm-confirm-device-admin' = 'revoke' } -JsonBody '{}'
            if ($cleanup.StatusCode -ne 200 -and $cleanup.StatusCode -ne 409) {
                Write-Warning "Temporary STEP 15 device cleanup returned HTTP $($cleanup.StatusCode)."
            }
        }
        catch { Write-Warning 'Temporary STEP 15 device cleanup could not be confirmed.' }
    }
    if ($null -ne $client) { $client.Dispose() }
    if ($null -ne $handler) { $handler.Dispose() }
    $pairingCode = $null
    $pollingSecret = $null
    $deviceCookie = $null
    $csrfToken = $null
    $token = $null
    $credential = $null
    $vault = $null
}

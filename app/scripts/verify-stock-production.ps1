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

$resource = 'TM Cloud Production'
$userName = 'single-user'
$tmRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
if ([string]::IsNullOrWhiteSpace($OutputPath)) {
    $OutputPath = Join-Path $tmRoot 'dist\manual-stock-production-verification\result.json'
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
        $setCookies = @()
        if ($response.Headers.Contains('Set-Cookie')) {
            $setCookies = @($response.Headers.GetValues('Set-Cookie'))
        }
        $csp = ''
        if ($response.Headers.Contains('Content-Security-Policy')) {
            $csp = [string]::Join(';', $response.Headers.GetValues('Content-Security-Policy'))
        }
        $xFrameOptions = ''
        if ($response.Headers.Contains('X-Frame-Options')) {
            $xFrameOptions = [string]::Join(';', $response.Headers.GetValues('X-Frame-Options'))
        }
        [pscustomobject]@{
            StatusCode = [int]$response.StatusCode
            Body = $response.Content.ReadAsStringAsync().GetAwaiter().GetResult()
            SetCookies = $setCookies
            ContentSecurityPolicy = $csp
            XFrameOptions = $xFrameOptions
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

function Invoke-DesktopCommand {
    param(
        [System.Net.Http.HttpClient]$Client,
        [string]$Base,
        [string]$Token,
        [string]$Command,
        [object]$CommandArgs,
        [switch]$Confirm
    )
    $headers = @{}
    if ($Confirm) {
        $headers['x-tm-confirm-desktop-command'] = $Command
    }
    $body = @{ args = $CommandArgs } | ConvertTo-Json -Depth 12 -Compress
    $response = Invoke-TmRequest `
        -Client $Client `
        -Method ([System.Net.Http.HttpMethod]::Post) `
        -Uri "$Base/api/v1/desktop/commands/$Command" `
        -Token $Token `
        -Headers $headers `
        -JsonBody $body
    Require-Status $response 200 $Command
    return ($response.Body | ConvertFrom-Json).data
}

$vault = $null
$credential = $null
$token = $null
$handler = $null
$client = $null
$base = $null
$stage = 'credential-locker'
$addedSymbols = [System.Collections.Generic.List[string]]::new()
$deviceId = $null
$verified = $false
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
    $client.Timeout = [TimeSpan]::FromSeconds(60)

    $stage = 'readiness'
    $ready = Invoke-TmRequest -Client $client -Method ([System.Net.Http.HttpMethod]::Get) -Uri "$base/readyz" -Token $token
    Require-Status $ready 200 $stage

    $stage = 'operations-status'
    $ops = Invoke-TmRequest -Client $client -Method ([System.Net.Http.HttpMethod]::Get) -Uri "$base/api/v1/ops/status" -Token $token
    Require-Status $ops 200 $stage
    $opsData = ($ops.Body | ConvertFrom-Json).data
    if ([int]$opsData.database.schemaVersion -ne 12 -or
        [string]$opsData.remoteBackup.status -ne 'succeeded' -or
        [int]$opsData.remoteBackup.schemaVersion -ne 12 -or
        [string]$opsData.remoteBackup.integrityCheck -ne 'ok') {
        throw 'Production DB and remote backup must both be healthy schema 12.'
    }

    $stage = 'pwa-security'
    $pwa = Invoke-TmRequest -Client $client -Method ([System.Net.Http.HttpMethod]::Get) -Uri "$base/mobile/"
    Require-Status $pwa 200 $stage
    $hasStockTab = [regex]::IsMatch([string]$pwa.Body, 'id="tab-stocks"')
    $hasExternalFallback = [regex]::IsMatch(
        [string]$pwa.Body,
        'id="stock-fallback-link"'
    )
    $hasTradingViewFrame = [regex]::IsMatch(
        [string]$pwa.ContentSecurityPolicy,
        'frame-src.+https://s\.tradingview\.com.+https://www\.tradingview-widget\.com'
    )
    if (-not ($hasStockTab -and $hasExternalFallback -and $hasTradingViewFrame)) {
        throw (
            'The production PWA stock shell or TradingView CSP is missing. ' +
            "tab=$hasStockTab fallback=$hasExternalFallback " +
            "frame=$hasTradingViewFrame"
        )
    }
    $pwaScript = Invoke-TmRequest -Client $client -Method ([System.Net.Http.HttpMethod]::Get) -Uri "$base/mobile/app.js"
    Require-Status $pwaScript 200 "$stage-script"
    if ($pwaScript.Body -notmatch 'allow-scripts allow-same-origin allow-popups allow-popups-to-escape-sandbox' -or
        $pwaScript.Body -notmatch 'tradingview-widget\.com/embed-widget/advanced-chart/' -or
        $pwaScript.Body -match 'data:text/html;charset=utf-8' -or
        $pwaScript.Body -match 'embed-widget-advanced-chart\.js' -or
        $pwaScript.Body -match 'createElement\("script"\)' -or
        $pwaScript.Body -notmatch 'support_host' -or
        $pwaScript.Body -notmatch 'stock-chart-loading' -or
        $pwaScript.Body -match '__TAURI' -or
        $pwaScript.Body -notmatch '/mobile/stock-catalog\.json') {
        throw 'The production PWA TradingView sandbox contract is invalid.'
    }
    $stockCatalog = Invoke-TmRequest -Client $client -Method ([System.Net.Http.HttpMethod]::Get) -Uri "$base/mobile/stock-catalog.json"
    Require-Status $stockCatalog 200 "$stage-catalog"
    if ($stockCatalog.Body -notmatch '"ticker":"005930","name":"\uC0BC\uC131\uC804\uC790"' -or
        $stockCatalog.Body -notmatch '"ticker":"AAPL","name":"Apple Inc\."') {
        throw 'The production stock name-search catalog is incomplete.'
    }

    $stage = 'ai-cost-before'
    $costBeforeResponse = Invoke-TmRequest -Client $client -Method ([System.Net.Http.HttpMethod]::Get) -Uri "$base/api/v1/costs/status" -Token $token
    Require-Status $costBeforeResponse 200 $stage
    $costBefore = [int64](($costBeforeResponse.Body | ConvertFrom-Json).data.api.usedMicrousd)

    $stage = 'desktop-stock-sync'
    $watchlist = @(Invoke-DesktopCommand -Client $client -Base $base -Token $token -Command 'get_stock_watchlist' -CommandArgs @{})
    $fixtures = @(
        @{ market = 'NASDAQ'; ticker = 'AAPL'; displayName = 'Apple'; symbol = 'NASDAQ:AAPL' },
        @{ market = 'KRX'; ticker = '005930'; displayName = 'Samsung Electronics'; symbol = 'KRX:005930' }
    )
    foreach ($fixture in $fixtures) {
        if (-not @($watchlist | Where-Object { [string]$_.symbol -eq [string]$fixture.symbol }).Count) {
            $saved = Invoke-DesktopCommand `
                -Client $client `
                -Base $base `
                -Token $token `
                -Command 'upsert_stock_watchlist_item' `
                -CommandArgs @{ input = @{ market = $fixture.market; ticker = $fixture.ticker; displayName = $fixture.displayName } } `
                -Confirm
            if ([string]$saved.symbol -ne [string]$fixture.symbol) {
                throw "Unexpected normalized stock symbol: $($saved.symbol)"
            }
            $addedSymbols.Add([string]$fixture.symbol)
        }
    }

    $stage = 'temporary-mobile-pairing'
    $pairingStart = Invoke-TmRequest `
        -Client $client `
        -Method ([System.Net.Http.HttpMethod]::Post) `
        -Uri "$base/api/v1/device-pairings" `
        -Headers @{ Origin = $base } `
        -JsonBody (@{ deviceLabel = 'TM stock production verification' } | ConvertTo-Json -Compress)
    Require-Status $pairingStart 200 "$stage-start"
    $pairing = ($pairingStart.Body | ConvertFrom-Json).data
    $pairingId = [string]$pairing.pairing.id

    $pairingApproval = Invoke-TmRequest `
        -Client $client `
        -Method ([System.Net.Http.HttpMethod]::Post) `
        -Uri "$base/api/v1/admin/device-pairings/$pairingId/approve" `
        -Token $token `
        -Headers @{ 'x-tm-confirm-device-admin' = 'approve' } `
        -JsonBody (@{ code = [string]$pairing.code } | ConvertTo-Json -Compress)
    Require-Status $pairingApproval 200 "$stage-approve"

    $pairingComplete = Invoke-TmRequest `
        -Client $client `
        -Method ([System.Net.Http.HttpMethod]::Post) `
        -Uri "$base/api/v1/device-pairings/$pairingId/complete" `
        -Headers @{ Origin = $base } `
        -JsonBody (@{ pollingSecret = [string]$pairing.pollingSecret } | ConvertTo-Json -Compress)
    Require-Status $pairingComplete 200 "$stage-complete"
    $deviceId = [string](($pairingComplete.Body | ConvertFrom-Json).data.id)
    $cookiePairs = @($pairingComplete.SetCookies | ForEach-Object { ([string]$_).Split(';')[0] })
    $csrfPair = @($cookiePairs | Where-Object { $_ -like '__Host-tm_csrf=*' })
    if ($cookiePairs.Count -ne 2 -or $csrfPair.Count -ne 1) {
        throw 'The temporary mobile device cookies were not issued correctly.'
    }
    $cookieHeader = [string]::Join('; ', $cookiePairs)
    $csrfToken = $csrfPair[0].Substring('__Host-tm_csrf='.Length)

    $stage = 'mobile-stock-sync'
    $mobileList = Invoke-TmRequest `
        -Client $client `
        -Method ([System.Net.Http.HttpMethod]::Post) `
        -Uri "$base/api/v1/desktop/commands/get_stock_watchlist" `
        -Headers @{ Origin = $base; Cookie = $cookieHeader; 'x-tm-csrf' = $csrfToken } `
        -JsonBody '{"args":{}}'
    Require-Status $mobileList 200 $stage
    $mobileItems = @(($mobileList.Body | ConvertFrom-Json).data)
    foreach ($symbol in @('NASDAQ:AAPL', 'KRX:005930')) {
        if (-not @($mobileItems | Where-Object { [string]$_.symbol -eq $symbol }).Count) {
            throw "The temporary mobile device did not read $symbol."
        }
    }

    $stage = 'ai-cost-after'
    $costAfterResponse = Invoke-TmRequest -Client $client -Method ([System.Net.Http.HttpMethod]::Get) -Uri "$base/api/v1/costs/status" -Token $token
    Require-Status $costAfterResponse 200 $stage
    $costAfter = [int64](($costAfterResponse.Body | ConvertFrom-Json).data.api.usedMicrousd)
    if ($costAfter -ne $costBefore) {
        throw "Stock-only verification changed the AI cost ledger: $costBefore -> $costAfter."
    }

    $result = [ordered]@{
        verifiedAtUtc = [DateTime]::UtcNow.ToString('o')
        baseUri = $base
        schemaVersion = 12
        symbols = @('NASDAQ:AAPL', 'KRX:005930')
        desktopSync = $true
        mobileSync = $true
        crossOriginTradingViewSandbox = $true
        pwaCspVerified = $true
        aiCostBeforeMicrousd = $costBefore
        aiCostAfterMicrousd = $costAfter
    }
    $directory = Split-Path -Parent $OutputPath
    [System.IO.Directory]::CreateDirectory($directory) | Out-Null
    [System.IO.File]::WriteAllText(
        $OutputPath,
        ($result | ConvertTo-Json -Depth 10),
        [System.Text.UTF8Encoding]::new($false)
    )
    $verified = $true
}
catch {
    throw "Stock production verification failed at $stage. $($_.Exception.Message)"
}
finally {
    if ($null -ne $client -and -not [string]::IsNullOrWhiteSpace($base) -and
        -not [string]::IsNullOrWhiteSpace($token)) {
        foreach ($symbol in $addedSymbols) {
            try {
                Invoke-DesktopCommand `
                    -Client $client `
                    -Base $base `
                    -Token $token `
                    -Command 'delete_stock_watchlist_item' `
                    -CommandArgs @{ symbol = $symbol } `
                    -Confirm | Out-Null
            }
            catch {
                Write-Warning "Could not remove temporary stock $symbol. $($_.Exception.Message)"
                $verified = $false
            }
        }
        if (-not [string]::IsNullOrWhiteSpace($deviceId)) {
            try {
                $revoke = Invoke-TmRequest `
                    -Client $client `
                    -Method ([System.Net.Http.HttpMethod]::Post) `
                    -Uri "$base/api/v1/admin/devices/$deviceId/revoke" `
                    -Token $token `
                    -Headers @{ 'x-tm-confirm-device-admin' = 'revoke' }
                Require-Status $revoke 200 'temporary-device-revoke'
            }
            catch {
                Write-Warning "Could not revoke the temporary mobile device. $($_.Exception.Message)"
                $verified = $false
            }
        }
    }
    $token = $null
    if ($null -ne $client) { $client.Dispose() }
    if ($null -ne $handler) { $handler.Dispose() }
    $credential = $null
    $vault = $null
}

if (-not $verified) {
    throw 'Stock production verification cleanup did not complete.'
}
Write-Host 'TM stock production verification passed.' -ForegroundColor Green
Write-Host 'Windows/mobile sync: passed; AI cost change: 0 microUSD'
Write-Host "Result: $OutputPath"

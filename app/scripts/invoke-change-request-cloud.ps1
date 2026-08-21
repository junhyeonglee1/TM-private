[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet('Prepare', 'Status', 'Backup', 'Complete', 'Fail')]
    [string]$Action,

    [string]$BaseUri,

    [ValidatePattern('^[A-Za-z0-9._-]{1,64}$')]
    [string]$WorkerId = 'codex-cloud',

    [string]$RequestId,

    [string]$ClaimKey,

    [string]$Summary,

    [string]$PatchRef,

    [string]$Reason
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
Add-Type -AssemblyName System.Net.Http

$tmRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
$configPath = Join-Path $tmRoot 'data\cloud-client.json'
$resource = 'TM Cloud Production'
$userName = 'single-user'

function Require-Text {
    param(
        [string]$Name,
        [string]$Value,
        [int]$MaximumLength
    )
    if ([string]::IsNullOrWhiteSpace($Value) -or $Value.Length -gt $MaximumLength) {
        throw "$Name is required and must be at most $MaximumLength characters."
    }
}

function Require-Identifier {
    param([string]$Name, [string]$Value)
    Require-Text $Name $Value 128
    if ($Value -notmatch '^[A-Za-z0-9-]+$') {
        throw "$Name has an invalid format."
    }
}

if (-not (Test-Path -LiteralPath $configPath -PathType Leaf)) {
    throw 'TM cloud client configuration was not found.'
}
$configuration = Get-Content -LiteralPath $configPath -Encoding UTF8 -Raw | ConvertFrom-Json
if ($configuration.mode -ne 'cloud') {
    throw 'TM change request processing requires cloud mode.'
}
$configuredBaseUri = [string]$configuration.baseUrl
if ([string]::IsNullOrWhiteSpace($BaseUri)) {
    $BaseUri = $configuredBaseUri
} elseif ($BaseUri.TrimEnd('/') -ne $configuredBaseUri.TrimEnd('/')) {
    throw 'BaseUri must match the configured TM cloud origin.'
}

$uri = [System.Uri]$BaseUri
if (-not $uri.IsAbsoluteUri -or $uri.Scheme -ne 'https' -or
    -not [string]::IsNullOrEmpty($uri.UserInfo) -or
    -not [string]::IsNullOrEmpty($uri.Query) -or
    -not [string]::IsNullOrEmpty($uri.Fragment)) {
    throw 'TM cloud base URL must be a credential-free HTTPS origin.'
}
$base = $uri.GetLeftPart([System.UriPartial]::Authority).TrimEnd('/')

$command = switch ($Action) {
    'Prepare' { 'claim_next_change_request' }
    'Status' { 'list_change_requests' }
    'Backup' { 'create_backup' }
    'Complete' { 'complete_change_request' }
    'Fail' { 'fail_change_request' }
}

$commandArgs = switch ($Action) {
    'Prepare' {
        @{ workerId = $WorkerId }
    }
    'Status' {
        @{}
    }
    'Backup' {
        @{}
    }
    'Complete' {
        Require-Identifier 'RequestId' $RequestId
        Require-Identifier 'ClaimKey' $ClaimKey
        Require-Text 'Summary' $Summary 4000
        if (-not [string]::IsNullOrWhiteSpace($PatchRef) -and $PatchRef.Length -gt 200) {
            throw 'PatchRef must be at most 200 characters.'
        }
        @{
            requestId = $RequestId
            claimKey = $ClaimKey
            resultSummary = $Summary.Trim()
            patchRef = if ([string]::IsNullOrWhiteSpace($PatchRef)) { $null } else { $PatchRef.Trim() }
            workerId = $WorkerId
        }
    }
    'Fail' {
        Require-Identifier 'RequestId' $RequestId
        Require-Identifier 'ClaimKey' $ClaimKey
        Require-Text 'Reason' $Reason 4000
        @{
            requestId = $RequestId
            claimKey = $ClaimKey
            failureReason = $Reason.Trim()
            workerId = $WorkerId
        }
    }
}

$vault = $null
$credential = $null
$token = $null
$handler = $null
$client = $null
$request = $null
$response = $null
try {
    $vault = [Windows.Security.Credentials.PasswordVault,Windows.Security.Credentials,ContentType=WindowsRuntime]::new()
    $credential = $vault.Retrieve($resource, $userName)
    $credential.RetrievePassword()
    $token = [string]$credential.Password
    if ($token -notmatch '^tm_pat_v1_[A-Za-z0-9_-]{43}$') {
        throw 'TM production credential has an invalid format.'
    }

    $handler = [System.Net.Http.HttpClientHandler]::new()
    $handler.AllowAutoRedirect = $false
    $handler.UseCookies = $false
    $client = [System.Net.Http.HttpClient]::new($handler)
    $client.Timeout = [TimeSpan]::FromSeconds(30)

    $endpoint = "$base/api/v1/desktop/commands/$command"
    $request = [System.Net.Http.HttpRequestMessage]::new([System.Net.Http.HttpMethod]::Post, $endpoint)
    $request.Headers.Authorization = [System.Net.Http.Headers.AuthenticationHeaderValue]::new('Bearer', $token)
    if ($Action -ne 'Status') {
        [void]$request.Headers.TryAddWithoutValidation('x-tm-confirm-desktop-command', $command)
    }
    $body = @{ args = $commandArgs } | ConvertTo-Json -Depth 10 -Compress
    $request.Content = [System.Net.Http.StringContent]::new(
        $body,
        [System.Text.Encoding]::UTF8,
        'application/json'
    )

    $response = $client.SendAsync($request).GetAwaiter().GetResult()
    $content = $response.Content.ReadAsStringAsync().GetAwaiter().GetResult()
    if ($content.Length -gt (16 * 1024 * 1024)) {
        throw 'TM cloud response exceeded the safety limit.'
    }
    if (-not $response.IsSuccessStatusCode) {
        $requestId = if ($response.Headers.Contains('x-request-id')) {
            [string]::Join(',', $response.Headers.GetValues('x-request-id'))
        } else {
            'unavailable'
        }
        throw "TM cloud change request command failed with HTTP $([int]$response.StatusCode) (request $requestId)."
    }
    $payload = $content | ConvertFrom-Json
    if ($null -eq $payload -or -not ($payload.PSObject.Properties.Name -contains 'data')) {
        throw 'TM cloud response did not contain data.'
    }
    $payload.data | ConvertTo-Json -Depth 12
}
finally {
    $token = $null
    if ($null -ne $response) { $response.Dispose() }
    if ($null -ne $request) { $request.Dispose() }
    if ($null -ne $client) { $client.Dispose() }
    if ($null -ne $handler) { $handler.Dispose() }
    $credential = $null
    $vault = $null
}

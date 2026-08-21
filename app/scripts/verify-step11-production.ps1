[CmdletBinding()]
param(
    [ValidatePattern('^https://')]
    [string]$BaseUri = 'https://tm-server-production-5573.up.railway.app',

    [bool]$ExpectedConfigured = $false,

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
    $OutputPath = Join-Path $tmRoot 'dist\manual-step11-production-verification\result.json'
}
$OutputPath = [System.IO.Path]::GetFullPath($OutputPath)

function Invoke-TmRequest {
    param(
        [Parameter(Mandatory = $true)][System.Net.Http.HttpClient]$Client,
        [Parameter(Mandatory = $true)][string]$Method,
        [Parameter(Mandatory = $true)][string]$Uri,
        [Parameter(Mandatory = $true)][string]$Token,
        [string]$Confirmation,
        [string]$JsonBody
    )

    $request = $null
    $response = $null
    $content = $null
    try {
        $httpMethod = if ($Method -eq 'POST') {
            [System.Net.Http.HttpMethod]::Post
        }
        else {
            [System.Net.Http.HttpMethod]::Get
        }
        $request = [System.Net.Http.HttpRequestMessage]::new($httpMethod, $Uri)
        $request.Headers.Authorization = [System.Net.Http.Headers.AuthenticationHeaderValue]::new('Bearer', $Token)
        if (-not [string]::IsNullOrWhiteSpace($Confirmation)) {
            $request.Headers.Add('x-tm-confirm-ai-call', $Confirmation)
        }
        if ($Method -eq 'POST' -and $PSBoundParameters.ContainsKey('JsonBody')) {
            $content = [System.Net.Http.StringContent]::new($JsonBody, [System.Text.Encoding]::UTF8, 'application/json')
            $request.Content = $content
        }
        $response = $Client.SendAsync($request).GetAwaiter().GetResult()
        $body = $response.Content.ReadAsStringAsync().GetAwaiter().GetResult()
        return [pscustomobject]@{
            StatusCode = [int]$response.StatusCode
            Body = $body
        }
    }
    finally {
        if ($null -ne $response) { $response.Dispose() }
        if ($null -ne $content) { $content.Dispose() }
        if ($null -ne $request) { $request.Dispose() }
    }
}

$vault = $null
$credential = $null
$token = $null
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

    $client = [System.Net.Http.HttpClient]::new()
    $client.Timeout = [TimeSpan]::FromSeconds(30)

    $stage = 'authentication'
    $auth = Invoke-TmRequest -Client $client -Method 'GET' -Uri "$base/api/v1/auth/status" -Token $token
    if ($auth.StatusCode -ne 200) {
        throw "Authentication failed with HTTP $($auth.StatusCode)."
    }

    $stage = 'ai-status'
    $ai = Invoke-TmRequest -Client $client -Method 'GET' -Uri "$base/api/v1/ai/status" -Token $token
    if ($ai.StatusCode -ne 200) {
        throw "AI status failed with HTTP $($ai.StatusCode)."
    }
    $aiJson = $ai.Body | ConvertFrom-Json
    if ([string]$aiJson.data.model -ne 'gpt-5.6-terra') {
        throw "Expected gpt-5.6-terra, received $($aiJson.data.model)."
    }
    if ([bool]$aiJson.data.configured -ne $ExpectedConfigured) {
        throw "Expected configured=$ExpectedConfigured, received $($aiJson.data.configured)."
    }
    if ([string]$aiJson.data.responseStorage -ne 'disabled' -or
        [bool]$aiJson.data.assistantReadOnly -ne $true -or
        [string]$aiJson.data.assistantPromptVersion -ne 'step11-v1' -or
        [int64]$aiJson.data.assistantMaximumCostMicrousd -ne 250000 -or
        [int]$aiJson.data.assistantMaxToolCalls -ne 6 -or
        [int]$aiJson.data.assistantTimeoutSeconds -ne 60 -or
        [int64]$aiJson.data.budget.warningLimitMicrousd -ne 10000000 -or
        [int64]$aiJson.data.budget.hardLimitMicrousd -ne 20000000) {
        throw 'The STEP 11 AI safety contract does not match the approved defaults.'
    }

    $stage = 'billable-confirmation-guard'
    $unconfirmed = Invoke-TmRequest -Client $client -Method 'POST' -Uri "$base/api/v1/assistant/query" -Token $token -JsonBody '{"message":"non-billable guard check"}'
    if ($unconfirmed.StatusCode -ne 428) {
        throw "Unconfirmed assistant request returned HTTP $($unconfirmed.StatusCode), expected 428."
    }
    $unconfirmedJson = $unconfirmed.Body | ConvertFrom-Json
    if ([string]$unconfirmedJson.error.code -ne 'AI_CALL_CONFIRMATION_REQUIRED') {
        throw 'The assistant confirmation guard returned an unexpected error code.'
    }

    if (-not $ExpectedConfigured) {
        $stage = 'missing-key-guard'
        $missingKey = Invoke-TmRequest -Client $client -Method 'POST' -Uri "$base/api/v1/assistant/query" -Token $token -Confirmation 'assistant' -JsonBody '{"message":"missing key guard check"}'
        if ($missingKey.StatusCode -ne 503) {
            throw "Missing-key guard returned HTTP $($missingKey.StatusCode), expected 503."
        }
        $missingKeyJson = $missingKey.Body | ConvertFrom-Json
        if ([string]$missingKeyJson.error.code -ne 'OPENAI_NOT_CONFIGURED') {
            throw 'The missing-key guard returned an unexpected error code.'
        }
    }

    $stage = 'complete'
    $result = [ordered]@{
        success = $true
        verifiedAtUtc = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ss.fffZ')
        baseUri = $base
        authenticated = $true
        aiRouteStatus = $ai.StatusCode
        aiConfigured = [bool]$aiJson.data.configured
        aiModel = [string]$aiJson.data.model
        responseStorage = [string]$aiJson.data.responseStorage
        assistantReadOnly = [bool]$aiJson.data.assistantReadOnly
        assistantPromptVersion = [string]$aiJson.data.assistantPromptVersion
        assistantMaximumCostMicrousd = [int64]$aiJson.data.assistantMaximumCostMicrousd
        assistantMaxToolCalls = [int]$aiJson.data.assistantMaxToolCalls
        assistantTimeoutSeconds = [int]$aiJson.data.assistantTimeoutSeconds
        aiWarningLimitMicrousd = [int64]$aiJson.data.budget.warningLimitMicrousd
        aiHardLimitMicrousd = [int64]$aiJson.data.budget.hardLimitMicrousd
        confirmationGuardStatus = $unconfirmed.StatusCode
        missingKeyGuardStatus = if ($ExpectedConfigured) { $null } else { $missingKey.StatusCode }
        billableAiCallPerformed = $false
        productionMutationPerformed = $false
        tokenReadFromCredentialLocker = $true
        tokenPersistedByScript = $false
    }

    $outputDirectory = Split-Path -Parent $OutputPath
    New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null
    $result | ConvertTo-Json | Set-Content -LiteralPath $OutputPath -Encoding utf8

    Write-Host ''
    Write-Host 'STEP 11 production non-billable verification passed.' -ForegroundColor Green
    Write-Host "AI configured: $($result.aiConfigured)"
    Write-Host "Model: $($result.aiModel)"
    Write-Host 'Assistant: read-only, 6 tools max, 60 second timeout'
    Write-Host 'Per-request reservation: USD 0.25'
}
catch {
    $failure = [ordered]@{
        success = $false
        failedAtUtc = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ss.fffZ')
        stage = $stage
        reason = $_.Exception.Message
        billableAiCallPerformed = $false
        productionMutationPerformed = $false
    }
    try {
        $outputDirectory = Split-Path -Parent $OutputPath
        New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null
        $failure | ConvertTo-Json | Set-Content -LiteralPath $OutputPath -Encoding utf8
    }
    catch {
        Write-Warning 'Could not write the STEP 11 failure result.'
    }
    throw "STEP 11 production non-billable verification failed at ${stage}: $($_.Exception.Message)"
}
finally {
    if ($null -ne $client) { $client.Dispose() }
    $token = $null
    $credential = $null
    $vault = $null
}

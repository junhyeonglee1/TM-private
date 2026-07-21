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
    $OutputPath = Join-Path $tmRoot 'dist\manual-step12-production-verification\result.json'
}
$OutputPath = [System.IO.Path]::GetFullPath($OutputPath)

function Invoke-TmRequest {
    param(
        [Parameter(Mandatory = $true)][System.Net.Http.HttpClient]$Client,
        [Parameter(Mandatory = $true)][string]$Method,
        [Parameter(Mandatory = $true)][string]$Uri,
        [Parameter(Mandatory = $true)][string]$Token,
        [string]$JsonBody
    )

    $request = $null
    $response = $null
    $content = $null
    try {
        $request = [System.Net.Http.HttpRequestMessage]::new(
            [System.Net.Http.HttpMethod]::new($Method),
            $Uri
        )
        $request.Headers.Authorization = [System.Net.Http.Headers.AuthenticationHeaderValue]::new('Bearer', $Token)
        if ($PSBoundParameters.ContainsKey('JsonBody')) {
            $content = [System.Net.Http.StringContent]::new($JsonBody, [System.Text.Encoding]::UTF8, 'application/json')
            $request.Content = $content
        }
        $response = $Client.SendAsync($request).GetAwaiter().GetResult()
        [pscustomobject]@{
            StatusCode = [int]$response.StatusCode
            Body = $response.Content.ReadAsStringAsync().GetAwaiter().GetResult()
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
    if ($auth.StatusCode -ne 200) { throw "Authentication returned HTTP $($auth.StatusCode)." }

    $stage = 'operations-status'
    $ops = Invoke-TmRequest -Client $client -Method 'GET' -Uri "$base/api/v1/ops/status" -Token $token
    if ($ops.StatusCode -ne 200) { throw "Operations status returned HTTP $($ops.StatusCode)." }
    $opsJson = $ops.Body | ConvertFrom-Json
    if ([bool]$opsJson.data.database.ok -ne $true -or [int]$opsJson.data.database.schemaVersion -ne 6) {
        throw 'Production database is not healthy on schema 6.'
    }

    $stage = 'ai-status'
    $ai = Invoke-TmRequest -Client $client -Method 'GET' -Uri "$base/api/v1/ai/status" -Token $token
    if ($ai.StatusCode -ne 200) { throw "AI status returned HTTP $($ai.StatusCode)." }
    $aiJson = $ai.Body | ConvertFrom-Json
    if ([string]$aiJson.data.responseStorage -ne 'disabled' -or
        [bool]$aiJson.data.assistantReadOnly -ne $false -or
        [string]$aiJson.data.assistantPromptVersion -ne 'step12-v1' -or
        [int]$aiJson.data.assistantAutomaticReadToolCount -ne 7 -or
        [bool]$aiJson.data.assistantTaskCreateApprovalEnabled -ne $true -or
        [int]$aiJson.data.assistantActionApprovalTtlSeconds -ne 600 -or
        [bool]$aiJson.data.assistantAutoExecuteWithoutApproval -ne $false -or
        [bool]$aiJson.data.assistantApprovalExecutesImmediately -ne $true -or
        [int64]$aiJson.data.budget.warningLimitMicrousd -ne 10000000 -or
        [int64]$aiJson.data.budget.hardLimitMicrousd -ne 20000000) {
        throw 'The STEP 12 AI safety contract does not match the approved defaults.'
    }

    $stage = 'action-list'
    $actions = Invoke-TmRequest -Client $client -Method 'GET' -Uri "$base/api/v1/assistant/actions" -Token $token
    if ($actions.StatusCode -ne 200) { throw "Action list returned HTTP $($actions.StatusCode)." }
    $actionsJson = $actions.Body | ConvertFrom-Json
    if ($null -eq $actionsJson.data.items) { throw 'Action list response is missing items.' }

    $stage = 'approval-confirmation-guard'
    $guardId = [Guid]::NewGuid().ToString()
    $guardBody = '{"expectedRevision":1,"payloadSha256":"0000000000000000000000000000000000000000000000000000000000000000"}'
    $guard = Invoke-TmRequest -Client $client -Method 'POST' -Uri "$base/api/v1/assistant/actions/$guardId/approve" -Token $token -JsonBody $guardBody
    if ($guard.StatusCode -ne 428) {
        throw "Unconfirmed action approval returned HTTP $($guard.StatusCode), expected 428."
    }
    $guardJson = $guard.Body | ConvertFrom-Json
    if ([string]$guardJson.error.code -ne 'ASSISTANT_ACTION_PRECONDITION_REQUIRED' -and
        [string]$guardJson.error.code -ne 'ASSISTANT_ACTION_CONFIRMATION_REQUIRED') {
        throw 'The approval confirmation guard returned an unexpected error code.'
    }

    $result = [ordered]@{
        success = $true
        verifiedAtUtc = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ss.fffZ')
        baseUri = $base
        schemaVersion = [int]$opsJson.data.database.schemaVersion
        promptVersion = [string]$aiJson.data.assistantPromptVersion
        automaticReadToolCount = [int]$aiJson.data.assistantAutomaticReadToolCount
        taskCreateApprovalEnabled = [bool]$aiJson.data.assistantTaskCreateApprovalEnabled
        approvalTtlSeconds = [int]$aiJson.data.assistantActionApprovalTtlSeconds
        autoExecuteWithoutApproval = [bool]$aiJson.data.assistantAutoExecuteWithoutApproval
        approvalExecutesImmediately = [bool]$aiJson.data.assistantApprovalExecutesImmediately
        existingActionCount = @($actionsJson.data.items).Count
        approvalGuardStatus = $guard.StatusCode
        billableAiCallPerformed = $false
        productionMutationPerformed = $false
        tokenReadFromCredentialLocker = $true
        tokenPersistedByScript = $false
    }
    $outputDirectory = Split-Path -Parent $OutputPath
    New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null
    $result | ConvertTo-Json | Set-Content -LiteralPath $OutputPath -Encoding utf8

    Write-Host ''
    Write-Host 'STEP 12 production non-billable verification passed.' -ForegroundColor Green
    Write-Host "Schema: $($result.schemaVersion)"
    Write-Host "Prompt: $($result.promptVersion)"
    Write-Host 'No OpenAI call or production mutation was performed.'
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
        Write-Warning 'Could not write the STEP 12 failure result.'
    }
    throw "STEP 12 production verification failed at ${stage}: $($_.Exception.Message)"
}
finally {
    if ($null -ne $client) { $client.Dispose() }
    $token = $null
    $credential = $null
    $vault = $null
}

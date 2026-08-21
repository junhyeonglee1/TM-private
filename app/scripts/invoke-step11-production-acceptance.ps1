[CmdletBinding()]
param(
    [ValidatePattern('^https://')]
    [string]$BaseUri = 'https://tm-server-production-5573.up.railway.app',

    [string]$Message = '현재 TM에서 아직 완료되지 않은 작업을 최대 3개로 요약하고, 다음 행동을 제안해줘.',

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
    $OutputPath = Join-Path $tmRoot 'dist\manual-step11-production-acceptance\result.json'
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

function Get-BusinessDataHash {
    param(
        [Parameter(Mandatory = $true)][System.Net.Http.HttpClient]$Client,
        [Parameter(Mandatory = $true)][string]$Base,
        [Parameter(Mandatory = $true)][string]$Token
    )

    $snapshot = [ordered]@{}
    foreach ($route in @('projects', 'tasks', 'notes', 'sessions', 'worklogs')) {
        $response = Invoke-TmRequest -Client $Client -Method 'GET' -Uri "$Base/api/v1/$route" -Token $Token
        if ($response.StatusCode -ne 200) {
            throw "Business data snapshot failed for $route with HTTP $($response.StatusCode)."
        }
        $parsed = $response.Body | ConvertFrom-Json
        $snapshot[$route] = $parsed.data
    }

    $checklists = [ordered]@{}
    foreach ($task in @($snapshot.tasks.items | Sort-Object id)) {
        $taskId = [string]$task.id
        $response = Invoke-TmRequest -Client $Client -Method 'GET' -Uri "$Base/api/v1/checklist?taskId=$taskId" -Token $Token
        if ($response.StatusCode -ne 200) {
            throw "Business data snapshot failed for checklist task $taskId with HTTP $($response.StatusCode)."
        }
        $parsed = $response.Body | ConvertFrom-Json
        $checklists[$taskId] = $parsed.data
    }
    $snapshot['checklistByTask'] = $checklists
    $json = $snapshot | ConvertTo-Json -Compress -Depth 100
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($json)
    $sha256 = [System.Security.Cryptography.SHA256]::Create()
    try {
        return ([BitConverter]::ToString($sha256.ComputeHash($bytes))).Replace('-', '').ToLowerInvariant()
    }
    finally {
        $sha256.Dispose()
    }
}

$vault = $null
$credential = $null
$token = $null
$client = $null
$stage = 'credential-locker'
$billableCallStarted = $false

try {
    $uri = [System.Uri]$BaseUri
    if (-not $uri.IsAbsoluteUri -or $uri.Scheme -ne 'https' -or
        -not [string]::IsNullOrEmpty($uri.UserInfo) -or
        -not [string]::IsNullOrEmpty($uri.Query) -or
        -not [string]::IsNullOrEmpty($uri.Fragment)) {
        throw 'BaseUri must be a credential-free HTTPS origin.'
    }
    $base = $uri.GetLeftPart([System.UriPartial]::Authority)

    if ([string]::IsNullOrWhiteSpace($Message) -or [System.Text.Encoding]::UTF8.GetByteCount($Message) -gt 8192) {
        throw 'Message must contain 1 to 8192 UTF-8 bytes.'
    }

    $vault = [Windows.Security.Credentials.PasswordVault,Windows.Security.Credentials,ContentType=WindowsRuntime]::new()
    $credential = $vault.Retrieve($resource, $userName)
    $credential.RetrievePassword()
    $token = [string]$credential.Password
    if ($token -notmatch '^tm_pat_v1_[A-Za-z0-9_-]{43}$') {
        throw 'The Credential Locker TM production token has an invalid format.'
    }

    $client = [System.Net.Http.HttpClient]::new()
    $client.Timeout = [TimeSpan]::FromSeconds(90)

    $stage = 'configured-status'
    $status = Invoke-TmRequest -Client $client -Method 'GET' -Uri "$base/api/v1/ai/status" -Token $token
    if ($status.StatusCode -ne 200) {
        throw "AI status failed with HTTP $($status.StatusCode)."
    }
    $statusJson = $status.Body | ConvertFrom-Json
    if ([bool]$statusJson.data.configured -ne $true -or
        [string]$statusJson.data.model -ne 'gpt-5.6-terra' -or
        [bool]$statusJson.data.assistantReadOnly -ne $true) {
        throw 'The configured STEP 11 production contract is not active.'
    }

    $stage = 'business-data-before'
    $beforeHash = Get-BusinessDataHash -Client $client -Base $base -Token $token

    $stage = 'openai-probe'
    $billableCallStarted = $true
    $probe = Invoke-TmRequest -Client $client -Method 'POST' -Uri "$base/api/v1/ai/probe" -Token $token -Confirmation 'probe'
    if ($probe.StatusCode -ne 200) {
        $errorCode = try { [string](($probe.Body | ConvertFrom-Json).error.code) } catch { 'UNKNOWN' }
        throw "OpenAI probe failed with HTTP $($probe.StatusCode) ($errorCode)."
    }
    $probeJson = $probe.Body | ConvertFrom-Json
    if ([bool]$probeJson.data.matchedExpectedText -ne $true -or
        [bool]$probeJson.data.stored -ne $false -or
        [int64]$probeJson.data.estimatedCostMicrousd -gt 10000) {
        throw 'OpenAI probe response violated the STEP 11 contract.'
    }

    $stage = 'assistant-query'
    $assistantBody = @{ message = $Message } | ConvertTo-Json -Compress
    $assistant = Invoke-TmRequest -Client $client -Method 'POST' -Uri "$base/api/v1/assistant/query" -Token $token -Confirmation 'assistant' -JsonBody $assistantBody
    if ($assistant.StatusCode -ne 200) {
        $errorCode = try { [string](($assistant.Body | ConvertFrom-Json).error.code) } catch { 'UNKNOWN' }
        throw "Assistant query failed with HTTP $($assistant.StatusCode) ($errorCode)."
    }
    $assistantJson = $assistant.Body | ConvertFrom-Json
    $allowedTools = @('list_projects', 'list_tasks', 'list_checklist', 'list_notes', 'list_sessions', 'list_worklogs', 'search_tm')
    $unexpectedTools = @($assistantJson.data.toolsUsed | Where-Object { $_ -notin $allowedTools })
    if ([bool]$assistantJson.data.readOnly -ne $true -or
        [bool]$assistantJson.data.stored -ne $false -or
        [string]$assistantJson.data.model -ne 'gpt-5.6-terra' -or
        [string]$assistantJson.data.promptVersion -ne 'step11-v1' -or
        [int]$assistantJson.data.toolCallCount -lt 1 -or
        [int]$assistantJson.data.toolCallCount -gt 6 -or
        $unexpectedTools.Count -ne 0 -or
        [int64]$assistantJson.data.estimatedCostMicrousd -gt 250000 -or
        [string]::IsNullOrWhiteSpace([string]$assistantJson.data.answer)) {
        throw 'Assistant response violated the STEP 11 read-only contract.'
    }

    $stage = 'business-data-after'
    $afterHash = Get-BusinessDataHash -Client $client -Base $base -Token $token
    if ($beforeHash -ne $afterHash) {
        throw 'TM business data changed during the read-only assistant acceptance test.'
    }

    $stage = 'complete'
    $result = [ordered]@{
        success = $true
        verifiedAtUtc = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ss.fffZ')
        baseUri = $base
        model = [string]$assistantJson.data.model
        promptVersion = [string]$assistantJson.data.promptVersion
        responseStorage = 'disabled'
        readOnly = [bool]$assistantJson.data.readOnly
        probeMatched = [bool]$probeJson.data.matchedExpectedText
        probeEstimatedCostMicrousd = [int64]$probeJson.data.estimatedCostMicrousd
        assistantEstimatedCostMicrousd = [int64]$assistantJson.data.estimatedCostMicrousd
        assistantInputTokens = [int64]$assistantJson.data.usage.inputTokens
        assistantCachedInputTokens = [int64]$assistantJson.data.usage.cachedInputTokens
        assistantOutputTokens = [int64]$assistantJson.data.usage.outputTokens
        assistantToolCallCount = [int]$assistantJson.data.toolCallCount
        assistantToolsUsed = @($assistantJson.data.toolsUsed)
        assistantAnswerSha256 = ([BitConverter]::ToString(([System.Security.Cryptography.SHA256]::Create()).ComputeHash([System.Text.Encoding]::UTF8.GetBytes([string]$assistantJson.data.answer)))).Replace('-', '').ToLowerInvariant()
        assistantAnswerLength = ([string]$assistantJson.data.answer).Length
        businessDataSha256Before = $beforeHash
        businessDataSha256After = $afterHash
        businessDataUnchanged = $true
        monthlyCommittedMicrousd = [int64]$assistantJson.data.budget.committedMicrousd
        monthlyRemainingMicrousd = [int64]$assistantJson.data.budget.remainingMicrousd
        tokenReadFromCredentialLocker = $true
        sensitiveAnswerPersisted = $false
    }

    $outputDirectory = Split-Path -Parent $OutputPath
    New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null
    $result | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $OutputPath -Encoding utf8

    Write-Host ''
    Write-Host 'STEP 11 production billable acceptance passed.' -ForegroundColor Green
    Write-Host "Probe estimated cost: USD $([math]::Round($result.probeEstimatedCostMicrousd / 1000000, 6))"
    Write-Host "Assistant estimated cost: USD $([math]::Round($result.assistantEstimatedCostMicrousd / 1000000, 6))"
    Write-Host "Tools: $($result.assistantToolsUsed -join ', ')"
    Write-Host 'TM business data unchanged: True'
    Write-Host ''
    Write-Host 'Assistant answer:' -ForegroundColor Cyan
    Write-Host ([string]$assistantJson.data.answer)
}
catch {
    $failure = [ordered]@{
        success = $false
        failedAtUtc = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ss.fffZ')
        stage = $stage
        reason = $_.Exception.Message
        billableCallMayHaveStarted = $billableCallStarted
        sensitiveAnswerPersisted = $false
    }
    try {
        $outputDirectory = Split-Path -Parent $OutputPath
        New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null
        $failure | ConvertTo-Json | Set-Content -LiteralPath $OutputPath -Encoding utf8
    }
    catch {
        Write-Warning 'Could not write the STEP 11 acceptance failure result.'
    }
    throw "STEP 11 production acceptance failed at ${stage}: $($_.Exception.Message)"
}
finally {
    if ($null -ne $client) { $client.Dispose() }
    $token = $null
    $credential = $null
    $vault = $null
}

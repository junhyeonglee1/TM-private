[CmdletBinding()]
param(
    [ValidatePattern('^https://')]
    [string]$BaseUri = 'https://tm-server-production-5573.up.railway.app',

    [switch]$ExecuteBillableCall,

    [string]$OutputPath
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
Add-Type -AssemblyName System.Net.Http
$expectedSchemaVersion = 15

$resource = 'TM Cloud Production'
$userName = 'single-user'
$tmRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
if ([string]::IsNullOrWhiteSpace($OutputPath)) {
    $OutputPath = Join-Path $tmRoot 'dist\manual-step17-production-verification\result.json'
}
$OutputPath = [System.IO.Path]::GetFullPath($OutputPath)

function Invoke-TmRequest {
    param(
        [Parameter(Mandatory = $true)][System.Net.Http.HttpClient]$Client,
        [Parameter(Mandatory = $true)][System.Net.Http.HttpMethod]$Method,
        [Parameter(Mandatory = $true)][string]$Uri,
        [Parameter(Mandatory = $true)][string]$Token,
        [hashtable]$Headers,
        [string]$JsonBody
    )
    $request = $null
    $response = $null
    try {
        $request = [System.Net.Http.HttpRequestMessage]::new($Method, $Uri)
        $request.Headers.Authorization = [System.Net.Http.Headers.AuthenticationHeaderValue]::new('Bearer', $Token)
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
        throw "$Stage returned HTTP $($Response.StatusCode), expected $Expected."
    }
}

function Data-Digest {
    param($Envelope)
    $json = $Envelope.data | ConvertTo-Json -Depth 40 -Compress
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($json)
    $sha = [System.Security.Cryptography.SHA256]::Create()
    try {
        return ([BitConverter]::ToString($sha.ComputeHash($bytes))).Replace('-', '').ToLowerInvariant()
    }
    finally { $sha.Dispose() }
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
    $client.Timeout = [TimeSpan]::FromSeconds(60)

    $stage = 'readiness'
    $ready = Invoke-TmRequest -Client $client -Method ([System.Net.Http.HttpMethod]::Get) -Uri "$base/readyz" -Token $token
    Require-Status $ready 200 $stage
    $readyJson = $ready.Body | ConvertFrom-Json
    if ([string]$readyJson.data.status -ne 'ready') { throw 'Production readiness status is not ready.' }

    $stage = 'operations-status'
    $ops = Invoke-TmRequest -Client $client -Method ([System.Net.Http.HttpMethod]::Get) -Uri "$base/api/v1/ops/status" -Token $token
    Require-Status $ops 200 $stage
    $opsJson = $ops.Body | ConvertFrom-Json
    if ([int]$opsJson.data.database.schemaVersion -ne $expectedSchemaVersion -or
        [string]$opsJson.data.remoteBackup.status -ne 'succeeded' -or
        [int]$opsJson.data.remoteBackup.schemaVersion -ne $expectedSchemaVersion -or
        [string]$opsJson.data.remoteBackup.integrityCheck -ne 'ok' -or
        [bool]$opsJson.data.remoteBackup.schemaSemanticsValidated -ne $true -or
        [bool]$opsJson.data.controls.taskReportEnabled -ne $true -or
        [bool]$opsJson.data.controls.aiEnabled -ne $true -or
        [string]$opsJson.data.controls.incidentMode -ne 'normal') {
        throw 'STEP 17 production schema, backup, or controls are not ready.'
    }

    $stage = 'ai-contract'
    $ai = Invoke-TmRequest -Client $client -Method ([System.Net.Http.HttpMethod]::Get) -Uri "$base/api/v1/ai/status" -Token $token
    Require-Status $ai 200 $stage
    $aiJson = $ai.Body | ConvertFrom-Json
    if ([bool]$aiJson.data.configured -ne $true -or
        [string]$aiJson.data.assistantPromptVersion -ne 'calendar-assistant-v1' -or
        [string]$aiJson.data.taskReportPromptVersion -ne 'calendar-task-report-v1' -or
        [int]$aiJson.data.taskReportDailyLimit -ne 4 -or
        [int64]$aiJson.data.taskReportMaximumCostMicrousd -ne 50000 -or
        [int64]$aiJson.data.budget.hardLimitMicrousd -ne 20000000) {
        throw 'STEP 17 AI safety contract does not match the approved defaults.'
    }

    $stage = 'business-snapshot-before'
    $tasksBefore = Invoke-TmRequest -Client $client -Method ([System.Net.Http.HttpMethod]::Get) -Uri "$base/api/v1/tasks?limit=100&offset=0&sort=updated_desc" -Token $token
    $notesBefore = Invoke-TmRequest -Client $client -Method ([System.Net.Http.HttpMethod]::Get) -Uri "$base/api/v1/notes?limit=100&offset=0&sort=updated_desc" -Token $token
    Require-Status $tasksBefore 200 "$stage-tasks"
    Require-Status $notesBefore 200 "$stage-notes"
    $tasksBeforeJson = $tasksBefore.Body | ConvertFrom-Json
    $notesBeforeJson = $notesBefore.Body | ConvertFrom-Json
    $taskDigestBefore = Data-Digest $tasksBeforeJson
    $noteDigestBefore = Data-Digest $notesBeforeJson

    $stage = 'confirmation-required'
    $unconfirmed = Invoke-TmRequest -Client $client -Method ([System.Net.Http.HttpMethod]::Post) -Uri "$base/api/v1/assistant/task-report" -Token $token
    Require-Status $unconfirmed 428 $stage
    $unconfirmedJson = $unconfirmed.Body | ConvertFrom-Json
    if ([string]$unconfirmedJson.error.code -ne 'AI_CALL_CONFIRMATION_REQUIRED') {
        throw 'Unconfirmed billable Task report request was not rejected correctly.'
    }

    if (-not $ExecuteBillableCall) {
        throw 'Preflight passed. Re-run with -ExecuteBillableCall to authorize exactly one production OpenAI Task report call.'
    }

    $stage = 'billable-task-report'
    $reportResponse = Invoke-TmRequest -Client $client -Method ([System.Net.Http.HttpMethod]::Post) -Uri "$base/api/v1/assistant/task-report" -Token $token -Headers @{ 'x-tm-confirm-ai-call' = 'task-report' }
    Require-Status $reportResponse 200 $stage
    $reportJson = $reportResponse.Body | ConvertFrom-Json
    $report = $reportJson.data
    if ([string]$report.promptVersion -ne 'calendar-task-report-v1' -or
        [bool]$report.readOnly -ne $true -or
        [int]$report.candidateCount -gt 20 -or
        [int64]$report.estimatedCostMicrousd -gt 50000 -or
        [int]$report.limits.dailyCalls -ne 4 -or
        [int]$report.limits.maximumOutputTokens -ne 800) {
        throw 'The returned Task report exceeded or changed an approved safety boundary.'
    }
    if ([string]::IsNullOrWhiteSpace([string]$report.report.headline) -or
        [string]::IsNullOrWhiteSpace([string]$report.report.summary)) {
        throw 'The Task report headline or summary is empty.'
    }
    $priorities = @($report.report.priorities)
    if ($priorities.Count -gt 3) { throw 'The Task report returned more than three priorities.' }
    $scheduleHighlights = @($report.report.scheduleHighlights)
    if ($scheduleHighlights.Count -gt 5) { throw 'The Task report returned more than five schedule highlights.' }
    $allowedTaskIds = @{}
    foreach ($task in @($tasksBeforeJson.data.items)) { $allowedTaskIds[[string]$task.id] = $true }
    foreach ($priority in $priorities) {
        if (-not $allowedTaskIds.ContainsKey([string]$priority.taskId)) {
            throw 'The Task report returned a Task ID outside the production Task snapshot.'
        }
    }

    $stage = 'latest-report'
    $latestResponse = Invoke-TmRequest -Client $client -Method ([System.Net.Http.HttpMethod]::Get) -Uri "$base/api/v1/assistant/task-reports/latest" -Token $token
    Require-Status $latestResponse 200 $stage
    $latestJson = $latestResponse.Body | ConvertFrom-Json
    if ([string]$latestJson.data.runId -ne [string]$report.runId) {
        throw 'The latest Task report does not match the completed production run.'
    }

    $stage = 'business-snapshot-after'
    $tasksAfter = Invoke-TmRequest -Client $client -Method ([System.Net.Http.HttpMethod]::Get) -Uri "$base/api/v1/tasks?limit=100&offset=0&sort=updated_desc" -Token $token
    $notesAfter = Invoke-TmRequest -Client $client -Method ([System.Net.Http.HttpMethod]::Get) -Uri "$base/api/v1/notes?limit=100&offset=0&sort=updated_desc" -Token $token
    Require-Status $tasksAfter 200 "$stage-tasks"
    Require-Status $notesAfter 200 "$stage-notes"
    $taskDigestAfter = Data-Digest ($tasksAfter.Body | ConvertFrom-Json)
    $noteDigestAfter = Data-Digest ($notesAfter.Body | ConvertFrom-Json)
    if ($taskDigestAfter -ne $taskDigestBefore -or $noteDigestAfter -ne $noteDigestBefore) {
        throw 'Task or Note business data changed during the read-only Task report call.'
    }

    $result = [ordered]@{
        verifiedAtUtc = [DateTime]::UtcNow.ToString('o')
        baseUri = $base
        schemaVersion = $expectedSchemaVersion
        runId = [string]$report.runId
        status = [string]$report.status
        headline = [string]$report.report.headline
        candidateCount = [int]$report.candidateCount
        priorityCount = $priorities.Count
        scheduleHighlightCount = $scheduleHighlights.Count
        model = [string]$report.model
        promptVersion = [string]$report.promptVersion
        inputTokens = if ($null -eq $report.usage) { 0 } else { [int64]$report.usage.inputTokens }
        cachedInputTokens = if ($null -eq $report.usage) { 0 } else { [int64]$report.usage.cachedInputTokens }
        outputTokens = if ($null -eq $report.usage) { 0 } else { [int64]$report.usage.outputTokens }
        totalTokens = if ($null -eq $report.usage) { 0 } else { [int64]$report.usage.totalTokens }
        estimatedCostMicrousd = [int64]$report.estimatedCostMicrousd
        latencyMs = [int64]$report.latencyMs
        taskDataUnchanged = $true
        noteDataUnchanged = $true
        feedbackRecorded = $false
    }
    $directory = Split-Path -Parent $OutputPath
    [System.IO.Directory]::CreateDirectory($directory) | Out-Null
    $json = $result | ConvertTo-Json -Depth 10
    [System.IO.File]::WriteAllText($OutputPath, $json, [System.Text.UTF8Encoding]::new($false))
    Write-Host 'STEP 17 production Task report verification passed.' -ForegroundColor Green
    Write-Host "Run ID: $($result.runId)"
    Write-Host "Tokens: $($result.totalTokens), estimated cost: $($result.estimatedCostMicrousd) microUSD, latency: $($result.latencyMs) ms"
    Write-Host "Result: $OutputPath"
}
catch {
    throw "STEP 17 production verification failed at $stage. $($_.Exception.Message)"
}
finally {
    $token = $null
    if ($null -ne $client) { $client.Dispose() }
    if ($null -ne $handler) { $handler.Dispose() }
    $credential = $null
    $vault = $null
}

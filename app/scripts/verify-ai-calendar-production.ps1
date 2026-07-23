[CmdletBinding()]
param(
    [ValidatePattern('^https://')]
    [string]$BaseUri = 'https://tm-server-production-5573.up.railway.app'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
Add-Type -AssemblyName System.Net.Http

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
        $errorCode = try { [string](($Response.Body | ConvertFrom-Json).error.code) } catch { 'UNKNOWN' }
        throw "$Stage returned HTTP $($Response.StatusCode), expected $Expected ($errorCode)."
    }
}

function Invoke-CalendarCommand {
    param(
        [Parameter(Mandatory = $true)][System.Net.Http.HttpClient]$Client,
        [Parameter(Mandatory = $true)][string]$Base,
        [Parameter(Mandatory = $true)][string]$Token,
        [Parameter(Mandatory = $true)][string]$CommandName,
        [Parameter(Mandatory = $true)][hashtable]$CommandArguments
    )

    $body = @{ args = $CommandArguments } | ConvertTo-Json -Depth 12 -Compress
    $response = Invoke-TmRequest -Client $Client -Method ([System.Net.Http.HttpMethod]::Post) `
        -Uri "$Base/api/v1/desktop/commands/$CommandName" -Token $Token `
        -Headers @{ 'x-tm-confirm-desktop-command' = $CommandName } -JsonBody $body
    Require-Status $response 200 $CommandName
    return $response.Body | ConvertFrom-Json
}

$uri = [System.Uri]$BaseUri
if (-not $uri.IsAbsoluteUri -or $uri.Scheme -ne 'https' -or
    -not [string]::IsNullOrEmpty($uri.UserInfo) -or
    -not [string]::IsNullOrEmpty($uri.Query) -or
    -not [string]::IsNullOrEmpty($uri.Fragment)) {
    throw 'BaseUri must be a credential-free HTTPS origin.'
}
$base = $uri.GetLeftPart([System.UriPartial]::Authority)

$vault = [Windows.Security.Credentials.PasswordVault,Windows.Security.Credentials,ContentType=WindowsRuntime]::new()
$credential = $vault.Retrieve('TM Cloud Production', 'single-user')
$credential.RetrievePassword()
$token = [string]$credential.Password
if ($token -notmatch '^tm_pat_v1_[A-Za-z0-9_-]{43}$') {
    throw 'The Credential Locker TM production token has an invalid format.'
}

$handler = [System.Net.Http.HttpClientHandler]::new()
$handler.AllowAutoRedirect = $false
$handler.UseCookies = $false
$client = [System.Net.Http.HttpClient]::new($handler)
$client.Timeout = [TimeSpan]::FromSeconds(90)
$createdEvent = $null
$stamp = [DateTime]::UtcNow.ToString('yyyyMMddHHmmss')
$title = "TM AI calendar verification $stamp"
$privateMarker = "TM_AI_PRIVATE_MEMO_$stamp"
$today = [TimeZoneInfo]::ConvertTimeBySystemTimeZoneId(
    [DateTimeOffset]::UtcNow,
    'Korea Standard Time'
).ToString('yyyy-MM-dd')

try {
    $costBeforeResponse = Invoke-TmRequest -Client $client -Method ([System.Net.Http.HttpMethod]::Get) `
        -Uri "$base/api/v1/costs/status" -Token $token
    Require-Status $costBeforeResponse 200 'cost-before'
    $costBefore = ($costBeforeResponse.Body | ConvertFrom-Json).data.api

    $created = Invoke-CalendarCommand -Client $client -Base $base -Token $token `
        -CommandName 'create_calendar_event' -CommandArguments @{
            input = @{
                title = $title
                description = $privateMarker
                kind = 'payment'
                startDate = $today
                eventTime = $null
                recurrence = 'none'
                dayOfMonth = $null
                endsOn = $null
            }
        }
    $createdEvent = $created.data
    if ([string]::IsNullOrWhiteSpace([string]$createdEvent.id)) {
        throw 'Temporary calendar event did not return an ID.'
    }

    # Keep the verification message ASCII-only so Windows PowerShell 5.1 cannot
    # reinterpret a BOM-less UTF-8 script before the JSON body is encoded.
    $assistantBody = @{ message = "Use the TM calendar read tool. Report today's schedule and payment dates in Korean, based only on TM data." } |
        ConvertTo-Json -Compress
    $assistantResponse = Invoke-TmRequest -Client $client -Method ([System.Net.Http.HttpMethod]::Post) `
        -Uri "$base/api/v1/assistant/query" -Token $token `
        -Headers @{ 'x-tm-confirm-ai-call' = 'assistant' } -JsonBody $assistantBody
    Require-Status $assistantResponse 200 'assistant-query'
    $assistant = ($assistantResponse.Body | ConvertFrom-Json).data
    $assistantChecks = [ordered]@{
        promptVersion = [string]$assistant.promptVersion -eq 'calendar-assistant-v1'
        calendarToolUsed = 'list_calendar_occurrences' -in @($assistant.toolsUsed)
        noActionProposed = @($assistant.proposedActions).Count -eq 0
        titleSeen = [string]$assistant.answer -like "*$title*"
        privateMemoOmitted = [string]$assistant.answer -notlike "*$privateMarker*"
    }
    $failedAssistantChecks = @($assistantChecks.GetEnumerator() | Where-Object { -not $_.Value } | ForEach-Object Key)
    if ($failedAssistantChecks.Count -ne 0) {
        throw "General assistant calendar contract failed: $($failedAssistantChecks -join ', ')."
    }

    $reportResponse = Invoke-TmRequest -Client $client -Method ([System.Net.Http.HttpMethod]::Post) `
        -Uri "$base/api/v1/assistant/task-report" -Token $token `
        -Headers @{ 'x-tm-confirm-ai-call' = 'task-report' }
    Require-Status $reportResponse 200 'task-report'
    $report = ($reportResponse.Body | ConvertFrom-Json).data
    $matchingHighlights = @($report.report.scheduleHighlights | Where-Object {
        [string]$_.eventId -eq [string]$createdEvent.id -and
        [string]$_.title -eq $title -and
        [string]$_.kind -eq 'payment' -and
        [string]$_.date -eq $today
    })
    $reportText = $report.report | ConvertTo-Json -Depth 10 -Compress
    if ([string]$report.promptVersion -ne 'calendar-task-report-v1' -or
        [bool]$report.readOnly -ne $true -or
        [int]$report.candidateCount -gt 20 -or
        [int64]$report.estimatedCostMicrousd -gt 50000 -or
        $matchingHighlights.Count -ne 1 -or
        $reportText -like "*$privateMarker*") {
        throw 'Today report did not satisfy the calendar structure/privacy/cost contract.'
    }

    $costAfterResponse = Invoke-TmRequest -Client $client -Method ([System.Net.Http.HttpMethod]::Get) `
        -Uri "$base/api/v1/costs/status" -Token $token
    Require-Status $costAfterResponse 200 'cost-after'
    $costAfter = ($costAfterResponse.Body | ConvertFrom-Json).data.api

    [pscustomobject]@{
        success = $true
        promptVersions = @($assistant.promptVersion, $report.promptVersion)
        assistantToolsUsed = @($assistant.toolsUsed)
        assistantProposedActionCount = @($assistant.proposedActions).Count
        calendarToolReadOnly = $true
        assistantCalendarTitleSeen = $true
        assistantPrivateMemoOmitted = $true
        reportScheduleHighlightSeen = $true
        reportPrivateMemoOmitted = $true
        assistantEstimatedCostMicrousd = [int64]$assistant.estimatedCostMicrousd
        reportEstimatedCostMicrousd = [int64]$report.estimatedCostMicrousd
        monthlyApiUsedBeforeMicrousd = [int64]$costBefore.usedMicrousd
        monthlyApiUsedAfterMicrousd = [int64]$costAfter.usedMicrousd
    } | ConvertTo-Json -Depth 5 -Compress
}
finally {
    if ($null -ne $createdEvent -and -not [string]::IsNullOrWhiteSpace([string]$createdEvent.id)) {
        try {
            $null = Invoke-CalendarCommand -Client $client -Base $base -Token $token `
                -CommandName 'delete_calendar_event' -CommandArguments @{
                    eventId = [string]$createdEvent.id
                    expectedVersion = [int64]$createdEvent.version
                }
        }
        catch {
            Write-Warning 'Temporary calendar cleanup failed; inspect the verification event manually.'
            throw
        }
    }
    if ($null -ne $client) { $client.Dispose() }
    if ($null -ne $handler) { $handler.Dispose() }
    $token = $null
}

[CmdletBinding()]
param(
    [ValidatePattern('^https://')]
    [string]$BaseUri = 'https://tm-server-staging-staging.up.railway.app',

    [string]$OutputPath,

    [Security.SecureString]$ProvidedSecureToken
)

$ErrorActionPreference = 'Stop'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
Add-Type -AssemblyName System.Net.Http

if ([string]::IsNullOrWhiteSpace($OutputPath)) {
    $OutputPath = Join-Path $PSScriptRoot '..\dist\manual-step9-staging-verification\result.json'
}

function Invoke-TmRequest {
    param(
        [Parameter(Mandatory = $true)] [System.Net.Http.HttpClient]$Client,
        [Parameter(Mandatory = $true)] [string]$Method,
        [Parameter(Mandatory = $true)] [string]$Uri,
        [Parameter(Mandatory = $true)] [string]$Token,
        [hashtable]$Headers = @{},
        [string]$JsonBody
    )

    $request = New-Object System.Net.Http.HttpRequestMessage(
        (New-Object System.Net.Http.HttpMethod($Method)),
        $Uri
    )
    $response = $null
    try {
        $request.Headers.Authorization = New-Object System.Net.Http.Headers.AuthenticationHeaderValue('Bearer', $Token)
        foreach ($name in $Headers.Keys) {
            [void]$request.Headers.TryAddWithoutValidation($name, [string]$Headers[$name])
        }
        if ($PSBoundParameters.ContainsKey('JsonBody')) {
            $request.Content = New-Object System.Net.Http.StringContent(
                $JsonBody,
                [Text.Encoding]::UTF8,
                'application/json'
            )
        }

        $response = $Client.SendAsync($request).GetAwaiter().GetResult()
        $body = $response.Content.ReadAsStringAsync().GetAwaiter().GetResult()
        $etag = if ($null -ne $response.Headers.ETag) { $response.Headers.ETag.ToString() } else { $null }
        $replayedValues = $null
        $hasReplayed = $response.Headers.TryGetValues('X-TM-Idempotency-Replayed', [ref]$replayedValues)
        $replayed = $hasReplayed -and (@($replayedValues) -contains 'true')
        return [pscustomobject]@{
            StatusCode = [int]$response.StatusCode
            Body = $body
            ETag = $etag
            Replayed = $replayed
        }
    }
    finally {
        if ($null -ne $response) { $response.Dispose() }
        $request.Dispose()
    }
}

function Assert-Status {
    param(
        [Parameter(Mandatory = $true)] [object]$Response,
        [Parameter(Mandatory = $true)] [int]$Expected,
        [Parameter(Mandatory = $true)] [string]$Name
    )
    if ($Response.StatusCode -ne $Expected) {
        throw "$Name failed: HTTP $($Response.StatusCode), expected $Expected. Body: $($Response.Body)"
    }
}

$secureToken = if ($null -ne $ProvidedSecureToken) {
    $ProvidedSecureToken
}
else {
    Read-Host 'Paste TM Railway staging device token' -AsSecureString
}
$token = [System.Net.NetworkCredential]::new('', $secureToken).Password
$client = New-Object System.Net.Http.HttpClient
$client.Timeout = [TimeSpan]::FromSeconds(30)
$stage = 'token-input'
$successfulMutationPerformed = $false

try {
    if ([string]::IsNullOrWhiteSpace($token)) {
        throw 'The staging token was not entered.'
    }

    $base = $BaseUri.TrimEnd('/')
    $stage = 'authentication'
    $auth = Invoke-TmRequest -Client $client -Method 'GET' -Uri "$base/api/v1/auth/status" -Token $token
    Assert-Status -Response $auth -Expected 200 -Name 'Authentication'
    $authJson = $auth.Body | ConvertFrom-Json
    if ($authJson.data.authenticated -ne $true) {
        throw 'The server did not confirm authentication.'
    }

    $stage = 'operations-status'
    $ops = Invoke-TmRequest -Client $client -Method 'GET' -Uri "$base/api/v1/ops/status" -Token $token
    Assert-Status -Response $ops -Expected 200 -Name 'Operations status'
    $opsJson = $ops.Body | ConvertFrom-Json
    if ($opsJson.data.database.ok -ne $true -or [int]$opsJson.data.database.schemaVersion -ne 5) {
        throw 'The staging database health or schema version is unexpected.'
    }

    $stage = 'baseline-read'
    $tasksBefore = Invoke-TmRequest -Client $client -Method 'GET' -Uri "$base/api/v1/tasks" -Token $token
    Assert-Status -Response $tasksBefore -Expected 200 -Name 'Tasks baseline'
    $tasksBeforeJson = $tasksBefore.Body | ConvertFrom-Json
    $totalBefore = [int]$tasksBeforeJson.data.page.total

    $keyPrefix = [Guid]::NewGuid().ToString('N')
    $title = "STEP 9 staging smoke $keyPrefix"
    $createBody = @{ title = $title; status = 'todo' } | ConvertTo-Json -Compress
    $createHeaders = @{
        'Idempotency-Key' = "$keyPrefix-task-create"
        'If-None-Match' = '*'
        'X-TM-Confirm-Mutation' = 'task.create'
    }

    $stage = 'task-create'
    $created = Invoke-TmRequest -Client $client -Method 'POST' -Uri "$base/api/v1/tasks" `
        -Token $token -Headers $createHeaders -JsonBody $createBody
    Assert-Status -Response $created -Expected 201 -Name 'Task create'
    $createdJson = $created.Body | ConvertFrom-Json
    $taskId = [string]$createdJson.data.resourceId
    if ([string]::IsNullOrWhiteSpace($taskId) -or [int]$createdJson.data.version -ne 1) {
        throw 'Task create response is missing the resource ID or version 1.'
    }
    $successfulMutationPerformed = $true

    $stage = 'idempotency-replay'
    $replay = Invoke-TmRequest -Client $client -Method 'POST' -Uri "$base/api/v1/tasks" `
        -Token $token -Headers $createHeaders -JsonBody $createBody
    Assert-Status -Response $replay -Expected 201 -Name 'Idempotency replay'
    $replayJson = $replay.Body | ConvertFrom-Json
    if ($replay.Replayed -ne $true -or $replayJson.data.replayed -ne $true) {
        throw 'The repeated create request was not identified as an idempotent replay.'
    }

    $stage = 'task-update'
    $updatedTitle = "$title updated"
    $updateBody = @{ title = $updatedTitle } | ConvertTo-Json -Compress
    $updated = Invoke-TmRequest -Client $client -Method 'PATCH' -Uri "$base/api/v1/tasks/$taskId" `
        -Token $token -Headers @{
            'Idempotency-Key' = "$keyPrefix-task-update"
            'If-Match' = '"1"'
            'X-TM-Confirm-Mutation' = 'task.update'
        } -JsonBody $updateBody
    Assert-Status -Response $updated -Expected 200 -Name 'Task update'
    $updatedJson = $updated.Body | ConvertFrom-Json
    if ([int]$updatedJson.data.version -ne 2 -or $updated.ETag -ne '"2"') {
        throw 'The successful update did not return version 2.'
    }

    $stage = 'stale-version-conflict'
    $stale = Invoke-TmRequest -Client $client -Method 'PATCH' -Uri "$base/api/v1/tasks/$taskId" `
        -Token $token -Headers @{
            'Idempotency-Key' = "$keyPrefix-task-stale"
            'If-Match' = '"1"'
            'X-TM-Confirm-Mutation' = 'task.update'
        } -JsonBody '{"title":"stale overwrite must fail"}'
    Assert-Status -Response $stale -Expected 409 -Name 'Stale version conflict'
    $staleJson = $stale.Body | ConvertFrom-Json
    if ($staleJson.error.code -ne 'MUTATION_CONFLICT') {
        throw "Unexpected stale conflict code: $($staleJson.error.code)"
    }

    $stage = 'final-read'
    $tasksAfter = Invoke-TmRequest -Client $client -Method 'GET' -Uri "$base/api/v1/tasks" -Token $token
    Assert-Status -Response $tasksAfter -Expected 200 -Name 'Tasks final read'
    $tasksAfterJson = $tasksAfter.Body | ConvertFrom-Json
    $totalAfter = [int]$tasksAfterJson.data.page.total
    if ($totalAfter -ne ($totalBefore + 1)) {
        throw "Expected exactly one new staging task; before=$totalBefore after=$totalAfter"
    }

    $stage = 'complete'
    $result = [ordered]@{
        success = $true
        verifiedAtUtc = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ss.fffZ')
        baseUri = $base
        authenticated = $true
        schemaVersion = [int]$opsJson.data.database.schemaVersion
        remoteBackupStatus = [string]$opsJson.data.remoteBackup.status
        createdTaskId = $taskId
        createStatus = $created.StatusCode
        replayStatus = $replay.StatusCode
        replayConfirmed = $true
        updateStatus = $updated.StatusCode
        updatedVersion = 2
        staleUpdateStatus = $stale.StatusCode
        staleUpdateCode = [string]$staleJson.error.code
        tasksTotalBefore = $totalBefore
        tasksTotalAfter = $totalAfter
        tokenPersisted = $false
    }

    $outputDirectory = Split-Path -Parent $OutputPath
    New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null
    $result | ConvertTo-Json | Set-Content -LiteralPath $OutputPath -Encoding utf8

    Write-Host ''
    Write-Host 'STEP 9 staging verification succeeded.' -ForegroundColor Green
    Write-Host 'Authentication / schema 5 / mutation / replay / version conflict: PASS'
}
catch {
    $failure = [ordered]@{
        success = $false
        verifiedAtUtc = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ss.fffZ')
        baseUri = $BaseUri.TrimEnd('/')
        failedStage = $stage
        message = $_.Exception.Message
        successfulMutationPerformed = $successfulMutationPerformed
        tokenPersisted = $false
    }
    $outputDirectory = Split-Path -Parent $OutputPath
    New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null
    $failure | ConvertTo-Json | Set-Content -LiteralPath $OutputPath -Encoding utf8

    Write-Host ''
    Write-Host 'STEP 9 staging verification failed.' -ForegroundColor Red
    Write-Host "Failed stage: $stage"
    Write-Host $_.Exception.Message
}
finally {
    $client.Dispose()
    $token = $null
    $secureToken = $null
    $ProvidedSecureToken = $null
    Remove-Variable token, secureToken, ProvidedSecureToken, auth, authJson, ops, opsJson, `
        tasksBefore, tasksBeforeJson, created, createdJson, replay, replayJson, updated, `
        updatedJson, stale, staleJson, tasksAfter, tasksAfterJson -ErrorAction SilentlyContinue
    [void](Read-Host 'Press Enter to close')
}

[CmdletBinding()]
param(
    [ValidatePattern('^https://')]
    [string]$BaseUri = 'https://tm-server-production-5573.up.railway.app',

    [string]$OutputPath,

    [Security.SecureString]$ProvidedSecureToken
)

$ErrorActionPreference = 'Stop'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
Add-Type -AssemblyName System.Net.Http

if ([string]::IsNullOrWhiteSpace($OutputPath)) {
    $OutputPath = Join-Path $PSScriptRoot '..\dist\manual-step8-production-verification\result.json'
}

function Invoke-TmRequest {
    param(
        [Parameter(Mandatory = $true)]
        [System.Net.Http.HttpClient]$Client,

        [Parameter(Mandatory = $true)]
        [string]$Method,

        [Parameter(Mandatory = $true)]
        [string]$Uri,

        [Parameter(Mandatory = $true)]
        [string]$Token,

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
        $cacheControl = if ($null -ne $response.Headers.CacheControl) {
            $response.Headers.CacheControl.ToString()
        }
        else {
            $null
        }
        $hstsValues = $null
        $hasHsts = $response.Headers.TryGetValues('Strict-Transport-Security', [ref]$hstsValues)

        return [pscustomobject]@{
            StatusCode = [int]$response.StatusCode
            Body = $body
            ETag = $etag
            CacheControl = $cacheControl
            HasHsts = $hasHsts
        }
    }
    finally {
        if ($null -ne $response) { $response.Dispose() }
        $request.Dispose()
    }
}

function Assert-TmError {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Response,

        [Parameter(Mandatory = $true)]
        [int]$ExpectedStatus,

        [Parameter(Mandatory = $true)]
        [string]$ExpectedCode,

        [Parameter(Mandatory = $true)]
        [string]$CheckName
    )

    if ($Response.StatusCode -ne $ExpectedStatus) {
        throw "$CheckName 실패: HTTP $($Response.StatusCode), 예상 $ExpectedStatus"
    }
    $json = $Response.Body | ConvertFrom-Json
    if ($json.error.code -ne $ExpectedCode) {
        throw "$CheckName 오류 계약 불일치: $($json.error.code), 예상 $ExpectedCode"
    }
}

$secureToken = if ($null -ne $ProvidedSecureToken) {
    $ProvidedSecureToken
}
else {
    Read-Host 'TM 인증 토큰 입력' -AsSecureString
}
$token = [System.Net.NetworkCredential]::new('', $secureToken).Password
$client = New-Object System.Net.Http.HttpClient
$client.Timeout = [TimeSpan]::FromSeconds(20)
$stage = 'token-input'

try {
    if ([string]::IsNullOrWhiteSpace($token)) {
        throw '인증 토큰이 입력되지 않았습니다.'
    }

    $base = $BaseUri.TrimEnd('/')
    $stage = 'authentication'
    $auth = Invoke-TmRequest -Client $client -Method 'GET' `
        -Uri "$base/api/v1/auth/status" -Token $token
    if ($auth.StatusCode -ne 200) {
        throw "인증 확인 실패: HTTP $($auth.StatusCode)"
    }
    $authJson = $auth.Body | ConvertFrom-Json
    if ($authJson.data.authenticated -ne $true) {
        throw '서버가 예상한 인증 결과를 반환하지 않았습니다.'
    }

    $stage = 'baseline-read'
    $tasksBefore = Invoke-TmRequest -Client $client -Method 'GET' `
        -Uri "$base/api/v1/tasks" -Token $token
    $notesBefore = Invoke-TmRequest -Client $client -Method 'GET' `
        -Uri "$base/api/v1/notes" -Token $token
    if ($tasksBefore.StatusCode -ne 200 -or $notesBefore.StatusCode -ne 200) {
        throw 'mutation 전 read 기준선 조회에 실패했습니다.'
    }
    $tasksBeforeJson = $tasksBefore.Body | ConvertFrom-Json
    $notesBeforeJson = $notesBefore.Body | ConvertFrom-Json
    $tasksTotalBefore = [int]$tasksBeforeJson.data.page.total
    $notesTotalBefore = [int]$notesBeforeJson.data.page.total

    $keyPrefix = [Guid]::NewGuid().ToString('N')
    $missingCreateHeaders = @{
        'Idempotency-Key' = "$keyPrefix-task-create"
        'X-TM-Confirm-Mutation' = 'task.create'
    }
    $stage = 'task-create-precondition'
    $taskCreate = Invoke-TmRequest -Client $client -Method 'POST' `
        -Uri "$base/api/v1/tasks" -Token $token -Headers $missingCreateHeaders `
        -JsonBody '{"title":"must not be created"}'
    Assert-TmError -Response $taskCreate -ExpectedStatus 428 `
        -ExpectedCode 'MUTATION_PRECONDITION_REQUIRED' -CheckName 'Task create precondition'

    $stage = 'note-create-confirmation'
    $noteCreate = Invoke-TmRequest -Client $client -Method 'POST' `
        -Uri "$base/api/v1/notes" -Token $token -Headers @{
            'Idempotency-Key' = "$keyPrefix-note-create"
            'X-TM-Confirm-Mutation' = 'task.create'
            'If-None-Match' = '*'
        } -JsonBody '{"noteType":"decision","title":"must not be created"}'
    Assert-TmError -Response $noteCreate -ExpectedStatus 428 `
        -ExpectedCode 'MUTATION_CONFIRMATION_REQUIRED' -CheckName 'Note create confirmation'

    $missingId = '00000000-0000-0000-0000-000000000000'
    $stage = 'task-update-not-found'
    $taskUpdate = Invoke-TmRequest -Client $client -Method 'PATCH' `
        -Uri "$base/api/v1/tasks/$missingId" -Token $token -Headers @{
            'Idempotency-Key' = "$keyPrefix-task-update"
            'X-TM-Confirm-Mutation' = 'task.update'
            'If-Match' = '"1"'
        } -JsonBody '{"title":"must not be updated"}'
    Assert-TmError -Response $taskUpdate -ExpectedStatus 404 `
        -ExpectedCode 'MUTATION_RESOURCE_NOT_FOUND' -CheckName 'Task update missing resource'

    $stage = 'note-update-not-found'
    $noteUpdate = Invoke-TmRequest -Client $client -Method 'PATCH' `
        -Uri "$base/api/v1/notes/$missingId" -Token $token -Headers @{
            'Idempotency-Key' = "$keyPrefix-note-update"
            'X-TM-Confirm-Mutation' = 'note.update'
            'If-Match' = '"1"'
        } -JsonBody '{"title":"must not be updated"}'
    Assert-TmError -Response $noteUpdate -ExpectedStatus 404 `
        -ExpectedCode 'MUTATION_RESOURCE_NOT_FOUND' -CheckName 'Note update missing resource'

    $stage = 'checklist-update-not-found'
    $checklistUpdate = Invoke-TmRequest -Client $client -Method 'PATCH' `
        -Uri "$base/api/v1/checklist/$missingId" -Token $token -Headers @{
            'Idempotency-Key' = "$keyPrefix-checklist-update"
            'X-TM-Confirm-Mutation' = 'checklist.set_done'
            'If-Match' = '"1"'
        } -JsonBody '{"isDone":true}'
    Assert-TmError -Response $checklistUpdate -ExpectedStatus 404 `
        -ExpectedCode 'MUTATION_RESOURCE_NOT_FOUND' -CheckName 'Checklist update missing resource'

    $stage = 'delete-prohibition'
    $deleteTask = Invoke-TmRequest -Client $client -Method 'DELETE' `
        -Uri "$base/api/v1/tasks/$missingId" -Token $token
    Assert-TmError -Response $deleteTask -ExpectedStatus 405 `
        -ExpectedCode 'METHOD_NOT_ALLOWED' -CheckName 'Task delete prohibition'

    $stage = 'unchanged-read'
    $tasksAfter = Invoke-TmRequest -Client $client -Method 'GET' `
        -Uri "$base/api/v1/tasks" -Token $token
    $notesAfter = Invoke-TmRequest -Client $client -Method 'GET' `
        -Uri "$base/api/v1/notes" -Token $token
    $tasksAfterJson = $tasksAfter.Body | ConvertFrom-Json
    $notesAfterJson = $notesAfter.Body | ConvertFrom-Json
    $tasksTotalAfter = [int]$tasksAfterJson.data.page.total
    $notesTotalAfter = [int]$notesAfterJson.data.page.total
    if ($tasksTotalAfter -ne $tasksTotalBefore -or $notesTotalAfter -ne $notesTotalBefore) {
        throw '비파괴 검사 전후 Task 또는 Note 개수가 변경되었습니다.'
    }
    if ($tasksBefore.ETag -ne $tasksAfter.ETag -or $notesBefore.ETag -ne $notesAfter.ETag) {
        throw '비파괴 검사 전후 collection ETag가 변경되었습니다.'
    }
    if ($tasksAfter.CacheControl -ne 'no-store' -or -not $tasksAfter.HasHsts) {
        throw '인증된 응답의 보안 header가 예상과 다릅니다.'
    }

    $stage = 'complete'
    $result = [ordered]@{
        success = $true
        verifiedAtUtc = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ss.fffZ')
        baseUri = $base
        authenticatedStatus = $auth.StatusCode
        authenticated = $true
        taskCreateMissingPreconditionStatus = $taskCreate.StatusCode
        noteCreateWrongConfirmationStatus = $noteCreate.StatusCode
        taskUpdateMissingResourceStatus = $taskUpdate.StatusCode
        noteUpdateMissingResourceStatus = $noteUpdate.StatusCode
        checklistUpdateMissingResourceStatus = $checklistUpdate.StatusCode
        deleteTaskStatus = $deleteTask.StatusCode
        tasksTotalBefore = $tasksTotalBefore
        tasksTotalAfter = $tasksTotalAfter
        notesTotalBefore = $notesTotalBefore
        notesTotalAfter = $notesTotalAfter
        collectionEtagsUnchanged = $true
        cacheControl = $tasksAfter.CacheControl
        hstsPresent = $tasksAfter.HasHsts
        successfulMutationPerformed = $false
        tokenPersisted = $false
    }

    $outputDirectory = Split-Path -Parent $OutputPath
    New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null
    $result | ConvertTo-Json | Set-Content -LiteralPath $OutputPath -Encoding utf8

    Write-Host ''
    Write-Host 'STEP 8 production 비파괴 인증 검증 성공' -ForegroundColor Green
    Write-Host 'Task/Note create 보호장치: 428'
    Write-Host 'Task/Note/Checklist 누락 대상 수정: 404'
    Write-Host 'DELETE 금지: 405 METHOD_NOT_ALLOWED'
    Write-Host "Task/Note 개수 불변: $tasksTotalBefore / $notesTotalBefore"
}
catch {
    $message = $_.Exception.Message
    $failure = [ordered]@{
        success = $false
        verifiedAtUtc = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ss.fffZ')
        baseUri = $BaseUri.TrimEnd('/')
        failedStage = $stage
        message = $message
        successfulMutationPerformed = $false
        tokenPersisted = $false
    }
    $outputDirectory = Split-Path -Parent $OutputPath
    New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null
    $failure | ConvertTo-Json | Set-Content -LiteralPath $OutputPath -Encoding utf8

    Write-Host ''
    Write-Host 'STEP 8 production 비파괴 인증 검증 실패' -ForegroundColor Red
    Write-Host "실패 단계: $stage"
    Write-Host $message
}
finally {
    $client.Dispose()
    $token = $null
    $secureToken = $null
    $ProvidedSecureToken = $null
    Remove-Variable token, secureToken, ProvidedSecureToken, keyPrefix, auth, authJson, `
        tasksBefore, tasksBeforeJson, notesBefore, notesBeforeJson, taskCreate, noteCreate, `
        taskUpdate, noteUpdate, checklistUpdate, deleteTask, tasksAfter, tasksAfterJson, `
        notesAfter, notesAfterJson -ErrorAction SilentlyContinue
}

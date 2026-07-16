[CmdletBinding()]
param(
    [ValidatePattern('^https://')]
    [string]$BaseUri = 'https://tm-server-production-5573.up.railway.app',

    [string]$OutputPath = (Join-Path $PSScriptRoot '..\dist\manual-step7-production-verification\result.json'),

    [Security.SecureString]$ProvidedSecureToken
)

$ErrorActionPreference = 'Stop'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
Add-Type -AssemblyName System.Net.Http

function Invoke-TmRequest {
    param(
        [Parameter(Mandatory = $true)]
        [System.Net.Http.HttpClient]$Client,

        [Parameter(Mandatory = $true)]
        [System.Net.Http.HttpMethod]$Method,

        [Parameter(Mandatory = $true)]
        [string]$Uri,

        [Parameter(Mandatory = $true)]
        [string]$Token,

        [string]$IfNoneMatch,
        [string]$JsonBody
    )

    $request = New-Object System.Net.Http.HttpRequestMessage($Method, $Uri)
    $response = $null
    try {
        $request.Headers.Authorization = New-Object System.Net.Http.Headers.AuthenticationHeaderValue('Bearer', $Token)
        if ($IfNoneMatch) {
            [void]$request.Headers.TryAddWithoutValidation('If-None-Match', $IfNoneMatch)
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
    $auth = Invoke-TmRequest -Client $client -Method ([Net.Http.HttpMethod]::Get) `
        -Uri "$base/api/v1/auth/status" -Token $token
    if ($auth.StatusCode -ne 200) {
        throw "인증 확인 실패: HTTP $($auth.StatusCode)"
    }
    $authJson = $auth.Body | ConvertFrom-Json
    if ($authJson.data.authenticated -ne $true) {
        throw '서버가 예상한 인증 결과를 반환하지 않았습니다.'
    }

    $stage = 'read-only-tasks'
    $tasks = Invoke-TmRequest -Client $client -Method ([Net.Http.HttpMethod]::Get) `
        -Uri "$base/api/v1/tasks" -Token $token
    if ($tasks.StatusCode -ne 200 -or [string]::IsNullOrWhiteSpace($tasks.ETag)) {
        throw "read-only 조회 실패: HTTP $($tasks.StatusCode)"
    }
    $tasksJson = $tasks.Body | ConvertFrom-Json
    $returned = [int]$tasksJson.data.page.returned
    $total = [int]$tasksJson.data.page.total
    if (@($tasksJson.data.items).Count -ne $returned) {
        throw 'read-only 조회의 pagination 계약이 일치하지 않습니다.'
    }

    $stage = 'etag-revalidation'
    $notModified = Invoke-TmRequest -Client $client -Method ([Net.Http.HttpMethod]::Get) `
        -Uri "$base/api/v1/tasks" -Token $token -IfNoneMatch $tasks.ETag
    if ($notModified.StatusCode -ne 304) {
        throw "ETag 재검증 실패: HTTP $($notModified.StatusCode)"
    }

    $stage = 'mutation-block'
    $mutation = Invoke-TmRequest -Client $client -Method ([Net.Http.HttpMethod]::Post) `
        -Uri "$base/api/v1/tasks" -Token $token -JsonBody '{"title":"must not be created"}'
    if ($mutation.StatusCode -ne 405) {
        throw "mutation 차단 실패: HTTP $($mutation.StatusCode)"
    }
    $mutationJson = $mutation.Body | ConvertFrom-Json
    if ($mutationJson.error.code -ne 'METHOD_NOT_ALLOWED') {
        throw 'mutation 차단 오류 계약이 일치하지 않습니다.'
    }

    if ($tasks.CacheControl -ne 'no-store' -or -not $tasks.HasHsts) {
        throw 'read-only 응답의 보안 header가 예상과 다릅니다.'
    }

    $stage = 'complete'
    $result = [ordered]@{
        success = $true
        verifiedAtUtc = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ss.fffZ')
        baseUri = $base
        authenticatedStatus = $auth.StatusCode
        authenticated = $true
        tasksStatus = $tasks.StatusCode
        tasksReturned = $returned
        tasksTotal = $total
        etagPresent = $true
        notModifiedStatus = $notModified.StatusCode
        mutationStatus = $mutation.StatusCode
        mutationErrorCode = $mutationJson.error.code
        cacheControl = $tasks.CacheControl
        hstsPresent = $tasks.HasHsts
        tokenPersisted = $false
    }

    $outputDirectory = Split-Path -Parent $OutputPath
    New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null
    $result | ConvertTo-Json | Set-Content -LiteralPath $OutputPath -Encoding utf8

    Write-Host ''
    Write-Host 'STEP 7 production 인증 검증 성공' -ForegroundColor Green
    Write-Host "GET /api/v1/tasks: 200 (total=$total)"
    Write-Host 'If-None-Match: 304'
    Write-Host 'POST /api/v1/tasks: 405 METHOD_NOT_ALLOWED'
}
catch {
    $message = $_.Exception.Message
    $failure = [ordered]@{
        success = $false
        verifiedAtUtc = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ss.fffZ')
        baseUri = $BaseUri.TrimEnd('/')
        failedStage = $stage
        message = $message
        tokenPersisted = $false
    }
    $outputDirectory = Split-Path -Parent $OutputPath
    New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null
    $failure | ConvertTo-Json | Set-Content -LiteralPath $OutputPath -Encoding utf8

    Write-Host ''
    Write-Host 'STEP 7 production 인증 검증 실패' -ForegroundColor Red
    Write-Host "실패 단계: $stage"
    Write-Host $message
}
finally {
    $client.Dispose()
    $token = $null
    $secureToken = $null
    $ProvidedSecureToken = $null
    Remove-Variable token, secureToken, ProvidedSecureToken, auth, tasks, tasksJson, notModified, mutation, mutationJson -ErrorAction SilentlyContinue
}

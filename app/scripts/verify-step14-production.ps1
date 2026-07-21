[CmdletBinding()]
param(
    [ValidatePattern('^https://')]
    [string]$BaseUri = 'https://tm-server-production-5573.up.railway.app',

    [ValidateRange(60, 600)]
    [int]$TimeoutSeconds = 240,

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
    $OutputPath = Join-Path $tmRoot 'dist\manual-step14-production-verification\result.json'
}
$OutputPath = [System.IO.Path]::GetFullPath($OutputPath)

function Invoke-TmRequest {
    param(
        [Parameter(Mandatory = $true)][System.Net.Http.HttpClient]$Client,
        [Parameter(Mandatory = $true)][string]$Uri,
        [Parameter(Mandatory = $true)][string]$Token
    )

    $request = $null
    $response = $null
    try {
        $request = [System.Net.Http.HttpRequestMessage]::new(
            [System.Net.Http.HttpMethod]::Get,
            $Uri
        )
        $request.Headers.Authorization = [System.Net.Http.Headers.AuthenticationHeaderValue]::new('Bearer', $Token)
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
    $auth = Invoke-TmRequest -Client $client -Uri "$base/api/v1/auth/status" -Token $token
    if ($auth.StatusCode -ne 200) { throw "Authentication returned HTTP $($auth.StatusCode)." }

    $stage = 'scheduler-and-backup-readiness'
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    $opsJson = $null
    $ready = $false
    do {
        $ops = Invoke-TmRequest -Client $client -Uri "$base/api/v1/ops/status" -Token $token
        if ($ops.StatusCode -eq 200) {
            $opsJson = $ops.Body | ConvertFrom-Json
            $ready = (
                [bool]$opsJson.data.database.ok -eq $true -and
                [int]$opsJson.data.database.schemaVersion -eq 8 -and
                [string]$opsJson.data.scheduler.status -eq 'healthy' -and
                [int]$opsJson.data.scheduler.enabledJobCount -eq 2 -and
                [int]$opsJson.data.scheduler.effectCount -ge 1 -and
                [int]$opsJson.data.scheduler.deadLetterCount -eq 0 -and
                [bool]$opsJson.data.scheduler.openaiCallsEnabled -eq $false -and
                [string]$opsJson.data.remoteBackup.status -eq 'succeeded' -and
                [int]$opsJson.data.remoteBackup.schemaVersion -eq 8 -and
                [string]$opsJson.data.remoteBackup.integrityCheck -eq 'ok'
            )
            if ($ready) { break }
        }
        Start-Sleep -Seconds 5
    } while ([DateTime]::UtcNow -lt $deadline)

    if ($null -eq $opsJson) { throw 'Operations status was not available.' }
    if (-not $ready) {
        throw 'Schema 8 scheduler or the verified schema 8 remote backup did not become ready before timeout.'
    }
    if ([int]$opsJson.data.scheduler.pollIntervalSeconds -ne 30 -or
        [int]$opsJson.data.scheduler.leaseSeconds -ne 300 -or
        [string]$opsJson.data.scheduler.timezone -ne 'Asia/Seoul' -or
        [string]$opsJson.data.scheduler.quietHoursStart -ne '22:00' -or
        [string]$opsJson.data.scheduler.quietHoursEnd -ne '07:00' -or
        [bool]$opsJson.data.scheduler.quietHoursUserFacingOnly -ne $true) {
        throw 'The STEP 14 scheduler policy does not match the approved defaults.'
    }

    $result = [ordered]@{
        success = $true
        verifiedAtUtc = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ss.fffZ')
        baseUri = $base
        schemaVersion = [int]$opsJson.data.database.schemaVersion
        schedulerStatus = [string]$opsJson.data.scheduler.status
        enabledJobCount = [int]$opsJson.data.scheduler.enabledJobCount
        effectCount = [int]$opsJson.data.scheduler.effectCount
        deadLetterCount = [int]$opsJson.data.scheduler.deadLetterCount
        queueDepth = [int]$opsJson.data.scheduler.queueDepth
        lastSucceededAt = [string]$opsJson.data.scheduler.lastSucceededAt
        pollIntervalSeconds = [int]$opsJson.data.scheduler.pollIntervalSeconds
        leaseSeconds = [int]$opsJson.data.scheduler.leaseSeconds
        timezone = [string]$opsJson.data.scheduler.timezone
        quietHours = "$($opsJson.data.scheduler.quietHoursStart)-$($opsJson.data.scheduler.quietHoursEnd)"
        remoteBackupStatus = [string]$opsJson.data.remoteBackup.status
        remoteBackupSchemaVersion = [int]$opsJson.data.remoteBackup.schemaVersion
        remoteBackupIntegrityCheck = [string]$opsJson.data.remoteBackup.integrityCheck
        billableAiCallPerformed = $false
        productionBusinessMutationPerformed = $false
        tokenReadFromCredentialLocker = $true
        tokenPersistedByScript = $false
    }
    $outputDirectory = Split-Path -Parent $OutputPath
    New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null
    $result | ConvertTo-Json | Set-Content -LiteralPath $OutputPath -Encoding utf8

    Write-Host ''
    Write-Host 'STEP 14 production non-billable scheduler verification passed.' -ForegroundColor Green
    Write-Host "Schema: $($result.schemaVersion)"
    Write-Host "Scheduler: $($result.schedulerStatus), jobs $($result.enabledJobCount), effects $($result.effectCount)"
    Write-Host "Remote backup: $($result.remoteBackupStatus), schema $($result.remoteBackupSchemaVersion)"
    Write-Host 'No OpenAI call or production business-data mutation was performed.'
}
catch {
    $failure = [ordered]@{
        success = $false
        failedAtUtc = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ss.fffZ')
        stage = $stage
        reason = $_.Exception.Message
        billableAiCallPerformed = $false
        productionBusinessMutationPerformed = $false
    }
    try {
        $outputDirectory = Split-Path -Parent $OutputPath
        New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null
        $failure | ConvertTo-Json | Set-Content -LiteralPath $OutputPath -Encoding utf8
    }
    catch {
        Write-Warning 'Could not write the STEP 14 failure result.'
    }
    throw "STEP 14 production verification failed at ${stage}: $($_.Exception.Message)"
}
finally {
    if ($null -ne $client) { $client.Dispose() }
    $token = $null
    $credential = $null
    $vault = $null
}

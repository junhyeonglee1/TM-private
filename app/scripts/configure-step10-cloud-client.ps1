[CmdletBinding()]
param(
    [ValidateSet('Cloud', 'Local')]
    [string]$Mode = 'Cloud',
    [string]$BaseUrl = 'https://tm-server-production-5573.up.railway.app',
    [int]$ExpectedProjectCount = -1,
    [int]$ExpectedTaskCount = -1,
    [string]$OutputPath
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
Add-Type -AssemblyName System.Net.Http

$resource = 'TM Cloud Production'
$userName = 'single-user'
$tmRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
$configPath = Join-Path $tmRoot 'data\cloud-client.json'
$configDirectory = Split-Path -Parent $configPath
if ([string]::IsNullOrWhiteSpace($OutputPath)) {
    $OutputPath = Join-Path $tmRoot 'dist\manual-step10-cloud-client-verification\result.json'
}
$OutputPath = [System.IO.Path]::GetFullPath($OutputPath)

function Get-TmPasswordVault {
    return [Windows.Security.Credentials.PasswordVault,Windows.Security.Credentials,ContentType=WindowsRuntime]::new()
}

function Remove-TmCredentialIfPresent {
    param(
        [Parameter(Mandatory)]$Vault
    )

    $existingCredentials = @($Vault.RetrieveAll()) | Where-Object {
        $_.Resource -eq $resource -and $_.UserName -eq $userName
    }
    foreach ($existing in $existingCredentials) {
        $Vault.Remove($existing)
    }
}

function Invoke-TmRequest {
    param(
        [Parameter(Mandatory)][System.Net.Http.HttpClient]$Client,
        [Parameter(Mandatory)][string]$Method,
        [Parameter(Mandatory)][string]$Uri,
        [Parameter(Mandatory)][string]$Token,
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

$secureToken = $null
$token = $null
$client = $null
$verified = $null
$credential = $null
$verification = $null

try {
    if ($Mode -eq 'Cloud') {
        $uri = [System.Uri]$BaseUrl
        if (-not $uri.IsAbsoluteUri -or $uri.Scheme -ne 'https' -or -not [string]::IsNullOrEmpty($uri.UserInfo) -or -not [string]::IsNullOrEmpty($uri.Query) -or -not [string]::IsNullOrEmpty($uri.Fragment)) {
            throw 'BaseUrl must be a credential-free HTTPS origin.'
        }
        $base = $uri.GetLeftPart([System.UriPartial]::Authority)

        $client = [System.Net.Http.HttpClient]::new()
        $client.Timeout = [TimeSpan]::FromSeconds(30)

        for ($attempt = 1; $attempt -le 5; $attempt++) {
            $secureToken = Read-Host "TM production auth token (attempt $attempt of 5)" -AsSecureString
            $token = [System.Net.NetworkCredential]::new('', $secureToken).Password.Trim()
            if ($token -notmatch '^tm_pat_v1_[A-Za-z0-9_-]{43}$') {
                Write-Warning "Token format is invalid (length $($token.Length)). Try again."
                $token = $null
                $secureToken.Dispose()
                $secureToken = $null
                Set-Clipboard -Value '[TM] secure credential input cleared' -ErrorAction SilentlyContinue
                continue
            }

            $auth = Invoke-TmRequest -Client $client -Method 'GET' -Uri "$base/api/v1/auth/status" -Token $token
            if ($auth.StatusCode -eq 401) {
                Write-Warning 'Production rejected this token. Check the current token and try again.'
                $token = $null
                $secureToken.Dispose()
                $secureToken = $null
                Set-Clipboard -Value '[TM] secure credential input cleared' -ErrorAction SilentlyContinue
                continue
            }
            if ($auth.StatusCode -ne 200) {
                throw "Authentication status failed with HTTP $($auth.StatusCode)."
            }
            $authJson = $auth.Body | ConvertFrom-Json
            if ($authJson.data.authenticated -ne $true) {
                throw 'Production did not report an authenticated session.'
            }

            $ops = Invoke-TmRequest -Client $client -Method 'GET' -Uri "$base/api/v1/ops/status" -Token $token
            if ($ops.StatusCode -ne 200) {
                throw "Operations status failed with HTTP $($ops.StatusCode)."
            }
            $opsJson = $ops.Body | ConvertFrom-Json
            if ($opsJson.data.database.ok -ne $true -or [int]$opsJson.data.database.schemaVersion -ne 5) {
                throw 'Production database health or schema verification failed.'
            }
            if ([string]$opsJson.data.remoteBackup.status -ne 'succeeded') {
                throw "The latest production remote backup is not successful: $($opsJson.data.remoteBackup.status)"
            }

            $snapshotResponse = Invoke-TmRequest -Client $client -Method 'POST' -Uri "$base/api/v1/desktop/commands/get_app_snapshot" -Token $token -JsonBody '{"args":{}}'
            if ($snapshotResponse.StatusCode -ne 200) {
                throw "Production desktop snapshot failed with HTTP $($snapshotResponse.StatusCode)."
            }
            $snapshotJson = $snapshotResponse.Body | ConvertFrom-Json
            $projectCount = @($snapshotJson.data.projects).Count
            $taskCount = @($snapshotJson.data.tasks).Count
            if ($ExpectedProjectCount -ge 0 -and $projectCount -ne $ExpectedProjectCount) {
                throw "Production project count mismatch: expected $ExpectedProjectCount, received $projectCount."
            }
            if ($ExpectedTaskCount -ge 0 -and $taskCount -ne $ExpectedTaskCount) {
                throw "Production task count mismatch: expected $ExpectedTaskCount, received $taskCount."
            }

            $importRoute = Invoke-TmRequest -Client $client -Method 'POST' -Uri "$base/api/v1/ops/import" -Token $token -JsonBody '{}'
            if ($importRoute.StatusCode -ne 404) {
                throw "Production import route is still available (HTTP $($importRoute.StatusCode))."
            }

            $vault = Get-TmPasswordVault
            Remove-TmCredentialIfPresent -Vault $vault
            $credential = [Windows.Security.Credentials.PasswordCredential,Windows.Security.Credentials,ContentType=WindowsRuntime]::new($resource, $userName, $token)
            $vault.Add($credential)

            $verified = $vault.Retrieve($resource, $userName)
            $verified.RetrievePassword()
            if ($verified.Password -ne $token) {
                throw 'Windows Credential Locker verification failed.'
            }

            $verification = [ordered]@{
                success = $true
                verifiedAtUtc = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ss.fffZ')
                baseUrl = $base
                authenticated = $true
                schemaVersion = [int]$opsJson.data.database.schemaVersion
                databaseOk = [bool]$opsJson.data.database.ok
                remoteBackupStatus = [string]$opsJson.data.remoteBackup.status
                projectCount = $projectCount
                taskCount = $taskCount
                desktopSnapshotVerified = $true
                importRouteStatus = $importRoute.StatusCode
                credentialResource = $resource
                tokenPersistedInCredentialLocker = $true
                tokenPersistedInFile = $false
            }
            break
        }

        if ($null -eq $verification) {
            throw 'Cloud client setup stopped after 5 invalid token attempts.'
        }

        $config = [ordered]@{
            mode = 'cloud'
            baseUrl = $base
        }
    }
    else {
        $config = [ordered]@{
            mode = 'local'
            baseUrl = $null
        }
        $verification = [ordered]@{
            success = $true
            verifiedAtUtc = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ss.fffZ')
            mode = 'local'
            tokenPersistedInFile = $false
        }
    }

    [System.IO.Directory]::CreateDirectory($configDirectory) | Out-Null
    $temporaryPath = "$configPath.tmp"
    $json = $config | ConvertTo-Json -Compress
    [System.IO.File]::WriteAllText($temporaryPath, $json, [System.Text.UTF8Encoding]::new($false))
    Move-Item -LiteralPath $temporaryPath -Destination $configPath -Force

    $outputDirectory = Split-Path -Parent $OutputPath
    [System.IO.Directory]::CreateDirectory($outputDirectory) | Out-Null
    [System.IO.File]::WriteAllText(
        $OutputPath,
        ($verification | ConvertTo-Json -Depth 10),
        [System.Text.UTF8Encoding]::new($false)
    )

    Write-Host ''
    Write-Host "TM data mode configured: $($config.mode)" -ForegroundColor Green
    Write-Host "Configuration: $configPath"
    Write-Host "Verification: $OutputPath"
    if ($Mode -eq 'Cloud') {
        Write-Host "Production snapshot: $($verification.projectCount) projects / $($verification.taskCount) tasks"
        Write-Host 'The token is stored only in Windows Credential Locker.' -ForegroundColor Green
    }
}
finally {
    if ($null -ne $client) { $client.Dispose() }
    if ($null -ne $secureToken) { $secureToken.Dispose() }
    $token = $null
    $verified = $null
    $credential = $null
    Set-Clipboard -Value '[TM] secure credential input cleared' -ErrorAction SilentlyContinue
    Remove-Variable token, secureToken, verified, credential, auth, authJson, ops, opsJson, snapshotResponse, snapshotJson, importRoute -ErrorAction SilentlyContinue
}

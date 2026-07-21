[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$SnapshotPath,
    [Parameter(Mandatory)]
    [string]$PreparationResultPath,
    [string]$BaseUrl = 'https://tm-server-production-5573.up.railway.app'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$snapshot = [System.IO.Path]::GetFullPath($SnapshotPath)
$preparationResult = [System.IO.Path]::GetFullPath($PreparationResultPath)
if (-not (Test-Path -LiteralPath $snapshot -PathType Leaf)) {
    throw "Cutover snapshot was not found: $snapshot"
}
if (-not (Test-Path -LiteralPath $preparationResult -PathType Leaf)) {
    throw "Cutover preparation result was not found: $preparationResult"
}
$prepared = Get-Content -Raw -LiteralPath $preparationResult | ConvertFrom-Json
$logicalSha256 = [string]$prepared.snapshot.manifest.logicalSha256
if ($logicalSha256 -notmatch '^[a-f0-9]{64}$') {
    throw 'The preparation result does not contain a valid logical SHA-256.'
}
$actualFileSha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $snapshot).Hash.ToLowerInvariant()
if ($actualFileSha256 -ne [string]$prepared.snapshot.sha256) {
    throw 'The snapshot file SHA-256 no longer matches the preparation result.'
}

$secureToken = $null
$token = $null
$client = $null
$request = $null
$response = $null
$content = $null
$responseJson = $null
$importVerified = $false
try {
    Add-Type -AssemblyName System.Net.Http
    $client = [System.Net.Http.HttpClient]::new()
    $client.Timeout = [TimeSpan]::FromMinutes(5)
    $endpoint = ([System.Uri]$BaseUrl).GetLeftPart([System.UriPartial]::Authority) + '/api/v1/ops/import'

    for ($attempt = 1; $attempt -le 5; $attempt++) {
        $secureToken = Read-Host "TM production auth token (attempt $attempt of 5)" -AsSecureString
        $token = [System.Net.NetworkCredential]::new('', $secureToken).Password.Trim()
        if ($token -notmatch '^tm_pat_v1_[A-Za-z0-9_-]{43}$') {
            Write-Warning "Token format is invalid (length $($token.Length)). Expected tm_pat_v1_ followed by 43 characters. Try again."
            $token = $null
            $secureToken.Dispose()
            $secureToken = $null
            Set-Clipboard -Value '[TM] secure credential input cleared' -ErrorAction SilentlyContinue
            continue
        }

        $request = [System.Net.Http.HttpRequestMessage]::new([System.Net.Http.HttpMethod]::Post, $endpoint)
        $request.Headers.Authorization = [System.Net.Http.Headers.AuthenticationHeaderValue]::new('Bearer', $token)
        $request.Headers.Add('x-tm-confirm-import', $logicalSha256)
        $content = [System.Net.Http.ByteArrayContent]::new([System.IO.File]::ReadAllBytes($snapshot))
        $content.Headers.ContentType = [System.Net.Http.Headers.MediaTypeHeaderValue]::new('application/vnd.sqlite3')
        $request.Content = $content
        $response = $client.SendAsync($request).GetAwaiter().GetResult()
        $responseText = $response.Content.ReadAsStringAsync().GetAwaiter().GetResult()
        $responseJson = $responseText | ConvertFrom-Json

        if ([int]$response.StatusCode -eq 401) {
            Write-Warning 'Production rejected this token. Check the current TM production token and try again.'
            $response.Dispose()
            $response = $null
            $request.Dispose()
            $request = $null
            $content.Dispose()
            $content = $null
            $token = $null
            $secureToken.Dispose()
            $secureToken = $null
            Set-Clipboard -Value '[TM] secure credential input cleared' -ErrorAction SilentlyContinue
            continue
        }
        if (-not $response.IsSuccessStatusCode) {
            $code = [string]$responseJson.error.code
            throw "Production import was rejected: HTTP $([int]$response.StatusCode) $code"
        }
        if (-not $responseJson.data.imported -or [string]$responseJson.data.manifest.logicalSha256 -ne $logicalSha256) {
            throw 'Production import response did not match the prepared source manifest.'
        }
        $importVerified = $true
        break
    }
    if (-not $importVerified) {
        throw 'Production import stopped after 5 invalid token attempts.'
    }
}
finally {
    if ($null -ne $content) { $content.Dispose() }
    if ($null -ne $request) { $request.Dispose() }
    if ($null -ne $response) { $response.Dispose() }
    if ($null -ne $client) { $client.Dispose() }
    $token = $null
    if ($null -ne $secureToken) { $secureToken.Dispose() }
    Set-Clipboard -Value '[TM] secure credential input cleared' -ErrorAction SilentlyContinue
}

$outputDirectory = Join-Path (Split-Path -Parent $preparationResult) 'production-import'
[System.IO.Directory]::CreateDirectory($outputDirectory) | Out-Null
$outputPath = Join-Path $outputDirectory 'result.json'
$result = [ordered]@{
    verifiedAt = [DateTimeOffset]::UtcNow.ToString('o')
    requestId = [string]$responseJson.requestId
    imported = [bool]$responseJson.data.imported
    byteSize = [int64]$responseJson.data.byteSize
    fileSha256 = [string]$responseJson.data.fileSha256
    preImportBackupFile = [string]$responseJson.data.preImportBackupFile
    manifest = $responseJson.data.manifest
    tokenPersistedByScript = $false
}
[System.IO.File]::WriteAllText(
    $outputPath,
    ($result | ConvertTo-Json -Depth 20),
    [System.Text.UTF8Encoding]::new($false)
)

Write-Host 'STEP 10 production import verified.' -ForegroundColor Green
Write-Host "Logical SHA-256: $logicalSha256"
Write-Host "Result: $outputPath"

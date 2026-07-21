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

$secureToken = Read-Host 'TM production auth token' -AsSecureString
$token = [System.Net.NetworkCredential]::new('', $secureToken).Password
$client = [System.Net.Http.HttpClient]::new()
$client.Timeout = [TimeSpan]::FromMinutes(5)
$request = $null
$response = $null
$content = $null
try {
    if ($token -notmatch '^tm_pat_v1_[A-Za-z0-9_-]{43}$') {
        throw 'The TM production token format is invalid.'
    }
    $endpoint = ([System.Uri]$BaseUrl).GetLeftPart([System.UriPartial]::Authority) + '/api/v1/ops/import'
    $request = [System.Net.Http.HttpRequestMessage]::new([System.Net.Http.HttpMethod]::Post, $endpoint)
    $request.Headers.Authorization = [System.Net.Http.Headers.AuthenticationHeaderValue]::new('Bearer', $token)
    $request.Headers.Add('x-tm-confirm-import', $logicalSha256)
    $content = [System.Net.Http.ByteArrayContent]::new([System.IO.File]::ReadAllBytes($snapshot))
    $content.Headers.ContentType = [System.Net.Http.Headers.MediaTypeHeaderValue]::new('application/vnd.sqlite3')
    $request.Content = $content
    $response = $client.SendAsync($request).GetAwaiter().GetResult()
    $responseText = $response.Content.ReadAsStringAsync().GetAwaiter().GetResult()
    $responseJson = $responseText | ConvertFrom-Json
    if (-not $response.IsSuccessStatusCode) {
        $code = [string]$responseJson.error.code
        throw "Production import was rejected: HTTP $([int]$response.StatusCode) $code"
    }
    if (-not $responseJson.data.imported -or [string]$responseJson.data.manifest.logicalSha256 -ne $logicalSha256) {
        throw 'Production import response did not match the prepared source manifest.'
    }
}
finally {
    if ($null -ne $content) { $content.Dispose() }
    if ($null -ne $request) { $request.Dispose() }
    if ($null -ne $response) { $response.Dispose() }
    $client.Dispose()
    $token = $null
    $secureToken.Dispose()
    Set-Clipboard -Value '[TM] secure credential input cleared'
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

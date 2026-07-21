[CmdletBinding()]
param(
    [ValidatePattern('^https://')]
    [string]$BaseUri = 'https://tm-server-production-5573.up.railway.app',

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
    $OutputPath = Join-Path $tmRoot 'dist\manual-step13-production-verification\result.json'
}
$OutputPath = [System.IO.Path]::GetFullPath($OutputPath)

function Invoke-TmRequest {
    param(
        [Parameter(Mandatory = $true)][System.Net.Http.HttpClient]$Client,
        [Parameter(Mandatory = $true)][string]$Method,
        [Parameter(Mandatory = $true)][string]$Uri,
        [Parameter(Mandatory = $true)][string]$Token
    )

    $request = $null
    $response = $null
    try {
        $request = [System.Net.Http.HttpRequestMessage]::new(
            [System.Net.Http.HttpMethod]::new($Method),
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
    $auth = Invoke-TmRequest -Client $client -Method 'GET' -Uri "$base/api/v1/auth/status" -Token $token
    if ($auth.StatusCode -ne 200) { throw "Authentication returned HTTP $($auth.StatusCode)." }

    $stage = 'operations-status'
    $ops = Invoke-TmRequest -Client $client -Method 'GET' -Uri "$base/api/v1/ops/status" -Token $token
    if ($ops.StatusCode -ne 200) { throw "Operations status returned HTTP $($ops.StatusCode)." }
    $opsJson = $ops.Body | ConvertFrom-Json
    if ([bool]$opsJson.data.database.ok -ne $true -or [int]$opsJson.data.database.schemaVersion -ne 7) {
        throw 'Production database is not healthy on schema 7.'
    }

    $stage = 'ai-status'
    $ai = Invoke-TmRequest -Client $client -Method 'GET' -Uri "$base/api/v1/ai/status" -Token $token
    if ($ai.StatusCode -ne 200) { throw "AI status returned HTTP $($ai.StatusCode)." }
    $aiJson = $ai.Body | ConvertFrom-Json
    if ([string]$aiJson.data.responseStorage -ne 'disabled' -or
        [string]$aiJson.data.assistantPromptVersion -ne 'step13-v1' -or
        [int]$aiJson.data.assistantAutomaticReadToolCount -ne 8 -or
        [bool]$aiJson.data.assistantMemoryEnabled -ne $true -or
        [bool]$aiJson.data.assistantMemoryAutomaticStorage -ne $false -or
        [bool]$aiJson.data.assistantMemoryApprovalRequired -ne $true -or
        [string]$aiJson.data.assistantMemoryOpenaiSensitivity -ne 'normal_and_explicitly_allowed_only' -or
        [string]$aiJson.data.assistantMemoryRetrieval -ne 'sqlite_fts5_structured_filters' -or
        [bool]$aiJson.data.assistantMemoryVectorServiceUsed -ne $false -or
        [int]$aiJson.data.assistantMemoryContextMaxItems -ne 12 -or
        [int]$aiJson.data.assistantMemoryContextMaxBytes -ne 6144 -or
        [bool]$aiJson.data.assistantAutoExecuteWithoutApproval -ne $false) {
        throw 'The STEP 13 memory safety contract does not match the approved defaults.'
    }

    $stage = 'memory-list'
    $memories = Invoke-TmRequest -Client $client -Method 'GET' -Uri "$base/api/v1/assistant/memories" -Token $token
    if ($memories.StatusCode -ne 200) { throw "Memory list returned HTTP $($memories.StatusCode)." }
    $memoriesJson = $memories.Body | ConvertFrom-Json
    if ($null -eq $memoriesJson.data.items) { throw 'Memory list response is missing items.' }

    $stage = 'bounded-memory-search'
    $query = [System.Uri]::EscapeDataString("step13-contract-no-match-$([Guid]::NewGuid().ToString('N'))")
    $search = Invoke-TmRequest -Client $client -Method 'GET' -Uri "$base/api/v1/assistant/memories/search?query=$query&openaiOnly=true&limit=1&maxBytes=256" -Token $token
    if ($search.StatusCode -ne 200) { throw "Bounded memory search returned HTTP $($search.StatusCode)." }
    $searchJson = $search.Body | ConvertFrom-Json
    if ([string]$searchJson.data.retrieval -ne 'sqlite_fts5_structured_filters' -or
        [int]$searchJson.data.bytesUsed -gt 256 -or
        @($searchJson.data.items).Count -gt 1) {
        throw 'The bounded memory search contract returned an unexpected result.'
    }

    $result = [ordered]@{
        success = $true
        verifiedAtUtc = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ss.fffZ')
        baseUri = $base
        schemaVersion = [int]$opsJson.data.database.schemaVersion
        promptVersion = [string]$aiJson.data.assistantPromptVersion
        automaticReadToolCount = [int]$aiJson.data.assistantAutomaticReadToolCount
        memoryEnabled = [bool]$aiJson.data.assistantMemoryEnabled
        automaticMemoryStorage = [bool]$aiJson.data.assistantMemoryAutomaticStorage
        memoryApprovalRequired = [bool]$aiJson.data.assistantMemoryApprovalRequired
        openaiSensitivity = [string]$aiJson.data.assistantMemoryOpenaiSensitivity
        retrieval = [string]$aiJson.data.assistantMemoryRetrieval
        vectorServiceUsed = [bool]$aiJson.data.assistantMemoryVectorServiceUsed
        contextMaxItems = [int]$aiJson.data.assistantMemoryContextMaxItems
        contextMaxBytes = [int]$aiJson.data.assistantMemoryContextMaxBytes
        existingMemoryCount = @($memoriesJson.data.items).Count
        boundedSearchItemCount = @($searchJson.data.items).Count
        boundedSearchBytesUsed = [int]$searchJson.data.bytesUsed
        billableAiCallPerformed = $false
        productionMemoryMutationPerformed = $false
        tokenReadFromCredentialLocker = $true
        tokenPersistedByScript = $false
    }
    $outputDirectory = Split-Path -Parent $OutputPath
    New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null
    $result | ConvertTo-Json | Set-Content -LiteralPath $OutputPath -Encoding utf8

    Write-Host ''
    Write-Host 'STEP 13 production non-billable, read-only verification passed.' -ForegroundColor Green
    Write-Host "Schema: $($result.schemaVersion)"
    Write-Host "Prompt: $($result.promptVersion)"
    Write-Host "Memory context ceiling: $($result.contextMaxItems) items / $($result.contextMaxBytes) bytes"
    Write-Host 'No OpenAI call or production memory mutation was performed.'
}
catch {
    $failure = [ordered]@{
        success = $false
        failedAtUtc = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ss.fffZ')
        stage = $stage
        reason = $_.Exception.Message
        billableAiCallPerformed = $false
        productionMemoryMutationPerformed = $false
    }
    try {
        $outputDirectory = Split-Path -Parent $OutputPath
        New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null
        $failure | ConvertTo-Json | Set-Content -LiteralPath $OutputPath -Encoding utf8
    }
    catch {
        Write-Warning 'Could not write the STEP 13 failure result.'
    }
    throw "STEP 13 production verification failed at ${stage}: $($_.Exception.Message)"
}
finally {
    if ($null -ne $client) { $client.Dispose() }
    $token = $null
    $credential = $null
    $vault = $null
}

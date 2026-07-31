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
$uncategorizedName = ([string][char]0xAE30) + ([string][char]0xD0C0)
$tmRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
if ([string]::IsNullOrWhiteSpace($OutputPath)) {
    $OutputPath = Join-Path $tmRoot 'dist\manual-uncategorized-migration-baseline\baseline.json'
}
$OutputPath = [System.IO.Path]::GetFullPath($OutputPath)

function Invoke-TmGet {
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
        $request.Headers.Authorization =
            [System.Net.Http.Headers.AuthenticationHeaderValue]::new('Bearer', $Token)
        $response = $Client.SendAsync($request).GetAwaiter().GetResult()
        $body = $response.Content.ReadAsStringAsync().GetAwaiter().GetResult()
        if ([int]$response.StatusCode -ne 200) {
            throw "GET $Uri returned HTTP $([int]$response.StatusCode)."
        }
        return ($body | ConvertFrom-Json)
    }
    finally {
        if ($null -ne $response) { $response.Dispose() }
        if ($null -ne $request) { $request.Dispose() }
    }
}

function Get-Sha256Hex {
    param(
        [Parameter(Mandatory = $true)]
        [AllowEmptyString()]
        [string]$Text
    )

    $sha256 = [System.Security.Cryptography.SHA256]::Create()
    try {
        $bytes = [System.Text.Encoding]::UTF8.GetBytes($Text)
        return ([System.BitConverter]::ToString($sha256.ComputeHash($bytes))).Replace('-', '').ToLowerInvariant()
    }
    finally {
        $sha256.Dispose()
    }
}

function Get-TmCollection {
    param(
        [Parameter(Mandatory = $true)][System.Net.Http.HttpClient]$Client,
        [Parameter(Mandatory = $true)][string]$Base,
        [Parameter(Mandatory = $true)][string]$Token,
        [ValidateSet('projects', 'tasks')][string]$ResourceName
    )

    $sort = if ($ResourceName -eq 'projects') { 'name' } else { 'updated_desc' }
    $items = [System.Collections.Generic.List[object]]::new()
    $offset = 0
    $total = $null
    $pageCount = 0
    do {
        $payload = Invoke-TmGet `
            -Client $Client `
            -Uri "$Base/api/v1/${ResourceName}?limit=100&offset=$offset&sort=$sort" `
            -Token $Token
        if ($null -eq $payload.data -or $null -eq $payload.data.page) {
            throw "The $ResourceName endpoint returned an invalid collection envelope."
        }
        foreach ($item in @($payload.data.items)) {
            $items.Add($item)
        }
        if ($null -eq $total) {
            $total = [int]$payload.data.page.total
        }
        elseif ($total -ne [int]$payload.data.page.total) {
            throw "The $ResourceName collection changed while the baseline was captured."
        }
        $pageCount += 1
        if ($pageCount -gt 1000) {
            throw "The $ResourceName collection exceeded the bounded page count."
        }
        $nextOffset = $payload.data.page.nextOffset
        if ($null -eq $nextOffset) { break }
        $offset = [int]$nextOffset
    } while ($true)

    if ($items.Count -ne $total) {
        throw "The $ResourceName baseline snapshot is incomplete."
    }
    $canonicalItems = @($items | ForEach-Object { $_ })
    return [pscustomobject]@{ Total = $total; Items = $canonicalItems }
}

$vault = $null
$credential = $null
$token = $null
$handler = $null
$client = $null
try {
    $uri = [System.Uri]$BaseUri
    if (-not $uri.IsAbsoluteUri -or $uri.Scheme -ne 'https' -or
        -not [string]::IsNullOrEmpty($uri.UserInfo) -or
        -not [string]::IsNullOrEmpty($uri.Query) -or
        -not [string]::IsNullOrEmpty($uri.Fragment)) {
        throw 'BaseUri must be a credential-free HTTPS origin.'
    }
    $base = $uri.GetLeftPart([System.UriPartial]::Authority)

    $vault =
        [Windows.Security.Credentials.PasswordVault,Windows.Security.Credentials,ContentType=WindowsRuntime]::new()
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

    $operations = Invoke-TmGet -Client $client -Uri "$base/api/v1/ops/status" -Token $token
    $ops = $operations.data
    if (-not [bool]$ops.database.ok -or
        [int]$ops.database.schemaVersion -ne 13 -or
        [string]$ops.remoteBackup.status -ne 'succeeded' -or
        [int]$ops.remoteBackup.schemaVersion -ne 13 -or
        [string]$ops.remoteBackup.integrityCheck -ne 'ok') {
        throw 'Production must have a healthy schema 13 database and verified off-volume schema 13 backup.'
    }

    $projects = Get-TmCollection `
        -Client $client -Base $base -Token $token -ResourceName 'projects'
    $tasks = Get-TmCollection `
        -Client $client -Base $base -Token $token -ResourceName 'tasks'
    $otherProjects = @(
        $projects.Items | Where-Object { ([string]$_.name).Trim() -eq $uncategorizedName }
    )
    if ($otherProjects.Count -ne 1) {
        throw 'Schema 13 production must contain exactly one active uncategorized project.'
    }
    $unassigned = @($tasks.Items | Where-Object { $null -eq $_.projectId })
    if ($unassigned.Count -lt 1) {
        throw 'Schema 13 production has no unassigned Task to verify during migration.'
    }

    $taskBaseline = @(
        $unassigned |
            Sort-Object -Property id |
            ForEach-Object {
                [ordered]@{
                    id = [string]$_.id
                    titleSha256 = Get-Sha256Hex -Text ([string]$_.title)
                    descriptionSha256 = Get-Sha256Hex -Text ([string]$_.description)
                    status = [string]$_.status
                    priority = [int]$_.priority
                    dueDate = if ($null -eq $_.dueDate) { $null } else { [string]$_.dueDate }
                    completedAt = if ($null -eq $_.completedAt) { $null } else { [string]$_.completedAt }
                    createdAt = [string]$_.createdAt
                    updatedAt = [string]$_.updatedAt
                    version = [int64]$_.version
                }
            }
    )
    $baseline = [ordered]@{
        formatVersion = 1
        capturedAtUtc = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ss.fffZ')
        baseUri = $base
        sourceSchemaVersion = 13
        verifiedRemoteBackup = $true
        projectTotal = [int]$projects.Total
        uncategorizedProject = [ordered]@{ id = [string]$otherProjects[0].id }
        unassignedTaskCount = $taskBaseline.Count
        unassignedTasks = $taskBaseline
        plaintextTaskContentStored = $false
        secretsStored = $false
    }

    $directory = Split-Path -Parent $OutputPath
    [System.IO.Directory]::CreateDirectory($directory) | Out-Null
    [System.IO.File]::WriteAllText(
        $OutputPath,
        ($baseline | ConvertTo-Json -Depth 12),
        [System.Text.UTF8Encoding]::new($false)
    )
}
finally {
    if ($null -ne $client) { $client.Dispose() }
    if ($null -ne $handler) { $handler.Dispose() }
    $token = $null
    $credential = $null
    $vault = $null
}

Write-Host 'TM schema 13 uncategorized migration baseline captured.' -ForegroundColor Green
Write-Host 'Task contents: SHA-256 only; production mutation: none; AI call: none'
Write-Host "Baseline: $OutputPath"

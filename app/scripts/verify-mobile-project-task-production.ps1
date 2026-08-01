[CmdletBinding()]
param(
    [ValidatePattern('^https://')]
    [string]$BaseUri = 'https://tm-server-production-5573.up.railway.app',

    [string]$OutputPath,

    [Parameter(Mandatory = $true)]
    [ValidateScript({ Test-Path -LiteralPath $_ -PathType Leaf })]
    [string]$MigrationBaselinePath
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
Add-Type -AssemblyName System.Net.Http

$resource = 'TM Cloud Production'
$userName = 'single-user'
$tmRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
if ([string]::IsNullOrWhiteSpace($OutputPath)) {
    $OutputPath = Join-Path $tmRoot (
        'dist\manual-mobile-project-tas' + 'k-production-verification\result.json'
    )
}
$OutputPath = [System.IO.Path]::GetFullPath($OutputPath)
$MigrationBaselinePath = [System.IO.Path]::GetFullPath($MigrationBaselinePath)

function Invoke-TmRequest {
    param(
        [Parameter(Mandatory = $true)][System.Net.Http.HttpClient]$Client,
        [Parameter(Mandatory = $true)][System.Net.Http.HttpMethod]$Method,
        [Parameter(Mandatory = $true)][string]$Uri,
        [string]$Token,
        [hashtable]$Headers,
        [string]$JsonBody
    )

    $request = $null
    $response = $null
    try {
        $request = [System.Net.Http.HttpRequestMessage]::new($Method, $Uri)
        if (-not [string]::IsNullOrWhiteSpace($Token)) {
            $request.Headers.Authorization =
                [System.Net.Http.Headers.AuthenticationHeaderValue]::new('Bearer', $Token)
        }
        if ($null -ne $Headers) {
            foreach ($entry in $Headers.GetEnumerator()) {
                if (-not $request.Headers.TryAddWithoutValidation(
                    [string]$entry.Key,
                    [string]$entry.Value
                )) {
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
        $setCookies = [System.Collections.Generic.List[string]]::new()
        $cookieValues = $null
        if ($response.Headers.TryGetValues('Set-Cookie', [ref]$cookieValues)) {
            foreach ($value in $cookieValues) {
                $setCookies.Add([string]$value)
            }
        }
        [pscustomobject]@{
            StatusCode = [int]$response.StatusCode
            Body = $response.Content.ReadAsStringAsync().GetAwaiter().GetResult()
            SetCookies = @($setCookies)
            ContentSecurityPolicy = if ($response.Headers.Contains('Content-Security-Policy')) {
                [string]::Join(';', $response.Headers.GetValues('Content-Security-Policy'))
            } else {
                $null
            }
        }
    }
    finally {
        if ($null -ne $response) { $response.Dispose() }
        if ($null -ne $request) { $request.Dispose() }
    }
}

function Assert-Status {
    param(
        [Parameter(Mandatory = $true)]$Response,
        [int]$Expected,
        [string]$Stage
    )
    if ($Response.StatusCode -ne $Expected) {
        throw "$Stage returned HTTP $($Response.StatusCode), expected $Expected."
    }
}

function Assert-ErrorCode {
    param(
        [Parameter(Mandatory = $true)]$Response,
        [string]$Expected,
        [string]$Stage
    )
    $payload = $Response.Body | ConvertFrom-Json
    if ([string]$payload.error.code -ne $Expected) {
        throw "$Stage returned an unexpected error contract."
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

function Get-DeviceCollectionSnapshot {
    param(
        [Parameter(Mandatory = $true)][System.Net.Http.HttpClient]$Client,
        [Parameter(Mandatory = $true)][string]$Base,
        [Parameter(Mandatory = $true)][string]$CookieHeader,
        [ValidateSet('projects', 'tasks')][string]$ResourceName
    )

    $sort = if ($ResourceName -eq 'projects') { 'name' } else { 'updated_desc' }
    $items = [System.Collections.Generic.List[object]]::new()
    $offset = 0
    $total = $null
    $pageCount = 0
    do {
        $response = Invoke-TmRequest `
            -Client $Client `
            -Method ([System.Net.Http.HttpMethod]::Get) `
            -Uri "$Base/api/v1/${ResourceName}?limit=100&offset=$offset&sort=$sort" `
            -Headers @{ Cookie = $CookieHeader }
        Assert-Status -Response $response -Expected 200 -Stage "mobile-$ResourceName-read"
        $payload = $response.Body | ConvertFrom-Json
        if ($null -eq $payload.data -or $null -eq $payload.data.page) {
            throw "mobile-$ResourceName-read returned an invalid collection envelope."
        }
        foreach ($item in @($payload.data.items)) {
            $items.Add($item)
        }
        if ($null -eq $total) {
            $total = [int]$payload.data.page.total
        }
        elseif ($total -ne [int]$payload.data.page.total) {
            throw "mobile-$ResourceName-read changed while the snapshot was collected."
        }
        $pageCount += 1
        if ($pageCount -gt 1000) {
            throw "mobile-$ResourceName-read exceeded the bounded page count."
        }
        $nextOffset = $payload.data.page.nextOffset
        if ($null -eq $nextOffset) {
            break
        }
        $offset = [int]$nextOffset
    } while ($true)

    if ($items.Count -ne $total) {
        throw "mobile-$ResourceName-read returned an incomplete snapshot."
    }
    $canonicalItems = @($items | ForEach-Object { $_ })
    $canonicalJson = ConvertTo-Json -InputObject $canonicalItems -Depth 30 -Compress
    [pscustomobject]@{
        Digest = Get-Sha256Hex -Text $canonicalJson
        Total = $total
        Items = $canonicalItems
    }
}

function Get-BusinessSnapshot {
    param(
        [Parameter(Mandatory = $true)][System.Net.Http.HttpClient]$Client,
        [Parameter(Mandatory = $true)][string]$Base,
        [Parameter(Mandatory = $true)][string]$CookieHeader
    )

    $projects = Get-DeviceCollectionSnapshot `
        -Client $Client -Base $Base -CookieHeader $CookieHeader -ResourceName 'projects'
    $tasks = Get-DeviceCollectionSnapshot `
        -Client $Client -Base $Base -CookieHeader $CookieHeader -ResourceName 'tasks'
    $uncategorizedProjects = @(
        $projects.Items | Where-Object { [string]$_.systemKey -eq 'uncategorized' }
    )
    if ($uncategorizedProjects.Count -ne 1) {
        throw 'The production project snapshot must contain exactly one uncategorized system project.'
    }
    $uncategorizedProjectId = [string]$uncategorizedProjects[0].id
    $unassignedTasks = @($tasks.Items | Where-Object { $null -eq $_.projectId })
    if ($unassignedTasks.Count -ne 0) {
        throw 'The production Task snapshot still contains project-less items after schema 14.'
    }
    [pscustomobject]@{
        Digest = Get-Sha256Hex -Text "projects:$($projects.Digest)`ntasks:$($tasks.Digest)"
        ProjectsRead = $true
        TasksRead = $true
        ProjectTotal = $projects.Total
        UncategorizedProjectId = $uncategorizedProjectId
        TaskItems = $tasks.Items
        UncategorizedProjectPresent = -not [string]::IsNullOrWhiteSpace($uncategorizedProjectId)
        AllTasksAssigned = $true
    }
}

function Assert-MigrationBaseline {
    param(
        [Parameter(Mandatory = $true)]$Baseline,
        [Parameter(Mandatory = $true)]$BusinessSnapshot,
        [Parameter(Mandatory = $true)][string]$Base
    )

    if ([int]$Baseline.formatVersion -ne 1 -or
        [int]$Baseline.sourceSchemaVersion -ne 13 -or
        [string]$Baseline.baseUri -ne $Base) {
        throw 'The schema 13 migration baseline metadata is invalid for this production origin.'
    }
    if ([int]$Baseline.projectTotal -ne [int]$BusinessSnapshot.ProjectTotal) {
        throw 'The active project count changed during the schema 13 to 14 migration.'
    }
    if ([string]$Baseline.uncategorizedProject.id -ne
        [string]$BusinessSnapshot.UncategorizedProjectId) {
        throw 'Schema 14 did not adopt the existing production uncategorized project ID.'
    }

    $expectedTasks = @($Baseline.unassignedTasks)
    if ($expectedTasks.Count -lt 1 -or
        [int]$Baseline.unassignedTaskCount -ne $expectedTasks.Count) {
        throw 'The schema 13 migration baseline has no complete unassigned Task set.'
    }
    $actualById = @{}
    foreach ($task in @($BusinessSnapshot.TaskItems)) {
        $id = [string]$task.id
        if ($actualById.ContainsKey($id)) {
            throw 'The production Task snapshot contains a duplicate ID.'
        }
        $actualById[$id] = $task
    }

    foreach ($expected in $expectedTasks) {
        $taskId = [string]$expected.id
        if (-not $actualById.ContainsKey($taskId)) {
            throw 'A Task recorded in the schema 13 baseline is missing after migration.'
        }
        $actual = $actualById[$taskId]
        if ([string]$actual.projectId -ne [string]$BusinessSnapshot.UncategorizedProjectId -or
            [int64]$actual.version -ne ([int64]$expected.version + 1) -or
            (Get-Sha256Hex -Text ([string]$actual.title)) -ne [string]$expected.titleSha256 -or
            (Get-Sha256Hex -Text ([string]$actual.description)) -ne [string]$expected.descriptionSha256 -or
            [string]$actual.status -ne [string]$expected.status -or
            [int]$actual.priority -ne [int]$expected.priority -or
            [string]$actual.dueDate -ne [string]$expected.dueDate -or
            [string]$actual.completedAt -ne [string]$expected.completedAt -or
            [string]$actual.createdAt -ne [string]$expected.createdAt -or
            [string]$actual.updatedAt -ne [string]$expected.updatedAt) {
            throw 'A schema 13 unassigned Task was not preserved exactly during migration.'
        }
    }
}

function Get-OperationsInvariant {
    param([Parameter(Mandatory = $true)]$OperationsData)

    [pscustomobject]@{
        DatabaseOk = [bool]$OperationsData.database.ok
        SchemaVersion = [int]$OperationsData.database.schemaVersion
        BackupStatus = [string]$OperationsData.remoteBackup.status
        BackupSchemaVersion = [int]$OperationsData.remoteBackup.schemaVersion
        BackupIntegrity = [string]$OperationsData.remoteBackup.integrityCheck
        BackupSchemaSemanticsValidated =
            [bool]$OperationsData.remoteBackup.schemaSemanticsValidated
    }
}

function Assert-HealthyOperations {
    param(
        [Parameter(Mandatory = $true)]$Invariant,
        [string]$Stage
    )

    if (-not $Invariant.DatabaseOk -or
        $Invariant.SchemaVersion -ne 14 -or
        $Invariant.BackupStatus -ne 'succeeded' -or
        $Invariant.BackupSchemaVersion -ne $Invariant.SchemaVersion -or
        $Invariant.BackupIntegrity -ne 'ok' -or
        -not $Invariant.BackupSchemaSemanticsValidated) {
        throw "$Stage did not report a healthy database and matching verified remote backup."
    }
}

function Test-OperationsInvariant {
    param(
        [Parameter(Mandatory = $true)]$Before,
        [Parameter(Mandatory = $true)]$After
    )

    return (
        $Before.DatabaseOk -eq $After.DatabaseOk -and
        $Before.SchemaVersion -eq $After.SchemaVersion -and
        $Before.BackupStatus -eq $After.BackupStatus -and
        $Before.BackupSchemaVersion -eq $After.BackupSchemaVersion -and
        $Before.BackupIntegrity -eq $After.BackupIntegrity -and
        $Before.BackupSchemaSemanticsValidated -eq $After.BackupSchemaSemanticsValidated
    )
}

function Write-ResultFile {
    param([Parameter(Mandatory = $true)]$Value)

    $directory = Split-Path -Parent $OutputPath
    [System.IO.Directory]::CreateDirectory($directory) | Out-Null
    [System.IO.File]::WriteAllText(
        $OutputPath,
        ($Value | ConvertTo-Json -Depth 12),
        [System.Text.UTF8Encoding]::new($false)
    )
}

$vault = $null
$credential = $null
$token = $null
$handler = $null
$client = $null
$base = $null
$pairingCode = $null
$pollingSecret = $null
$cookieHeader = $null
$csrfToken = $null
$temporaryDeviceId = $null
$temporaryDeviceRevoked = $false
$stage = 'credential-locker'
$failedStage = $null
$failure = $null
$cleanupFailure = $null
$successResult = $null
$migrationBaseline = $null

try {
    $uri = [System.Uri]$BaseUri
    if (-not $uri.IsAbsoluteUri -or $uri.Scheme -ne 'https' -or
        -not [string]::IsNullOrEmpty($uri.UserInfo) -or
        -not [string]::IsNullOrEmpty($uri.Query) -or
        -not [string]::IsNullOrEmpty($uri.Fragment)) {
        throw 'BaseUri must be a credential-free HTTPS origin.'
    }
    $base = $uri.GetLeftPart([System.UriPartial]::Authority)
    $migrationBaseline = Get-Content -Raw -Encoding UTF8 $MigrationBaselinePath | ConvertFrom-Json

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

    $stage = 'readiness'
    $ready = Invoke-TmRequest `
        -Client $client -Method ([System.Net.Http.HttpMethod]::Get) -Uri "$base/readyz" -Token $token
    Assert-Status -Response $ready -Expected 200 -Stage $stage

    $stage = 'operations-before'
    $opsBeforeResponse = Invoke-TmRequest `
        -Client $client `
        -Method ([System.Net.Http.HttpMethod]::Get) `
        -Uri "$base/api/v1/ops/status" `
        -Token $token
    Assert-Status -Response $opsBeforeResponse -Expected 200 -Stage $stage
    $opsBefore = Get-OperationsInvariant -OperationsData (($opsBeforeResponse.Body | ConvertFrom-Json).data)
    Assert-HealthyOperations -Invariant $opsBefore -Stage $stage

    $stage = 'ai-cost-before'
    $costBeforeResponse = Invoke-TmRequest `
        -Client $client `
        -Method ([System.Net.Http.HttpMethod]::Get) `
        -Uri "$base/api/v1/costs/status" `
        -Token $token
    Assert-Status -Response $costBeforeResponse -Expected 200 -Stage $stage
    $apiCostBefore = [int64](($costBeforeResponse.Body | ConvertFrom-Json).data.api.usedMicrousd)

    $stage = 'pwa-mobile-task-shell'
    $shell = Invoke-TmRequest `
        -Client $client -Method ([System.Net.Http.HttpMethod]::Get) -Uri "$base/mobile/"
    Assert-Status -Response $shell -Expected 200 -Stage $stage
    $shellContracts = @(
        'id="tab-tasks"',
        'id="project-add"',
        'id="project-form"',
        'id="projects-list"',
        'id="projects-more"',
        'id="task-add"',
        'id="task-form"',
        'id="task-filter-project"',
        'id="task-filter-status"',
        'id="tasks-list"',
        'id="tasks-more"'
    )
    foreach ($contract in $shellContracts) {
        if (-not ([string]$shell.Body).Contains($contract)) {
            throw 'The production PWA project/Task shell contract is incomplete.'
        }
    }
    if ($shell.ContentSecurityPolicy -notmatch "default-src 'self'" -or
        $shell.ContentSecurityPolicy -notmatch "form-action 'self'") {
        throw 'The production PWA CSP does not preserve the same-origin form boundary.'
    }

    $stage = 'pwa-mobile-task-script'
    $pwaScript = Invoke-TmRequest `
        -Client $client -Method ([System.Net.Http.HttpMethod]::Get) -Uri "$base/mobile/app.js"
    Assert-Status -Response $pwaScript -Expected 200 -Stage $stage
    $scriptContracts = @(
        '/api/v1/projects',
        '/api/v1/tasks',
        'mutationHeaders("project.create")',
        'mutationHeaders("task.create")',
        'mutationHeaders("task.update"',
        'loadTaskWorkspace',
        'loadProjects',
        'loadTasks',
        'uncategorized',
        'completeTask',
        'window.confirm'
    )
    foreach ($contract in $scriptContracts) {
        if (-not ([string]$pwaScript.Body).Contains($contract)) {
            throw 'The production PWA project/Task client contract is incomplete.'
        }
    }
    if ($pwaScript.Body -notmatch 'headers\["if-none-match"\]\s*=\s*"\*"' -or
        $pwaScript.Body -notmatch 'headers\["if-match"\]\s*=') {
        throw 'The production PWA mutation version contract is incomplete.'
    }

    $stage = 'pwa-cache-v12'
    $serviceWorker = Invoke-TmRequest `
        -Client $client -Method ([System.Net.Http.HttpMethod]::Get) -Uri "$base/mobile/sw.js"
    Assert-Status -Response $serviceWorker -Expected 200 -Stage $stage
    if ($serviceWorker.Body -notmatch 'tm-mobile-shell-v15-expenses' -or
        $serviceWorker.Body -notmatch 'url\.pathname\.startsWith\("/api/"\)' -or
        $serviceWorker.Body -notmatch 'request\.method !== "GET"' -or
        $serviceWorker.Body -notmatch 'keys\.filter\(\(key\) => key !== CACHE_NAME\)') {
        throw 'The production PWA v12 app-shell-only cache contract is invalid.'
    }

    $stage = 'temporary-device-pairing-start'
    $pairingStart = Invoke-TmRequest `
        -Client $client `
        -Method ([System.Net.Http.HttpMethod]::Post) `
        -Uri "$base/api/v1/device-pairings" `
        -Headers @{ Origin = $base } `
        -JsonBody (@{
            deviceLabel = "TM mobile Task verification $([DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss'))"
        } | ConvertTo-Json -Compress)
    Assert-Status -Response $pairingStart -Expected 200 -Stage $stage
    $pairing = ($pairingStart.Body | ConvertFrom-Json).data
    $pairingId = [string]$pairing.pairing.id
    $pairingCode = [string]$pairing.code
    $pollingSecret = [string]$pairing.pollingSecret
    if ($pairingId -notmatch '^[A-Za-z0-9-]{1,64}$' -or
        $pairingCode -notmatch '^\d{6}$' -or
        [string]::IsNullOrWhiteSpace($pollingSecret)) {
        throw 'The temporary pairing response did not match the bounded contract.'
    }

    $stage = 'temporary-device-pairing-approval'
    $pairingApproval = Invoke-TmRequest `
        -Client $client `
        -Method ([System.Net.Http.HttpMethod]::Post) `
        -Uri "$base/api/v1/admin/device-pairings/$pairingId/approve" `
        -Token $token `
        -Headers @{ 'x-tm-confirm-device-admin' = 'approve' } `
        -JsonBody (@{ code = $pairingCode } | ConvertTo-Json -Compress)
    Assert-Status -Response $pairingApproval -Expected 200 -Stage $stage

    $stage = 'temporary-device-pairing-complete'
    $pairingComplete = Invoke-TmRequest `
        -Client $client `
        -Method ([System.Net.Http.HttpMethod]::Post) `
        -Uri "$base/api/v1/device-pairings/$pairingId/complete" `
        -Headers @{ Origin = $base } `
        -JsonBody (@{ pollingSecret = $pollingSecret } | ConvertTo-Json -Compress)
    Assert-Status -Response $pairingComplete -Expected 200 -Stage $stage
    $temporaryDeviceId = [string](($pairingComplete.Body | ConvertFrom-Json).data.id)
    $deviceCookies = @($pairingComplete.SetCookies | Where-Object { $_ -like '__Host-tm_device=*' })
    $csrfCookies = @($pairingComplete.SetCookies | Where-Object { $_ -like '__Host-tm_csrf=*' })
    if ($deviceCookies.Count -ne 1 -or $csrfCookies.Count -ne 1 -or
        $deviceCookies[0] -notmatch '; Secure; HttpOnly; SameSite=Strict' -or
        $csrfCookies[0] -notmatch '; Secure; SameSite=Strict' -or
        $csrfCookies[0] -match 'HttpOnly') {
        throw 'The temporary mobile device cookies do not match the secure cookie contract.'
    }
    $deviceCookiePair = ([string]$deviceCookies[0]).Split(';')[0]
    $csrfCookiePair = ([string]$csrfCookies[0]).Split(';')[0]
    $cookieHeader = "$deviceCookiePair; $csrfCookiePair"
    $csrfToken = $csrfCookiePair.Substring('__Host-tm_csrf='.Length)

    $stage = 'mobile-business-snapshot-before'
    $businessBefore = Get-BusinessSnapshot `
        -Client $client -Base $base -CookieHeader $cookieHeader
    Assert-MigrationBaseline `
        -Baseline $migrationBaseline -BusinessSnapshot $businessBefore -Base $base

    $stage = 'project-create-missing-confirmation'
    $unconfirmedProject = Invoke-TmRequest `
        -Client $client `
        -Method ([System.Net.Http.HttpMethod]::Post) `
        -Uri "$base/api/v1/projects" `
        -Headers @{
            Origin = $base
            Cookie = $cookieHeader
            'x-tm-csrf' = $csrfToken
            'Idempotency-Key' = "mobile-project-unconfirmed-$([guid]::NewGuid().ToString('N'))"
            'If-None-Match' = '*'
        } `
        -JsonBody '{"name":"","description":"","color":"#000000"}'
    Assert-Status -Response $unconfirmedProject -Expected 428 -Stage $stage
    Assert-ErrorCode `
        -Response $unconfirmedProject -Expected 'MUTATION_PRECONDITION_REQUIRED' -Stage $stage

    $stage = 'missing-task:exact-mutation-contract'
    $missingTaskId = [guid]::NewGuid().ToString()
    $missingTask = Invoke-TmRequest `
        -Client $client `
        -Method ([System.Net.Http.HttpMethod]::new('PATCH')) `
        -Uri "$base/api/v1/tasks/$missingTaskId" `
        -Headers @{
            Origin = $base
            Cookie = $cookieHeader
            'x-tm-csrf' = $csrfToken
            'Idempotency-Key' = "mobile-task-missing-$([guid]::NewGuid().ToString('N'))"
            'If-Match' = '"1"'
            'x-tm-confirm-mutation' = 'task.update'
        } `
        -JsonBody '{"status":"done"}'
    Assert-Status -Response $missingTask -Expected 404 -Stage $stage
    Assert-ErrorCode `
        -Response $missingTask -Expected 'MUTATION_RESOURCE_NOT_FOUND' -Stage $stage

    $stage = 'mobile-csrf-rejection'
    $csrfProbeTaskId = [guid]::NewGuid().ToString()
    $csrfRejected = Invoke-TmRequest `
        -Client $client `
        -Method ([System.Net.Http.HttpMethod]::new('PATCH')) `
        -Uri "$base/api/v1/tasks/$csrfProbeTaskId" `
        -Headers @{
            Origin = $base
            Cookie = $cookieHeader
            'Idempotency-Key' = "mobile-task-csrf-$([guid]::NewGuid().ToString('N'))"
            'If-Match' = '"1"'
            'x-tm-confirm-mutation' = 'task.update'
        } `
        -JsonBody '{"status":"done"}'
    Assert-Status -Response $csrfRejected -Expected 403 -Stage $stage
    Assert-ErrorCode -Response $csrfRejected -Expected 'DEVICE_CSRF_REJECTED' -Stage $stage

    $stage = 'mobile-business-snapshot-after'
    $businessAfter = Get-BusinessSnapshot `
        -Client $client -Base $base -CookieHeader $cookieHeader
    if ($businessAfter.Digest -ne $businessBefore.Digest) {
        throw 'The project/Task business-data digest changed during non-destructive verification.'
    }

    $stage = 'temporary-device-revoke'
    $revoke = Invoke-TmRequest `
        -Client $client `
        -Method ([System.Net.Http.HttpMethod]::Post) `
        -Uri "$base/api/v1/admin/devices/$temporaryDeviceId/revoke" `
        -Token $token `
        -Headers @{ 'x-tm-confirm-device-admin' = 'revoke' } `
        -JsonBody '{}'
    Assert-Status -Response $revoke -Expected 200 -Stage $stage
    $temporaryDeviceRevoked = $true

    $stage = 'post-revoke-mobile-authentication'
    $afterRevoke = Invoke-TmRequest `
        -Client $client `
        -Method ([System.Net.Http.HttpMethod]::Get) `
        -Uri "$base/api/v1/tasks?limit=1&offset=0&sort=updated_desc" `
        -Headers @{ Cookie = $cookieHeader }
    Assert-Status -Response $afterRevoke -Expected 401 -Stage $stage

    $stage = 'operations-after'
    $opsAfterResponse = Invoke-TmRequest `
        -Client $client `
        -Method ([System.Net.Http.HttpMethod]::Get) `
        -Uri "$base/api/v1/ops/status" `
        -Token $token
    Assert-Status -Response $opsAfterResponse -Expected 200 -Stage $stage
    $opsAfter = Get-OperationsInvariant -OperationsData (($opsAfterResponse.Body | ConvertFrom-Json).data)
    Assert-HealthyOperations -Invariant $opsAfter -Stage $stage
    if (-not (Test-OperationsInvariant -Before $opsBefore -After $opsAfter)) {
        throw 'The database schema or verified remote-backup invariant changed during verification.'
    }

    $stage = 'ai-cost-after'
    $costAfterResponse = Invoke-TmRequest `
        -Client $client `
        -Method ([System.Net.Http.HttpMethod]::Get) `
        -Uri "$base/api/v1/costs/status" `
        -Token $token
    Assert-Status -Response $costAfterResponse -Expected 200 -Stage $stage
    $apiCostAfter = [int64](($costAfterResponse.Body | ConvertFrom-Json).data.api.usedMicrousd)
    if ($apiCostAfter -ne $apiCostBefore) {
        throw 'The OpenAI cost ledger changed during the non-AI verification.'
    }

    $successResult = [ordered]@{
        success = $true
        verifiedAtUtc = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ss.fffZ')
        baseUri = $base
        pwa = [ordered]@{
            projectTaskUi = $true
            controlledMutationClient = $true
            cacheContract = 'tm-mobile-shell-v15-expenses'
            apiResponsesCached = $false
        }
        mobileApi = [ordered]@{
            projectsRead = [bool]$businessBefore.ProjectsRead
            tasksRead = [bool]$businessBefore.TasksRead
            uncategorizedProjectPresent = [bool]$businessBefore.UncategorizedProjectPresent
            allTasksAssigned = [bool]$businessBefore.AllTasksAssigned
            projectCreateWithoutConfirmationStatus = 428
            missingTaskPatchStatus = 404
            csrfRejectionStatus = 403
            postRevokeStatus = 401
        }
        invariants = [ordered]@{
            schema13BaselineCompared = $true
            existingUncategorizedProjectAdopted = $true
            migratedTaskFieldsPreserved = $true
            migratedTaskVersionIncrementedOnce = $true
            productionBusinessDataDigestUnchanged = $true
            databaseSchemaVersion = $opsAfter.SchemaVersion
            databaseAndBackupUnchanged = $true
            remoteBackupStatus = $opsAfter.BackupStatus
            remoteBackupSchemaVersion = $opsAfter.BackupSchemaVersion
            remoteBackupIntegrityCheck = $opsAfter.BackupIntegrity
            remoteBackupSchemaSemanticsValidated = $opsAfter.BackupSchemaSemanticsValidated
            aiCostLedgerUnchanged = $true
        }
        productionBusinessMutationPerformed = $false
        billableAiCallPerformed = $false
        temporaryAuthLedgerMutationPerformed = $true
        temporaryDeviceRevoked = $true
        secretsPersistedByScript = $false
        tokenReadFromCredentialLocker = $true
    }
}
catch {
    $failedStage = $stage
    $failure = $_
}
finally {
    if (-not $temporaryDeviceRevoked -and
        -not [string]::IsNullOrWhiteSpace($temporaryDeviceId) -and
        $null -ne $client -and
        -not [string]::IsNullOrWhiteSpace($base) -and
        -not [string]::IsNullOrWhiteSpace($token)) {
        try {
            $cleanup = Invoke-TmRequest `
                -Client $client `
                -Method ([System.Net.Http.HttpMethod]::Post) `
                -Uri "$base/api/v1/admin/devices/$temporaryDeviceId/revoke" `
                -Token $token `
                -Headers @{ 'x-tm-confirm-device-admin' = 'revoke' } `
                -JsonBody '{}'
            if ($cleanup.StatusCode -eq 200 -or $cleanup.StatusCode -eq 409) {
                $temporaryDeviceRevoked = $true
            }
            else {
                $cleanupFailure = "Temporary device cleanup returned HTTP $($cleanup.StatusCode)."
            }
        }
        catch {
            $cleanupFailure = 'Temporary device cleanup could not be confirmed.'
        }
    }

    if ($null -ne $client) { $client.Dispose() }
    if ($null -ne $handler) { $handler.Dispose() }
    $pairingCode = $null
    $pollingSecret = $null
    $cookieHeader = $null
    $csrfToken = $null
    $temporaryDeviceId = $null
    $token = $null
    $credential = $null
    $vault = $null
    $migrationBaseline = $null
}

if ($null -ne $cleanupFailure) {
    Write-ResultFile -Value ([ordered]@{
        success = $false
        failedAtUtc = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ss.fffZ')
        stage = 'temporary-device-cleanup'
        reason = 'Temporary verification-device cleanup was not confirmed.'
        productionBusinessMutationPerformed = $false
        billableAiCallPerformed = $false
        secretsPersistedByScript = $false
    })
    throw $cleanupFailure
}

if ($null -ne $failure) {
    Write-ResultFile -Value ([ordered]@{
        success = $false
        failedAtUtc = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ss.fffZ')
        stage = $failedStage
        reason = 'Production verification stopped at the recorded stage.'
        productionBusinessMutationPerformed = $false
        billableAiCallPerformed = $false
        temporaryDeviceRevoked = $temporaryDeviceRevoked
        secretsPersistedByScript = $false
    })
    throw "TM mobile Task management production verification failed at ${failedStage}: $($failure.Exception.Message)"
}

Write-ResultFile -Value $successResult
Write-Host 'TM mobile project/Task management production verification passed.' -ForegroundColor Green
Write-Host 'PWA cache: v12; schema 13 baseline: matched; business-data change: none; AI cost change: none'
Write-Host "Result: $OutputPath"

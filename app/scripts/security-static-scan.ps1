[CmdletBinding()]
param(
    [string]$RepositoryRoot
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if ([string]::IsNullOrWhiteSpace($RepositoryRoot)) {
    $RepositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
}
else {
    $RepositoryRoot = [System.IO.Path]::GetFullPath($RepositoryRoot)
}

$appRoot = Join-Path $RepositoryRoot 'app'
if (-not (Test-Path -LiteralPath (Join-Path $appRoot 'Cargo.lock') -PathType Leaf)) {
    throw 'TM repository root was not found.'
}

$tracked = @(& git -C $RepositoryRoot ls-files --cached --others --exclude-standard)
if ($LASTEXITCODE -ne 0) {
    throw 'git ls-files failed.'
}

$violations = [System.Collections.Generic.List[string]]::new()
$forbiddenNames = @(
    '(^|/)\.env($|\.)',
    '(^|/)(id_rsa|id_ed25519)$',
    '\.(pem|p12|pfx|key)$',
    '(^|/)(credentials|secrets)\.json$'
)
$binaryExtensions = @('.exe', '.dll', '.so', '.dylib', '.png', '.jpg', '.jpeg', '.gif', '.webp', '.ico', '.zip', '.db', '.sqlite', '.sqlite3')
$secretPatterns = [ordered]@{
    'OpenAI API key' = 'sk-(?:proj-)?[A-Za-z0-9_-]{20,}'
    'TM primary token' = 'tm_pat_v1_[A-Za-z0-9_-]{43}'
    'TM device token' = 'tm_dev_v1_[A-Fa-f0-9]{64}'
    'AWS access key' = 'AKIA[0-9A-Z]{16}'
    'GitHub token' = 'gh[pousr]_[A-Za-z0-9]{36,}'
    'private key block' = '-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----'
}

foreach ($relative in $tracked) {
    $normalized = $relative.Replace('\', '/')
    if ($normalized -eq 'app/scripts/security-static-scan.ps1') {
        continue
    }
    foreach ($pattern in $forbiddenNames) {
        if ($normalized -match $pattern -and $normalized -notmatch '\.env\.example$') {
            $violations.Add("forbidden tracked secret file: $normalized")
        }
    }
    if ($binaryExtensions -contains [System.IO.Path]::GetExtension($relative).ToLowerInvariant()) {
        continue
    }
    $fullPath = Join-Path $RepositoryRoot $relative
    if (-not (Test-Path -LiteralPath $fullPath -PathType Leaf)) {
        continue
    }
    try {
        $content = [System.IO.File]::ReadAllText($fullPath, [System.Text.Encoding]::UTF8)
    }
    catch {
        continue
    }
    foreach ($entry in $secretPatterns.GetEnumerator()) {
        if ([System.Text.RegularExpressions.Regex]::IsMatch($content, $entry.Value)) {
            $violations.Add("$($entry.Key) pattern found in $normalized")
        }
    }
}

$workflowRoot = Join-Path $RepositoryRoot '.github\workflows'
if (Test-Path -LiteralPath $workflowRoot -PathType Container) {
    foreach ($workflow in Get-ChildItem -LiteralPath $workflowRoot -File -Include '*.yml', '*.yaml') {
        $content = [System.IO.File]::ReadAllText($workflow.FullName, [System.Text.Encoding]::UTF8)
        if ($content -match '(?m)^\s*pull_request_target\s*:') {
            $violations.Add("pull_request_target is forbidden: $($workflow.Name)")
        }
        foreach ($match in [System.Text.RegularExpressions.Regex]::Matches($content, '(?m)^\s*-?\s*uses:\s*([^\s#]+)')) {
            $reference = $match.Groups[1].Value
            if ($reference.StartsWith('./')) {
                continue
            }
            if ($reference -notmatch '@[0-9a-fA-F]{40}$') {
                $violations.Add("GitHub Action is not pinned to a commit SHA in $($workflow.Name): $reference")
            }
        }
    }
}

$dockerfile = [System.IO.File]::ReadAllText((Join-Path $appRoot 'Dockerfile'), [System.Text.Encoding]::UTF8)
if ($dockerfile -notmatch 'useradd\s+--system\s+--uid\s+10001\s+--create-home\s+tm') {
    $violations.Add('Docker runtime user hardening is missing.')
}
if ($dockerfile -notmatch 'ENTRYPOINT \["/usr/local/bin/railway-entrypoint"\]') {
    $violations.Add('Docker runtime entrypoint is not the reviewed Railway wrapper.')
}

$containerSupplyChainVerifier = Join-Path $appRoot 'scripts\verify-step16-container-supply-chain.ps1'
if (-not (Test-Path -LiteralPath $containerSupplyChainVerifier -PathType Leaf)) {
    $violations.Add('The STEP 16 container supply-chain verifier is missing.')
}
else {
    try {
        & $containerSupplyChainVerifier
    }
    catch {
        $violations.Add("The Dockerfile does not match the reviewed container input lock: $($_.Exception.Message)")
    }
}

$step16Workflow = [System.IO.File]::ReadAllText(
    (Join-Path $RepositoryRoot '.github\workflows\step16-security.yml'),
    [System.Text.Encoding]::UTF8
)
foreach ($required in @(
    'runs-on: ubuntu-24.04',
    'python-version: "3.12.13"',
    'architecture: x64',
    'verify-step16-container-supply-chain.ps1',
    'TM_SOURCE_SHA: ${{ github.event.pull_request.head.sha || github.sha }}',
    'ref: ${{ env.TM_SOURCE_SHA }}',
    'git archive --format=tar',
    '"${TM_SOURCE_SHA}:app"',
    '--label "org.opencontainers.image.revision=${TM_SOURCE_SHA}"',
    '-CommitSha $env:TM_SOURCE_SHA',
    '--platform linux/amd64',
    'requirements-pip-audit-lock.txt',
    'tm-step16-container-provenance',
    'actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a'
)) {
    if (-not $step16Workflow.Contains($required)) {
        $violations.Add("The STEP 16 workflow is missing container provenance control: $required")
    }
}
$containerBuildCommands = [System.Text.RegularExpressions.Regex]::Matches(
    $step16Workflow,
    '(?m)^\s*docker build\s+\\\s*$'
)
if ($containerBuildCommands.Count -ne 1 -or
    $step16Workflow -match '(?m)docker build[^\r\n]*\bapp\s*$' -or
    $step16Workflow -notmatch '"\$\{TM_STEP16_CONTEXT\}"') {
    $violations.Add('STEP 16 must build exactly once from the archived commit context, not the mutable checkout.')
}
if (Test-Path -LiteralPath $containerSupplyChainVerifier -PathType Leaf) {
    $containerSupplyChainSource = [System.IO.File]::ReadAllText(
        $containerSupplyChainVerifier,
        [System.Text.Encoding]::UTF8
    )
    if (-not $containerSupplyChainSource.Contains("kind = 'tm-step16-container-provenance'") -or
        -not $containerSupplyChainSource.Contains('railwayProductionImageRelationship') -or
        -not $containerSupplyChainSource.Contains("receiptRequired = `$true")) {
        $violations.Add('The STEP 16 provenance does not distinguish the CI image from the Railway production rebuild.')
    }
}

$pipAuditLockPath = Join-Path $appRoot 'sidecars\requirements-pip-audit-lock.txt'
if (-not (Test-Path -LiteralPath $pipAuditLockPath -PathType Leaf)) {
    $violations.Add('The hash-locked pip-audit toolchain is missing.')
}
else {
    $pipAuditLock = [System.IO.File]::ReadAllText($pipAuditLockPath, [System.Text.Encoding]::UTF8)
    $pipAuditHashes = [System.Text.RegularExpressions.Regex]::Matches($pipAuditLock, '--hash=sha256:[0-9a-f]{64}')
    if ($pipAuditLock -notmatch '(?m)^pip-audit==2\.10\.0\s+\\$' -or
        $pipAuditHashes.Count -lt 29 -or
        $pipAuditLock -match '(?i)--(?:extra-)?index-url|https?://') {
        $violations.Add('The pip-audit CI toolchain is not fully versioned and hash locked.')
    }
}

$entrypoint = [System.IO.File]::ReadAllText((Join-Path $appRoot 'scripts\railway-entrypoint.sh'), [System.Text.Encoding]::UTF8)
if ($entrypoint -notmatch 'exec\s+gosu\s+tm:tm\s+/usr/local/bin/tm-server' -or
    $entrypoint -notmatch 'gosu\s+tm:tm\s+/usr/local/bin/railway-backup-loop') {
    $violations.Add('The Railway entrypoint does not drop server and backup processes to the tm user.')
}

$changeRequestCloud = [System.IO.File]::ReadAllText(
    (Join-Path $appRoot 'scripts\invoke-change-request-cloud.ps1'),
    [System.Text.Encoding]::UTF8
)
if ($changeRequestCloud -notmatch 'Windows\.Security\.Credentials\.PasswordVault' -or
    $changeRequestCloud -notmatch 'BaseUri must match the configured TM cloud origin' -or
    $changeRequestCloud -notmatch 'x-tm-confirm-desktop-command' -or
    $changeRequestCloud -notmatch 'AllowAutoRedirect\s*=\s*\$false') {
    $violations.Add('The cloud change request client credential and origin safeguards are incomplete.')
}

$desktopCloudClient = [System.IO.File]::ReadAllText(
    (Join-Path $appRoot 'src-tauri\src\cloud_client.rs'),
    [System.Text.Encoding]::UTF8
)
if ($desktopCloudClient -notmatch 'const\s+CREATE_NO_WINDOW:\s*u32\s*=\s*0x0800_0000;' -or
    $desktopCloudClient -notmatch '\.creation_flags\(CREATE_NO_WINDOW\)') {
    $violations.Add('The Windows credential helper must run without allocating a console window.')
}

$databaseSource = [System.IO.File]::ReadAllText((Join-Path $appRoot 'crates\tm-core\src\database.rs'), [System.Text.Encoding]::UTF8)
$schemaMatch = [System.Text.RegularExpressions.Regex]::Match(
    $databaseSource,
    'const\s+SCHEMA_VERSION:\s*i64\s*=\s*(\d+);'
)
$backupOnce = [System.IO.File]::ReadAllText((Join-Path $appRoot 'scripts\railway-backup-once.sh'), [System.Text.Encoding]::UTF8)
if (-not $schemaMatch.Success) {
    $violations.Add('The current database schema version could not be determined.')
}
else {
    $expectedBackupGuard = 'if [ "$schema_version" -lt 1 ] || [ "$schema_version" -gt ' + $schemaMatch.Groups[1].Value + ' ]; then'
    if (-not $backupOnce.Contains($expectedBackupGuard)) {
        $violations.Add('The remote backup schema guard does not match the current database schema version.')
    }
}

$trivyIgnorePath = Join-Path $appRoot '.trivyignore.yaml'
if (-not (Test-Path -LiteralPath $trivyIgnorePath -PathType Leaf)) {
    $violations.Add('The reviewed Trivy exception file is missing.')
}
else {
    $trivyIgnore = [System.IO.File]::ReadAllText($trivyIgnorePath, [System.Text.Encoding]::UTF8)
    $ignoreIds = [System.Text.RegularExpressions.Regex]::Matches($trivyIgnore, '(?m)^\s*-\s+id:\s*(\S+)\s*$')
    if ($ignoreIds.Count -ne 1 -or $ignoreIds[0].Groups[1].Value -ne 'AVD-DS-0002' -or
        $trivyIgnore -notmatch '(?m)^\s+expired_at:\s*2026-10-20\s*$' -or
        $trivyIgnore -notmatch '(?i)Railway entrypoint.+Volume.+gosu') {
        $violations.Add('The Trivy exception must contain only the time-bounded reviewed Railway Volume entrypoint exception.')
    }
}

$expenseRailwayPath = Join-Path $appRoot 'scripts\configure-expense-railway.ps1'
if (-not (Test-Path -LiteralPath $expenseRailwayPath -PathType Leaf)) {
    $violations.Add('The schema 15 expense Railway configuration script is missing.')
}
else {
    $expenseRailway = [System.IO.File]::ReadAllText($expenseRailwayPath, [System.Text.Encoding]::UTF8)
    foreach ($required in @(
        "`$ProjectId = '7fcb22b5-db34-4e2b-a12a-cbc60391ff5f'",
        "`$Environment = 'production'",
        "`$Service = 'tm-server'",
        "`$BaseUri = 'https://tm-server-production-5573.up.railway.app'",
        '[switch]$InitializeNewKey',
        '[switch]$RunGuardSelfTest',
        'expenseLedgerEmpty',
        'expenseKeyInitialized',
        'expenseKeyInitializationAllowed',
        'expenseCryptoReady',
        'Assert-ExpenseKeyFinalProof',
        'Get-CredentialFailureAction',
        'Assert-VerifiedRemoteBackup',
        "'TM_EXPENSE_DATA_KEY_V1', '--stdin', '--skip-deploys'",
        'TM_BUILD_COMMIT_SHA',
        'receiptId',
        'expiresAtUtc',
        'remoteBackupDatabaseSha256',
        'sourceDeploymentRequired = $true',
        '$process.StandardInput.Write($EncodedKey)',
        'recoveryCredentialMatchVerified = $recoveryCredentialMatchVerified',
        'secretValueWrittenToResult = $false'
    )) {
        if (-not $expenseRailway.Contains($required)) {
            $violations.Add("The expense Railway key guard is missing: $required")
        }
    }
    if ($expenseRailway -match 'Set-Clipboard' -or
        $expenseRailway -match 'TM_EXPENSE_DATA_KEY_V1\s*=' -or
        $expenseRailway -match '(?i)Write-(?:Host|Output|Verbose|Debug).{0,80}(?:encodedKey|TM_EXPENSE_DATA_KEY)') {
        $violations.Add('The expense Railway script may expose or place the data key outside stdin.')
    }
}

$expenseDeployPath = Join-Path $appRoot 'scripts\deploy-verified-expense-railway.ps1'
if (-not (Test-Path -LiteralPath $expenseDeployPath -PathType Leaf)) {
    $violations.Add('The verified schema 15 Railway source deployment script is missing.')
}
else {
    $expenseDeploy = [System.IO.File]::ReadAllText($expenseDeployPath, [System.Text.Encoding]::UTF8)
    foreach ($required in @(
        "`$repository = 'junhyeonglee1/TM-private'",
        "`$expectedBranch = 'agent/step10-cloud-cutover'",
        'ExpectedHeadSha',
        "WorkflowName 'STEP 10 Windows build'",
        "WorkflowName 'STEP 16 security'",
        'status --porcelain=v1 --untracked-files=all',
        'ConfigurationResultPath',
        '$git -C $tmRoot archive',
        'Get-DeploymentIdFromUpload',
        'Wait-VerifiedRailwayDeployment',
        'productionImageDigest',
        '$PSCmdlet.ShouldProcess',
        'deploymentId'
    )) {
        if (-not $expenseDeploy.Contains($required)) {
            $violations.Add("The verified Railway source deployment guard is missing: $required")
        }
    }
}

$expenseVerifier = [System.IO.File]::ReadAllText(
    (Join-Path $appRoot 'scripts\verify-expense-production.ps1'),
    [System.Text.Encoding]::UTF8
)
if ($expenseVerifier -notmatch "BaseUri = 'https://tm-server-production-5573\.up\.railway\.app'" -or
    $expenseVerifier -notmatch 'ExpectedDeploymentId' -or
    $expenseVerifier -notmatch 'ExpectedHeadSha' -or
    $expenseVerifier -notmatch 'deploymentProvenance' -or
    $expenseVerifier -notmatch 'latestPreMigrationCreatedAt' -or
    $expenseVerifier -notmatch 'remoteBackup\.checkedAt' -or
    $expenseVerifier -match '(?i)HttpMethod\]::(?:Post|Put|Patch|Delete)' -or
    $expenseVerifier -match '(?i)-Method\s+(?:Post|Put|Patch|Delete)' -or
    $expenseVerifier -match '\[switch\]\$Mutate' -or
    $expenseVerifier -match 'synthetic-recurring-create') {
    $violations.Add('The production expense verifier must remain origin-pinned, backup-aware, and read-only.')
}

$windowsBuildWorkflow = [System.IO.File]::ReadAllText(
    (Join-Path $RepositoryRoot '.github\workflows\windows-step10-build.yml'),
    [System.Text.Encoding]::UTF8
)
if ($windowsBuildWorkflow -notmatch 'configure-expense-railway\.ps1\s+-RunGuardSelfTest') {
    $violations.Add('The Windows workflow does not execute the production expense key guard self-test.')
}
if ($windowsBuildWorkflow -notmatch 'deploy-verified-expense-railway\.ps1[\s\S]{0,300}-RunGuardSelfTest') {
    $violations.Add('The Windows workflow does not execute the verified Railway deployment guard self-test.')
}

$expenseDecryptorSource = [System.IO.File]::ReadAllText(
    (Join-Path $appRoot 'src-tauri\src\expense_decryptor.rs'),
    [System.Text.Encoding]::UTF8
)
if ($expenseDecryptorSource -notmatch 'const\s+CREATE_NO_WINDOW:\s*u32\s*=\s*0x0800_0000;' -or
    $expenseDecryptorSource -notmatch '\.creation_flags\(CREATE_NO_WINDOW\)' -or
    $expenseDecryptorSource -notmatch '\.stdin\(Stdio::piped\(\)\)') {
    $violations.Add('The expense decryptor must receive secrets over stdin without a console window.')
}

$expenseImportSource = [System.IO.File]::ReadAllText(
    (Join-Path $appRoot 'src-tauri\src\expense_import.rs'),
    [System.Text.Encoding]::UTF8
)
if ($expenseImportSource -notmatch 'fn\s+validate_xls_container\(' -or
    $expenseImportSource -notmatch 'fn\s+preflight_xls_container\(' -or
    $expenseImportSource -notmatch 'cfb::CompoundFile::open' -or
    $expenseImportSource -notmatch 'fn\s+validate_biff_workbook_stream\(' -or
    $expenseImportSource -notmatch 'MAX_CFB_ENTRIES\.div_ceil' -or
    $expenseImportSource -match 'compound\.walk\(\)' -or
    $expenseImportSource -notmatch 'XLS external links, embedded objects, and macro projects are not accepted') {
    $violations.Add('Legacy XLS input is missing the bounded raw CFB and BIFF security inspection.')
}

if ($backupOnce -notmatch 'expense_ai_request_bindings' -or
    $backupOnce -notmatch 'idx_expense_rules_classification_unique' -or
    $backupOnce -notmatch 'raw\.source_id <> posting\.source_id' -or
    $backupOnce -notmatch 'link\.posting_id = posting\.id' -or
    $backupOnce -notmatch 'event\.primary_posting_id IS NOT link\.posting_id') {
    $violations.Add('The remote backup script is missing schema 15 ledger and index semantics.')
}

$sidecarSource = [System.IO.File]::ReadAllText(
    (Join-Path $appRoot 'sidecars\tm_office_decryptor.py'),
    [System.Text.Encoding]::UTF8
)
if ($sidecarSource -notmatch 'sys\.stdin\.buffer\.read' -or
    $sidecarSource -notmatch 'io\.BytesIO' -or
    $sidecarSource -match '(?m)^\s*(?:import|from)\s+tempfile\b' -or
    $sidecarSource -match '(?m)^\s*(?:with\s+)?open\s*\(') {
    $violations.Add('The expense workbook sidecar must decrypt through bounded memory-only streams.')
}

$gitIgnore = [System.IO.File]::ReadAllText((Join-Path $RepositoryRoot '.gitignore'), [System.Text.Encoding]::UTF8)
$expenseExampleDirectory = '/' +
    ([string][char]0xAC70) + ([string][char]0xB798) + ([string][char]0xB0B4) + ([string][char]0xC5ED) + ' ' +
    ([string][char]0xC608) + ([string][char]0xC528) + '/'
if (-not $gitIgnore.Contains($expenseExampleDirectory)) {
    $violations.Add('The real expense example directory is not explicitly ignored.')
}

if ($violations.Count -gt 0) {
    $violations | ForEach-Object { Write-Error $_ }
    throw "STEP 16 static security scan failed with $($violations.Count) finding(s)."
}

Write-Host "STEP 16 static security scan passed for $($tracked.Count) repository files." -ForegroundColor Green

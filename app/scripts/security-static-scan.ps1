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
    'test ! -e .git/info/attributes',
    'GIT_ATTR_NOSYSTEM=1 git --no-replace-objects -c core.attributesFile=/dev/null',
    'archive --format=tar',
    '"${TM_SOURCE_SHA}:app"',
    '--label "org.opencontainers.image.revision=${TM_SOURCE_SHA}"',
    '-CommitSha $env:TM_SOURCE_SHA',
    '--platform linux/amd64',
    'requirements-pip-audit-lock.txt',
    'operations-toolchain.lock.json',
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
        'ApprovalReceiptPath',
        'ApprovalNonce',
        '[switch]$RunGuardSelfTest',
        'expenseLedgerEmpty',
        'expenseKeyInitialized',
        'expenseKeyInitializationAllowed',
        'expenseCryptoReady',
        'Assert-ExpenseKeyFinalProof',
        'Get-CredentialFailureAction',
        'Assert-VerifiedRemoteBackup',
        'Get-TmVerifiedActionsArtifactEvidence',
        'Assert-ExpenseKeyApprovalReceipt',
        'Assert-TmPendingReceiptState',
        'Resolve-TmVerifiedGit',
        'Resolve-TmVerifiedGh',
        'Assert-TmSignedToolMatches',
        'Production expense configuration refuses -Force and -Confirm:$false.',
        "'TM_EXPENSE_DATA_KEY_V1', '--stdin', '--skip-deploys'",
        'TM_BUILD_COMMIT_SHA',
        'receiptId',
        'expiresAtUtc',
        'remoteBackupDatabaseSha256',
        'sourceDeploymentRequired = $true',
        'Invoke-TmBoundedProcess',
        'Get-ExpenseDataKeyFingerprint',
        'expenseKeyFingerprint',
        'productionFingerprintComparisonDeferred',
        'recoveryCredentialVaultRoundTripVerified',
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
        'Get-TmVerifiedActionsArtifactEvidence',
        'Assert-TmReleaseEvidenceMatches',
        'Assert-TmPendingReceiptState',
        'Expand-TmSafeSourceArchive',
        'Get-TmDirectoryManifestSha256',
        'final-live-read-only-preflight',
        'Production source deployment refuses -Force and -Confirm:$false.',
        'configurationReceiptStateConsumed',
        'Assert-TmCanonicalGitState',
        'ConfigurationResultPath',
        'Invoke-TmVerifiedGitText',
        'Invoke-TmBoundedProcess',
        'ConvertFrom-TmJsonArrayItems',
        'sourceArchiveSha256',
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

$expenseReleaseEvidencePath = Join-Path $appRoot 'scripts\expense-release-evidence.ps1'
if (-not (Test-Path -LiteralPath $expenseReleaseEvidencePath -PathType Leaf)) {
    $violations.Add('The schema 15 expense release evidence helper is missing.')
}
else {
    $expenseReleaseEvidence = [System.IO.File]::ReadAllText(
        $expenseReleaseEvidencePath,
        [System.Text.Encoding]::UTF8
    )
    foreach ($required in @(
        'dpapi-current-user-v1',
        'DataProtectionScope]::CurrentUser',
        'Get-TmVerifiedActionsArtifactEvidence',
        'run_attempt',
        'artifact.digest',
        'downloadedZipSha256',
        'Invoke-TmGhArtifactZipDownload',
        'StandardOutput.BaseStream.ReadAsync',
        'Expand-TmVerifiedFlatArtifactZip',
        '[System.IO.Compression.ZipArchive]',
        'Read-TmHashManifest',
        'ARTIFACT-SHA256SUMS.txt',
        'Assert-TmReceiptIntegrityProof',
        'Assert-TmPendingReceiptState',
        "[System.IO.File]::Move(`$statePath, `$consumedPath)",
        'Resolve-TmVerifiedRailwayCli',
        'Resolve-TmVerifiedGit',
        'Resolve-TmVerifiedGh',
        'Assert-TmSafeGitEnvironment',
        '--no-replace-objects',
        'GIT_ATTR_NOSYSTEM',
        'Assert-TmZipCentralDirectoryBounds',
        'Invoke-TmBoundedProcess',
        'Get-ExpenseDataKeyFingerprint',
        'Export-TmVerifiedFlatPayload',
        'operations-toolchain.lock.json',
        '68cc3bcdc591289a5c1d5246b76e83081dbf120fd672fbfa2c013de60a10ec52',
        'Get-AuthenticodeSignature'
    )) {
        if (-not $expenseReleaseEvidence.Contains($required)) {
            $violations.Add("The expense release evidence guard is missing: $required")
        }
    }
    if ($expenseReleaseEvidence -match '(?i)gh\s+run\s+download' -or
        $expenseReleaseEvidence -match '(?i)Resolve-Tm(?:Git|Gh)Executable') {
        $violations.Add('The release helper must use raw artifact ZIP streaming and locked Git/GitHub CLI paths.')
    }
}

$buildReleasePath = Join-Path $appRoot 'scripts\build-release.ps1'
if (-not (Test-Path -LiteralPath $buildReleasePath -PathType Leaf)) {
    $violations.Add('The reviewed STEP 10 artifact retrieval script is missing.')
}
else {
    $buildRelease = [System.IO.File]::ReadAllText($buildReleasePath, [System.Text.Encoding]::UTF8)
    foreach ($required in @(
        'expense-release-evidence.ps1',
        'Resolve-TmVerifiedGit',
        'Assert-TmCanonicalGitState',
        'Resolve-TmVerifiedGh',
        'Get-TmVerifiedActionsArtifactEvidence',
        "ExpectedWorkflowPath '.github/workflows/windows-step10-build.yml'",
        "ArtifactKind step10",
        "ArtifactName 'tm-step10-windows-x64'",
        'PayloadDestination',
        'artifactDigest',
        'downloadedZipSha256'
    )) {
        if (-not $buildRelease.Contains($required)) {
            $violations.Add("The STEP 10 artifact retrieval guard is missing: $required")
        }
    }
    if ($buildRelease -match '(?i)Get-Command\s+gh' -or
        $buildRelease -match '(?i)gh\s+run\s+(?:view|download)') {
        $violations.Add('STEP 10 artifact retrieval must not bypass the locked raw-ZIP evidence helper.')
    }
}

$operationsToolchainPath = Join-Path $appRoot 'operations-toolchain.lock.json'
if (-not (Test-Path -LiteralPath $operationsToolchainPath -PathType Leaf)) {
    $violations.Add('The reviewed operations toolchain lock is missing.')
}
else {
    try {
        $operationsToolchain = [System.IO.File]::ReadAllText(
            $operationsToolchainPath,
            [System.Text.Encoding]::UTF8
        ) | ConvertFrom-Json
        if ([int]$operationsToolchain.schemaVersion -ne 1 -or
            [string]$operationsToolchain.railwayCli.version -cne '5.28.1' -or
            [string]$operationsToolchain.railwayCli.sha256 -cne
                '68cc3bcdc591289a5c1d5246b76e83081dbf120fd672fbfa2c013de60a10ec52' -or
            [string]$operationsToolchain.git.relativePath -cne
                '.cache/codex-runtimes/codex-primary-runtime/dependencies/native/git/cmd/git.exe' -or
            [string]$operationsToolchain.githubCli.absolutePath -cne
                'C:\Program Files\GitHub CLI\gh.exe') {
            $violations.Add('The operations toolchain lock does not match the reviewed release policy.')
        }
    }
    catch {
        $violations.Add('The operations toolchain lock is not valid JSON.')
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
    $expenseVerifier -notmatch 'latestPreMigrationSha256' -or
    $expenseVerifier -notmatch 'latestPreMigrationSchemaVersion' -or
    $expenseVerifier -notmatch 'latestPreMigrationIntegrityCheck' -or
    $expenseVerifier -notmatch 'remoteBackup\.checkedAt' -or
    $expenseVerifier -notmatch 'expenseKeyInitializationAllowed' -or
    $expenseVerifier -notmatch 'expenseKeyFingerprint' -or
    $expenseVerifier -notmatch 'expenseRecoveryKeyMatchVerified' -or
    $expenseVerifier -notmatch 'ConvertFrom-TmJsonArrayItems' -or
    $expenseVerifier -notmatch 'Assert-TmReceiptIntegrityProof \$deploymentResult' -or
    $expenseVerifier -notmatch 'Expand-TmSafeSourceArchive' -or
    $expenseVerifier -match '\$\(\$Response\.Body\)' -or
    $expenseVerifier -match '(?i)HttpMethod\]::(?:Post|Put|Patch|Delete)' -or
    $expenseVerifier -match '(?i)-Method\s+(?:Post|Put|Patch|Delete)' -or
    $expenseVerifier -match '\[switch\]\$Mutate' -or
    $expenseVerifier -match 'synthetic-recurring-create') {
    $violations.Add('The production expense verifier must remain origin-pinned, backup-aware, and read-only.')
}

$expenseCryptoSource = [System.IO.File]::ReadAllText(
    (Join-Path $appRoot 'crates\tm-server\src\expense_crypto.rs'),
    [System.Text.Encoding]::UTF8
)
$serverSource = [System.IO.File]::ReadAllText(
    (Join-Path $appRoot 'crates\tm-server\src\lib.rs'),
    [System.Text.Encoding]::UTF8
)
if ($expenseCryptoSource -notmatch 'tm-expense:key-fingerprint:v1\\0' -or
    $expenseCryptoSource -notmatch 'tm_exp_kfp_v1_' -or
    $serverSource -notmatch 'expense_key_fingerprint' -or
    $serverSource -notmatch 'latest_pre_migration_sha256' -or
    $serverSource -notmatch 'latest_pre_migration_schema_semantics_validated') {
    $violations.Add('The server is missing the reviewed expense-key or pre-migration backup proof fields.')
}

$windowsBuildWorkflow = [System.IO.File]::ReadAllText(
    (Join-Path $RepositoryRoot '.github\workflows\windows-step10-build.yml'),
    [System.Text.Encoding]::UTF8
)
if ($windowsBuildWorkflow -notmatch 'configure-expense-railway\.ps1\s+-RunGuardSelfTest') {
    $violations.Add('The Windows workflow does not execute the production expense key guard self-test.')
}
if ($windowsBuildWorkflow -match 'Get-ChildItem[\s\S]{0,200}(?:tm|tm-cli)\.exe' -or
    $windowsBuildWorkflow -notmatch "x86_64-pc-windows-msvc\\release" -or
    $windowsBuildWorkflow -notmatch 'TM_DESKTOP_EXE_SHA256' -or
    $windowsBuildWorkflow -notmatch 'TM_CLI_EXE_SHA256') {
    $violations.Add('The Windows workflow must bind exact target executables from the current run, not cache mtime discovery.')
}
if ($windowsBuildWorkflow -notmatch 'deploy-verified-expense-railway\.ps1[\s\S]{0,300}-RunGuardSelfTest') {
    $violations.Add('The Windows workflow does not execute the verified Railway deployment guard self-test.')
}
if ($windowsBuildWorkflow -notmatch
        'Verify production expense deployment guards on Windows PowerShell 5\.1[\s\S]{0,100}shell:\s*powershell' -or
    $windowsBuildWorkflow -notmatch '\$PSVersionTable\.PSVersion\.Major\s+-ne\s+5') {
    $violations.Add('The Windows workflow does not exercise the Railway JSON array guard on PowerShell 5.1.')
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

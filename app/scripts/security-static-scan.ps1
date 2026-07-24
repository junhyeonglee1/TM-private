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

if ($violations.Count -gt 0) {
    $violations | ForEach-Object { Write-Error $_ }
    throw "STEP 16 static security scan failed with $($violations.Count) finding(s)."
}

Write-Host "STEP 16 static security scan passed for $($tracked.Count) repository files." -ForegroundColor Green

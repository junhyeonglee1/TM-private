$script:TmReceiptProofKind = 'dpapi-current-user-v1'
$script:TmReceiptEntropy = [System.Text.Encoding]::UTF8.GetBytes('TM schema15 expense release receipt v1')
$script:TmOperationsToolchainLockPath = [System.IO.Path]::GetFullPath(
    (Join-Path $PSScriptRoot '..\operations-toolchain.lock.json')
)

function Initialize-TmProtectedData {
    try {
        [void][System.Security.Cryptography.ProtectedData]
        return
    }
    catch {
        try {
            Add-Type -AssemblyName System.Security.Cryptography.ProtectedData
        }
        catch {
            Add-Type -AssemblyName System.Security
        }
    }
}

function ConvertTo-TmCanonicalJsonValue {
    param([AllowNull()]$Value)

    if ($null -eq $Value) { return 'null' }
    if ($Value -is [bool]) {
        if ($Value) { return 'true' }
        return 'false'
    }
    if ($Value -is [string] -or $Value -is [char] -or
        $Value -is [Guid] -or $Value -is [DateTime] -or
        $Value -is [DateTimeOffset] -or $Value -is [Version]) {
        return (ConvertTo-Json -InputObject ([string]$Value) -Compress)
    }
    if ($Value -is [byte] -or $Value -is [sbyte] -or
        $Value -is [int16] -or $Value -is [uint16] -or
        $Value -is [int32] -or $Value -is [uint32] -or
        $Value -is [int64] -or $Value -is [uint64] -or
        $Value -is [decimal] -or $Value -is [double] -or $Value -is [single]) {
        return ([System.Convert]::ToString($Value, [System.Globalization.CultureInfo]::InvariantCulture))
    }
    if ($Value -is [System.Collections.IDictionary]) {
        $parts = [System.Collections.Generic.List[string]]::new()
        foreach ($key in @($Value.Keys | ForEach-Object { [string]$_ } | Sort-Object)) {
            $encodedKey = ConvertTo-Json -InputObject $key -Compress
            $encodedValue = ConvertTo-TmCanonicalJsonValue $Value[$key]
            $parts.Add("${encodedKey}:${encodedValue}")
        }
        return '{' + ($parts -join ',') + '}'
    }
    if ($Value -is [System.Collections.IEnumerable]) {
        $parts = [System.Collections.Generic.List[string]]::new()
        foreach ($item in $Value) {
            $parts.Add((ConvertTo-TmCanonicalJsonValue $item))
        }
        return '[' + ($parts -join ',') + ']'
    }

    $properties = @($Value.PSObject.Properties | Where-Object {
        $_.MemberType -in @('NoteProperty', 'Property', 'AliasProperty', 'ScriptProperty')
    } | Sort-Object Name)
    if ($properties.Count -eq 0) {
        return (ConvertTo-Json -InputObject ([string]$Value) -Compress)
    }
    $parts = [System.Collections.Generic.List[string]]::new()
    foreach ($property in $properties) {
        $encodedKey = ConvertTo-Json -InputObject ([string]$property.Name) -Compress
        $encodedValue = ConvertTo-TmCanonicalJsonValue $property.Value
        $parts.Add("${encodedKey}:${encodedValue}")
    }
    return '{' + ($parts -join ',') + '}'
}

function Get-TmUnsignedReceiptBinding {
    param([Parameter(Mandatory = $true)]$Receipt)

    $unsigned = [ordered]@{}
    if ($Receipt -is [System.Collections.IDictionary]) {
        foreach ($key in $Receipt.Keys) {
            if ([string]$key -ne 'integrityProof') {
                $unsigned[[string]$key] = $Receipt[$key]
            }
        }
    }
    else {
        foreach ($property in $Receipt.PSObject.Properties) {
            if ([string]$property.Name -ne 'integrityProof') {
                $unsigned[[string]$property.Name] = $property.Value
            }
        }
    }
    return ConvertTo-TmCanonicalJsonValue $unsigned
}

function Add-TmReceiptIntegrityProof {
    param([Parameter(Mandatory = $true)][System.Collections.IDictionary]$Receipt)

    if ($Receipt.Contains('integrityProof') -or
        -not $Receipt.Contains('integrityProofKind') -or
        [string]$Receipt.integrityProofKind -cne $script:TmReceiptProofKind) {
        throw 'The release receipt cannot be protected in its current state.'
    }
    Initialize-TmProtectedData
    $plain = [System.Text.Encoding]::UTF8.GetBytes((Get-TmUnsignedReceiptBinding $Receipt))
    try {
        $protected = [System.Security.Cryptography.ProtectedData]::Protect(
            $plain,
            $script:TmReceiptEntropy,
            [System.Security.Cryptography.DataProtectionScope]::CurrentUser
        )
        $Receipt['integrityProof'] = [Convert]::ToBase64String($protected)
    }
    finally {
        if ($null -ne $plain) { [Array]::Clear($plain, 0, $plain.Length) }
        if ($null -ne $protected) { [Array]::Clear($protected, 0, $protected.Length) }
    }
}

function Assert-TmReceiptIntegrityProof {
    param([Parameter(Mandatory = $true)]$Receipt)

    if ([string]$Receipt.integrityProofKind -cne $script:TmReceiptProofKind -or
        [string]$Receipt.integrityProof -notmatch '^[A-Za-z0-9+/]+={0,2}$') {
        throw 'The release receipt has no supported integrity proof.'
    }
    Initialize-TmProtectedData
    try {
        $protected = [Convert]::FromBase64String([string]$Receipt.integrityProof)
        $plain = [System.Security.Cryptography.ProtectedData]::Unprotect(
            $protected,
            $script:TmReceiptEntropy,
            [System.Security.Cryptography.DataProtectionScope]::CurrentUser
        )
        $actual = [System.Text.Encoding]::UTF8.GetString($plain)
    }
    catch {
        throw 'The release receipt integrity proof is invalid for this Windows user.'
    }
    finally {
        if ($null -ne $plain) { [Array]::Clear($plain, 0, $plain.Length) }
        if ($null -ne $protected) { [Array]::Clear($protected, 0, $protected.Length) }
    }
    $expected = Get-TmUnsignedReceiptBinding $Receipt
    if (-not [string]::Equals($actual, $expected, [System.StringComparison]::Ordinal)) {
        throw 'The release receipt was changed after it was approved.'
    }
}

function New-TmApprovalNonce {
    $bytes = [byte[]]::new(16)
    $random = [System.Security.Cryptography.RandomNumberGenerator]::Create()
    try {
        $random.GetBytes($bytes)
        return [Convert]::ToBase64String($bytes).TrimEnd('=').Replace('+', '-').Replace('/', '_')
    }
    finally {
        [Array]::Clear($bytes, 0, $bytes.Length)
        $random.Dispose()
    }
}

function Get-ExpenseDataKeyFingerprint {
    param([Parameter(Mandatory = $true)][string]$EncodedKey)

    if ($EncodedKey -notmatch '^tm_exp_v1_([A-Za-z0-9_-]{43})$') {
        throw 'The expense encryption key cannot be fingerprinted because its format is invalid.'
    }
    $payload = ([string]$Matches[1]).Replace('-', '+').Replace('_', '/') + '='
    $keyBytes = $null
    $domainBytes = $null
    $digest = $null
    $hmac = $null
    try {
        $keyBytes = [Convert]::FromBase64String($payload)
        if ($keyBytes.Length -ne 32 -or @($keyBytes | Where-Object { $_ -ne 0 }).Count -eq 0) {
            throw 'The expense encryption key fingerprint input is invalid.'
        }
        $domainBytes = [System.Text.Encoding]::UTF8.GetBytes("tm-expense:key-fingerprint:v1`0")
        $hmac = [System.Security.Cryptography.HMACSHA256]::new($keyBytes)
        $digest = $hmac.ComputeHash($domainBytes)
        return 'tm_exp_kfp_v1_' + ([BitConverter]::ToString($digest)).Replace('-', '').ToLowerInvariant()
    }
    finally {
        if ($null -ne $digest) { [Array]::Clear($digest, 0, $digest.Length) }
        if ($null -ne $domainBytes) { [Array]::Clear($domainBytes, 0, $domainBytes.Length) }
        if ($null -ne $keyBytes) { [Array]::Clear($keyBytes, 0, $keyBytes.Length) }
        if ($null -ne $hmac) { $hmac.Dispose() }
        $payload = $null
    }
}

function Write-TmJsonNoBom {
    param(
        [Parameter(Mandatory = $true)]$Value,
        [Parameter(Mandatory = $true)][string]$Path,
        [int]$Depth = 12
    )
    $directory = Split-Path -Parent $Path
    New-Item -ItemType Directory -Force -Path $directory | Out-Null
    [System.IO.File]::WriteAllText(
        $Path,
        ($Value | ConvertTo-Json -Depth $Depth),
        [System.Text.UTF8Encoding]::new($false)
    )
}

function Read-TmBoundedJsonFile {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [uint64]$MaximumBytes = 1048576
    )
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw 'The required protected release receipt is missing.'
    }
    $item = Get-Item -LiteralPath $Path
    if ([uint64]$item.Length -lt 2 -or [uint64]$item.Length -gt $MaximumBytes) {
        throw 'The protected release receipt has an invalid size.'
    }
    try {
        return [System.IO.File]::ReadAllText($Path, [System.Text.Encoding]::UTF8) | ConvertFrom-Json
    }
    catch {
        throw 'The protected release receipt is not valid JSON.'
    }
}

function Get-TmReceiptStateRoot {
    if ([string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) {
        throw 'LOCALAPPDATA is unavailable for release receipt replay protection.'
    }
    return Join-Path $env:LOCALAPPDATA 'TM\expense-release-receipts'
}

function Get-TmReceiptStatePath {
    param(
        [Parameter(Mandatory = $true)][ValidateSet('approval', 'configuration')][string]$StateKind,
        [Parameter(Mandatory = $true)][string]$ReceiptId,
        [string]$StateRoot
    )
    $parsed = [Guid]::Empty
    if (-not [Guid]::TryParse($ReceiptId, [ref]$parsed)) {
        throw 'The release receipt ID is invalid.'
    }
    if ([string]::IsNullOrWhiteSpace($StateRoot)) { $StateRoot = Get-TmReceiptStateRoot }
    $root = [System.IO.Path]::GetFullPath($StateRoot)
    return Join-Path $root ("{0}-{1}.pending.dpapi" -f $StateKind, $parsed.ToString('D'))
}

function Read-TmBoundedBinaryFile {
    [OutputType([byte[]])]
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][ValidateRange(1, 1048576)][int]$MaximumBytes
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw 'The protected binary state file is missing.'
    }
    $item = Get-Item -LiteralPath $Path -Force
    if ($item.PSIsContainer -or
        ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0 -or
        [uint64]$item.Length -lt 1 -or
        [uint64]$item.Length -gt [uint64]$MaximumBytes) {
        throw 'The protected binary state file is not a bounded regular file.'
    }

    $stream = $null
    $buffer = $null
    $completed = $false
    try {
        $stream = [System.IO.File]::Open(
            $Path,
            [System.IO.FileMode]::Open,
            [System.IO.FileAccess]::Read,
            [System.IO.FileShare]::None
        )
        $length = [uint64]$stream.Length
        if ($length -lt 1 -or $length -gt [uint64]$MaximumBytes) {
            throw 'The protected binary state changed outside its size bound.'
        }
        $buffer = [byte[]]::new([int]$length)
        $readTotal = 0
        while ($readTotal -lt $buffer.Length) {
            $read = $stream.Read($buffer, $readTotal, $buffer.Length - $readTotal)
            if ($read -le 0) {
                throw 'The protected binary state ended before its declared length.'
            }
            $readTotal += $read
        }
        if ([uint64]$stream.Length -ne $length -or $stream.ReadByte() -ne -1) {
            throw 'The protected binary state changed while it was being read.'
        }
        $completed = $true
        return ,$buffer
    }
    finally {
        if ($null -ne $stream) { $stream.Dispose() }
        if (-not $completed -and $null -ne $buffer) {
            [Array]::Clear($buffer, 0, $buffer.Length)
        }
    }
}

function New-TmPendingReceiptState {
    param(
        [Parameter(Mandatory = $true)][ValidateSet('approval', 'configuration')][string]$StateKind,
        [Parameter(Mandatory = $true)][string]$ReceiptId,
        [Parameter(Mandatory = $true)][string]$ReceiptPath,
        [Parameter(Mandatory = $true)][string]$ExpiresAtUtc,
        [string]$StateRoot
    )
    if (-not (Test-Path -LiteralPath $ReceiptPath -PathType Leaf)) {
        throw 'The protected release receipt file is missing.'
    }
    $receiptSha256 = (Get-FileHash -LiteralPath $ReceiptPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $statePath = Get-TmReceiptStatePath -StateKind $StateKind -ReceiptId $ReceiptId -StateRoot $StateRoot
    $stateDirectory = Split-Path -Parent $statePath
    New-Item -ItemType Directory -Force -Path $stateDirectory | Out-Null
    if (Test-Path -LiteralPath $statePath) {
        throw 'A pending state already exists for this release receipt.'
    }
    $state = [ordered]@{
        stateVersion = 1
        stateKind = $StateKind
        receiptId = $ReceiptId
        receiptSha256 = $receiptSha256
        expiresAtUtc = $ExpiresAtUtc
    }
    $plain = [System.Text.Encoding]::UTF8.GetBytes((ConvertTo-TmCanonicalJsonValue $state))
    $protected = $null
    Initialize-TmProtectedData
    try {
        $protected = [System.Security.Cryptography.ProtectedData]::Protect(
            $plain,
            $script:TmReceiptEntropy,
            [System.Security.Cryptography.DataProtectionScope]::CurrentUser
        )
        $stream = [System.IO.File]::Open(
            $statePath,
            [System.IO.FileMode]::CreateNew,
            [System.IO.FileAccess]::Write,
            [System.IO.FileShare]::None
        )
        try { $stream.Write($protected, 0, $protected.Length) } finally { $stream.Dispose() }
    }
    finally {
        if ($null -ne $plain) { [Array]::Clear($plain, 0, $plain.Length) }
        if ($null -ne $protected) { [Array]::Clear($protected, 0, $protected.Length) }
    }
    return [pscustomobject]@{ Path = $statePath; ReceiptSha256 = $receiptSha256 }
}

function Assert-TmPendingReceiptState {
    param(
        [Parameter(Mandatory = $true)][ValidateSet('approval', 'configuration')][string]$StateKind,
        [Parameter(Mandatory = $true)][string]$ReceiptId,
        [Parameter(Mandatory = $true)][string]$ReceiptPath,
        [string]$StateRoot,
        [switch]$Consume,
        [switch]$AllowExpired
    )
    $statePath = Get-TmReceiptStatePath -StateKind $StateKind -ReceiptId $ReceiptId -StateRoot $StateRoot
    if (-not (Test-Path -LiteralPath $statePath -PathType Leaf)) {
        throw 'The release receipt is missing its pending single-use state or was already consumed.'
    }
    $protected = Read-TmBoundedBinaryFile -Path $statePath -MaximumBytes 65536
    $plain = $null
    Initialize-TmProtectedData
    try {
        $plain = [System.Security.Cryptography.ProtectedData]::Unprotect(
            $protected,
            $script:TmReceiptEntropy,
            [System.Security.Cryptography.DataProtectionScope]::CurrentUser
        )
        $state = [System.Text.Encoding]::UTF8.GetString($plain) | ConvertFrom-Json
    }
    catch {
        throw 'The release receipt pending state is invalid for this Windows user.'
    }
    finally {
        if ($null -ne $plain) { [Array]::Clear($plain, 0, $plain.Length) }
        if ($null -ne $protected) { [Array]::Clear($protected, 0, $protected.Length) }
    }
    $receiptSha256 = (Get-FileHash -LiteralPath $ReceiptPath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ([int]$state.stateVersion -ne 1 -or
        [string]$state.stateKind -cne $StateKind -or
        [string]$state.receiptId -cne $ReceiptId -or
        [string]$state.receiptSha256 -cne $receiptSha256) {
        throw 'The release receipt does not match its protected single-use state.'
    }
    $expiresAt = [DateTimeOffset]::MinValue
    if (-not [DateTimeOffset]::TryParse(
        [string]$state.expiresAtUtc,
        [System.Globalization.CultureInfo]::InvariantCulture,
        [System.Globalization.DateTimeStyles]::RoundtripKind,
        [ref]$expiresAt
    ) -or (-not $AllowExpired -and
        $expiresAt.ToUniversalTime() -lt [DateTimeOffset]::UtcNow)) {
        throw 'The release receipt pending state has expired.'
    }
    if ($Consume) {
        $consumedPath = $statePath -replace '\.pending\.dpapi$', '.consumed.dpapi'
        if (Test-Path -LiteralPath $consumedPath) {
            throw 'The release receipt was already consumed.'
        }
        [System.IO.File]::Move($statePath, $consumedPath)
    }
    return $receiptSha256
}

function Assert-TmConsumedReceiptState {
    param(
        [Parameter(Mandatory = $true)][ValidateSet('approval', 'configuration')][string]$StateKind,
        [Parameter(Mandatory = $true)][string]$ReceiptId,
        [Parameter(Mandatory = $true)][string]$ReceiptPath,
        [string]$StateRoot
    )
    $pendingPath = Get-TmReceiptStatePath -StateKind $StateKind -ReceiptId $ReceiptId `
        -StateRoot $StateRoot
    $statePath = $pendingPath -replace '\.pending\.dpapi$', '.consumed.dpapi'
    if (-not (Test-Path -LiteralPath $statePath -PathType Leaf)) {
        throw 'The release receipt has no protected consumed state.'
    }
    $protected = Read-TmBoundedBinaryFile -Path $statePath -MaximumBytes 65536
    $plain = $null
    Initialize-TmProtectedData
    try {
        $plain = [System.Security.Cryptography.ProtectedData]::Unprotect(
            $protected,
            $script:TmReceiptEntropy,
            [System.Security.Cryptography.DataProtectionScope]::CurrentUser
        )
        $state = [System.Text.Encoding]::UTF8.GetString($plain) | ConvertFrom-Json
    }
    catch {
        throw 'The release receipt consumed state is invalid for this Windows user.'
    }
    finally {
        if ($null -ne $plain) { [Array]::Clear($plain, 0, $plain.Length) }
        if ($null -ne $protected) { [Array]::Clear($protected, 0, $protected.Length) }
    }
    $receiptSha256 = (Get-FileHash -LiteralPath $ReceiptPath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ([int]$state.stateVersion -ne 1 -or
        [string]$state.stateKind -cne $StateKind -or
        [string]$state.receiptId -cne $ReceiptId -or
        [string]$state.receiptSha256 -cne $receiptSha256) {
        throw 'The release receipt does not match its protected consumed state.'
    }
    return $receiptSha256
}

function Assert-TmExactJsonProperties {
    param(
        [Parameter(Mandatory = $true)]$Value,
        [Parameter(Mandatory = $true)][string[]]$Names,
        [Parameter(Mandatory = $true)][string]$Label
    )
    if ($null -eq $Value) { throw "$Label is missing." }
    $actual = @($Value.PSObject.Properties.Name | Sort-Object)
    $expected = @($Names | Sort-Object)
    if (($actual -join "`n") -cne ($expected -join "`n")) {
        throw "$Label has unexpected or missing properties."
    }
}

function Read-TmOperationsToolchainLock {
    if (-not (Test-Path -LiteralPath $script:TmOperationsToolchainLockPath -PathType Leaf)) {
        throw 'The reviewed operations toolchain lock is missing.'
    }
    $item = Get-Item -LiteralPath $script:TmOperationsToolchainLockPath
    if ($item.Length -lt 2 -or $item.Length -gt 16384) {
        throw 'The reviewed operations toolchain lock has an invalid size.'
    }
    try {
        $lock = [System.IO.File]::ReadAllText(
            $script:TmOperationsToolchainLockPath,
            [System.Text.Encoding]::UTF8
        ) | ConvertFrom-Json
    }
    catch { throw 'The reviewed operations toolchain lock is not valid JSON.' }
    Assert-TmExactJsonProperties $lock @('schemaVersion', 'railwayCli', 'git', 'githubCli') 'toolchain lock'
    Assert-TmExactJsonProperties $lock.railwayCli `
        @('packageName', 'version', 'sha256', 'installKind', 'authenticodePolicy') `
        'toolchain lock railwayCli'
    Assert-TmExactJsonProperties $lock.git `
        @('installKind', 'relativePath', 'authenticodeStatus', 'signerOrganization') `
        'toolchain lock git'
    Assert-TmExactJsonProperties $lock.githubCli `
        @('installKind', 'absolutePath', 'authenticodeStatus', 'signerOrganization') `
        'toolchain lock githubCli'
    if ([int]$lock.schemaVersion -ne 1 -or
        [string]$lock.railwayCli.packageName -cne '@railway/cli' -or
        [string]$lock.railwayCli.version -cne '5.28.1' -or
        [string]$lock.railwayCli.sha256 -cne '68cc3bcdc591289a5c1d5246b76e83081dbf120fd672fbfa2c013de60a10ec52' -or
        [string]$lock.railwayCli.installKind -cne 'official-pnpm-store' -or
        [string]$lock.railwayCli.authenticodePolicy -cne 'allow-unsigned-exact-sha256' -or
        [string]$lock.git.installKind -cne 'codex-bundled-runtime' -or
        [string]$lock.git.relativePath -cne '.cache/codex-runtimes/codex-primary-runtime/dependencies/native/git/cmd/git.exe' -or
        [string]$lock.git.authenticodeStatus -cne 'Valid' -or
        [string]$lock.git.signerOrganization -cne 'Johannes Schindelin' -or
        [string]$lock.githubCli.installKind -cne 'official-program-files' -or
        [string]$lock.githubCli.absolutePath -cne 'C:\Program Files\GitHub CLI\gh.exe' -or
        [string]$lock.githubCli.authenticodeStatus -cne 'Valid' -or
        [string]$lock.githubCli.signerOrganization -cne 'GitHub, Inc.') {
        throw 'The reviewed operations toolchain lock does not match schema version 1 policy.'
    }
    return $lock
}

function Resolve-TmVerifiedGit {
    $lock = Read-TmOperationsToolchainLock
    $relative = ([string]$lock.git.relativePath) -replace '/', '\'
    $path = [System.IO.Path]::GetFullPath((Join-Path $env:USERPROFILE $relative))
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw 'The signed Codex-bundled Git executable was not found.'
    }
    $signature = Get-AuthenticodeSignature -LiteralPath $path
    if ([string]$signature.Status -cne 'Valid' -or
        $null -eq $signature.SignerCertificate -or
        [string]$signature.SignerCertificate.Subject -notmatch 'O=Johannes Schindelin(?:,|$)') {
        throw 'The Codex-bundled Git executable has an unexpected signature.'
    }
    $versionResult = Invoke-TmBoundedProcess -FilePath $path `
        -Arguments ([string[]]@('--version')) -TimeoutSeconds 15 -MaximumCapturedCharacters 16384
    $versionOutput = @(
        [string]$versionResult.StandardOutput
        [string]$versionResult.StandardError
    ) | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }
    $versionText = (@($versionOutput) -join "`n").Trim()
    if ($versionResult.ExitCode -ne 0 -or $versionText -notmatch '^git version [0-9]+\.[0-9]+\.[0-9]+') {
        throw 'The Codex-bundled Git executable did not report a valid version.'
    }
    return [pscustomobject]@{
        path = $path
        version = ($versionText -replace '^git version\s+', '').Trim()
        sha256 = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
        authenticodeStatus = [string]$signature.Status
        signerSubject = [string]$signature.SignerCertificate.Subject
        installKind = [string]$lock.git.installKind
    }
}

function Assert-TmSafeGitEnvironment {
    $allowed = @(
        'GIT_ALLOW_PROTOCOLS',
        'GIT_HTTP_PROXY',
        'GIT_HTTPS_PROXY',
        'GIT_PAGER',
        'GIT_SSH_COMMAND',
        'GIT_TERMINAL_PROMPT'
    )
    $unsafe = @(
        [System.Environment]::GetEnvironmentVariables().Keys | ForEach-Object {
            [string]$_
        } | Where-Object {
            $_ -match '^(?i:GIT_)' -and $_.ToUpperInvariant() -notin $allowed -and
            -not [string]::IsNullOrEmpty([string][System.Environment]::GetEnvironmentVariable($_))
        }
    )
    if ($unsafe.Count -ne 0) {
        throw 'Release Git operations refuse repository, object, replace-ref, or config environment overrides.'
    }
}

function Invoke-TmVerifiedGitText {
    param(
        [Parameter(Mandatory = $true)][string]$GitPath,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [Parameter(Mandatory = $true)][string]$FailureMessage,
        [switch]$AllowEmpty,
        [switch]$AllowExitCodeOne
    )
    $previousGitAttrNoSystem = [System.Environment]::GetEnvironmentVariable('GIT_ATTR_NOSYSTEM')
    try {
        [System.Environment]::SetEnvironmentVariable('GIT_ATTR_NOSYSTEM', '1')
        $output = @(& $GitPath --no-replace-objects @Arguments 2>&1)
        $exitCode = $LASTEXITCODE
    }
    finally {
        [System.Environment]::SetEnvironmentVariable('GIT_ATTR_NOSYSTEM', $previousGitAttrNoSystem)
    }
    if (($exitCode -ne 0 -and -not ($AllowExitCodeOne -and $exitCode -eq 1)) -or
        (-not $AllowEmpty -and $output.Count -eq 0)) {
        throw $FailureMessage
    }
    return $output
}

function Assert-TmCanonicalGitState {
    param(
        [Parameter(Mandatory = $true)][string]$GitPath,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot,
        [Parameter(Mandatory = $true)][string]$Repository,
        [Parameter(Mandatory = $true)][string]$Branch,
        [Parameter(Mandatory = $true)][string]$ExpectedHeadSha
    )
    Assert-TmSafeGitEnvironment
    $root = [System.IO.Path]::GetFullPath($RepositoryRoot).TrimEnd('\')
    $expectedGitDirectory = [System.IO.Path]::GetFullPath((Join-Path $root '.git')).TrimEnd('\')
    $rootItem = Get-Item -LiteralPath $root
    $gitItem = Get-Item -LiteralPath $expectedGitDirectory
    if (($rootItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0 -or
        -not $gitItem.PSIsContainer -or
        ($gitItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0 -or
        (Test-Path -LiteralPath (Join-Path $expectedGitDirectory 'objects\info\alternates')) -or
        (Test-Path -LiteralPath (Join-Path $expectedGitDirectory 'objects\info\http-alternates')) -or
        (Test-Path -LiteralPath (Join-Path $expectedGitDirectory 'info\attributes'))) {
        throw 'Release approval requires a local, non-reparse Git repository without external object or attributes files.'
    }

    $prefix = @('-C', $root)
    $origin = ((Invoke-TmVerifiedGitText -GitPath $GitPath `
        -Arguments ($prefix + @('config', '--local', '--get', 'remote.origin.url')) `
        -FailureMessage 'Unable to inspect the canonical Git origin.') -join "`n").Trim()
    $currentBranch = ((Invoke-TmVerifiedGitText -GitPath $GitPath `
        -Arguments ($prefix + @('branch', '--show-current')) `
        -FailureMessage 'Unable to inspect the canonical Git branch.') -join "`n").Trim()
    $headSha = ((Invoke-TmVerifiedGitText -GitPath $GitPath `
        -Arguments ($prefix + @('rev-parse', 'HEAD^{commit}')) `
        -FailureMessage 'Unable to inspect the canonical Git commit.') -join "`n").Trim().ToLowerInvariant()
    $topLevel = ((Invoke-TmVerifiedGitText -GitPath $GitPath `
        -Arguments ($prefix + @('rev-parse', '--show-toplevel')) `
        -FailureMessage 'Unable to inspect the canonical Git top-level.') -join "`n").Trim()
    $gitDirectory = ((Invoke-TmVerifiedGitText -GitPath $GitPath `
        -Arguments ($prefix + @('rev-parse', '--absolute-git-dir')) `
        -FailureMessage 'Unable to inspect the canonical Git directory.') -join "`n").Trim()
    $commonDirectory = ((Invoke-TmVerifiedGitText -GitPath $GitPath `
        -Arguments ($prefix + @('rev-parse', '--path-format=absolute', '--git-common-dir')) `
        -FailureMessage 'Unable to inspect the canonical Git common directory.') -join "`n").Trim()
    $replacementRefs = @(Invoke-TmVerifiedGitText -GitPath $GitPath `
        -Arguments ($prefix + @('replace', '-l')) `
        -FailureMessage 'Unable to inspect Git replacement refs.' -AllowEmpty)
    $externalAttributeFiles = @(Invoke-TmVerifiedGitText -GitPath $GitPath `
        -Arguments ($prefix + @('config', '--show-origin', '--get-all', 'core.attributesFile')) `
        -FailureMessage 'Unable to inspect external Git attributes configuration.' `
        -AllowEmpty -AllowExitCodeOne)
    $workingTree = @(Invoke-TmVerifiedGitText -GitPath $GitPath `
        -Arguments ($prefix + @('status', '--porcelain=v1', '--untracked-files=all')) `
        -FailureMessage 'Unable to inspect the canonical Git worktree.' -AllowEmpty)
    $normalizedOrigin = $origin -replace '\.git$', ''
    if ($normalizedOrigin -cne "https://github.com/$Repository" -or
        $currentBranch -cne $Branch -or
        $headSha -cne $ExpectedHeadSha.ToLowerInvariant() -or
        -not [string]::Equals(
            [System.IO.Path]::GetFullPath($topLevel).TrimEnd('\'),
            $root,
            [System.StringComparison]::OrdinalIgnoreCase
        ) -or
        -not [string]::Equals(
            [System.IO.Path]::GetFullPath($gitDirectory).TrimEnd('\'),
            $expectedGitDirectory,
            [System.StringComparison]::OrdinalIgnoreCase
        ) -or
        -not [string]::Equals(
            [System.IO.Path]::GetFullPath($commonDirectory).TrimEnd('\'),
            $expectedGitDirectory,
            [System.StringComparison]::OrdinalIgnoreCase
        ) -or
        $replacementRefs.Count -ne 0 -or
        $externalAttributeFiles.Count -ne 0 -or
        $workingTree.Count -ne 0) {
        throw 'Release approval requires the clean canonical branch at the exact expected commit.'
    }
    return [pscustomobject]@{ Origin = $normalizedOrigin; Branch = $currentBranch; HeadSha = $headSha }
}

function Resolve-TmVerifiedGh {
    $lock = Read-TmOperationsToolchainLock
    $path = [System.IO.Path]::GetFullPath([string]$lock.githubCli.absolutePath)
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw 'The official GitHub CLI executable was not found under Program Files.'
    }
    $signature = Get-AuthenticodeSignature -LiteralPath $path
    if ([string]$signature.Status -cne 'Valid' -or
        $null -eq $signature.SignerCertificate -or
        [string]$signature.SignerCertificate.Subject -notmatch 'O="?GitHub, Inc\."?(?:,|$)') {
        throw 'The GitHub CLI executable has an unexpected Authenticode signer.'
    }
    $versionResult = Invoke-TmBoundedProcess -FilePath $path `
        -Arguments ([string[]]@('--version')) -TimeoutSeconds 15 -MaximumCapturedCharacters 16384
    $versionOutput = @(
        [string]$versionResult.StandardOutput
        [string]$versionResult.StandardError
    ) | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }
    $versionText = (@($versionOutput) -join "`n").Trim()
    $versionMatch = [regex]::Match($versionText, '^gh version ([0-9]+\.[0-9]+\.[0-9]+)')
    if ($versionResult.ExitCode -ne 0 -or -not $versionMatch.Success) {
        throw 'The GitHub CLI executable did not report a valid version.'
    }
    $version = [string]$versionMatch.Groups[1].Value
    return [pscustomobject]@{
        path = $path
        version = $version
        sha256 = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
        authenticodeStatus = [string]$signature.Status
        signerSubject = [string]$signature.SignerCertificate.Subject
        installKind = [string]$lock.githubCli.installKind
    }
}

function Invoke-TmGhJson {
    param(
        [Parameter(Mandatory = $true)][string]$GhPath,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [Parameter(Mandatory = $true)][string]$FailureMessage
    )
    $output = & $GhPath @Arguments 2>&1
    if ($LASTEXITCODE -ne 0) { throw $FailureMessage }
    try { return $output | ConvertFrom-Json } catch { throw $FailureMessage }
}

function Read-TmHashManifest {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][string]$ManifestName,
        [Parameter(Mandatory = $true)][string[]]$RequiredFiles,
        [switch]$AllowDotPrefix
    )
    $manifestPath = Join-Path $Root $ManifestName
    if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
        throw "The Actions artifact does not contain $ManifestName."
    }
    if ((Get-Item -LiteralPath $manifestPath).Length -gt 1048576) {
        throw 'The Actions artifact hash manifest is too large.'
    }
    $manifest = @{}
    foreach ($line in Get-Content -LiteralPath $manifestPath) {
        if ($line -notmatch '^([0-9a-fA-F]{64})\s{2}(.+)$') {
            throw 'The Actions artifact hash manifest contains an invalid entry.'
        }
        $expectedHash = ([string]$Matches[1]).ToLowerInvariant()
        $name = [string]$Matches[2]
        if ($AllowDotPrefix -and $name.StartsWith('./')) { $name = $name.Substring(2) }
        if (($name -cne '.dockerignore' -and
                $name -notmatch '^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$') -or
            $name -eq $ManifestName -or $manifest.ContainsKey($name)) {
            throw 'The Actions artifact hash manifest contains an unsafe or duplicate file name.'
        }
        $manifest[$name] = $expectedHash
    }
    foreach ($name in $RequiredFiles) {
        if (-not $manifest.ContainsKey($name)) {
            throw "The Actions artifact hash manifest does not contain $name."
        }
    }
    $actualFiles = @(
        Get-ChildItem -LiteralPath $Root -File -Force -Recurse | ForEach-Object {
            if (($_.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw 'The Actions artifact contains a reparse point.'
            }
            $relative = $_.FullName.Substring(([System.IO.Path]::GetFullPath($Root)).Length)
            $relative = $relative.TrimStart([char[]]@('\', '/')) -replace '\\', '/'
            if ($relative -match '/') { throw 'The Actions artifact contains an unexpected nested file.' }
            $relative
        }
    )
    $expectedFiles = @($manifest.Keys) + $ManifestName
    $actualFileList = (@($actualFiles | Sort-Object) -join "`n")
    $expectedFileList = (@($expectedFiles | Sort-Object) -join "`n")
    if ($actualFileList -cne $expectedFileList) {
        throw 'The Actions artifact files do not exactly match their hash manifest.'
    }
    $fileHashes = [ordered]@{}
    foreach ($name in @($manifest.Keys | Sort-Object)) {
        $path = Join-Path $Root $name
        $item = Get-Item -LiteralPath $path
        if ($item.Length -lt 1 -or $item.Length -gt 536870912) {
            throw "The Actions artifact file has an invalid size: $name"
        }
        $actual = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($actual -cne [string]$manifest[$name]) {
            throw "The Actions artifact SHA-256 verification failed: $name"
        }
        $fileHashes[$name] = $actual
    }
    return [pscustomobject]@{
        ManifestSha256 = (Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash.ToLowerInvariant()
        FileHashes = $fileHashes
    }
}

function Export-TmVerifiedFlatPayload {
    param(
        [Parameter(Mandatory = $true)][string]$SourceRoot,
        [Parameter(Mandatory = $true)][string]$DestinationRoot,
        [Parameter(Mandatory = $true)][string[]]$Files,
        [Parameter(Mandatory = $true)][string]$ManifestName,
        [Parameter(Mandatory = $true)]$Manifest
    )
    $source = [System.IO.Path]::GetFullPath($SourceRoot).TrimEnd('\') + '\'
    $destination = [System.IO.Path]::GetFullPath($DestinationRoot).TrimEnd('\') + '\'
    if (Test-Path -LiteralPath $destination.TrimEnd('\')) {
        throw 'The verified payload export destination already exists.'
    }
    $expected = [System.Collections.Generic.HashSet[string]]::new(
        [System.StringComparer]::OrdinalIgnoreCase
    )
    foreach ($name in $Files) {
        if ($name.Contains('\') -or $name.Contains('/') -or -not $expected.Add($name)) {
            throw 'The verified payload export allowlist is invalid.'
        }
    }
    if ($expected.Count -lt 1 -or -not $expected.Contains($ManifestName)) {
        throw 'The verified payload export is missing its manifest.'
    }

    try {
        New-Item -ItemType Directory -Path $destination.TrimEnd('\') -Force | Out-Null
        foreach ($name in $Files) {
            $sourcePath = [System.IO.Path]::GetFullPath((Join-Path $source $name))
            $destinationPath = [System.IO.Path]::GetFullPath((Join-Path $destination $name))
            if (-not $sourcePath.StartsWith($source, [System.StringComparison]::OrdinalIgnoreCase) -or
                -not $destinationPath.StartsWith($destination, [System.StringComparison]::OrdinalIgnoreCase) -or
                -not (Test-Path -LiteralPath $sourcePath -PathType Leaf)) {
                throw 'The verified payload export source is invalid.'
            }
            [System.IO.File]::Copy($sourcePath, $destinationPath, $false)
            $actual = (Get-FileHash -LiteralPath $destinationPath -Algorithm SHA256).Hash.ToLowerInvariant()
            $expectedHash = if ($name -ceq $ManifestName) {
                [string]$Manifest.ManifestSha256
            }
            else {
                [string]$Manifest.FileHashes[$name]
            }
            if ($expectedHash -notmatch '^[0-9a-f]{64}$' -or $actual -cne $expectedHash) {
                throw 'A copied Actions artifact file failed its verified SHA-256 binding.'
            }
        }
        $exported = @(Get-ChildItem -LiteralPath $destination.TrimEnd('\') -File -Force -Recurse)
        if ($exported.Count -ne $expected.Count -or @($exported | Where-Object {
                    -not $expected.Contains($_.Name) -or
                    -not [string]::Equals(
                        $_.DirectoryName,
                        $destination.TrimEnd('\'),
                        [System.StringComparison]::OrdinalIgnoreCase
                    )
                }).Count -ne 0) {
            throw 'The verified payload export contains an unexpected file.'
        }
    }
    catch {
        if (Test-Path -LiteralPath $destination.TrimEnd('\') -PathType Container) {
            Remove-Item -LiteralPath $destination.TrimEnd('\') -Recurse -Force
        }
        throw
    }
}

function Invoke-TmGhArtifactZipDownload {
    param(
        [Parameter(Mandatory = $true)][string]$GhPath,
        [Parameter(Mandatory = $true)][string]$Repository,
        [Parameter(Mandatory = $true)][long]$ArtifactId,
        [Parameter(Mandatory = $true)][string]$DestinationPath,
        [Parameter(Mandatory = $true)][uint64]$MaximumBytes
    )
    if ($Repository -notmatch '^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$' -or $ArtifactId -lt 1) {
        throw 'The GitHub artifact download target is invalid.'
    }
    if (Test-Path -LiteralPath $DestinationPath) {
        throw 'The GitHub artifact ZIP destination already exists.'
    }
    $arguments = "api --method GET --hostname github.com -H Accept:application/vnd.github+json -H X-GitHub-Api-Version:2022-11-28 repos/$Repository/actions/artifacts/$ArtifactId/zip"
    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $GhPath
    $startInfo.Arguments = $arguments
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.EnvironmentVariables.Remove('GH_DEBUG')
    $process = [System.Diagnostics.Process]::new()
    $file = $null
    $stderrTask = $null
    $buffer = [byte[]]::new(65536)
    $written = [uint64]0
    $deadline = [DateTimeOffset]::UtcNow.AddMinutes(5)
    try {
        $process.StartInfo = $startInfo
        if (-not $process.Start()) { throw 'GitHub CLI could not start the artifact download.' }
        $stderrTask = $process.StandardError.ReadToEndAsync()
        $file = [System.IO.File]::Open(
            $DestinationPath,
            [System.IO.FileMode]::CreateNew,
            [System.IO.FileAccess]::Write,
            [System.IO.FileShare]::None
        )
        while ($true) {
            $readTask = $process.StandardOutput.BaseStream.ReadAsync($buffer, 0, $buffer.Length)
            while (-not $readTask.Wait(1000)) {
                if ([DateTimeOffset]::UtcNow -ge $deadline) {
                    try { $process.Kill() } catch { }
                    throw 'GitHub artifact download exceeded the fixed timeout.'
                }
            }
            $read = [int]$readTask.Result
            if ($read -eq 0) { break }
            $written += [uint64]$read
            if ($written -gt $MaximumBytes) {
                try { $process.Kill() } catch { }
                throw 'GitHub artifact ZIP exceeded the approved size limit.'
            }
            $file.Write($buffer, 0, $read)
        }
        $file.Flush($true)
        $file.Dispose()
        $file = $null
        if (-not $process.WaitForExit(30000)) {
            try { $process.Kill() } catch { }
            throw 'GitHub artifact download did not terminate cleanly.'
        }
        $null = $stderrTask.GetAwaiter().GetResult()
        if ($process.ExitCode -ne 0 -or $written -lt 1) {
            throw 'GitHub CLI rejected the artifact ZIP download.'
        }
    }
    catch {
        if ($null -ne $file) { $file.Dispose(); $file = $null }
        if (Test-Path -LiteralPath $DestinationPath -PathType Leaf) {
            Remove-Item -LiteralPath $DestinationPath -Force
        }
        throw
    }
    finally {
        [Array]::Clear($buffer, 0, $buffer.Length)
        if ($null -ne $file) { $file.Dispose() }
        $process.Dispose()
        $startInfo = $null
    }
}

function Read-TmExactStreamBytes {
    param(
        [Parameter(Mandatory = $true)][System.IO.Stream]$Stream,
        [Parameter(Mandatory = $true)][uint64]$Offset,
        [Parameter(Mandatory = $true)][int]$Count
    )
    if ($Count -lt 1 -or $Offset -gt [uint64][long]::MaxValue -or
        $Offset -gt [uint64]$Stream.Length -or
        [uint64]$Count -gt ([uint64]$Stream.Length - $Offset)) {
        throw 'The ZIP metadata points outside the bounded archive.'
    }
    $buffer = [byte[]]::new($Count)
    $Stream.Position = [long]$Offset
    $readTotal = 0
    while ($readTotal -lt $Count) {
        $read = $Stream.Read($buffer, $readTotal, $Count - $readTotal)
        if ($read -le 0) { throw 'The ZIP metadata ended unexpectedly.' }
        $readTotal += $read
    }
    return ,$buffer
}

function Assert-TmZipCentralDirectoryBounds {
    param(
        [Parameter(Mandatory = $true)][string]$ZipPath,
        [Parameter(Mandatory = $true)][uint64]$MaximumEntries,
        [Parameter(Mandatory = $true)][uint64]$MaximumCentralDirectoryBytes
    )
    $stream = $null
    $tail = $null
    try {
        $stream = [System.IO.File]::Open(
            $ZipPath,
            [System.IO.FileMode]::Open,
            [System.IO.FileAccess]::Read,
            [System.IO.FileShare]::Read
        )
        if ($stream.Length -lt 22) { throw 'The ZIP archive is too short.' }
        $tailLength = [int][Math]::Min([long]65557, $stream.Length)
        $tailOffset = [uint64]($stream.Length - $tailLength)
        $tail = Read-TmExactStreamBytes -Stream $stream -Offset $tailOffset -Count $tailLength
        $eocdIndex = -1
        for ($index = $tailLength - 22; $index -ge 0; $index--) {
            if ([BitConverter]::ToUInt32($tail, $index) -ne [uint32]0x06054b50) { continue }
            $commentLength = [BitConverter]::ToUInt16($tail, $index + 20)
            if ($index + 22 + $commentLength -eq $tailLength) {
                $eocdIndex = $index
                break
            }
        }
        if ($eocdIndex -lt 0) { throw 'The ZIP end-of-central-directory record is missing.' }

        $eocdOffset = $tailOffset + [uint64]$eocdIndex
        $diskNumber = [BitConverter]::ToUInt16($tail, $eocdIndex + 4)
        $centralDisk = [BitConverter]::ToUInt16($tail, $eocdIndex + 6)
        $entriesOnDisk16 = [BitConverter]::ToUInt16($tail, $eocdIndex + 8)
        $entries16 = [BitConverter]::ToUInt16($tail, $eocdIndex + 10)
        $centralSize32 = [BitConverter]::ToUInt32($tail, $eocdIndex + 12)
        $centralOffset32 = [BitConverter]::ToUInt32($tail, $eocdIndex + 16)
        if ($diskNumber -ne 0 -or $centralDisk -ne 0) {
            throw 'Multi-disk ZIP archives are not accepted.'
        }

        $entries = [uint64]$entries16
        $centralSize = [uint64]$centralSize32
        $centralOffset = [uint64]$centralOffset32
        $centralBoundary = $eocdOffset
        $usesZip64 = $entriesOnDisk16 -eq [uint16]::MaxValue -or
            $entries16 -eq [uint16]::MaxValue -or
            $centralSize32 -eq [uint32]::MaxValue -or
            $centralOffset32 -eq [uint32]::MaxValue
        if ($usesZip64) {
            if ($eocdOffset -lt 20) { throw 'The ZIP64 locator is missing.' }
            $locatorOffset = $eocdOffset - 20
            $locator = Read-TmExactStreamBytes -Stream $stream -Offset $locatorOffset -Count 20
            if ([BitConverter]::ToUInt32($locator, 0) -ne [uint32]0x07064b50 -or
                [BitConverter]::ToUInt32($locator, 4) -ne 0 -or
                [BitConverter]::ToUInt32($locator, 16) -ne 1) {
                throw 'The ZIP64 locator is invalid or multi-disk.'
            }
            $zip64Offset = [BitConverter]::ToUInt64($locator, 8)
            if ($zip64Offset -ge $locatorOffset) { throw 'The ZIP64 directory offset is invalid.' }
            $zip64 = Read-TmExactStreamBytes -Stream $stream -Offset $zip64Offset -Count 56
            $zip64RecordSize = [BitConverter]::ToUInt64($zip64, 4)
            if ([BitConverter]::ToUInt32($zip64, 0) -ne [uint32]0x06064b50 -or
                $zip64RecordSize -lt 44 -or
                $zip64RecordSize -gt 1048576 -or
                $zip64Offset + 12 + $zip64RecordSize -gt $locatorOffset -or
                [BitConverter]::ToUInt32($zip64, 16) -ne 0 -or
                [BitConverter]::ToUInt32($zip64, 20) -ne 0) {
                throw 'The ZIP64 end-of-central-directory record is invalid.'
            }
            $entriesOnDisk = [BitConverter]::ToUInt64($zip64, 24)
            $entries = [BitConverter]::ToUInt64($zip64, 32)
            $centralSize = [BitConverter]::ToUInt64($zip64, 40)
            $centralOffset = [BitConverter]::ToUInt64($zip64, 48)
            if ($entriesOnDisk -ne $entries) { throw 'The ZIP64 archive spans multiple disks.' }
            $centralBoundary = $zip64Offset
        }
        elseif ($entriesOnDisk16 -ne $entries16) {
            throw 'The ZIP archive spans multiple disks.'
        }

        if ($entries -lt 1 -or $entries -gt $MaximumEntries -or
            $centralSize -lt ($entries * 46) -or
            $centralSize -gt $MaximumCentralDirectoryBytes -or
            $centralOffset -gt $centralBoundary -or
            $centralSize -ne ($centralBoundary - $centralOffset)) {
            throw 'The ZIP central directory exceeds the approved metadata bounds.'
        }

        # ZipArchive materializes its entry collection before callers can inspect
        # Entries.Count. Parse the central directory ourselves first so a forged
        # EOCD count cannot make that materialization exceed the approved bound.
        $centralEnd = $centralOffset + $centralSize
        $cursor = $centralOffset
        $actualEntries = [uint64]0
        while ($cursor -lt $centralEnd) {
            if ($actualEntries -ge $MaximumEntries -or
                ($centralEnd - $cursor) -lt 46) {
                throw 'The ZIP central directory contains too many or truncated records.'
            }
            $header = $null
            try {
                $header = Read-TmExactStreamBytes -Stream $stream -Offset $cursor -Count 46
                if ([BitConverter]::ToUInt32($header, 0) -ne [uint32]0x02014b50) {
                    throw 'The ZIP central directory contains an invalid record signature.'
                }
                $fileNameBytes = [uint64][BitConverter]::ToUInt16($header, 28)
                $extraFieldBytes = [uint64][BitConverter]::ToUInt16($header, 30)
                $commentBytes = [uint64][BitConverter]::ToUInt16($header, 32)
            }
            finally {
                if ($null -ne $header) { [Array]::Clear($header, 0, $header.Length) }
            }
            if ($fileNameBytes -lt 1) {
                throw 'The ZIP central directory contains an empty entry name.'
            }
            $recordBytes = [uint64]46 + $fileNameBytes + $extraFieldBytes + $commentBytes
            if ($recordBytes -gt ($centralEnd - $cursor)) {
                throw 'A ZIP central-directory variable field exceeds the approved boundary.'
            }
            $cursor += $recordBytes
            $actualEntries += [uint64]1
        }
        if ($cursor -ne $centralEnd -or $actualEntries -ne $entries) {
            throw 'The ZIP central-directory record count does not match its end record.'
        }
        return [pscustomobject]@{
            EntryCount = $actualEntries
            CentralDirectoryBytes = $centralSize
            UsesZip64 = $usesZip64
        }
    }
    finally {
        if ($null -ne $tail) { [Array]::Clear($tail, 0, $tail.Length) }
        if ($null -ne $stream) { $stream.Dispose() }
    }
}

function Invoke-TmBoundedProcess {
    param(
        [Parameter(Mandatory = $true)][string]$FilePath,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [string]$WorkingDirectory,
        [AllowNull()][string]$StandardInput,
        [ValidateRange(1, 900)][int]$TimeoutSeconds = 120,
        [ValidateRange(1024, 4194304)][int]$MaximumCapturedCharacters = 1048576,
        [switch]$DiscardOutput
    )
    foreach ($argument in $Arguments) {
        if ([string]::IsNullOrWhiteSpace($argument) -or
            $argument -notmatch '^[A-Za-z0-9_./:=+@,-]+$') {
            throw 'A bounded process argument is outside the fixed safe character set.'
        }
    }
    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $FilePath
    $startInfo.Arguments = $Arguments -join ' '
    if (-not [string]::IsNullOrWhiteSpace($WorkingDirectory)) {
        $startInfo.WorkingDirectory = [System.IO.Path]::GetFullPath($WorkingDirectory)
    }
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardInput = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true

    $process = [System.Diagnostics.Process]::new()
    $stdoutBuffer = [char[]]::new(4096)
    $stderrBuffer = [char[]]::new(4096)
    $stdoutBuilder = [System.Text.StringBuilder]::new()
    $stderrBuilder = [System.Text.StringBuilder]::new()
    $stdoutTask = $null
    $stderrTask = $null
    $stdoutDone = $false
    $stderrDone = $false
    $observedCharacters = [uint64]0
    $deadline = [DateTimeOffset]::UtcNow.AddSeconds($TimeoutSeconds)
    $started = $false
    try {
        $process.StartInfo = $startInfo
        if (-not $process.Start()) { throw 'The bounded production process could not start.' }
        $started = $true
        $stdoutTask = $process.StandardOutput.ReadAsync($stdoutBuffer, 0, $stdoutBuffer.Length)
        $stderrTask = $process.StandardError.ReadAsync($stderrBuffer, 0, $stderrBuffer.Length)
        if ($null -ne $StandardInput) { $process.StandardInput.Write($StandardInput) }
        $process.StandardInput.Close()

        while (-not $stdoutDone -or -not $stderrDone) {
            if ([DateTimeOffset]::UtcNow -ge $deadline) {
                try { $process.Kill() } catch { }
                throw 'The bounded production process exceeded its fixed timeout.'
            }
            $pending = [System.Collections.Generic.List[System.Threading.Tasks.Task]]::new()
            if (-not $stdoutDone) { $pending.Add($stdoutTask) }
            if (-not $stderrDone) { $pending.Add($stderrTask) }
            $null = [System.Threading.Tasks.Task]::WaitAny($pending.ToArray(), 1000)

            if (-not $stdoutDone -and $stdoutTask.IsCompleted) {
                $count = [int]$stdoutTask.GetAwaiter().GetResult()
                if ($count -eq 0) {
                    $stdoutDone = $true
                }
                else {
                    $observedCharacters += [uint64]$count
                    if (-not $DiscardOutput) { $null = $stdoutBuilder.Append($stdoutBuffer, 0, $count) }
                    if ($observedCharacters -gt [uint64]$MaximumCapturedCharacters) {
                        try { $process.Kill() } catch { }
                        throw 'The bounded production process exceeded its output limit.'
                    }
                    $stdoutTask = $process.StandardOutput.ReadAsync($stdoutBuffer, 0, $stdoutBuffer.Length)
                }
            }
            if (-not $stderrDone -and $stderrTask.IsCompleted) {
                $count = [int]$stderrTask.GetAwaiter().GetResult()
                if ($count -eq 0) {
                    $stderrDone = $true
                }
                else {
                    $observedCharacters += [uint64]$count
                    if (-not $DiscardOutput) { $null = $stderrBuilder.Append($stderrBuffer, 0, $count) }
                    if ($observedCharacters -gt [uint64]$MaximumCapturedCharacters) {
                        try { $process.Kill() } catch { }
                        throw 'The bounded production process exceeded its output limit.'
                    }
                    $stderrTask = $process.StandardError.ReadAsync($stderrBuffer, 0, $stderrBuffer.Length)
                }
            }
        }
        $remaining = [int][Math]::Max(
            1,
            [Math]::Min(30000, ($deadline - [DateTimeOffset]::UtcNow).TotalMilliseconds)
        )
        if (-not $process.WaitForExit($remaining)) {
            try { $process.Kill() } catch { }
            throw 'The bounded production process did not terminate cleanly.'
        }
        return [pscustomobject]@{
            ExitCode = [int]$process.ExitCode
            StandardOutput = if ($DiscardOutput) { '' } else { $stdoutBuilder.ToString() }
            StandardError = if ($DiscardOutput) { '' } else { $stderrBuilder.ToString() }
        }
    }
    finally {
        if ($started -and -not $process.HasExited) {
            try { $process.Kill() } catch { }
        }
        [Array]::Clear($stdoutBuffer, 0, $stdoutBuffer.Length)
        [Array]::Clear($stderrBuffer, 0, $stderrBuffer.Length)
        $stdoutBuilder.Clear() | Out-Null
        $stderrBuilder.Clear() | Out-Null
        $process.Dispose()
        $startInfo = $null
    }
}

function Expand-TmVerifiedFlatArtifactZip {
    param(
        [Parameter(Mandatory = $true)][string]$ZipPath,
        [Parameter(Mandatory = $true)][string]$DestinationRoot,
        [Parameter(Mandatory = $true)][string[]]$AllowedFiles,
        [Parameter(Mandatory = $true)][uint64]$MaximumTotalBytes
    )
    Add-Type -AssemblyName System.IO.Compression
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    if (Test-Path -LiteralPath $DestinationRoot) {
        throw 'The verified artifact extraction root already exists.'
    }
    New-Item -ItemType Directory -Path $DestinationRoot | Out-Null
    $root = [System.IO.Path]::GetFullPath($DestinationRoot).TrimEnd('\') + '\'
    $allowed = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
    foreach ($name in $AllowedFiles) {
        if (-not $allowed.Add($name.Normalize([System.Text.NormalizationForm]::FormC))) {
            throw 'The artifact allowlist contains a duplicate name.'
        }
    }
    $null = Assert-TmZipCentralDirectoryBounds -ZipPath $ZipPath `
        -MaximumEntries 64 -MaximumCentralDirectoryBytes 4194304
    $archive = $null
    $stream = $null
    try {
        $stream = [System.IO.File]::OpenRead($ZipPath)
        $archive = [System.IO.Compression.ZipArchive]::new(
            $stream,
            [System.IO.Compression.ZipArchiveMode]::Read,
            $false
        )
        if ($archive.Entries.Count -ne $allowed.Count -or $archive.Entries.Count -lt 1 -or
            $archive.Entries.Count -gt 64) {
            throw 'The GitHub artifact ZIP entry count does not match its exact allowlist.'
        }
        $entries = [System.Collections.Generic.List[object]]::new()
        $seen = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
        $totalBytes = [uint64]0
        foreach ($entry in $archive.Entries) {
            $name = [string]$entry.FullName
            if ([string]::IsNullOrWhiteSpace($name) -or $name.IndexOf([char]0) -ge 0 -or
                [System.IO.Path]::IsPathRooted($name) -or $name.Contains(':') -or
                $name.Contains('\') -or $name.Contains('/') -or
                $name.EndsWith('.') -or $name.EndsWith(' ')) {
                throw 'The GitHub artifact ZIP contains an unsafe or nested entry name.'
            }
            $normalized = $name.Normalize([System.Text.NormalizationForm]::FormC)
            $baseName = [System.IO.Path]::GetFileNameWithoutExtension($normalized)
            if ($normalized -in @('.', '..') -or
                $baseName -match '^(?i:CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])$' -or
                -not $allowed.Contains($normalized) -or -not $seen.Add($normalized)) {
                throw 'The GitHub artifact ZIP contains an unexpected or duplicate entry.'
            }
            $unixType = (($entry.ExternalAttributes -shr 16) -band 0xF000)
            $windowsAttributes = ($entry.ExternalAttributes -band 0xFFFF)
            if ($unixType -eq 0xA000 -or
                ($windowsAttributes -band [int][System.IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw 'The GitHub artifact ZIP contains a link or reparse entry.'
            }
            $length = [uint64]$entry.Length
            $compressedLength = [uint64]$entry.CompressedLength
            if ($length -lt 1 -or $length -gt 536870912 -or
                ($compressedLength -eq 0 -and $length -gt 0) -or
                ($compressedLength -gt 0 -and ([double]$length / [double]$compressedLength) -gt 200.0)) {
                throw 'The GitHub artifact ZIP entry violates size or compression limits.'
            }
            $totalBytes += $length
            if ($totalBytes -gt $MaximumTotalBytes) {
                throw 'The GitHub artifact ZIP exceeds the total uncompressed size limit.'
            }
            $destination = [System.IO.Path]::GetFullPath((Join-Path $root $normalized))
            if (-not $destination.StartsWith($root, [System.StringComparison]::OrdinalIgnoreCase)) {
                throw 'The GitHub artifact ZIP entry escapes its extraction root.'
            }
            $entries.Add([pscustomobject]@{ Entry = $entry; Destination = $destination; Length = $length })
        }
        foreach ($item in $entries) {
            $input = $item.Entry.Open()
            $output = [System.IO.File]::Open(
                $item.Destination,
                [System.IO.FileMode]::CreateNew,
                [System.IO.FileAccess]::Write,
                [System.IO.FileShare]::None
            )
            try {
                $buffer = [byte[]]::new(65536)
                $copied = [uint64]0
                while (($read = $input.Read($buffer, 0, $buffer.Length)) -gt 0) {
                    $copied += [uint64]$read
                    if ($copied -gt [uint64]$item.Length) {
                        throw 'The GitHub artifact ZIP entry expanded beyond its declared size.'
                    }
                    $output.Write($buffer, 0, $read)
                }
                if ($copied -ne [uint64]$item.Length) {
                    throw 'The GitHub artifact ZIP entry did not match its declared size.'
                }
            }
            finally {
                if ($null -ne $buffer) { [Array]::Clear($buffer, 0, $buffer.Length) }
                $output.Dispose()
                $input.Dispose()
            }
        }
    }
    catch {
        if ($null -ne $archive) { $archive.Dispose(); $archive = $null }
        if ($null -ne $stream) { $stream.Dispose(); $stream = $null }
        if (Test-Path -LiteralPath $DestinationRoot -PathType Container) {
            Remove-Item -LiteralPath $DestinationRoot -Recurse -Force
        }
        throw
    }
    finally {
        if ($null -ne $archive) { $archive.Dispose() }
        if ($null -ne $stream) { $stream.Dispose() }
    }
}

function Expand-TmSafeSourceArchive {
    param(
        [Parameter(Mandatory = $true)][string]$ZipPath,
        [Parameter(Mandatory = $true)][string]$DestinationRoot
    )
    Add-Type -AssemblyName System.IO.Compression
    if (Test-Path -LiteralPath $DestinationRoot) { throw 'The source staging root already exists.' }
    $null = Assert-TmZipCentralDirectoryBounds -ZipPath $ZipPath `
        -MaximumEntries 5000 -MaximumCentralDirectoryBytes 67108864
    New-Item -ItemType Directory -Path $DestinationRoot | Out-Null
    $root = [System.IO.Path]::GetFullPath($DestinationRoot).TrimEnd('\') + '\'
    $stream = $null
    $archive = $null
    try {
        $stream = [System.IO.File]::OpenRead($ZipPath)
        $archive = [System.IO.Compression.ZipArchive]::new(
            $stream,
            [System.IO.Compression.ZipArchiveMode]::Read,
            $false
        )
        if ($archive.Entries.Count -lt 1 -or $archive.Entries.Count -gt 5000) {
            throw 'The exact source archive entry count is invalid.'
        }
        $seen = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
        $validated = [System.Collections.Generic.List[object]]::new()
        $totalBytes = [uint64]0
        foreach ($entry in $archive.Entries) {
            $name = [string]$entry.FullName
            if ([string]::IsNullOrWhiteSpace($name) -or $name.IndexOf([char]0) -ge 0 -or
                [System.IO.Path]::IsPathRooted($name) -or $name.Contains(':') -or
                $name.Contains('\') -or -not $name.StartsWith('app/', [System.StringComparison]::Ordinal) -or
                $name.Contains('//')) {
                throw 'The exact source archive contains an unsafe path.'
            }
            $isDirectory = $name.EndsWith('/')
            $trimmed = if ($isDirectory) { $name.TrimEnd('/') } else { $name }
            $segments = @($trimmed.Split('/'))
            if ($segments.Count -lt 1 -or @($segments | Where-Object {
                $_ -in @('', '.', '..') -or $_.EndsWith('.') -or $_.EndsWith(' ') -or
                [System.IO.Path]::GetFileNameWithoutExtension($_) -match
                    '^(?i:CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])$'
            }).Count -gt 0) {
                throw 'The exact source archive contains an unsafe path segment.'
            }
            $normalized = $trimmed.Normalize([System.Text.NormalizationForm]::FormC)
            if (-not $seen.Add($normalized)) { throw 'The exact source archive contains a duplicate destination.' }
            $unixType = (($entry.ExternalAttributes -shr 16) -band 0xF000)
            $windowsAttributes = ($entry.ExternalAttributes -band 0xFFFF)
            if ($unixType -eq 0xA000 -or
                ($windowsAttributes -band [int][System.IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw 'The exact source archive contains a link or reparse entry.'
            }
            $length = [uint64]$entry.Length
            $compressedLength = [uint64]$entry.CompressedLength
            if ($isDirectory -and $length -ne 0) { throw 'A source archive directory contains data.' }
            if (-not $isDirectory -and ($length -gt 536870912 -or
                    ($compressedLength -eq 0 -and $length -gt 0) -or
                    ($compressedLength -gt 0 -and ([double]$length / [double]$compressedLength) -gt 200.0))) {
                throw 'The exact source archive violates size or compression limits.'
            }
            $totalBytes += $length
            if ($totalBytes -gt [uint64]2147483648) { throw 'The exact source archive is too large.' }
            $relativeWindows = $normalized -replace '/', '\'
            $destination = [System.IO.Path]::GetFullPath((Join-Path $root $relativeWindows))
            if (-not $destination.StartsWith($root, [System.StringComparison]::OrdinalIgnoreCase)) {
                throw 'The exact source archive escapes its staging root.'
            }
            $validated.Add([pscustomobject]@{
                Entry = $entry; Destination = $destination; Length = $length; IsDirectory = $isDirectory
            })
        }
        foreach ($item in $validated) {
            if ($item.IsDirectory) {
                New-Item -ItemType Directory -Force -Path $item.Destination | Out-Null
                continue
            }
            $parent = Split-Path -Parent $item.Destination
            New-Item -ItemType Directory -Force -Path $parent | Out-Null
            $input = $item.Entry.Open()
            $output = [System.IO.File]::Open(
                $item.Destination,
                [System.IO.FileMode]::CreateNew,
                [System.IO.FileAccess]::Write,
                [System.IO.FileShare]::None
            )
            try {
                $buffer = [byte[]]::new(65536)
                $copied = [uint64]0
                while (($read = $input.Read($buffer, 0, $buffer.Length)) -gt 0) {
                    $copied += [uint64]$read
                    if ($copied -gt [uint64]$item.Length) { throw 'A source entry exceeded its declared size.' }
                    $output.Write($buffer, 0, $read)
                }
                if ($copied -ne [uint64]$item.Length) { throw 'A source entry did not match its declared size.' }
            }
            finally {
                if ($null -ne $buffer) { [Array]::Clear($buffer, 0, $buffer.Length) }
                $output.Dispose()
                $input.Dispose()
            }
        }
    }
    catch {
        if ($null -ne $archive) { $archive.Dispose(); $archive = $null }
        if ($null -ne $stream) { $stream.Dispose(); $stream = $null }
        if (Test-Path -LiteralPath $DestinationRoot -PathType Container) {
            Remove-Item -LiteralPath $DestinationRoot -Recurse -Force
        }
        throw
    }
    finally {
        if ($null -ne $archive) { $archive.Dispose() }
        if ($null -ne $stream) { $stream.Dispose() }
    }
}

function Get-TmDirectoryManifestSha256 {
    param([Parameter(Mandatory = $true)][string]$Root)
    $resolvedRoot = [System.IO.Path]::GetFullPath($Root).TrimEnd('\') + '\'
    $lines = [System.Collections.Generic.List[string]]::new()
    foreach ($file in Get-ChildItem -LiteralPath $Root -File -Force -Recurse | Sort-Object FullName) {
        if (($file.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw 'The staged source contains a reparse point.'
        }
        $relative = $file.FullName.Substring($resolvedRoot.Length) -replace '\\', '/'
        $hash = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        $lines.Add("$hash  $($file.Length)  $relative")
    }
    if ($lines.Count -lt 1 -or $lines.Count -gt 5000) { throw 'The staged source manifest is empty or too large.' }
    $bytes = [System.Text.Encoding]::UTF8.GetBytes(($lines -join "`n") + "`n")
    $sha = [System.Security.Cryptography.SHA256]::Create()
    try {
        return ([BitConverter]::ToString($sha.ComputeHash($bytes))).Replace('-', '').ToLowerInvariant()
    }
    finally {
        [Array]::Clear($bytes, 0, $bytes.Length)
        $sha.Dispose()
    }
}

function Get-TmVerifiedActionsArtifactEvidence {
    param(
        [Parameter(Mandatory = $true)][string]$GhPath,
        [Parameter(Mandatory = $true)][string]$Repository,
        [Parameter(Mandatory = $true)][long]$RunId,
        [Parameter(Mandatory = $true)][string]$ExpectedHeadSha,
        [Parameter(Mandatory = $true)][string]$ExpectedBranch,
        [Parameter(Mandatory = $true)][string]$ExpectedWorkflowName,
        [Parameter(Mandatory = $true)][string]$ExpectedWorkflowPath,
        [Parameter(Mandatory = $true)][ValidateSet('step10', 'step16')][string]$ArtifactKind,
        [Parameter(Mandatory = $true)][string]$ArtifactName,
        [string]$PayloadDestination
    )
    $run = Invoke-TmGhJson -GhPath $GhPath `
        -Arguments @('api', "repos/$Repository/actions/runs/$RunId") `
        -FailureMessage "Unable to inspect GitHub Actions run $RunId."
    if ([long]$run.id -ne $RunId -or
        [string]$run.repository.full_name -cne $Repository -or
        [string]$run.name -cne $ExpectedWorkflowName -or
        [string]$run.path -cne $ExpectedWorkflowPath -or
        [string]$run.status -cne 'completed' -or
        [string]$run.conclusion -cne 'success' -or
        [string]$run.head_branch -cne $ExpectedBranch -or
        ([string]$run.head_sha).ToLowerInvariant() -cne $ExpectedHeadSha.ToLowerInvariant() -or
        [long]$run.run_attempt -lt 1 -or
        [string]$run.event -notin @('push', 'workflow_dispatch')) {
        throw "GitHub Actions run $RunId does not prove the exact reviewed workflow and commit."
    }
    $allArtifacts = [System.Collections.Generic.List[object]]::new()
    $artifactIds = [System.Collections.Generic.HashSet[long]]::new()
    $expectedTotal = $null
    for ($page = 1; $page -le 10; $page++) {
        $artifactPage = Invoke-TmGhJson -GhPath $GhPath `
            -Arguments @('api', "repos/$Repository/actions/runs/$RunId/artifacts?per_page=100&page=$page") `
            -FailureMessage "Unable to inspect artifacts for GitHub Actions run $RunId."
        $pageTotal = [long]$artifactPage.total_count
        if ($pageTotal -lt 0 -or $pageTotal -gt 1000 -or
            ($null -ne $expectedTotal -and $pageTotal -ne [long]$expectedTotal)) {
            throw 'GitHub Actions artifact enumeration changed or exceeded its fixed cap.'
        }
        if ($null -eq $expectedTotal) { $expectedTotal = $pageTotal }
        $pageItems = @($artifactPage.artifacts)
        foreach ($item in $pageItems) {
            $itemId = [long]$item.id
            if ($itemId -lt 1 -or -not $artifactIds.Add($itemId)) {
                throw 'GitHub Actions artifact enumeration contains an invalid or duplicate ID.'
            }
            $allArtifacts.Add($item)
        }
        if ($allArtifacts.Count -ge [long]$expectedTotal) { break }
        if ($pageItems.Count -eq 0) { break }
    }
    if ($null -eq $expectedTotal -or $allArtifacts.Count -ne [long]$expectedTotal) {
        throw 'GitHub Actions artifact enumeration was incomplete.'
    }
    $artifacts = @($allArtifacts | Where-Object { [string]$_.name -ceq $ArtifactName })
    if ($artifacts.Count -ne 1) {
        throw "GitHub Actions run $RunId does not contain exactly one $ArtifactName artifact."
    }
    $artifact = $artifacts[0]
    $maximumArtifactBytes = if ($ArtifactKind -eq 'step10') { [uint64]1073741824 } else { [uint64]268435456 }
    if ($artifact.expired -isnot [bool] -or $artifact.expired -ne $false -or
        [long]$artifact.id -lt 1 -or [uint64]$artifact.size_in_bytes -lt 1 -or
        [uint64]$artifact.size_in_bytes -gt $maximumArtifactBytes -or
        [string]$artifact.digest -notmatch '^sha256:[0-9a-fA-F]{64}$') {
        throw "GitHub Actions artifact $ArtifactName is expired or has incomplete immutable metadata."
    }

    $workspaceRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("tm-actions-{0}-{1}" -f $RunId, [Guid]::NewGuid().ToString('N'))
    $zipPath = Join-Path $workspaceRoot 'artifact.zip'
    $downloadRoot = Join-Path $workspaceRoot 'payload'
    New-Item -ItemType Directory -Path $workspaceRoot | Out-Null
    try {
        if ($ArtifactKind -eq 'step10') {
            $required = @('tm.exe', 'tm-cli.exe', 'tm-office-decryptor.exe')
            $manifestName = 'SHA256SUMS.txt'
            $allowed = @($required) + $manifestName
            $maximumExpandedBytes = [uint64]1073741824
        }
        else {
            $required = @(
                'container-provenance.json', 'dpkg-packages.tsv', 'image-files.sha256',
                'image-inspect.json', 'trivy-image.json', 'Dockerfile', '.dockerignore',
                'Cargo.lock', 'container-supply-chain.lock.json', 'operations-toolchain.lock.json'
            )
            $manifestName = 'ARTIFACT-SHA256SUMS.txt'
            $allowed = @($required) + $manifestName
            $maximumExpandedBytes = [uint64]268435456
        }
        Invoke-TmGhArtifactZipDownload -GhPath $GhPath -Repository $Repository `
            -ArtifactId ([long]$artifact.id) -DestinationPath $zipPath `
            -MaximumBytes $maximumArtifactBytes
        $zipSha256 = (Get-FileHash -LiteralPath $zipPath -Algorithm SHA256).Hash.ToLowerInvariant()
        if ("sha256:$zipSha256" -cne ([string]$artifact.digest).ToLowerInvariant()) {
            throw 'The downloaded GitHub artifact ZIP does not match its REST digest.'
        }
        Expand-TmVerifiedFlatArtifactZip -ZipPath $zipPath -DestinationRoot $downloadRoot `
            -AllowedFiles $allowed -MaximumTotalBytes $maximumExpandedBytes
        if ($ArtifactKind -eq 'step10') {
            $manifest = Read-TmHashManifest -Root $downloadRoot -ManifestName $manifestName `
                -RequiredFiles $required
            $provenanceVerified = $false
        }
        else {
            $manifest = Read-TmHashManifest -Root $downloadRoot `
                -ManifestName $manifestName -RequiredFiles $required -AllowDotPrefix
            $provenancePath = Join-Path $downloadRoot 'container-provenance.json'
            try { $provenance = [System.IO.File]::ReadAllText($provenancePath) | ConvertFrom-Json } catch {
                throw 'STEP 16 container provenance is not valid JSON.'
            }
            Assert-TmExactJsonProperties $provenance @(
                'schemaVersion', 'kind', 'generatedAtUtc', 'attestationScope',
                'railwayProductionImageRelationship', 'source', 'inputs', 'ciImage',
                'tooling', 'productionDeployment'
            ) 'STEP 16 provenance'
            Assert-TmExactJsonProperties $provenance.source @(
                'repository', 'commitSha', 'gitRef', 'eventName', 'workflowRunId',
                'workflowRunAttempt', 'sourceArchiveSha256'
            ) 'STEP 16 provenance source'
            Assert-TmExactJsonProperties $provenance.inputs @(
                'platform', 'rustToolchain', 'debianSnapshot', 'dockerfileSha256',
                'dockerIgnoreSha256', 'cargoLockSha256', 'supplyChainLockSha256',
                'baseImages'
            ) 'STEP 16 provenance inputs'
            Assert-TmExactJsonProperties $provenance.ciImage @(
                'reference', 'configDigest', 'digestType', 'os', 'architecture',
                'revisionLabel', 'sourceLabel'
            ) 'STEP 16 provenance CI image'
            Assert-TmExactJsonProperties $provenance.tooling @(
                'dockerVersion', 'buildxVersion'
            ) 'STEP 16 provenance tooling'
            Assert-TmExactJsonProperties $provenance.productionDeployment @(
                'imageDigest', 'deploymentId', 'receiptRequired',
                'equalityWithCiConfigDigestExpected'
            ) 'STEP 16 provenance production deployment'
            $generatedAt = [DateTimeOffset]::MinValue
            $runStartedAt = [DateTimeOffset]::MinValue
            $runUpdatedAt = [DateTimeOffset]::MinValue
            if (-not [DateTimeOffset]::TryParse(
                    [string]$provenance.generatedAtUtc,
                    [System.Globalization.CultureInfo]::InvariantCulture,
                    [System.Globalization.DateTimeStyles]::RoundtripKind,
                    [ref]$generatedAt
                ) -or -not [DateTimeOffset]::TryParse(
                    [string]$run.run_started_at,
                    [System.Globalization.CultureInfo]::InvariantCulture,
                    [System.Globalization.DateTimeStyles]::RoundtripKind,
                    [ref]$runStartedAt
                ) -or -not [DateTimeOffset]::TryParse(
                    [string]$run.updated_at,
                    [System.Globalization.CultureInfo]::InvariantCulture,
                    [System.Globalization.DateTimeStyles]::RoundtripKind,
                    [ref]$runUpdatedAt
                ) -or $generatedAt.ToUniversalTime() -lt $runStartedAt.ToUniversalTime().AddMinutes(-5) -or
                $generatedAt.ToUniversalTime() -gt $runUpdatedAt.ToUniversalTime().AddMinutes(5)) {
                throw 'STEP 16 provenance timestamp is outside the exact Actions run.'
            }
            if ([int]$provenance.schemaVersion -ne 1 -or
                [string]$provenance.kind -cne 'tm-step16-container-provenance' -or
                [string]$provenance.attestationScope -cne 'ci-built-image-only' -or
                [string]$provenance.railwayProductionImageRelationship -cne
                    'independent-rebuild-requires-separate-deployment-receipt' -or
                [string]$provenance.source.repository -cne $Repository -or
                ([string]$provenance.source.commitSha).ToLowerInvariant() -cne $ExpectedHeadSha.ToLowerInvariant() -or
                [string]$provenance.source.gitRef -cne "refs/heads/$ExpectedBranch" -or
                [string]$provenance.source.eventName -cne [string]$run.event -or
                [long]$provenance.source.workflowRunId -ne $RunId -or
                [long]$provenance.source.workflowRunAttempt -ne [long]$run.run_attempt -or
                [string]$provenance.source.sourceArchiveSha256 -notmatch '^[0-9a-fA-F]{64}$' -or
                [string]$provenance.inputs.platform -cne 'linux/amd64' -or
                [string]$provenance.inputs.rustToolchain -cne '1.97.0' -or
                [string]$provenance.inputs.debianSnapshot -cne '20260731T000000Z' -or
                @($provenance.inputs.baseImages).Count -ne 2 -or
                @($provenance.inputs.baseImages | Where-Object {
                    [string]$_.platform -cne 'linux/amd64' -or
                    [string]$_.indexDigest -notmatch '^sha256:[0-9a-f]{64}$' -or
                    [string]$_.platformManifestDigest -notmatch '^sha256:[0-9a-f]{64}$'
                }).Count -ne 0 -or
                [string]$provenance.ciImage.reference -cne 'tm-server:step16' -or
                [string]$provenance.ciImage.configDigest -notmatch '^sha256:[0-9a-f]{64}$' -or
                [string]$provenance.ciImage.digestType -cne 'docker-image-config' -or
                [string]$provenance.ciImage.os -cne 'linux' -or
                [string]$provenance.ciImage.architecture -cne 'amd64' -or
                ([string]$provenance.ciImage.revisionLabel).ToLowerInvariant() -cne $ExpectedHeadSha.ToLowerInvariant() -or
                [string]$provenance.ciImage.sourceLabel -cne "https://github.com/$Repository" -or
                $provenance.productionDeployment.receiptRequired -isnot [bool] -or
                $provenance.productionDeployment.receiptRequired -ne $true -or
                $provenance.productionDeployment.equalityWithCiConfigDigestExpected -isnot [bool] -or
                $provenance.productionDeployment.equalityWithCiConfigDigestExpected -ne $false -or
                $null -ne $provenance.productionDeployment.imageDigest -or
                $null -ne $provenance.productionDeployment.deploymentId -or
                [string]::IsNullOrWhiteSpace([string]$provenance.tooling.dockerVersion) -or
                [string]::IsNullOrWhiteSpace([string]$provenance.tooling.buildxVersion)) {
                throw 'STEP 16 container provenance is not bound to the exact Actions run and commit.'
            }
            $inputHashes = @{
                Dockerfile = [string]$provenance.inputs.dockerfileSha256
                '.dockerignore' = [string]$provenance.inputs.dockerIgnoreSha256
                'Cargo.lock' = [string]$provenance.inputs.cargoLockSha256
                'container-supply-chain.lock.json' = [string]$provenance.inputs.supplyChainLockSha256
            }
            foreach ($name in $inputHashes.Keys) {
                if ([string]$manifest.FileHashes[$name] -cne $inputHashes[$name].ToLowerInvariant()) {
                    throw "STEP 16 provenance input hash does not match its artifact: $name"
                }
            }
            $reviewedAppRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
            foreach ($name in $inputHashes.Keys) {
                $reviewedPath = Join-Path $reviewedAppRoot $name
                if (-not (Test-Path -LiteralPath $reviewedPath -PathType Leaf) -or
                    [string]$manifest.FileHashes[$name] -cne
                        (Get-FileHash -LiteralPath $reviewedPath -Algorithm SHA256).Hash.ToLowerInvariant()) {
                    throw "STEP 16 artifact input does not match the reviewed commit file: $name"
                }
            }
            $supplyChainPath = Join-Path $downloadRoot 'container-supply-chain.lock.json'
            try {
                $supplyChain = [System.IO.File]::ReadAllText(
                    $supplyChainPath,
                    [System.Text.Encoding]::UTF8
                ) | ConvertFrom-Json
            }
            catch { throw 'STEP 16 container supply-chain lock is not valid JSON.' }
            Assert-TmExactJsonProperties $supplyChain @(
                'schemaVersion', 'platform', 'rustToolchain', 'debianSnapshot',
                'builder', 'runtime'
            ) 'STEP 16 container supply-chain lock'
            $lockedStages = @($supplyChain.builder, $supplyChain.runtime)
            foreach ($lockedStage in $lockedStages) {
                Assert-TmExactJsonProperties $lockedStage @(
                    'stage', 'image', 'manifestDigest', 'platformManifestDigest'
                ) 'STEP 16 locked base image'
            }
            if ([int]$supplyChain.schemaVersion -ne 1 -or
                [string]$supplyChain.platform -cne [string]$provenance.inputs.platform -or
                [string]$supplyChain.rustToolchain -cne [string]$provenance.inputs.rustToolchain -or
                [string]$supplyChain.debianSnapshot -cne [string]$provenance.inputs.debianSnapshot) {
                throw 'STEP 16 provenance does not match its reviewed container supply-chain policy.'
            }
            foreach ($lockedStage in $lockedStages) {
                $resolved = @($provenance.inputs.baseImages | Where-Object {
                    [string]$_.stage -ceq [string]$lockedStage.stage
                })
                if ($resolved.Count -ne 1) {
                    throw 'STEP 16 provenance does not contain exactly one locked base-image stage.'
                }
                Assert-TmExactJsonProperties $resolved[0] @(
                    'stage', 'image', 'indexDigest', 'platform', 'platformManifestDigest'
                ) 'STEP 16 resolved base image'
                if ([string]$resolved[0].image -cne [string]$lockedStage.image -or
                    [string]$resolved[0].indexDigest -cne [string]$lockedStage.manifestDigest -or
                    [string]$resolved[0].platformManifestDigest -cne
                        [string]$lockedStage.platformManifestDigest -or
                    [string]$resolved[0].platform -cne [string]$supplyChain.platform) {
                    throw 'STEP 16 resolved base-image provenance changed from its reviewed lock.'
                }
            }
            try {
                $imageInspect = @(
                    [System.IO.File]::ReadAllText(
                        (Join-Path $downloadRoot 'image-inspect.json'),
                        [System.Text.Encoding]::UTF8
                    ) | ConvertFrom-Json
                )
            }
            catch { throw 'STEP 16 image inspection evidence is not valid JSON.' }
            if ($imageInspect.Count -ne 1 -or
                [string]$imageInspect[0].Id -cne [string]$provenance.ciImage.configDigest -or
                [string]$imageInspect[0].Os -cne [string]$provenance.ciImage.os -or
                [string]$imageInspect[0].Architecture -cne [string]$provenance.ciImage.architecture -or
                [string]$imageInspect[0].Config.Labels.'org.opencontainers.image.revision' -cne
                    [string]$provenance.ciImage.revisionLabel -or
                [string]$imageInspect[0].Config.Labels.'org.opencontainers.image.source' -cne
                    [string]$provenance.ciImage.sourceLabel) {
                throw 'STEP 16 CI image inspection does not match its provenance record.'
            }
            $reviewedToolchainHash = (Get-FileHash -LiteralPath $script:TmOperationsToolchainLockPath -Algorithm SHA256).Hash.ToLowerInvariant()
            if ([string]$manifest.FileHashes['operations-toolchain.lock.json'] -cne $reviewedToolchainHash) {
                throw 'STEP 16 artifact does not contain the exact reviewed operations toolchain lock.'
            }
            $provenanceVerified = $true
        }
        if (-not [string]::IsNullOrWhiteSpace($PayloadDestination)) {
            Export-TmVerifiedFlatPayload -SourceRoot $downloadRoot `
                -DestinationRoot $PayloadDestination -Files $allowed `
                -ManifestName $manifestName -Manifest $manifest
        }
        return [pscustomobject]@{
            workflowName = $ExpectedWorkflowName
            workflowPath = $ExpectedWorkflowPath
            runId = $RunId
            runAttempt = [long]$run.run_attempt
            event = [string]$run.event
            headSha = ([string]$run.head_sha).ToLowerInvariant()
            artifactName = $ArtifactName
            artifactId = [long]$artifact.id
            artifactDigest = ([string]$artifact.digest).ToLowerInvariant()
            downloadedZipSha256 = $zipSha256
            artifactSizeBytes = [uint64]$artifact.size_in_bytes
            manifestSha256 = $manifest.ManifestSha256
            manifestVerified = $true
            provenanceVerified = $provenanceVerified
            verifiedFileHashes = $manifest.FileHashes
        }
    }
    finally {
        if (Test-Path -LiteralPath $workspaceRoot -PathType Container) {
            Remove-Item -LiteralPath $workspaceRoot -Recurse -Force
        }
    }
}

function Get-TmReleaseEvidenceBinding {
    param([Parameter(Mandatory = $true)]$Evidence)
    return ConvertTo-TmCanonicalJsonValue ([ordered]@{
        workflowName = [string]$Evidence.workflowName
        workflowPath = [string]$Evidence.workflowPath
        runId = [long]$Evidence.runId
        runAttempt = [long]$Evidence.runAttempt
        event = [string]$Evidence.event
        headSha = ([string]$Evidence.headSha).ToLowerInvariant()
        artifactName = [string]$Evidence.artifactName
        artifactId = [long]$Evidence.artifactId
        artifactDigest = ([string]$Evidence.artifactDigest).ToLowerInvariant()
        downloadedZipSha256 = ([string]$Evidence.downloadedZipSha256).ToLowerInvariant()
        artifactSizeBytes = [uint64]$Evidence.artifactSizeBytes
        manifestSha256 = ([string]$Evidence.manifestSha256).ToLowerInvariant()
        manifestVerified = $Evidence.manifestVerified
        provenanceVerified = $Evidence.provenanceVerified
        verifiedFileHashes = $Evidence.verifiedFileHashes
    })
}

function Assert-TmReleaseEvidenceMatches {
    param(
        [Parameter(Mandatory = $true)]$Expected,
        [Parameter(Mandatory = $true)]$Actual,
        [Parameter(Mandatory = $true)][string]$Name
    )
    if ((Get-TmReleaseEvidenceBinding $Expected) -cne (Get-TmReleaseEvidenceBinding $Actual)) {
        throw "$Name Actions artifact evidence changed after approval."
    }
}

function Resolve-TmVerifiedRailwayCli {
    $lock = Read-TmOperationsToolchainLock
    $base = [System.IO.Path]::GetFullPath((Join-Path $env:LOCALAPPDATA 'pnpm\store\v11\links\@railway\cli'))
    if (-not (Test-Path -LiteralPath $base -PathType Container)) { throw 'The official Railway CLI pnpm installation was not found.' }
    $candidates = @()
    foreach ($file in Get-ChildItem -LiteralPath $base -Filter railway.exe -File -Recurse) {
        $packageRoot = $file.Directory.Parent.FullName
        $packageJsonPath = Join-Path $packageRoot 'package.json'
        if (-not (Test-Path -LiteralPath $packageJsonPath -PathType Leaf)) { continue }
        try { $package = [System.IO.File]::ReadAllText($packageJsonPath) | ConvertFrom-Json } catch { continue }
        $version = [Version]::new()
        if ([string]$package.name -cne [string]$lock.railwayCli.packageName -or
            -not [Version]::TryParse([string]$package.version, [ref]$version) -or
            $version.ToString() -cne [string]$lock.railwayCli.version) { continue }
        $candidates += [pscustomobject]@{ File = $file; Package = $package; Version = $version }
    }
    if ($candidates.Count -ne 1) { throw 'The exact locked Railway CLI installation is missing or ambiguous.' }
    $candidate = $candidates[0]
    $path = [System.IO.Path]::GetFullPath($candidate.File.FullName)
    if (-not $path.StartsWith($base + '\', [System.StringComparison]::OrdinalIgnoreCase)) {
        throw 'The Railway CLI resolved outside its official pnpm installation root.'
    }
    $fileItem = Get-Item -LiteralPath $path
    if (($fileItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw 'The Railway CLI executable is a reparse point.'
    }
    $actualSha256 = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualSha256 -cne [string]$lock.railwayCli.sha256) {
        throw 'The Railway CLI executable does not match the reviewed exact SHA-256 lock.'
    }
    $signature = Get-AuthenticodeSignature -LiteralPath $path
    if ([string]$lock.railwayCli.authenticodePolicy -cne 'allow-unsigned-exact-sha256' -or
        [string]$signature.Status -notin @('Valid', 'NotSigned')) {
        throw 'The Railway CLI Authenticode state violates the reviewed exact-hash policy.'
    }
    $versionResult = Invoke-TmBoundedProcess -FilePath $path `
        -Arguments ([string[]]@('--version')) -TimeoutSeconds 15 -MaximumCapturedCharacters 16384
    $versionOutput = @(
        [string]$versionResult.StandardOutput
        [string]$versionResult.StandardError
    ) | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }
    $versionText = (@($versionOutput) -join "`n").Trim()
    if ($versionResult.ExitCode -ne 0 -or
        $versionText -notmatch "^railway\s+$([regex]::Escape($candidate.Version.ToString()))\s*$") {
        throw 'The Railway CLI executable version does not match its package metadata.'
    }
    return [pscustomobject]@{
        path = $path
        version = $candidate.Version.ToString()
        sha256 = $actualSha256
        authenticodeStatus = [string]$signature.Status
        signerThumbprint = if ($null -ne $signature.SignerCertificate) {
            [string]$signature.SignerCertificate.Thumbprint
        } else { '' }
        installKind = [string]$lock.railwayCli.installKind
    }
}

function Get-TmRailwayCliBinding {
    param([Parameter(Mandatory = $true)]$Evidence)
    return ConvertTo-TmCanonicalJsonValue ([ordered]@{
        version = [string]$Evidence.version
        sha256 = ([string]$Evidence.sha256).ToLowerInvariant()
        authenticodeStatus = [string]$Evidence.authenticodeStatus
        signerThumbprint = [string]$Evidence.signerThumbprint
        installKind = [string]$Evidence.installKind
    })
}

function Assert-TmRailwayCliMatches {
    param(
        [Parameter(Mandatory = $true)]$Expected,
        [Parameter(Mandatory = $true)]$Actual
    )
    if ((Get-TmRailwayCliBinding $Expected) -cne (Get-TmRailwayCliBinding $Actual)) {
        throw 'The Railway CLI changed after release approval.'
    }
}

function Get-TmSignedToolBinding {
    param([Parameter(Mandatory = $true)]$Evidence)
    return ConvertTo-TmCanonicalJsonValue ([ordered]@{
        version = [string]$Evidence.version
        sha256 = ([string]$Evidence.sha256).ToLowerInvariant()
        authenticodeStatus = [string]$Evidence.authenticodeStatus
        signerSubject = [string]$Evidence.signerSubject
        installKind = [string]$Evidence.installKind
    })
}

function Assert-TmSignedToolMatches {
    param(
        [Parameter(Mandatory = $true)]$Expected,
        [Parameter(Mandatory = $true)]$Actual,
        [Parameter(Mandatory = $true)][string]$Name
    )
    if ((Get-TmSignedToolBinding $Expected) -cne (Get-TmSignedToolBinding $Actual)) {
        throw "$Name changed after release approval."
    }
}

function Invoke-TmReleaseEvidenceGuardSelfTest {
    $root = Join-Path ([System.IO.Path]::GetTempPath()) "tm-release-evidence-$([Guid]::NewGuid().ToString('N'))"
    try {
        $null = Read-TmOperationsToolchainLock
        New-Item -ItemType Directory -Force -Path $root | Out-Null
        foreach ($name in @('tm.exe', 'tm-cli.exe', 'tm-office-decryptor.exe')) {
            [System.IO.File]::WriteAllText((Join-Path $root $name), "synthetic-$name")
        }
        $lines = @('tm.exe', 'tm-cli.exe', 'tm-office-decryptor.exe') | ForEach-Object {
            '{0}  {1}' -f (Get-FileHash -LiteralPath (Join-Path $root $_) -Algorithm SHA256).Hash.ToLowerInvariant(), $_
        }
        [System.IO.File]::WriteAllLines((Join-Path $root 'SHA256SUMS.txt'), $lines)
        $manifest = Read-TmHashManifest -Root $root -ManifestName 'SHA256SUMS.txt' `
            -RequiredFiles @('tm.exe', 'tm-cli.exe', 'tm-office-decryptor.exe')
        if ([string]$manifest.ManifestSha256 -notmatch '^[0-9a-f]{64}$') {
            throw 'Release evidence self-test did not verify the synthetic manifest.'
        }

        $exportRoot = Join-Path $root 'verified-export'
        Export-TmVerifiedFlatPayload -SourceRoot $root -DestinationRoot $exportRoot `
            -Files @('tm.exe', 'tm-cli.exe', 'tm-office-decryptor.exe', 'SHA256SUMS.txt') `
            -ManifestName 'SHA256SUMS.txt' -Manifest $manifest
        if (@(Get-ChildItem -LiteralPath $exportRoot -File -Force).Count -ne 4) {
            throw 'Release evidence self-test did not export the exact verified payload.'
        }
        Remove-Item -LiteralPath $exportRoot -Recurse -Force

        $flatZipPath = Join-Path $root 'flat-artifact.zip'
        $flatExtractRoot = Join-Path $root 'flat-extract'
        Add-Type -AssemblyName System.IO.Compression
        $zipStream = [System.IO.File]::Open(
            $flatZipPath,
            [System.IO.FileMode]::CreateNew,
            [System.IO.FileAccess]::ReadWrite,
            [System.IO.FileShare]::None
        )
        $zipArchive = [System.IO.Compression.ZipArchive]::new(
            $zipStream,
            [System.IO.Compression.ZipArchiveMode]::Create,
            $false
        )
        try {
            foreach ($name in @('one.txt', 'two.txt')) {
                $entry = $zipArchive.CreateEntry($name)
                $writer = [System.IO.StreamWriter]::new(
                    $entry.Open(),
                    [System.Text.UTF8Encoding]::new($false)
                )
                try { $writer.Write("synthetic-$name") } finally { $writer.Dispose() }
            }
        }
        finally {
            $zipArchive.Dispose()
            $zipStream.Dispose()
        }
        Expand-TmVerifiedFlatArtifactZip -ZipPath $flatZipPath `
            -DestinationRoot $flatExtractRoot -AllowedFiles @('one.txt', 'two.txt') `
            -MaximumTotalBytes 1024
        if (-not (Test-Path -LiteralPath (Join-Path $flatExtractRoot 'one.txt') -PathType Leaf)) {
            throw 'Release evidence self-test did not safely extract the flat artifact ZIP.'
        }

        $forgedCountZipPath = Join-Path $root 'forged-count.zip'
        [System.IO.File]::Copy($flatZipPath, $forgedCountZipPath)
        $zipBytes = $null
        try {
            $zipBytes = [System.IO.File]::ReadAllBytes($forgedCountZipPath)
            $eocdIndex = -1
            for ($index = $zipBytes.Length - 22; $index -ge 0; $index--) {
                if ([BitConverter]::ToUInt32($zipBytes, $index) -ne [uint32]0x06054b50) { continue }
                $commentLength = [BitConverter]::ToUInt16($zipBytes, $index + 20)
                if ($index + 22 + $commentLength -eq $zipBytes.Length) {
                    $eocdIndex = $index
                    break
                }
            }
            if ($eocdIndex -lt 0) { throw 'Release evidence self-test could not locate the synthetic EOCD.' }
            $zipBytes[$eocdIndex + 8] = 1
            $zipBytes[$eocdIndex + 9] = 0
            $zipBytes[$eocdIndex + 10] = 1
            $zipBytes[$eocdIndex + 11] = 0
            [System.IO.File]::WriteAllBytes($forgedCountZipPath, $zipBytes)
        }
        finally {
            if ($null -ne $zipBytes) { [Array]::Clear($zipBytes, 0, $zipBytes.Length) }
        }
        $forgedCountRejected = $false
        try {
            Assert-TmZipCentralDirectoryBounds -ZipPath $forgedCountZipPath `
                -MaximumEntries 64 -MaximumCentralDirectoryBytes 4194304 | Out-Null
        }
        catch {
            $forgedCountRejected = $_.Exception.Message -like '*record count*'
        }
        if (-not $forgedCountRejected) {
            throw 'Release evidence self-test accepted a forged ZIP entry count.'
        }

        $step16Root = Join-Path $root 'step16'
        New-Item -ItemType Directory -Force -Path $step16Root | Out-Null
        $step16Files = @(
            'container-provenance.json', 'dpkg-packages.tsv', 'image-files.sha256',
            'image-inspect.json', 'trivy-image.json', 'Dockerfile', '.dockerignore',
            'Cargo.lock', 'container-supply-chain.lock.json', 'operations-toolchain.lock.json'
        )
        foreach ($name in $step16Files) {
            [System.IO.File]::WriteAllText((Join-Path $step16Root $name), "synthetic-$name")
        }
        $step16Lines = $step16Files | ForEach-Object {
            '{0}  ./{1}' -f (Get-FileHash -LiteralPath (Join-Path $step16Root $_) -Algorithm SHA256).Hash.ToLowerInvariant(), $_
        }
        [System.IO.File]::WriteAllLines(
            (Join-Path $step16Root 'ARTIFACT-SHA256SUMS.txt'),
            $step16Lines
        )
        $null = Read-TmHashManifest -Root $step16Root `
            -ManifestName 'ARTIFACT-SHA256SUMS.txt' -RequiredFiles $step16Files -AllowDotPrefix

        $receiptPath = Join-Path $root 'approval.json'
        $now = [DateTimeOffset]::UtcNow
        $receipt = [ordered]@{
            receiptVersion = 1
            kind = 'tm-expense-key-approval'
            approvalId = [Guid]::NewGuid().ToString('D')
            approvalNonce = New-TmApprovalNonce
            createdAtUtc = $now.ToString('o')
            expiresAtUtc = $now.AddMinutes(15).ToString('o')
            integrityProofKind = $script:TmReceiptProofKind
        }
        Add-TmReceiptIntegrityProof $receipt
        Write-TmJsonNoBom -Value $receipt -Path $receiptPath
        Assert-TmReceiptIntegrityProof $receipt
        $null = New-TmPendingReceiptState -StateKind approval -ReceiptId $receipt.approvalId `
            -ReceiptPath $receiptPath -ExpiresAtUtc $receipt.expiresAtUtc -StateRoot (Join-Path $root 'state')
        $null = Assert-TmPendingReceiptState -StateKind approval -ReceiptId $receipt.approvalId `
            -ReceiptPath $receiptPath -StateRoot (Join-Path $root 'state') -Consume
        $replayRejected = $false
        try {
            Assert-TmPendingReceiptState -StateKind approval -ReceiptId $receipt.approvalId `
                -ReceiptPath $receiptPath -StateRoot (Join-Path $root 'state') -Consume | Out-Null
        }
        catch { $replayRejected = $true }
        if (-not $replayRejected) { throw 'Release receipt self-test did not reject replay.' }

        $oversizedReceiptPath = Join-Path $root 'oversized-approval.json'
        $oversizedReceipt = [ordered]@{
            receiptVersion = 1
            kind = 'tm-expense-key-approval'
            approvalId = [Guid]::NewGuid().ToString('D')
            approvalNonce = New-TmApprovalNonce
            createdAtUtc = $now.ToString('o')
            expiresAtUtc = $now.AddMinutes(15).ToString('o')
            integrityProofKind = $script:TmReceiptProofKind
        }
        Add-TmReceiptIntegrityProof $oversizedReceipt
        Write-TmJsonNoBom -Value $oversizedReceipt -Path $oversizedReceiptPath
        $null = New-TmPendingReceiptState -StateKind approval `
            -ReceiptId $oversizedReceipt.approvalId -ReceiptPath $oversizedReceiptPath `
            -ExpiresAtUtc $oversizedReceipt.expiresAtUtc -StateRoot (Join-Path $root 'state')
        $oversizedStatePath = Get-TmReceiptStatePath -StateKind approval `
            -ReceiptId $oversizedReceipt.approvalId -StateRoot (Join-Path $root 'state')
        $oversizedBytes = [byte[]]::new(65537)
        try { [System.IO.File]::WriteAllBytes($oversizedStatePath, $oversizedBytes) } finally {
            [Array]::Clear($oversizedBytes, 0, $oversizedBytes.Length)
        }
        $oversizedRejected = $false
        try {
            Assert-TmPendingReceiptState -StateKind approval `
                -ReceiptId $oversizedReceipt.approvalId -ReceiptPath $oversizedReceiptPath `
                -StateRoot (Join-Path $root 'state') | Out-Null
        }
        catch {
            $oversizedRejected = $_.Exception.Message -like '*bounded regular file*'
        }
        if (-not $oversizedRejected) {
            throw 'Release receipt self-test accepted an oversized pending state.'
        }
        $receipt.approvalNonce = New-TmApprovalNonce
        $tamperRejected = $false
        try { Assert-TmReceiptIntegrityProof $receipt } catch { $tamperRejected = $true }
        if (-not $tamperRejected) { throw 'Release receipt self-test did not reject tampering.' }
    }
    finally {
        if (Test-Path -LiteralPath $root -PathType Container) {
            Remove-Item -LiteralPath $root -Recurse -Force
        }
    }
    Write-Host 'Expense release evidence guard self-test: PASS' -ForegroundColor Green
}

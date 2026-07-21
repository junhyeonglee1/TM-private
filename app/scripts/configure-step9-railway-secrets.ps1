[CmdletBinding()]
param(
    [string]$ProjectId = '7fcb22b5-db34-4e2b-a12a-cbc60391ff5f',
    [string]$Service = 'tm-server',
    [string]$Bucket = 'tm-encrypted-backups',
    [ValidateRange(1, 3650)]
    [int]$StagingTokenValidDays = 90
)

$ErrorActionPreference = 'Stop'
trap {
    Write-Host ''
    Write-Host "Secure setup stopped: $($_.Exception.Message)" -ForegroundColor Red
    [void](Read-Host 'Press Enter to close')
    break
}
$pnpm = 'C:\Users\tkfk0\.cache\codex-runtimes\codex-primary-runtime\dependencies\bin\fallback\pnpm.cmd'
$nodeBin = 'C:\Users\tkfk0\.cache\codex-runtimes\codex-primary-runtime\dependencies\node\bin'
if (-not (Test-Path -LiteralPath $pnpm)) {
    throw 'Bundled pnpm was not found.'
}
$env:PATH = "$nodeBin;$([IO.Path]::GetDirectoryName($pnpm));$env:PATH"

function Invoke-RailwayValueSet {
    param(
        [Parameter(Mandatory)] [string]$Environment,
        [Parameter(Mandatory)] [string]$Name,
        [Parameter(Mandatory)] [string]$Value
    )

    Write-Host "  Setting $Environment / $Name ..."
    $output = $Value | & $pnpm dlx '@railway/cli@5.26.0' variable set $Name --stdin `
        --skip-deploys --project $ProjectId --service $Service --environment $Environment 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to set Railway variable $Name in $Environment."
    }
}

Write-Host 'STEP 9 secure setup'
Write-Host '1. Create a password-manager entry named: TM Railway restic repository password'
Write-Host '2. Generate at least 32 random characters in that entry.'
Write-Host '3. Paste it below. It will not be displayed or written to disk.'
$securePassword = Read-Host 'Restic repository password' -AsSecureString
$password = [System.Net.NetworkCredential]::new('', $securePassword).Password
if ($password.Length -lt 32) {
    throw 'The restic repository password must contain at least 32 characters.'
}

$bucketRaw = & $pnpm dlx '@railway/cli@5.26.0' bucket credentials --bucket $Bucket --json
if ($LASTEXITCODE -ne 0) {
    throw 'Failed to read Railway Bucket credentials.'
}
$bucketCredentials = $bucketRaw | ConvertFrom-Json

Write-Host ''
Write-Host 'If you saved the staging token from the previous window, paste it below.'
Write-Host 'If not, press Enter and a new token will be generated and copied.'
$secureStagingToken = Read-Host 'Existing staging TM token (optional)' -AsSecureString
$stagingToken = [System.Net.NetworkCredential]::new('', $secureStagingToken).Password
$randomBytes = New-Object byte[] 32
if ([string]::IsNullOrWhiteSpace($stagingToken)) {
    $random = [System.Security.Cryptography.RandomNumberGenerator]::Create()
    try {
        $random.GetBytes($randomBytes)
    }
    finally {
        $random.Dispose()
    }
    $secret = [Convert]::ToBase64String($randomBytes).TrimEnd('=').Replace('+', '-').Replace('/', '_')
    $stagingToken = "tm_pat_v1_$secret"
    Set-Clipboard -Value $stagingToken
    Write-Host ''
    Write-Host 'A new separate staging TM token is now on the clipboard.'
    Write-Host 'Save or replace it in the password manager as: TM Railway staging device'
    [void](Read-Host 'After saving it, press Enter')
}
elseif ($stagingToken -notmatch '^tm_pat_v1_[A-Za-z0-9_-]{43}$') {
    throw 'The existing staging TM token has an invalid format.'
}
$sha256 = [System.Security.Cryptography.SHA256]::Create()
try {
    $hashBytes = $sha256.ComputeHash([System.Text.Encoding]::UTF8.GetBytes($stagingToken))
}
finally {
    $sha256.Dispose()
}
$stagingHash = -join ($hashBytes | ForEach-Object { $_.ToString('x2') })
$stagingExpiry = [DateTime]::UtcNow.AddDays($StagingTokenValidDays).ToString(
    'yyyy-MM-ddTHH:mm:ss.fffZ',
    [Globalization.CultureInfo]::InvariantCulture
)
$sharedValues = @{
    TM_BACKUP_S3_ENDPOINT          = [string]$bucketCredentials.endpoint
    TM_BACKUP_S3_BUCKET            = [string]$bucketCredentials.bucketName
    TM_BACKUP_S3_ACCESS_KEY_ID     = [string]$bucketCredentials.accessKeyId
    TM_BACKUP_S3_SECRET_ACCESS_KEY = [string]$bucketCredentials.secretAccessKey
    TM_BACKUP_S3_REGION            = [string]$bucketCredentials.region
    TM_BACKUP_REPOSITORY_PASSWORD  = $password
}

foreach ($environment in @('staging', 'production')) {
    Write-Host "Configuring $environment backup variables. This can take about one minute."
    foreach ($entry in $sharedValues.GetEnumerator()) {
        Invoke-RailwayValueSet -Environment $environment -Name $entry.Key -Value $entry.Value
    }
}
Invoke-RailwayValueSet -Environment staging -Name 'TM_BACKUP_REPOSITORY_PREFIX' -Value 'tm-staging'
Invoke-RailwayValueSet -Environment staging -Name 'TM_BACKUP_ENABLED' -Value 'true'
Invoke-RailwayValueSet -Environment staging -Name 'TM_AUTH_TOKEN_SHA256' -Value $stagingHash
Invoke-RailwayValueSet -Environment staging -Name 'TM_AUTH_TOKEN_EXPIRES_AT' -Value $stagingExpiry
Invoke-RailwayValueSet -Environment production -Name 'TM_BACKUP_REPOSITORY_PREFIX' -Value 'tm-production'
Invoke-RailwayValueSet -Environment production -Name 'TM_BACKUP_ENABLED' -Value 'false'

[Array]::Clear($randomBytes, 0, $randomBytes.Length)
$password = $null
$stagingToken = $null
$securePassword.Dispose()
$secureStagingToken.Dispose()
# Windows PowerShell 5.1 can translate an empty string into a null clipboard
# value and throw "Parameter name: text" after every Railway variable has
# already been configured. Replace any copied secret with harmless text.
Set-Clipboard -Value '[TM] staging token cleared'
Write-Host ''
Write-Host 'Secure STEP 9 Railway variables are configured.'
Write-Host 'Production backup remains disabled until the staging restore drill passes.'
[void](Read-Host 'Press Enter to close')

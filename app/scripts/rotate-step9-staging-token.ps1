[CmdletBinding()]
param(
    [string]$ProjectId = '7fcb22b5-db34-4e2b-a12a-cbc60391ff5f',
    [string]$Service = 'tm-server',
    [ValidateRange(1, 3650)]
    [int]$ValidDays = 90
)

$ErrorActionPreference = 'Stop'
trap {
    Write-Host ''
    Write-Host "Staging token rotation stopped: $($_.Exception.Message)" -ForegroundColor Red
    [void](Read-Host 'Press Enter to close')
    break
}

$pnpm = 'C:\Users\tkfk0\.cache\codex-runtimes\codex-primary-runtime\dependencies\bin\fallback\pnpm.cmd'
$nodeBin = 'C:\Users\tkfk0\.cache\codex-runtimes\codex-primary-runtime\dependencies\node\bin'
if (-not (Test-Path -LiteralPath $pnpm)) {
    throw 'Bundled pnpm was not found.'
}
$env:PATH = "$nodeBin;$([IO.Path]::GetDirectoryName($pnpm));$env:PATH"

$randomBytes = New-Object byte[] 32
$random = [System.Security.Cryptography.RandomNumberGenerator]::Create()
try {
    $random.GetBytes($randomBytes)
}
finally {
    $random.Dispose()
}
$secret = [Convert]::ToBase64String($randomBytes).TrimEnd('=').Replace('+', '-').Replace('/', '_')
$token = "tm_pat_v1_$secret"
Set-Clipboard -Value $token

Write-Host 'A new staging-only TM token is on the clipboard.' -ForegroundColor Green
Write-Host 'Save it in the password manager as: TM Railway staging device'
Write-Host 'Do not reuse the production token.'
while ($true) {
    $choice = Read-Host 'After saving, press Enter to apply; type R to copy the token again'
    if ([string]::IsNullOrWhiteSpace($choice)) {
        break
    }
    if ($choice.Trim() -ieq 'R') {
        Set-Clipboard -Value $token
        Write-Host 'The staging token was copied to the clipboard again.' -ForegroundColor Green
        continue
    }
    Write-Host 'Enter or R only.' -ForegroundColor Yellow
}

$sha256 = [System.Security.Cryptography.SHA256]::Create()
try {
    $hashBytes = $sha256.ComputeHash([System.Text.Encoding]::UTF8.GetBytes($token))
}
finally {
    $sha256.Dispose()
}
$tokenHash = -join ($hashBytes | ForEach-Object { $_.ToString('x2') })
$expiry = [DateTime]::UtcNow.AddDays($ValidDays).ToString(
    'yyyy-MM-ddTHH:mm:ss.fffZ',
    [Globalization.CultureInfo]::InvariantCulture
)

Write-Host 'Applying the new token hash to Railway staging ...'
$output = $tokenHash | & $pnpm dlx '@railway/cli@5.26.0' variable set TM_AUTH_TOKEN_SHA256 --stdin `
    --skip-deploys --project $ProjectId --service $Service --environment staging 2>&1
if ($LASTEXITCODE -ne 0) {
    throw 'Failed to set the staging token hash.'
}
$output = $expiry | & $pnpm dlx '@railway/cli@5.26.0' variable set TM_AUTH_TOKEN_EXPIRES_AT --stdin `
    --skip-deploys --project $ProjectId --service $Service --environment staging 2>&1
if ($LASTEXITCODE -ne 0) {
    throw 'Failed to set the staging token expiry.'
}

[Array]::Clear($randomBytes, 0, $randomBytes.Length)
$token = $null
$tokenHash = $null
Set-Clipboard -Value '[TM] staging token cleared'

Write-Host ''
Write-Host 'Staging token rotation completed.' -ForegroundColor Green
[void](Read-Host 'Press Enter to close')

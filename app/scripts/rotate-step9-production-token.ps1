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
    Write-Host "Production token rotation stopped: $($_.Exception.Message)" -ForegroundColor Red
    Write-Host 'Keep the new token in the password manager. Do not paste it into chat or a file.' -ForegroundColor Yellow
    [void](Read-Host 'Press Enter to close')
    break
}

$railwayStore = Join-Path $env:LOCALAPPDATA 'pnpm\store\v11\links\@railway\cli'
$railway = Get-ChildItem -LiteralPath $railwayStore -Filter 'railway.exe' -File -Recurse |
    Sort-Object FullName -Descending |
    Select-Object -First 1 -ExpandProperty FullName
if ([string]::IsNullOrWhiteSpace($railway) -or -not (Test-Path -LiteralPath $railway)) {
    throw 'Railway CLI was not found.'
}

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

Write-Host ''
Write-Host 'A new production TM token is on the clipboard.' -ForegroundColor Green
Write-Host 'Save it in the password manager as: TM Railway production device'
Write-Host 'The previous production token will stop working after the redeploy.' -ForegroundColor Yellow
while ($true) {
    $choice = Read-Host 'After saving, press Enter to apply; type R to copy the token again'
    if ([string]::IsNullOrWhiteSpace($choice)) {
        break
    }
    if ($choice.Trim() -ieq 'R') {
        Set-Clipboard -Value $token
        Write-Host 'The production token was copied to the clipboard again.' -ForegroundColor Green
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

Write-Host ''
Write-Host 'Applying the new token hash to Railway production ...'
$output = $tokenHash | & $railway variable set TM_AUTH_TOKEN_SHA256 --stdin `
    --skip-deploys --project $ProjectId --service $Service --environment production 2>&1
if ($LASTEXITCODE -ne 0) {
    throw "Failed to set the production token hash: $($output -join ' ')"
}

$output = $expiry | & $railway variable set TM_AUTH_TOKEN_EXPIRES_AT --stdin `
    --skip-deploys --project $ProjectId --service $Service --environment production 2>&1
if ($LASTEXITCODE -ne 0) {
    throw "Failed to set the production token expiry: $($output -join ' ')"
}

Write-Host 'Starting a production redeploy ...'
$deploymentOutput = & $railway redeploy --project $ProjectId --service $Service `
    --environment production --yes --json 2>&1
if ($LASTEXITCODE -ne 0) {
    throw "Failed to start the production redeploy: $($deploymentOutput -join ' ')"
}

[Array]::Clear($randomBytes, 0, $randomBytes.Length)
$secret = $null
$tokenHash = $null
$hashBytes = $null

Write-Host ''
Write-Host 'Production token rotation was applied and a redeploy was started.' -ForegroundColor Green
Write-Host "Token expiry (UTC): $expiry"
while ($true) {
    $choice = Read-Host 'Press Enter to clear the clipboard and close; type R to copy the token once more'
    if ([string]::IsNullOrWhiteSpace($choice)) {
        break
    }
    if ($choice.Trim() -ieq 'R') {
        Set-Clipboard -Value $token
        Write-Host 'The production token was copied to the clipboard again.' -ForegroundColor Green
        continue
    }
    Write-Host 'Enter or R only.' -ForegroundColor Yellow
}

$token = $null
Set-Clipboard -Value '[TM] production token cleared'
Write-Host 'Clipboard cleared.' -ForegroundColor Green

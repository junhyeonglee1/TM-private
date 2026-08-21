[CmdletBinding(SupportsShouldProcess = $true, ConfirmImpact = 'High')]
param(
    [string]$ProjectId = '7fcb22b5-db34-4e2b-a12a-cbc60391ff5f',
    [string]$Environment = 'production',
    [string]$Service = 'tm-server',
    [string]$WorkspaceId = '79f8bc24-07f7-4992-b306-8cedf542a551',
    [decimal]$HardLimitUsd = 30,
    [string]$ResultPath,
    [switch]$Apply,
    [switch]$Force
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$tmRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
if ([string]::IsNullOrWhiteSpace($ResultPath)) {
    $ResultPath = Join-Path $tmRoot 'dist\manual-cost-status-railway\result.json'
}
$ResultPath = [System.IO.Path]::GetFullPath($ResultPath)
$resultDirectory = Split-Path -Parent $ResultPath
New-Item -ItemType Directory -Force -Path $resultDirectory | Out-Null

if ($WorkspaceId -notmatch '^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$') {
    throw 'WorkspaceId must be a UUID.'
}
if ($HardLimitUsd -le 0) {
    throw 'HardLimitUsd must be greater than zero.'
}

$railway = Get-ChildItem -LiteralPath (Join-Path $env:LOCALAPPDATA 'pnpm\store\v11\links\@railway\cli') `
    -Filter railway.exe -File -Recurse |
    Sort-Object FullName -Descending |
    Select-Object -First 1 -ExpandProperty FullName
if ([string]::IsNullOrWhiteSpace($railway) -or -not (Test-Path -LiteralPath $railway -PathType Leaf)) {
    throw 'Railway CLI was not found.'
}

$summary = "environment=$Environment service=$Service workspace=$WorkspaceId hardLimitUsd=$HardLimitUsd"
if (-not $Apply) {
    Write-Host "Dry run: $summary"
    Write-Host 'This action stores a workspace-scoped Railway API token in a sealed service variable.'
    Write-Host 'Re-run with -Apply only after approving that workspace-wide credential scope.'
    exit 0
}
if ($Force) { $ConfirmPreference = 'None' }
if (-not $PSCmdlet.ShouldProcess("Railway $Environment/$Service", "Configure automatic cost status: $summary")) {
    exit 0
}

$secureToken = $null
$token = $null
$process = $null
$stage = 'secure-input'

try {
    for ($attempt = 1; $attempt -le 5; $attempt++) {
        $secureToken = Read-Host "Railway workspace token (attempt $attempt of 5)" -AsSecureString
        $token = [System.Net.NetworkCredential]::new('', $secureToken).Password.Trim()
        $validLength = $token.Length -ge 20 -and $token.Length -le 512
        $validCharacters = $token -notmatch '[\s"''`]' -and $token -match '^[!-~]+$'
        if ($validLength -and $validCharacters) {
            break
        }
        Write-Warning 'The Railway workspace token format is invalid. Try again.'
        $token = $null
        $secureToken.Dispose()
        $secureToken = $null
        Set-Clipboard -Value '[TM] secure credential input cleared' -ErrorAction SilentlyContinue
    }
    if ([string]::IsNullOrWhiteSpace($token)) {
        throw 'No valid Railway workspace token was entered.'
    }

    $stage = 'non-secret-variables'
    & $railway variable set `
        "TM_RAILWAY_WORKSPACE_ID=$WorkspaceId" `
        "TM_RAILWAY_HARD_LIMIT_USD=$HardLimitUsd" `
        --project $ProjectId `
        --environment $Environment `
        --service $Service `
        --skip-deploys | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw 'Railway rejected the non-secret cost status variables.'
    }

    $stage = 'sealed-token-variable'
    $arguments = @(
        'variable', 'set', 'TM_RAILWAY_API_TOKEN', '--stdin',
        '--project', $ProjectId,
        '--environment', $Environment,
        '--service', $Service
    )
    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $railway
    $startInfo.Arguments = ($arguments -join ' ')
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardInput = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true

    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    if (-not $process.Start()) {
        throw 'Railway CLI could not be started.'
    }
    $process.StandardInput.Write($token)
    $process.StandardInput.Close()
    $standardOutput = $process.StandardOutput.ReadToEnd()
    $standardError = $process.StandardError.ReadToEnd()
    $process.WaitForExit()
    if ($process.ExitCode -ne 0) {
        throw "Railway rejected the sealed token variable (exit $($process.ExitCode))."
    }

    $stage = 'complete'
    [ordered]@{
        success = $true
        configuredAtUtc = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ss.fffZ')
        projectId = $ProjectId
        environment = $Environment
        service = $Service
        workspaceId = $WorkspaceId
        hardLimitUsd = $HardLimitUsd
        variableNames = @('TM_RAILWAY_API_TOKEN', 'TM_RAILWAY_WORKSPACE_ID', 'TM_RAILWAY_HARD_LIMIT_USD')
        secretValuePersistedLocally = $false
        deploymentTriggered = $true
    } | ConvertTo-Json | Set-Content -LiteralPath $ResultPath -Encoding utf8

    Write-Host ''
    Write-Host 'Railway automatic cost status variables were configured.' -ForegroundColor Green
    Write-Host 'The workspace token was not written to the result file, console, or clipboard.'
    Write-Host 'One Railway deployment was triggered.'
}
catch {
    [ordered]@{
        success = $false
        failedAtUtc = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ss.fffZ')
        stage = $stage
        reason = $_.Exception.Message
        secretValuePersistedLocally = $false
    } | ConvertTo-Json | Set-Content -LiteralPath $ResultPath -Encoding utf8
    throw "Railway cost status configuration failed at ${stage}: $($_.Exception.Message)"
}
finally {
    $standardOutput = $null
    $standardError = $null
    $token = $null
    if ($null -ne $secureToken) { $secureToken.Dispose() }
    if ($null -ne $process) { $process.Dispose() }
    Set-Clipboard -Value '[TM] secure credential input cleared' -ErrorAction SilentlyContinue
}

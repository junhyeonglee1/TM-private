[CmdletBinding()]
param(
    [string]$ProjectId = '7fcb22b5-db34-4e2b-a12a-cbc60391ff5f',
    [string]$EnvironmentId = 'bce65358-1686-4fe9-bd84-d51fac5358c0',
    [string]$ServiceId = 'f8e4b51d-afb3-4429-a5e6-206a1a957bb0',
    [string]$ResultPath
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$tmRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
if ([string]::IsNullOrWhiteSpace($ResultPath)) {
    $ResultPath = Join-Path $tmRoot 'dist\manual-step11-openai-secret\result.json'
}
$ResultPath = [System.IO.Path]::GetFullPath($ResultPath)
$resultDirectory = Split-Path -Parent $ResultPath
New-Item -ItemType Directory -Force -Path $resultDirectory | Out-Null

$railway = Get-ChildItem (Join-Path $env:LOCALAPPDATA 'pnpm\store\v11\links\@railway\cli') `
    -Filter railway.exe -File -Recurse |
    Sort-Object FullName -Descending |
    Select-Object -First 1 -ExpandProperty FullName
if ([string]::IsNullOrWhiteSpace($railway) -or -not (Test-Path -LiteralPath $railway -PathType Leaf)) {
    throw 'Railway CLI was not found.'
}

$secureKey = $null
$apiKey = $null
$process = $null
$stage = 'secure-input'

try {
    for ($attempt = 1; $attempt -le 5; $attempt++) {
        $secureKey = Read-Host "OPENAI_API_KEY (attempt $attempt of 5)" -AsSecureString
        $apiKey = [System.Net.NetworkCredential]::new('', $secureKey).Password.Trim()
        if ($apiKey -match '^sk-[A-Za-z0-9_-]{20,2045}$') {
            break
        }
        Write-Warning 'The OpenAI API key format is invalid. Try again.'
        $apiKey = $null
        $secureKey.Dispose()
        $secureKey = $null
        Set-Clipboard -Value '[TM] secure credential input cleared' -ErrorAction SilentlyContinue
    }
    if ([string]::IsNullOrWhiteSpace($apiKey)) {
        throw 'No valid OpenAI API key was entered.'
    }

    $stage = 'railway-sealed-variable'
    $arguments = @(
        'variable', 'set', 'OPENAI_API_KEY', '--stdin',
        '--project', $ProjectId,
        '--environment', $EnvironmentId,
        '--service', $ServiceId
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
    $process.StandardInput.Write($apiKey)
    $process.StandardInput.Close()
    $standardOutput = $process.StandardOutput.ReadToEnd()
    $standardError = $process.StandardError.ReadToEnd()
    $process.WaitForExit()
    if ($process.ExitCode -ne 0) {
        throw "Railway rejected the sealed variable update (exit $($process.ExitCode))."
    }

    $stage = 'complete'
    [ordered]@{
        success = $true
        configuredAtUtc = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ss.fffZ')
        projectId = $ProjectId
        environmentId = $EnvironmentId
        serviceId = $ServiceId
        variableName = 'OPENAI_API_KEY'
        secretValuePersistedLocally = $false
        deploymentTriggered = $true
    } | ConvertTo-Json | Set-Content -LiteralPath $ResultPath -Encoding utf8

    Write-Host ''
    Write-Host 'OPENAI_API_KEY was sent to the Railway sealed variable input.' -ForegroundColor Green
    Write-Host 'The secret value was not written to the result file, console, or clipboard.'
    Write-Host 'Railway deployment was triggered.'
}
catch {
    [ordered]@{
        success = $false
        failedAtUtc = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ss.fffZ')
        stage = $stage
        reason = $_.Exception.Message
        secretValuePersistedLocally = $false
    } | ConvertTo-Json | Set-Content -LiteralPath $ResultPath -Encoding utf8
    throw "STEP 11 OpenAI secret configuration failed at ${stage}: $($_.Exception.Message)"
}
finally {
    $standardOutput = $null
    $standardError = $null
    $apiKey = $null
    if ($null -ne $secureKey) { $secureKey.Dispose() }
    if ($null -ne $process) { $process.Dispose() }
    Set-Clipboard -Value '[TM] secure credential input cleared' -ErrorAction SilentlyContinue
}

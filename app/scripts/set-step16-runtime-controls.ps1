[CmdletBinding(SupportsShouldProcess = $true, ConfirmImpact = 'High')]
param(
    [ValidateSet('normal', 'read-only', 'lockdown')]
    [string]$IncidentMode = 'normal',

    [bool]$AiEnabled = $true,

    [ValidateSet('production', 'staging')]
    [string]$Environment = 'production',

    [switch]$Apply
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$project = '7fcb22b5-db34-4e2b-a12a-cbc60391ff5f'
$service = if ($Environment -eq 'production') { 'tm-server' } else { 'tm-server-staging' }
$railway = Get-ChildItem -LiteralPath (Join-Path $env:LOCALAPPDATA 'pnpm\store\v11\links\@railway\cli') `
    -Filter railway.exe -File -Recurse |
    Sort-Object FullName -Descending |
    Select-Object -First 1 -ExpandProperty FullName
if ([string]::IsNullOrWhiteSpace($railway)) {
    throw 'Railway CLI was not found.'
}

$aiValue = $AiEnabled.ToString().ToLowerInvariant()
$summary = "environment=$Environment service=$service incident=$IncidentMode aiEnabled=$aiValue"
if (-not $Apply) {
    Write-Host "Dry run: $summary"
    Write-Host 'Re-run with -Apply after confirming the incident response decision.'
    exit 0
}

if (-not $PSCmdlet.ShouldProcess("Railway $Environment/$service", "Apply STEP 16 runtime controls: $summary")) {
    exit 0
}

& $railway variable set `
    "TM_INCIDENT_MODE=$IncidentMode" `
    "TM_AI_ENABLED=$aiValue" `
    --project $project `
    --service $service `
    --environment $Environment | Out-Null
if ($LASTEXITCODE -ne 0) {
    throw 'Railway rejected the STEP 16 runtime control update.'
}

Write-Host "STEP 16 runtime controls applied: $summary" -ForegroundColor Green
Write-Host 'A Railway deployment was started. Verify /readyz and /api/v1/ops/status before taking another action.'

[CmdletBinding(SupportsShouldProcess = $true, ConfirmImpact = 'High')]
param(
    [ValidateSet('true', 'false')]
    [string]$Enabled,

    [ValidateSet('production', 'staging')]
    [string]$Environment = 'production',

    [switch]$Apply,

    [switch]$Force
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

$summary = "environment=$Environment service=$service taskReportEnabled=$Enabled"
if (-not $Apply) {
    Write-Host "Dry run: $summary"
    Write-Host 'Re-run with -Apply after confirming the feature rollout decision.'
    exit 0
}
if ($Force) { $ConfirmPreference = 'None' }
if (-not $PSCmdlet.ShouldProcess("Railway $Environment/$service", "Apply STEP 17 feature control: $summary")) {
    exit 0
}

& $railway variable set `
    "TM_TASK_REPORT_ENABLED=$Enabled" `
    --project $project `
    --service $service `
    --environment $Environment | Out-Null
if ($LASTEXITCODE -ne 0) {
    throw 'Railway rejected the STEP 17 Task report feature control update.'
}

Write-Host "STEP 17 feature control applied: $summary" -ForegroundColor Green
Write-Host 'A Railway deployment was started. Verify readiness and operations status before continuing.'

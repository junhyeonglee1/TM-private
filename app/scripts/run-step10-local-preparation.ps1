[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

& (Join-Path $PSScriptRoot 'prepare-step10-local-snapshot.ps1')

Write-Host 'STEP 10 downloaded Windows build and local snapshot preparation completed.' -ForegroundColor Green

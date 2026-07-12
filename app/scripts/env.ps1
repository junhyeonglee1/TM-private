$tmRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$nodeDir = "C:\Users\tkfk0\.cache\codex-runtimes\codex-primary-runtime\dependencies\node\bin"

$env:CARGO_HOME = Join-Path $tmRoot "dist\cache\cargo-home"
$env:CARGO_TARGET_DIR = Join-Path $tmRoot "dist\build\cargo"
$env:PATH = "$nodeDir;$env:PATH"

Write-Output "TM build environment configured for this PowerShell process."


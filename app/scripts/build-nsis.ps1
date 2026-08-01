[CmdletBinding()]
param(
    [switch]$Approved
)

$ErrorActionPreference = 'Stop'

if (-not $Approved) {
    throw 'NSIS bundling is not approved. Re-run only after explicit approval and pass -Approved.'
}

# Windows Application Control blocks Cargo-generated build scripts on this PC.
# Do not retry locally or with administrator privileges. The supported release
# path is app/scripts/build-release.ps1, which consumes the successful and
# hash-verified STEP 10 Windows build artifact without executing its binaries.
throw @'
Local NSIS bundling is disabled by TM build policy because it would invoke Cargo and Tauri on this PC.
Use build-release.ps1 -Approved -RunId <successful STEP 10 run id> -ExpectedHeadSha <40-character commit SHA> to retrieve tm.exe, tm-cli.exe, and tm-office-decryptor.exe.
If an installer is required later, add an NSIS job to GitHub Actions and verify that installer there.
'@

[CmdletBinding()]
param(
    [ValidateSet('Cloud', 'Local')]
    [string]$Mode = 'Cloud',
    [string]$BaseUrl = 'https://tm-server-production-5573.up.railway.app'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$resource = 'TM Cloud Production'
$userName = 'single-user'
$configPath = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\data\cloud-client.json'))
$configDirectory = Split-Path -Parent $configPath

function Get-TmPasswordVault {
    return [Windows.Security.Credentials.PasswordVault,Windows.Security.Credentials,ContentType=WindowsRuntime]::new()
}

function Remove-TmCredentialIfPresent {
    param(
        [Parameter(Mandatory)]$Vault
    )

    $existingCredentials = @($Vault.RetrieveAll()) | Where-Object {
        $_.Resource -eq $resource -and $_.UserName -eq $userName
    }
    foreach ($existing in $existingCredentials) {
        $Vault.Remove($existing)
    }
}

if ($Mode -eq 'Cloud') {
    $uri = [System.Uri]$BaseUrl
    if (-not $uri.IsAbsoluteUri -or $uri.Scheme -ne 'https' -or -not [string]::IsNullOrEmpty($uri.UserInfo) -or -not [string]::IsNullOrEmpty($uri.Query) -or -not [string]::IsNullOrEmpty($uri.Fragment)) {
        throw 'BaseUrl must be a credential-free HTTPS origin.'
    }

    $secureToken = Read-Host 'TM production auth token' -AsSecureString
    $token = [System.Net.NetworkCredential]::new('', $secureToken).Password
    try {
        if ($token -notmatch '^tm_pat_v1_[A-Za-z0-9_-]{43}$') {
            throw 'The TM production token format is invalid.'
        }

        $vault = Get-TmPasswordVault
        Remove-TmCredentialIfPresent -Vault $vault
        $credential = [Windows.Security.Credentials.PasswordCredential,Windows.Security.Credentials,ContentType=WindowsRuntime]::new($resource, $userName, $token)
        $vault.Add($credential)

        $verified = $vault.Retrieve($resource, $userName)
        $verified.RetrievePassword()
        if ($verified.Password -ne $token) {
            throw 'Windows Credential Locker verification failed.'
        }
    }
    finally {
        if ($null -ne $secureToken) {
            $secureToken.Dispose()
        }
        $token = $null
        $verified = $null
        $credential = $null
        Set-Clipboard -Value '[TM] secure credential input cleared'
    }

    $config = [ordered]@{
        mode = 'cloud'
        baseUrl = $uri.GetLeftPart([System.UriPartial]::Authority)
    }
}
else {
    $config = [ordered]@{
        mode = 'local'
        baseUrl = $null
    }
}

[System.IO.Directory]::CreateDirectory($configDirectory) | Out-Null
$temporaryPath = "$configPath.tmp"
$json = $config | ConvertTo-Json -Compress
[System.IO.File]::WriteAllText($temporaryPath, $json, [System.Text.UTF8Encoding]::new($false))
Move-Item -LiteralPath $temporaryPath -Destination $configPath -Force

Write-Host "TM data mode configured: $($config.mode)" -ForegroundColor Green
Write-Host "Configuration: $configPath"
if ($Mode -eq 'Cloud') {
    Write-Host 'The token is stored only in Windows Credential Locker.' -ForegroundColor Green
}

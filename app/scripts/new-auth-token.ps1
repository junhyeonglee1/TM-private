[CmdletBinding()]
param(
    [ValidateRange(1, 3650)]
    [int]$ValidDays = 90
)

$ErrorActionPreference = 'Stop'

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

$sha256 = [System.Security.Cryptography.SHA256]::Create()
try {
    $hashBytes = $sha256.ComputeHash([System.Text.Encoding]::UTF8.GetBytes($token))
}
finally {
    $sha256.Dispose()
}
$hash = -join ($hashBytes | ForEach-Object { $_.ToString('x2') })
$expiresAt = [DateTime]::UtcNow.AddDays($ValidDays).ToString(
    'yyyy-MM-ddTHH:mm:ss.fffZ',
    [Globalization.CultureInfo]::InvariantCulture
)

Set-Clipboard -Value $token
[Array]::Clear($randomBytes, 0, $randomBytes.Length)

Write-Output 'TM 인증 토큰 원문을 클립보드에 복사했습니다.'
Write-Output '지금 비밀번호 관리자에 저장한 뒤 클립보드를 지우세요.'
Write-Output '토큰 원문을 채팅, 터미널, 파일 또는 Railway에 입력하지 마세요.'
Write-Output ''
Write-Output "TM_AUTH_TOKEN_SHA256=$hash"
Write-Output "TM_AUTH_TOKEN_EXPIRES_AT=$expiresAt"

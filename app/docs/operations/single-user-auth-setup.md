# STEP 5 단일 사용자 인증

이 문서는 Railway의 `tm-server`를 인터넷에 공개하기 전에 단일 사용자 인증 경계를 준비하고 검증하는 절차다. OpenAI API와 TM 사용자 데이터 API는 이 단계에서도 활성화하지 않는다.

## 초기 인증 모델

| 항목 | 결정 |
| --- | --- |
| 사용자 | 소유자 본인 1명 |
| 자격 증명 | 256-bit 무작위 장기 bearer token |
| 서버 저장 | 토큰 원문이 아닌 SHA-256 해시만 Railway 환경변수에 저장 |
| 활성 토큰 | 초기 버전은 1개 |
| 권장 만료 | 90일, 배포 전 사용자가 변경 가능 |
| 분실·폐기 | 새 토큰 해시로 교체하면 기존 토큰 즉시 무효화 |
| 다중 기기 | STEP 11 모바일 클라이언트에서 기기별 토큰 저장소로 확장 |

토큰은 256-bit 무작위 값이므로 SHA-256 해시만으로 원문을 현실적으로 복원할 수 없다. 비밀번호처럼 사람이 정한 낮은 엔트로피 문자열은 허용하지 않는다.

## 서버 안전장치

`cloud-authenticated` 프로필은 다음 조건을 모두 만족해야 시작한다.

- Railway project, environment, service ID와 `PORT`가 존재한다.
- `TM_SERVER_HOME`이 Railway Volume 안에 있다.
- `TM_AUTH_TOKEN_SHA256`이 64자리 SHA-256 hexadecimal 값이다.
- `TM_AUTH_TOKEN_EXPIRES_AT`이 미래의 RFC 3339 UTC 시각이다.
- `OPENAI_API_KEY`, `TM_OPENAI_*`, `TM_SERVER_BIND`가 설정되지 않았다.

공개 예외는 최소 응답을 반환하는 `/healthz`와 `/readyz`뿐이다. 그 외 존재하지 않는 경로까지 먼저 인증하므로 비인증 사용자는 API 구조를 열거할 수 없다.

- 실패 인증: 분당 20회, 초과 시 `429`와 `Retry-After: 60`
- 정상 인증 요청: 분당 120회
- 실패 시 토큰·Authorization header를 로그에 기록하지 않음
- 모든 응답에 `no-store`, CSP, HSTS, `nosniff`, frame 차단, referrer 차단 적용
- CORS 허용 header를 발행하지 않음
- Cookie 인증을 사용하지 않으므로 현재 bearer API에는 CSRF credential이 없음

## 사용자 작업 — 최초 토큰 생성

토큰은 사용자가 자신의 PowerShell에서 직접 생성한다. Codex가 대신 실행하지 않으며 원문을 채팅으로 받지 않는다.

```powershell
Set-Location 'C:\Users\tkfk0\Desktop\codex\TM\app'
powershell.exe -NoProfile -ExecutionPolicy Bypass -File '.\scripts\new-auth-token.ps1' -ValidDays 90
```

실행 결과:

1. 토큰 원문은 화면에 출력되지 않고 Windows 클립보드에만 복사된다.
2. 즉시 비밀번호 관리자에 `TM Railway primary device`라는 이름으로 저장한다.
3. 저장을 확인한 뒤 `Set-Clipboard -Value ''`로 클립보드를 지운다.
4. 화면에 출력된 `TM_AUTH_TOKEN_SHA256`과 `TM_AUTH_TOKEN_EXPIRES_AT`만 Railway에 입력한다.
5. 토큰 원문은 Railway, Git, 소스 파일, 채팅, 일반 로그에 입력하지 않는다.

90일이 아닌 기간을 원하면 `-ValidDays`만 변경한다. 만료 시각은 서버가 강제로 검사한다.

## Railway 적용 순서

1. 인증 코드를 `cloud-bootstrap` 상태로 먼저 배포한다.
2. 원격 build와 기존 `/readyz`가 정상인지 확인한다.
3. 사용자가 Railway Variables에 해시와 만료 시각을 직접 입력한다.
4. `TM_SERVER_PROFILE`을 `cloud-authenticated`로 변경한다.
5. Volume 서비스이므로 `railway.json`의 `multiRegionConfig`는 `null`로 두고 singleton 인스턴스 1개만 실행한다.
6. 새 배포가 정상 기동하고 로그에 비밀값이 없는지 확인한다.
7. 그 뒤에만 Railway 제공 HTTPS 도메인을 생성한다.
8. 공개 `/healthz`는 최소 정보만 반환하는지 확인한다.
9. 토큰 없음·잘못된 토큰은 `401`, 올바른 토큰은 `/api/v1/auth/status`에서 `200`인지 확인한다.
10. OpenAI와 TM 데이터 route는 계속 `404`인지 확인한다.

## 토큰을 화면에 남기지 않는 HTTPS 확인

production 도메인은 `https://tm-server-production-5573.up.railway.app`이다. PowerShell에서 다음과 같이 원문을 보안 입력으로 받는다.

```powershell
$secureToken = Read-Host 'TM 인증 토큰 입력' -AsSecureString
$token = [System.Net.NetworkCredential]::new('', $secureToken).Password
try {
    $result = Invoke-RestMethod `
        -Method Get `
        -Uri 'https://tm-server-production-5573.up.railway.app/api/v1/auth/status' `
        -Headers @{ Authorization = "Bearer $token" }

    if ($result.data.authenticated -ne $true) {
        throw '서버가 예상한 인증 결과를 반환하지 않았습니다.'
    }

    Write-Host 'TM 인증 성공' -ForegroundColor Green
}
finally {
    Remove-Variable secureToken, token, result -ErrorAction SilentlyContinue
}
```

명령 기록에는 토큰 원문이 남지 않는다.

STEP 7 read-only API까지 한 번에 검증하려면 저장소에 포함된 도구를 실행한다.

```powershell
Set-Location 'C:\Users\tkfk0\Desktop\codex\TM\app'
powershell.exe -NoProfile -ExecutionPolicy Bypass -File '.\scripts\verify-step7-production.ps1'
```

도구는 인증 상태, tasks 조회, ETag `304`, mutation `405`와 보안 header를 확인한다. 토큰 원문은 저장하지 않으며 결과 파일에는 상태 코드와 반환 개수만 기록한다.

STEP 8 write API 배포 후 실제 데이터를 만들지 않고 인증·precondition·금지 동작을 검증하려면 다음 도구를 실행한다.

```powershell
Set-Location 'C:\Users\tkfk0\Desktop\codex\TM\app'
powershell.exe -NoProfile -ExecutionPolicy Bypass -File '.\scripts\verify-step8-production.ps1'
```

이 도구는 유효한 token으로 Task·Note·Checklist write route까지 도달하되, 누락 precondition·잘못된 확인·존재하지 않는 UUID만 사용한다. 검사 전후 Task·Note 개수와 ETag가 같은지도 확인하며 성공 mutation은 수행하지 않는다.

## 2026-07-16 production 검증

- 인증 기반 배포 ID: `b49163c3-d882-41c0-9f98-e9e77d0ecae0`, 상태 `SUCCESS`
- STEP 7 read-only API 배포 ID: `d229020e-8d50-47d8-bcc8-782ac5909a49`, 상태 `SUCCESS`
- STEP 8 controlled write API 배포 ID: `9e3701df-acb7-4d89-b39a-ab665323a496`, 상태 `SUCCESS`
- 실행 프로필: `cloud-authenticated`
- 실행 구성: `multiRegionConfig: null`, singleton 인스턴스 1개
- Volume: `/var/lib/tm`, 5 GB, `Ready`
- 기존 SQLite 최초 초기화 시각 유지
- 공개 `/healthz`, `/readyz`: `200`
- 토큰 없음, 잘못된 토큰, 비인증 미등록 경로: `401`
- `Cache-Control: no-store`, CSP, HSTS, `nosniff`, frame·referrer 차단 header 확인
- 원문 토큰을 사용자 보안 입력으로 전달해 `/api/v1/auth/status`의 `200`과 `authenticated: true` 확인
- 인증된 `/api/v1/tasks`는 빈 cloud DB에서 `200`, `returned: 0`, `total: 0` 반환
- 동일 ETag의 `If-None-Match` 요청은 `304`, 인증된 `POST /api/v1/tasks`는 `405 METHOD_NOT_ALLOWED` 반환
- STEP 7 재배포 전후 SQLite 최초 초기화 시각 `2026-07-14T01:51:35.117Z` 유지
- STEP 8 `/readyz`가 schema 4·WAL·외래키·무결성 내부 검사 통과 후 `200` 반환
- 인증된 write precondition·operation 확인 오류는 `428`, 누락 resource 수정은 `404`, Task 삭제는 `405` 반환
- 비파괴 write 검사 전후 Task·Note 0건과 collection ETag 불변을 확인했으며 성공 mutation과 token 저장은 수행하지 않음

## 회전과 긴급 폐기

정기 회전 또는 분실 시 새 토큰을 생성하고 Railway의 해시·만료 시각을 함께 교체한다. 배포가 완료되는 즉시 이전 토큰은 사용할 수 없다. 초기 버전에는 중복 활성 기간이 없으므로 새 토큰을 비밀번호 관리자에 먼저 저장한 뒤 Railway 값을 변경한다.

의심되는 노출이 있으면 공개 도메인을 먼저 삭제하고 토큰을 교체한 뒤 다시 검증한다.

# STEP 16 사고 대응·복구 runbook

## 확정 운영 목표

| 항목 | 목표 |
|---|---|
| RPO | 24시간 |
| RTO | 2시간 |
| 잘못된 배포 rollback | 15분 |
| 외부 알림 | Railway·OpenAI 계정 email 및 provider in-app |
| 내부 알림 | Windows TM `기기 관리`의 운영 상태판 |
| 비용 ceiling | Railway $10 경고/$30 hard limit, OpenAI $10 email 경고/TM $20 hard stop |

## 심각도와 첫 조치

### P0 — 즉시 차단

인증 우회, primary/API/backup secret 노출, 사용자 데이터 변조, 비용 통제 실패가 확인되거나 강하게 의심되는 경우다.

1. 사건 시각과 Railway deployment ID만 기록한다. token/header/body는 복사하지 않는다.
2. `lockdown` 또는 domain 삭제로 외부 접근을 중단한다.
3. OpenAI key와 primary token을 새 값으로 회전하고 기존 값을 폐기한다.
4. 활성 모바일 기기를 전체 폐기한다.
5. 최신 backup·감사 원장·배포 source를 별도 위치에서 검증한다.
6. 새 credential과 정상 image로 read-only 상태를 먼저 확인한다.
7. 사용자 확인 후 `normal`로 복귀한다.

### P1 — 기능 제한

DB는 정상이지만 backup이 24시간 이상 오래됐거나 scheduler dead letter, 반복된 인증 제한, OpenAI 이상 비용이 발생한 경우다.

1. AI 문제면 `TM_AI_ENABLED=false`, 데이터 무결성 문제면 `read-only`를 적용한다.
2. `/api/v1/ops/status`와 Railway log에서 고정된 event·request ID만 확인한다.
3. 원인을 제거하고 backup/worker/비용 원장을 재검증한다.
4. 정상 확인 후에만 runtime control을 되돌린다.

### P2 — 관찰·계획 수정

단일 backup 재시도, 일시적인 upstream 429, 사용자 입력 오류처럼 안전장치가 정상 차단한 사건이다. 서비스를 유지하고 원인·빈도·다음 조치를 기록한다.

## runtime control 적용

기본은 dry run이다. 실제 변경에는 `-Apply`와 PowerShell 확인이 모두 필요하다.

```powershell
Set-Location 'C:\Users\tkfk0\Desktop\codex\TM\app'
powershell.exe -NoProfile -ExecutionPolicy Bypass -File '.\scripts\set-step16-runtime-controls.ps1' -IncidentMode read-only -AiEnabled $false -Apply
```

완전 차단:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File '.\scripts\set-step16-runtime-controls.ps1' -IncidentMode lockdown -AiEnabled $false -Apply
```

복귀:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File '.\scripts\set-step16-runtime-controls.ps1' -IncidentMode normal -AiEnabled $true -Apply
```

각 변경은 새 Railway deployment를 만든다. 다음 변경 전 deployment `SUCCESS`, `/readyz`, primary `/api/v1/ops/status`를 확인한다.

## domain 긴급 차단

runtime 배포를 기다릴 수 없는 P0에서만 수행한다. `railway domain`을 인자 없이 실행하면 새 domain을 생성하므로 절대 조회 명령으로 사용하지 않는다.

1. `railway domain list --service tm-server --environment production --project 7fcb22b5-db34-4e2b-a12a-cbc60391ff5f --json`으로 정확한 domain ID를 확인한다.
2. 대상이 production domain인지 다시 확인한다.
3. `railway domain delete <DOMAIN_OR_ID> --yes`에 project/service/environment를 모두 명시한다.
4. 복구 후 Railway 제공 domain을 다시 만들고 PWA origin·Secure cookie·CSRF 검증을 전부 반복한다.

domain 삭제·재생성은 URL이 바뀔 수 있어 자동 훈련하지 않는다. 실제 P0 또는 별도 사용자 승인 때만 수행한다.

## credential 회전 순서

### primary admin token

1. 새 token을 Windows 보안 입력에서 생성해 Credential Locker와 비밀번호 관리자에 먼저 저장한다.
2. Railway의 `TM_AUTH_TOKEN_SHA256`, `TM_AUTH_TOKEN_EXPIRES_AT`을 함께 변경한다.
3. 새 deployment가 성공하면 새 token으로 인증한다.
4. 이전 token이 `401`인지 확인한다.
5. 활성 기기 전체 폐기 여부를 결정한다.

도구는 `scripts/rotate-step9-production-token.ps1`을 사용한다. 원문을 shell history·Git·로그·검증 JSON에 기록하지 않는다.

### OpenAI API key

1. OpenAI project에서 새 restricted key를 만든다.
2. key 원문은 임시 보안 입력으로 Railway `OPENAI_API_KEY`에 전달한다.
3. 새 deployment에서 AI status와 1회의 명시적 probe를 확인한다.
4. 이전 key를 OpenAI platform에서 폐기한다.
5. TM 비용 원장과 platform Usage에서 probe 한 건만 증가했는지 확인한다.

### 기기 token

Windows TM에서 해당 기기 또는 활성 기기 전체를 폐기한다. 새 기기는 6자리 코드로 다시 등록한다. 서버 DB에는 token 원문이 없으므로 원문을 복구하거나 재발급하지 않는다.

## backup 복구

1. `railway-backup-restore-drill`로 최신 snapshot을 `/tmp`에 복원한다.
2. manifest SHA-256, schema, `integrity_check`, `foreign_key_check`를 확인한다.
3. 활성 `/var/lib/tm`에는 쓰지 않는다.
4. 실제 복구는 새 Volume/환경에 복원하고 동일 image의 migration과 `/readyz`를 통과시킨다.
5. read-only 비교 후 traffic을 전환한다.

현재 Volume에 과거 backup을 덮어쓰는 복구는 append-only 비용·mutation·기기 폐기 원장을 되감을 수 있어 금지한다.

## 배포 rollback

schema가 호환되는 경우 이전 검증 source를 새 Railway deployment로 다시 올린다. health gate가 성공하기 전 기존 정상 deployment를 제거하지 않는다. 이전 image로 돌아간 뒤 `/readyz`, 인증, read-only collection, backup 상태를 확인하고 원인을 수정한 현재 image를 다시 승격한다.

## 종료 조건

- 노출된 credential이 모두 폐기됨
- `/readyz`와 authenticated operations status 통과
- schema와 backup integrity 일치
- scheduler dead letter 0
- AI/TM/Railway 비용 ceiling 정상
- 재발 방지 test가 CI에 추가됨
- 사건 중 생성한 SSH/API token·임시 파일 삭제 확인

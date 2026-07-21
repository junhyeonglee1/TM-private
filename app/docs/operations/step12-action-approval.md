# STEP 12 실행 승인 운영 절차

## 배포 원칙

1. Windows CI에서 format, 전체 test, clippy를 통과시킨다.
2. Railway 배포 후 schema 6과 STEP 12 status contract를 비용 없이 확인한다.
3. 운영 업무 데이터를 쓰지 않고 격리된 임시 DB에서 제안→승인→재시도 acceptance를 수행한다.
4. 같은 source를 production에 배포하고 non-billable read-only verification을 먼저 수행한다.
5. production에서는 실제 Task를 만들지 않고 action route 인증·빈 목록·confirmation guard까지만 확인한다.

## 비과금 production 확인값

- `schemaVersion = 6`
- `assistantPromptVersion = step12-v1`
- `assistantAutomaticReadToolCount = 7`
- `assistantTaskCreateApprovalEnabled = true`
- `assistantActionApprovalTtlSeconds = 600`
- `assistantAutoExecuteWithoutApproval = false`
- `assistantApprovalExecutesImmediately = true`
- `responseStorage = disabled`
- OpenAI 월 warning USD 10, hard limit USD 20
- TM 내부 warning USD 10, hard stop USD 20

## 장애 대응

- `pending` 만료: 새 제안을 만든다. 기존 payload를 재활성화하지 않는다.
- `409`: 최신 action을 다시 조회하고 상태·revision·hash를 확인한다.
- 응답 유실: 동일 approval idempotency key와 동일 hash로 재전송한다.
- `executing` 정체: 동일 승인 identity로 재전송한다. 새로운 key를 만들지 않는다.
- `failed`: failure code와 mutation audit를 확인하고 새로운 제안으로 다시 시작한다.
- restore 필요: approval ledger가 현재와 정확히 일치하는 snapshot만 허용한다.

승인 API는 OpenAI를 호출하지 않으므로 승인 재시도 자체로 AI 요금이 발생하지 않는다.

## 2026-07-21 완료 검증

- GitHub Actions Windows runner에서 frontend lint·typecheck·test, Rust workspace format·test·Clippy, `tm.exe`·`tm-cli.exe` release build와 artifact 업로드를 통과했다.
- Railway Linux Docker build에서 전체 workspace test·Clippy와 `tm-server` release build를 통과했다. 특히 승인 흐름 6개 테스트와 schema 1 backup 복원·schema 6 migration 회귀 테스트를 실제 실행했다.
- production 배포 `dc4646e5-23b8-4774-ae26-5430f4f262a6`, image digest `sha256:ad93759f0ba5c50c5f257f536b64f7e2a90ffed6ef1e9c26f363c6f0ee933060`이 `SUCCESS`로 완료됐다.
- production에서 schema 6, prompt `step12-v1`, 자동 read tool 7개, `task.create` 승인, TTL 600초, 자동 실행 금지, 승인 즉시 실행 계약을 확인했다.
- 기존 action 0건과 confirmation header가 없는 승인 요청의 HTTP 428 차단을 확인했다.
- production 검증 중 OpenAI 호출과 데이터 mutation은 모두 0회였다.
- 첫 배포 `feb07aea-c1a0-4607-b9e7-97cb1089487d`는 legacy schema 1 test fixture가 새 승인 테이블을 제거하지 않아 Docker test에서 실패했다. 기존 production 인스턴스에는 영향이 없었고 fixture 수정 후 전체 검증을 다시 통과했다.

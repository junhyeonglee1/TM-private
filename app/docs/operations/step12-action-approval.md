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

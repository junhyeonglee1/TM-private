# STEP 9 백업·복구·모니터링·비용 안전장치

이 문서는 TM의 cloud 기준 원본 전환 전 필수 운영 기반을 정의한다. 아직 실제 사용자 데이터는 Railway로 이전하지 않는다.

## 확정 정책

| 항목 | 확정값 |
|---|---|
| 복구 목표 | RPO 24시간, RTO 2시간 |
| 외부 저장 | Singapore 리전 Railway Bucket `tm-encrypted-backups` |
| 암호화 | `restic` client-side encryption; 저장소 암호는 비밀번호 관리자와 Railway sealed variable에만 보관 |
| 보존 | 일 7개, 주 4개, 월 12개 |
| 환경 분리 | production과 staging을 분리하고 staging은 검증할 때만 실행 |
| Railway 비용 | USD 10 custom email 경고, USD 30 compute hard limit |
| OpenAI 비용 | USD 10 플랫폼 email 경고, TM 내부 USD 20 월 hard stop |
| 운영 지역 | Bucket `sin`; 서비스 리전은 각 Railway environment 설정을 따른다 |

Railway의 compute hard limit은 도달 시 workload를 모두 중단한다. USD 30 기준으로 기본 알림은 75%인 USD 22.50, 90%인 USD 27, 100%인 USD 30에서 오고, 별도 custom 경고를 USD 10에 둔다. OpenAI project monthly budget 알림은 soft limit이므로 API 호출을 중단하지 않는다. 실제 중단은 TM의 append-only 비용 원장과 요청 전 예약으로 수행한다.

## 백업 흐름

1. 컨테이너가 SQLite online `.backup` 명령으로 임시 snapshot을 만든다.
2. `integrity_check`, `foreign_key_check`, schema version을 검사한다.
3. snapshot과 SHA-256 manifest를 `restic`으로 암호화해 Bucket에 전송한다.
4. 일 7·주 4·월 12 정책으로 prune한다.
5. `restic check --read-data`로 실제 암호화 pack을 다시 읽어 검증한다.
6. 민감 정보가 없는 결과만 `backups/remote/status.json`에 기록한다.
7. 실패하면 고정된 실패 사유를 남기고 5분 후 재시도한다.

Railway Volume native backup은 Hobby plan에서 제공되지 않는다. 2026-07-16에 production과 staging Volume 모두 공식 GraphQL API로 일·주·월 schedule 적용을 시도했지만 `Not Authorized`로 거부되었고, dashboard의 Backups 화면도 표시되지 않았다. Pro로 올리기 전에는 이 기능에 의존하지 않으며, Bucket의 `restic` 복사본을 주 백업으로 사용한다. Pro 승격을 결정하면 native backup은 빠른 같은 project/environment 복구용 보조 계층으로 추가하되, Volume과 함께 삭제될 수 있으므로 Bucket backup을 대체하지 않는다.

## 환경과 secret

- staging과 production은 서로 다른 Volume instance와 `restic` prefix를 사용한다.
- staging token은 production token과 다르다.
- Bucket access key와 secret, repository password는 Railway variable과 사용자 비밀번호 관리자 외에는 저장하지 않는다.
- repository password를 잃으면 암호화 backup은 복구할 수 없다.
- Bucket 자체의 server-side encryption·versioning·object lock에 의존하지 않는다.
- 컨테이너가 침해되면 실행 중인 workload의 repository credential도 노출될 수 있다. 이 단순 구성의 잔여 위험은 STEP 14에서 별도 backup account/credential 분리 여부를 재평가한다.

필수 환경변수 이름만 다음과 같다. 실제 값은 문서·Git·로그에 남기지 않는다.

- `TM_BACKUP_ENABLED`
- `TM_BACKUP_REPOSITORY_PREFIX`
- `TM_BACKUP_REPOSITORY_PASSWORD`
- `TM_BACKUP_S3_ENDPOINT`
- `TM_BACKUP_S3_BUCKET`
- `TM_BACKUP_S3_ACCESS_KEY_ID`
- `TM_BACKUP_S3_SECRET_ACCESS_KEY`
- `TM_BACKUP_S3_REGION`

## 복구 훈련과 실제 복구

`railway-backup-restore-drill`은 최신 snapshot을 `/tmp`에만 복원하고 SHA-256, schema, SQLite integrity와 foreign key를 검사한다. 활성 DB는 바꾸지 않는다.

실제 장애 복구는 다음 순서를 지킨다.

1. public domain 또는 write traffic을 차단한다.
2. 새 빈 Volume 또는 별도 복구 environment를 준비한다.
3. 최신 검증 snapshot을 복원한다.
4. manifest SHA-256과 SQLite 무결성을 다시 확인한다.
5. 동일 image에서 schema migration과 `/readyz`를 통과시킨다.
6. read-only 조회와 행 개수·migration manifest를 비교한다.
7. 사용자 승인 후 traffic을 새 기준 원본으로 전환한다.

현재 append-only mutation·AI 비용 ledger가 있는 실행 중 DB에 과거 snapshot을 덮어쓰는 복원은 거부된다. 전체 Volume 손실처럼 현재 ledger가 없는 빈 환경에서 복구하거나, 별도 복구 환경에서 검증 후 전환해야 한다.

## 관측과 로그

- `/api/v1/ops/status`는 인증 후 DB 상태, 로컬 backup 개수, 외부 backup 결과만 반환한다.
- DB·backup 실제 경로, 환경변수, 인증 token/hash, Bucket credential은 반환하지 않는다.
- HTTP 로그는 request ID, allowlist된 route family, method, status, duration만 JSON으로 기록한다.
- URL의 임의 path, header, query, body는 로그에 기록하지 않는다.
- Railway의 deployment failed·crashed·OOM killed·usage alert는 email과 in-app으로 활성화되어 있다.
- CPU, RAM, disk, network 임계치 monitor는 Pro plan 전용이다. Hobby에서는 dashboard metric을 확인할 수 있지만 자동 임계치 알림은 사용할 수 없으므로 Pro 승격 전 잔여 위험으로 기록한다.
- Railway Hobby log 보존 기간보다 장기 이력이 필요해지면 STEP 14에서 외부 log sink를 추가한다.

## OpenAI 비용 원장

SQLite schema 5의 `ai_budget_ledger`는 수정·삭제가 불가능하다.

- API 호출 전에 최대 비용을 원자적으로 예약한다.
- 예약을 포함해 월 USD 20을 넘으면 네트워크 요청 전 `402 AI_MONTHLY_BUDGET_EXCEEDED`로 차단한다.
- 성공 후 input, cached input, output, total token과 실제 추정 비용으로 정산한다.
- 전송 여부를 알 수 없는 transport/response 오류는 예약액 전체를 보수적으로 비용 처리한다.
- 사전검사·인증 거절처럼 과금되지 않은 오류는 예약액을 해제한다.
- 오래된 backup으로 비용 원장을 되감는 복원은 거부한다.

현재 `gpt-5.6` 계산 기준은 2026-07-16 공식 가격인 input USD 5/M, cached input USD 0.50/M, output USD 30/M이다. 가격 또는 model을 바꿀 때 코드의 allowlist와 계산 기준을 함께 검토해야 한다.

## 승격 순서

1. staging에 image와 별도 token·Volume·backup prefix를 적용한다.
2. health/readiness와 schema 5를 확인한다.
3. 합성 Task 성공 mutation, 같은 idempotency key replay, stale version 충돌을 검증한다.
4. staging backup 생성과 빈 위치 restore drill을 통과시킨다.
5. production에는 같은 검증 image만 배포한다.
6. production backup을 활성화하고 첫 snapshot·restore drill·운영 상태를 확인한다.
7. staging workload를 다시 중지한다.

## 2026-07-16 검증 기록

- staging은 production과 다른 token, service, Volume, backup repository prefix로 분리했다.
- schema 5에서 Task 생성, idempotency replay, version 2 update, stale version `409 MUTATION_CONFLICT`를 검증했다.
- staging 암호화 backup snapshot `fa6d7de4`를 빈 임시 위치에 복원했다. 복원 DB는 schema 5, byte size 344064, SHA-256 `cd61d2302eacace9e6878d433021efb566edd21e7cb66cec25f667e4c3f144ad`로 manifest와 일치했고 SQLite integrity와 foreign key 검사를 통과했다.
- restore drill에 사용한 임시 Railway SSH key와 로컬 private/public key, known-hosts 파일은 모두 삭제했다.
- production에는 staging에서 검증한 동일 source를 deployment `4a0bd0d7-6a3d-4104-9eec-acb16bb78b7c`로 배포했다. production backup은 read-only schema 5 검증이 끝날 때까지 비활성 상태를 유지한다.
- production `cloud-authenticated` profile은 STEP 11 전까지 AI route를 의도적으로 노출하지 않는다. 따라서 STEP 9에서는 `/api/v1/ai/status`의 `404`를 격리 성공으로 보고, USD 10/20 runtime budget 상태 검증은 cloud AI route 활성화 시 수행한다.
- production 인증, DB health, schema 5 확인 후 encrypted backup을 활성화했고 deployment `b0cb9e0c-c7a0-4c51-af5f-8e18920813af`가 성공했다. 첫 production snapshot `ecb2b965`는 schema 5, byte size 344064로 생성됐다.
- production 인증 token을 새 256-bit token으로 회전하고 deployment `c4faad6b-a365-4e13-be20-fd907eb5b72e`의 성공을 확인했다. 새 token으로 read-only 인증, schema 5, 원격 backup `succeeded`, STEP 11 전 AI route 격리 `404`를 다시 검증했다.
- Railway workspace compute 사용량은 USD 10 email warning, USD 30 hard limit으로 설정했다.
- Railway 임시 API token은 native backup 지원 여부 확인 직후 모두 폐기했다.
- OpenAI API organization은 현재 free trial, credit USD 0.00, 결제수단 미등록 상태라 monthly budget UI가 활성화되지 않았다. USD 10 platform email alert는 사용자가 API 결제수단과 credit 구매를 승인한 뒤 설정한다. TM 내부 USD 20 hard stop은 결제수단과 무관하게 schema 5에서 적용된다.

## 2026-07-21 완료 기록

- production 최신 암호화 snapshot을 `/tmp`에 복원한 결과 schema 5, byte size 344064, SHA-256 `f3a2db077c12bb22d9f131534c8b8bca9e3c94a57455f30161060d14c0aa6ba6`로 manifest와 일치했고 SQLite integrity와 foreign key 검사를 통과했다. 활성 `/var/lib/tm` DB는 변경하지 않았다.
- production restore drill에 사용한 Railway 일회용 SSH key와 로컬 private/public key, 임시 known-hosts 파일을 모두 삭제하고 Railway 등록 key가 0개임을 확인했다.
- `tm-server-staging`의 active deployment를 0개로 중지했고 staging Volume은 보존했다.
- Railway Hobby에서는 Pro 전용 세부 resource monitor를 보류하고 기존 deployment failure·crash·OOM·usage 알림을 유지한다. OpenAI platform USD 10 alert는 결제수단을 등록하는 STEP 11에서 설정한다.

## 공식 참고

- [Railway Volume backups](https://docs.railway.com/volumes/backups)
- [Railway Buckets](https://docs.railway.com/storage-buckets)
- [Railway cost control](https://docs.railway.com/pricing/cost-control)
- [Railway observability](https://docs.railway.com/observability)
- [OpenAI API project budgets](https://help.openai.com/en/articles/9186755-managing-projects-in-the-api-platform)
- [OpenAI GPT-5.6 model and pricing](https://developers.openai.com/api/docs/models/gpt-5.6-sol)
- [restic S3 repository](https://restic.readthedocs.io/en/stable/030_preparing_a_new_repo.html)
- [restic retention](https://restic.readthedocs.io/en/stable/060_forget.html)

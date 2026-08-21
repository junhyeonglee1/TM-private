# STEP 12 AI 실행 승인 경계

STEP 12는 STEP 11의 7개 자동 read-only 도구를 유지하면서 첫 실행 도구로 `task.create`만 추가한다. AI가 할 수 있는 일은 잠긴 승인 요청을 만드는 것까지이며, 실제 Task 생성은 인증된 사용자의 별도 승인 요청에서만 수행된다.

## 확정 정책

| 항목 | 정책 |
| --- | --- |
| 자동 허용 | 기존 7개 조회·검색 도구 |
| 첫 실행 도구 | `task.create` |
| 승인 유효 시간 | 생성 후 10분 |
| 승인 방식 | 1회 승인 후 즉시 실행 |
| 잠금 대상 | title, description, project, due date, priority, status |
| 무결성 | immutable payload + SHA-256 + revision CAS |
| 중복 방지 | 승인 idempotency key + 고정 execution idempotency key |
| 비활성 AI mutation | task update, note create/update, checklist update |
| 금지 | 외부 전송, 금전, 삭제, 계정·권한 변경 |
| OpenAI 비용 | 제안 단계만 사용하며 승인·실행 단계는 OpenAI를 호출하지 않음 |

## 상태 machine

`pending`은 `approved`, `rejected`, `cancelled`, `expired` 중 하나로만 이동한다. 승인 요청은 곧바로 `approved -> executing`으로 전환되고 기존 controlled mutation 경계를 통해 실행된다. 성공하면 `completed`, domain 검증 실패면 `failed`가 된다. `executing`에서 프로세스가 중단되면 같은 승인 identity로 재시도할 수 있고, 고정 execution idempotency key가 Task 중복 생성을 막는다.

terminal 상태인 `completed`, `rejected`, `cancelled`, `expired`, `failed`는 변경할 수 없다. payload, hash, origin request ID, execution key, 생성·만료 시각은 SQLite trigger로 수정이 금지된다. 모든 전이는 `assistant_action_events`에 append-only로 기록된다.

## AI 제안 계약

오케스트레이터 prompt version은 `step12-v1`이다. 한 assistant 요청에서는 명시적인 Task 생성 요청이 있을 때 `propose_task_create`를 최대 한 번만 호출할 수 있다. 도구 결과와 API 응답은 `taskCreated: false`, action ID, revision, payload SHA-256, 만료 시각을 반환한다. 이 단계에서는 `tasks`와 mutation audit가 바뀌지 않는다.

## 인증된 승인 API

모든 경로는 `cloud-authenticated` profile에만 등록되며 bearer 인증을 먼저 통과해야 한다.

| Method | Route | 추가 조건 |
| --- | --- | --- |
| GET | `/api/v1/assistant/actions` | 최근 100개 조회, 만료 lazy transition |
| GET | `/api/v1/assistant/actions/{id}` | 안전한 preview와 결과만 반환 |
| POST | `/api/v1/assistant/actions/{id}/approve` | `Idempotency-Key`, `X-TM-Confirm-Action: task.create`, expected revision, payload SHA-256 |
| POST | `/api/v1/assistant/actions/{id}/reject` | `X-TM-Confirm-Action: reject`, expected revision, payload SHA-256 |
| POST | `/api/v1/assistant/actions/{id}/cancel` | `X-TM-Confirm-Action: cancel`, expected revision, payload SHA-256 |

승인은 locked preview와 같은 revision·hash를 제출해야 한다. 정확히 같은 승인을 재전송하면 저장된 실행 결과를 replay하며 Task를 다시 만들지 않는다. 다른 idempotency key, hash, 상태 또는 stale revision은 `409`로 거절한다.

## 실행·감사·복구

실제 생성은 `MutationApprovalPolicy::AiActionApproval`, actor `tm_ai_assistant`, expected version `absent`로 기존 mutation transaction을 호출한다. 따라서 Task, mutation idempotency record, before/after audit가 한 transaction에 기록된다. 승인 자체는 별도 ledger에 남는다.

backup과 migration manifest에는 두 approval ledger table이 포함된다. 현재 ledger를 누락하거나 되감는 restore는 거절한다. schema 5 backup을 정상 복원하는 경우에는 restore 후 schema 6 migration이 적용된다.

## 완료 검증

- 승인 전 Task 0건
- 승인 후 Task 1건과 `ai_action_approval` audit 1건
- 동일 승인 재시도와 동시 요청 후에도 Task 1건
- stale revision, hash 변조, 만료, 거절 후 승인 차단
- payload·terminal row·event ledger SQL 변조 차단
- `executing` 중단 상태에서 안전한 재시도
- 인증·confirmation header·body size 경계
- 승인 요청 처리 중 OpenAI 추가 호출 0회


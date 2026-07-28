# 통제된 TM write API v1

## 경계와 허용 범위

STEP 8의 write API는 Railway `cloud-authenticated` profile에서만 활성화한다. 단일 사용자 bearer 인증을 통과해도 아래 여섯 동작 외에는 실행할 수 없다.

| Operation | Endpoint | 허용 동작 |
|---|---|---|
| `project.create` | `POST /api/v1/projects` | Project 생성 |
| `task.create` | `POST /api/v1/tasks` | Task 생성 |
| `task.update` | `PATCH /api/v1/tasks/{id}` | Task 허용 필드 수정 |
| `note.create` | `POST /api/v1/notes` | Note 생성 |
| `note.update` | `PATCH /api/v1/notes/{id}` | Note 허용 필드 수정 |
| `checklist.set_done` | `PATCH /api/v1/checklist/{id}` | `isDone` 상태만 변경 |

영구 삭제, 휴지통 이동, checklist 생성·삭제·본문 수정, 금전·주식 거래, 외부 전송, 계정·인증·권한 변경, attachment·file·URL 변경은 route와 command가 없다. 유효한 토큰으로 호출해도 등록되지 않은 route는 `404`, 등록 route의 허용되지 않은 method는 `405 METHOD_NOT_ALLOWED`다.

이 API는 OpenAI를 호출하지 않는다. STEP 11의 read-only orchestrator에는 write tool을 등록하지 않으며 AI mutation 승인 흐름은 STEP 12에서 별도로 연결한다.

## 필수 precondition

모든 mutation은 다음 값을 모두 요구한다.

- `Authorization: Bearer ...`: STEP 5 단일 사용자 토큰
- `Idempotency-Key`: 16–128자의 ASCII 영문·숫자·`.`·`_`·`:`·`-`
- `X-TM-Confirm-Mutation`: endpoint의 operation 이름과 정확히 일치
- 생성: `If-None-Match: *`
- 수정: read DTO의 양의 정수 `version`을 `If-Match: "<version>"`으로 전달
- `Content-Type: application/json`

하나라도 없으면 mutation을 실행하기 전에 `428`을 반환한다. `If-Match` 형식이 잘못되면 `400`, 현재 version과 다르면 `409 MUTATION_CONFLICT`다. 생성 응답은 `201`, 수정 응답은 `200`이며 결과 resource version을 strong `ETag: "<version>"`으로 반환한다.

`X-TM-Confirm-Mutation` 검사는 현재의 승인 정책 hook이다. 지금은 여섯 operation 모두 `explicit_user_confirmation` 정책이며, 향후 AI가 제안한 실행을 승인·보류하는 정책은 이 hook을 STEP 12에서 확장한다.

## idempotency와 transaction

서버는 operation, actor, expected version, approval policy, typed command를 canonical JSON으로 직렬화해 SHA-256을 계산한다. request ID는 재시도마다 바뀔 수 있으므로 hash에서 제외한다.

- 처음 보는 key: domain mutation, idempotency 결과, 감사 이벤트를 하나의 `BEGIN IMMEDIATE` transaction에서 commit한다.
- 같은 key·같은 요청: 저장한 결과를 재사용하고 `replayed: true`와 `X-TM-Idempotency-Replayed: true`를 반환한다.
- 같은 key·다른 요청: `409 MUTATION_CONFLICT`로 거부한다.
- validation·version·DB 실패: 전체 transaction을 rollback하며 key와 감사 성공 이벤트를 남기지 않는다.
- 동시 중복 제출: 한 요청만 실제 변경하고 나머지는 저장된 결과를 재사용한다.

## version과 상태 전이

Task, Note, Checklist는 schema 4부터 1에서 시작하는 정수 `version`을 가진다. 내용이 변경될 때마다 같은 transaction에서 1씩 증가한다. timestamp가 같은 밀리초에 겹쳐도 version 충돌 검사가 유지된다.

Project는 수정 route가 없는 create-only resource라 별도 version 열이 없다. `project.create`는 생성 precondition인 `If-None-Match: *`를 요구하고 생성 결과와 감사 원장에는 논리적 version `1`을 기록한다.

원격 Task 상태 전이는 다음만 허용한다.

| 현재 | 허용 다음 상태 |
|---|---|
| `inbox` | `todo`, `in_progress`, `cancelled` |
| `todo` | `inbox`, `in_progress`, `blocked`, `done`, `cancelled` |
| `in_progress` | `todo`, `blocked`, `done`, `cancelled` |
| `blocked` | `todo`, `in_progress`, `cancelled` |
| `done` | `todo`, `in_progress` |
| `cancelled` | `inbox`, `todo` |

새 Task는 `done`이나 `cancelled`로 만들 수 없다. 실제 변경이 없는 update와 checklist 동일 상태 재설정도 거부한다.

## 입력·응답 제한

- JSON body 상한: 64KiB
- Project 이름: 공백 제거 후 1–500자
- Project description: 최대 20,000자
- Project color: 생략·`null` 또는 `#RRGGBB`
- Task·Note 제목: 공백 제거 후 1–500자
- Task description: 최대 20,000자
- Note body: 최대 50,000자
- Task priority: 0–4
- ID: UUID
- 날짜: `YYYY-MM-DD`
- 알 수 없는 JSON 필드: `400 INVALID_MUTATION_JSON`

요청·응답 DTO는 [write API JSON Schema](../contracts/tm-write-api-v1.schema.json)에 고정한다. 응답 item은 read API와 같은 allowlist DTO를 사용하므로 `deletedAt`, DB·파일·인증·감사 내부 필드는 노출하지 않는다.

## 감사와 복구 안전

성공한 실제 변경마다 `mutation_audit_events`에 actor, 최초 request ID, operation, resource, expected/resulting version, before/after, 결과, approval policy를 기록한다. `mutation_idempotency_records`와 감사 테이블은 SQLite trigger로 update와 delete를 차단한다. 로그에는 request ID, operation, resource ID, version, replay 여부만 남기며 제목·본문·token은 기록하지 않는다.

이미 mutation ledger가 있으면 그 ledger와 정확히 일치하지 않는 backup 복원을 거부한다. 따라서 오래된 backup이 감사·idempotency 이력을 조용히 지울 수 없다. 실제 rollback 이벤트와 운영 복구 절차는 STEP 9에서 확정한다.

## 오류 contract

| HTTP | Code | 의미 |
|---|---|---|
| 400 | `INVALID_MUTATION_JSON` | JSON 형식·필드 계약 위반 |
| 400 | `INVALID_MUTATION` | 길이·UUID·상태 전이 등 domain validation 실패 |
| 404 | `MUTATION_RESOURCE_NOT_FOUND` | 대상 resource 없음 |
| 409 | `MUTATION_CONFLICT` | stale version 또는 idempotency key 충돌 |
| 413 | `MUTATION_REQUEST_TOO_LARGE` | 64KiB 초과 |
| 428 | `MUTATION_PRECONDITION_REQUIRED` | 필수 header 누락 |
| 428 | `MUTATION_CONFIRMATION_REQUIRED` | operation 확인 불일치 |
| 500 | `MUTATION_FAILED` | 내부 실패; DB·SQL·경로는 숨김 |

모든 응답에는 request ID와 기존 `no-store`, CSP, HSTS, `nosniff`, frame·referrer 차단 header를 적용한다.

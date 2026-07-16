# 인증된 read-only TM API v1

## 경계

STEP 7의 API는 Railway `cloud-authenticated` profile에서만 활성화한다. `/healthz`와 `/readyz`를 제외한 모든 요청은 STEP 5의 단일 사용자 bearer token 인증과 정상 요청 rate limit을 먼저 통과해야 한다. 로컬 profile과 `cloud-bootstrap` profile에는 TM 데이터 route가 없다.

이 API는 OpenAI 호출 경계가 아니다. 인증된 TM client가 읽을 수 있는 데이터와 나중에 AI orchestrator가 모델에 전달해도 되는 데이터는 별도 정책이다. STEP 11과 STEP 12 전까지 어떤 TM 데이터도 OpenAI에 자동 전달하지 않는다.

## 공통 계약

- base path: `/api/v1`
- 이 문서의 collection 조회는 `GET`만 다룬다. STEP 8의 명시된 POST·PATCH는 [통제된 write API 계약](controlled-write-api-v1.md)을 따르며 그 밖의 method는 `405 METHOD_NOT_ALLOWED`다.
- pagination: `limit` 기본 50, 최소 1, 최대 100. `offset` 기본 0, 최대 10,000.
- 응답 상한: JSON envelope 전체 512KiB. 초과하면 `413 RESPONSE_TOO_LARGE`를 반환한다.
- versioning: URL major version과 [JSON Schema](../contracts/tm-read-api-v1.schema.json)로 DTO를 고정한다.
- cache/version: collection 콘텐츠 기반 strong `ETag`를 반환하고 `If-None-Match` 일치 시 `304`를 반환한다. Task·Note·Checklist의 정수 `version`은 write API의 optimistic concurrency에 사용하며 collection ETag와 별개다. 개인 데이터 응답은 계속 `Cache-Control: no-store`다.
- 오류: query, filter, sort가 허용 목록 밖이거나 형식이 틀리면 구조화된 `400 INVALID_QUERY`를 반환한다.
- DB 오류: client 응답에는 DB 경로, SQL, 내부 오류를 노출하지 않는다.
- 접근 로그: request ID, resource 이름, 전체·반환 행 수, offset만 기록한다. token, query 값, 제목, 본문은 기록하지 않는다.

collection 응답의 공통 형태:

```json
{
  "requestId": "019...",
  "data": {
    "items": [],
    "page": {
      "limit": 50,
      "offset": 0,
      "returned": 0,
      "total": 0,
      "nextOffset": null
    }
  }
}
```

## endpoint와 허용 query

| Endpoint | 허용 filter | 허용 sort | 기본 정렬 |
|---|---|---|---|
| `GET /api/v1/projects` | `archived` | `name`, `updated_desc` | 이름 오름차순, ID |
| `GET /api/v1/tasks` | `projectId`, `status`, `dueFrom`, `dueTo` | `updated_desc`, `due_asc`, `priority_desc` | 수정 시각 내림차순, ID |
| `GET /api/v1/checklist` | 필수 `taskId` | 고정 | 위치, ID |
| `GET /api/v1/tags` | 선택 `taskId` | 고정 | 이름 오름차순, ID |
| `GET /api/v1/sessions` | `projectId`, `status` | `started_desc`, `updated_desc` | 시작 시각 내림차순, ID |
| `GET /api/v1/worklogs` | `projectId`, `dateFrom`, `dateTo` | `date_desc`, `updated_desc` | 날짜·생성 시각 내림차순, ID |
| `GET /api/v1/notes` | `noteType`, `dateFrom`, `dateTo` | `updated_desc`, `date_desc` | 수정 시각 내림차순, ID |

모든 endpoint는 공통 `limit`, `offset`을 허용한다. ID는 UUID, 날짜는 `YYYY-MM-DD`, enum은 snake_case로 받는다. 알 수 없는 query key는 무시하지 않고 거부한다. 삭제된 항목은 어떤 route에서도 반환하지 않으며 project는 기본적으로 archive되지 않은 항목만 반환한다.

## field allowlist

| DTO | 반환 필드 |
|---|---|
| Project | `id`, `name`, `description`, `color`, `sortOrder`, `createdAt`, `updatedAt`, `archivedAt` |
| Task | `id`, `projectId`, `title`, `description`, `status`, `priority`, `dueDate`, `completedAt`, `createdAt`, `updatedAt`, `version` |
| Checklist | `id`, `taskId`, `body`, `isDone`, `sortOrder`, `createdAt`, `updatedAt`, `completedAt`, `version` |
| Tag | `id`, `name`, `color`, `createdAt` |
| Session | `id`, `projectId`, `goal`, `status`, `startedAt`, `endedAt`, `result`, `blockers`, `nextAction`, `createdAt`, `updatedAt` |
| Worklog | `id`, `sessionId`, `projectId`, `logDate`, `title`, `body`, `createdAt`, `updatedAt` |
| Note | `id`, `noteType`, `title`, `body`, `sourceWorklogId`, `noteDate`, `createdAt`, `updatedAt`, `version` |

내부 Rust model을 그대로 serialize하지 않고 위 DTO로 복사한다. 따라서 model에 새 필드가 생겨도 명시적으로 DTO와 Schema를 변경하기 전에는 외부 API에 노출되지 않는다.

## 노출하지 않는 데이터

다음 항목은 v1 route 자체가 없으며 DTO에도 포함하지 않는다.

- SQLite 파일, SQL, schema ledger, `app_state`, DB·backup 경로와 backup 원문
- API key, 인증 token·hash·만료 설정, Railway 환경변수
- `task_events`, `change_requests`, `change_request_events` 원문과 actor·before/after
- `digest_deliveries`와 외부 전달 reference
- attachment metadata, 상대·원본 파일 경로, checksum, 첨부파일 byte
- entity link의 file path와 URL
- 삭제 시각과 휴지통 데이터

위 항목 중 secret, 인증정보, DB·backup·log·파일 경로와 첨부 byte는 OpenAI에 절대 전달하지 않는다. 감사 ledger와 외부 전달 reference는 모델 입력에서 기본 제외하며, 향후 별도 기능에 꼭 필요한 경우에도 STEP 12의 명시적 정책과 사용자 승인이 선행되어야 한다. Task, Note, Session, Worklog의 본문은 API에 보인다는 이유만으로 모델에 자동 전달되지 않는다.

## 검증 범위

합성 임시 DB에서 다음을 자동 검증한다.

- 무인증 `401`, 인증된 7개 collection `200`
- DTO에 삭제·파일·DB·인증·감사 필드가 없음
- filter, pagination, sort allowlist와 잘못된 query `400`
- request ID와 무관한 콘텐츠 ETag 및 조건부 `304`
- 허용되지 않은 method의 `405`와 DB 미변경
- 512KiB 초과 `413`
- `cloud-bootstrap`에서 데이터 route `404`

실제 사용자 데이터의 cloud 이전은 STEP 10 전까지 금지된다. STEP 7 production 검증은 비어 있는 Railway DB와 합성되지 않은 빈 응답만 사용한다.

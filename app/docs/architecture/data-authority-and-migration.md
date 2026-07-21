# 데이터 기준 원본과 migration 안전 설계

## 목적과 현재 범위

TM의 첫 cloud 전환은 `로컬 기준 원본 → 검증된 일회성 cutover → cloud 기준 원본` 순서로 진행한다. 양방향 동기화와 dual-write는 도입하지 않는다. 두 DB를 동시에 기준 원본으로 취급하면 충돌 해결과 유실 판정이 복잡해지기 때문이다.

STEP 6에서는 실제 사용자 DB나 Railway 운영 DB를 복사하지 않는다. 합성 데이터가 든 임시 SQLite DB와 그 snapshot만 사용해 사전검사, 비교, 복구가 반복 가능한지 검증한다. 실제 이전과 기준 원본 전환은 STEP 10의 별도 승인 작업이다.

## 기준 원본 상태

| 시점 | 유일한 기준 원본 | 쓰기 정책 |
|---|---|---|
| 현재부터 STEP 10 전까지 | 로컬 TM DB | 로컬 앱만 쓰기 |
| STEP 10 점검·이전 구간 | 동결된 로컬 TM DB | 모든 쓰기 일시 중지 |
| STEP 10 검증 완료 후 | Railway Volume의 cloud TM DB | cloud API를 통한 쓰기만 허용 |
| cutover 검증 실패 | 기존 로컬 TM DB | cloud 쓰기 전에 복귀 |

cutover 뒤 cloud에 새 쓰기가 발생한 다음에는 과거 로컬 DB로 바로 되돌리지 않는다. 그 경우 cloud 백업을 기준으로 복구해야 하며 로컬과 cloud를 병합하지 않는다.

## 저장 데이터 inventory와 분류

| 영역 | 테이블·파일 | 분류 | 이전·노출 정책 |
|---|---|---|---|
| schema와 내부 상태 | `schema_migrations`, `app_state` | 운영 metadata | 이전 검증에는 포함하되 API와 OpenAI에는 직접 노출하지 않음 |
| 계획 | `projects`, `tasks`, `checklist_items`, `tags`, `task_tags`, `task_day_entries` | 개인 데이터 | 이전 대상. STEP 7에서 별도 DTO allowlist로만 조회 |
| Task 감사 이력 | `task_events` | 민감 개인·감사 데이터 | 이전 대상. 원문을 모델에 기본 전달하지 않음 |
| 작업 기록 | `work_sessions`, `session_tasks`, `worklogs` | 민감 개인 데이터 | 이전 대상. 최소 필드만 후속 API에서 허용 |
| 지식과 연결 | `notes`, `entity_links` | 고민·결정이 포함될 수 있는 고민감 데이터 | 이전 대상. OpenAI 전달은 목적별 명시 정책 필요 |
| 첨부 metadata | `attachments` | 개인·파일 metadata | SQLite manifest에 포함 |
| 첨부 원본 | TM 홈의 `attachments` 디렉터리 | 고위험 파일 데이터 | SQLite 밖의 별도 이전 lane. STEP 10에서 파일별 크기·checksum 검증 필요 |
| 전달 이력 | `digest_deliveries` | 운영·외부 참조 metadata | 중복 전송 방지를 위해 이전·복원 시 보존 |
| 승인·실행 감사 | `change_requests`, `change_request_events` | 보안·감사 데이터 | 이전 대상. 모델 입력에서 기본 제외하고 append-only 의미 보존 |
| 원격 mutation 통제 | `mutation_idempotency_records`, `mutation_audit_events` | 보안·감사 데이터 | 이전 대상. API·모델 입력에서 제외하고 append-only·중복 방지 의미 보존 |
| 검색 index | SQLite FTS `search_index` | 파생 데이터 | SQLite snapshot에는 포함되지만 논리 manifest에서 제외. 기준 테이블로 재생성 가능한 데이터 |

API 키, 인증 토큰 원문, 로그, DB 백업 파일, source snapshot은 TM 데이터 manifest와 실제 데이터 이전 대상에 넣지 않는다. secret은 Railway 환경변수와 사용자 비밀번호 관리자에만 둔다.

## 보존해야 하는 불변 조건

- SQLite `user_version`과 `schema_migrations` ledger가 현재 schema 5, migration `1, 2, 3, 4, 5`와 정확히 일치한다.
- `PRAGMA integrity_check`가 `ok`이며 `PRAGMA foreign_key_check` 결과가 없다.
- Task, checklist, tag, day entry, session 관계와 Note 다중 링크의 외래키가 유지된다.
- `task_events`, `change_request_events`, `mutation_audit_events`의 append-only 의미와 idempotency ledger가 유지된다.
- 확정된 day entry, 발송 완료 digest, 처리 이력이 생긴 change request를 과거 상태로 되감지 않는다.
- 실행 중 work session은 최대 한 건이고 entity link 중복·소유자 종류 제약이 유지된다.
- UUID, UTC RFC 3339 timestamp, `Asia/Seoul` 사용자 날짜 의미가 바뀌지 않는다.

## migration manifest v1

`tm-core`는 일관된 read transaction에서 다음 값만 만든다.

- format과 format version
- SQLite schema version과 적용 migration version 목록
- 논리 테이블별 행 개수와 SHA-256
- 위 값을 다시 묶은 전체 논리 SHA-256
- manifest 생성 시각

행 checksum은 컬럼명, SQLite 값의 실제 타입, 길이, 값을 canonical byte로 변환한 뒤 행 순서와 무관하게 계산한다. manifest에는 제목, Note 본문 같은 행 원문이 들어가지 않는다. 생성 시각은 기록용이며 두 DB의 논리 일치 판정에서는 제외한다.

## dry-run과 사전검사

1. 합성 fixture의 source manifest를 read transaction에서 생성한다.
2. SQLite online backup API로 일관된 snapshot을 만든다.
3. snapshot을 read-only로 열어 schema, migration ledger, integrity, foreign key를 검사한다.
4. source와 snapshot의 테이블별 행 개수·checksum·전체 checksum을 비교한다.
5. snapshot으로 복구한 뒤 동일 manifest가 다시 만들어지는지 자동 테스트한다.
6. schema가 다르거나 손상·관계 위반·논리 불일치가 있으면 cutover 후보를 거부한다.

현재 dry-run artifact는 SQLite DB만 포함한다. 첨부 원본 파일의 묶음 생성과 파일 checksum manifest는 실제 cutover 도구를 만드는 STEP 10에서 추가한다.

## STEP 10 실제 cutover runbook

실데이터 이전은 아래 순서와 별도 사용자 승인 없이는 시작하지 않는다.

1. 로컬 앱을 종료하고 쓰기 동결 시각을 기록한다.
2. 로컬 DB의 최종 백업과 migration manifest를 생성한다.
3. 첨부파일 bundle과 파일별 checksum manifest를 생성한다.
4. Railway cloud 서비스 쓰기를 maintenance mode로 막고 cloud 안전 백업을 만든다.
5. snapshot과 첨부 bundle을 cloud Volume으로 한 번만 가져온다.
6. cloud에서 read-only preflight와 source/cloud manifest 비교를 통과시킨다.
7. 대표 read-only API와 첨부파일 표본을 검증한다.
8. 모두 통과하면 cloud를 유일한 기준 원본으로 선언하고 cloud 쓰기를 연다.
9. 실패하면 cloud 쓰기를 열지 않은 채 기존 로컬 기준 원본으로 복귀한다.

로컬 DB는 cutover 이후 즉시 삭제하지 않고 암호화된 rollback 보관본으로 유지한다. 보관 기간과 삭제 시점은 STEP 10에서 사용자가 결정한다.

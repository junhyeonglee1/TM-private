# TM AI 비서 시스템 구축 로드맵

마지막 갱신: 2026-07-16

진행 원칙: 기능을 먼저 늘리지 않고, 안전한 서버·클라우드·AI 실행 기반을 선행 구축한다.

## 목표

TM을 일정, 운동, 소비, 자산 등의 개인 정보를 다루는 단일 사용자용 AI 비서로 확장할 수 있는 기반을 만든다. AI는 판단과 제안을 담당하고, 실제 데이터 저장과 실행 권한은 TM 서버가 통제한다.

현재 우선순위에서 제외한 기능:

- 운동 루틴 관리
- 일정·캘린더 관리
- 소비 패턴 분석
- 주식·자산 관리
- 자동 알림과 정기 리포트의 실제 내용

위 기능들은 서버, 인증, 권한, 데이터 경계, 오케스트레이터가 완성된 뒤 하나씩 추가한다.

## 역할 분담

### 사용자

- 클라우드·OpenAI 계정 생성과 요금제 선택
- API 키·비밀번호·OTP 등 비밀값 직접 입력
- 새 로그인, 결제, 외부 서비스 접근 권한 승인
- 데이터 공개 범위와 자동 실행 수준 결정
- 되돌리기 어렵거나 제품 방향을 바꾸는 중요한 결정
- 사용자 기기의 비밀값이 필요한 실제 동작만 직접 확인

### Codex

- 아키텍처 설계와 코드 작성
- 테스트·검증·문서화
- 보안 경계와 승인 흐름 구현
- 오류 분석과 수정
- 기존 인증 세션을 이용한 클라우드 설정·배포·운영 검증
- 각 단계의 로컬 Git 체크포인트 생성

API 키와 비밀번호는 채팅, Git, 소스 파일 또는 일반 로그에 기록하지 않는다.

## 2026-07-16 개편 기준

- 한 STEP에는 하나의 주요 위험과 하나의 Git 체크포인트만 둔다.
- 읽기 경계를 먼저 완성한 뒤 쓰기 권한을 추가한다.
- 로컬 DB는 통제된 cutover 전까지 기준 원본으로 유지한다.
- 양방향 동기화와 dual-write는 초기 기본안에서 제외한다.
- 실데이터를 클라우드로 옮기기 전에 백업·복구·모니터링을 준비한다.
- OpenAI 오케스트레이터는 데이터 API와 비용 제한이 완성된 뒤 연결한다.
- 사용자는 결정 게이트만 승인하고 구현·배포·검증·커밋은 Codex가 수행한다.

## 구축 Phase

| Phase | STEP | 목표 |
| --- | --- | --- |
| A. 기반 | 0–5 | 로컬 서버, OpenAI 연결 경계, Railway, 단일 사용자 인증 |
| B. 데이터 | 6–10 | 기준 원본, 읽기·쓰기 API, 운영 안전장치, 데스크톱 cloud cutover |
| C. AI | 11–14 | read-only 오케스트레이터, 실행 승인, 기억, 스케줄러 |
| D. 사용 | 15–17 | 모바일·기기 인증, 최종 보안 강화, 첫 실제 비서 기능 |

## 전체 상태

| STEP | 내용 | 상태 |
| --- | --- | --- |
| 0 | 현재 TM 기준선과 목표 범위 확인 | 완료 |
| 1 | 데이터·소스 백업과 로컬 Git 기준점 확보 | 완료 |
| 2 | localhost 전용 `tm-server` HTTP 기반 구축 | 완료 |
| 3 | OpenAI API 비밀키 경계와 실제 연결 확인 | 완료 |
| 4 | 클라우드 실행 환경 결정과 최소 배포 기반 준비 | 완료 |
| 5 | 단일 사용자 인증과 원격 접근 보호 | 완료 |
| 6 | 데이터 기준 원본과 migration 안전 설계 | 완료 |
| 7 | 인증된 read-only TM 데이터 API | 완료 |
| 8 | 통제된 write API와 감사 기록 | 완료 |
| 9 | 백업·복구·모니터링·비용 안전장치 | 완료 |
| 10 | 데스크톱 cloud mode와 일회성 cutover | 완료 |
| 11 | Cloud OpenAI와 read-only 오케스트레이터 | 진행 중 |
| 12 | 실행 승인·취소·도구 정책 | 대기 |
| 13 | 장기 기억·검색·컨텍스트 예산 | 대기 |
| 14 | 스케줄러·작업 큐·정기 실행 기반 | 대기 |
| 15 | 모바일·원격 클라이언트와 기기 인증 | 대기 |
| 16 | 보안 강화·사고 대응·운영 준비 | 대기 |
| 17 | 첫 실제 AI 비서 기능 | 대기 |

## STEP 0 — 현재 기준선 확인

상태: 완료

Action Item:

- [x] TM 버전과 현재 기능 확인
- [x] 로컬 SQLite 구조와 데이터 경로 확인
- [x] 기존 HTTP·클라우드·OpenAI 연결 부재 확인
- [x] 전체 테스트 기준선 확보
- [x] 부가 기능보다 시스템 기반을 먼저 만든다는 범위 확정

완료 조건:

- 기존 TM이 정상 동작하고, 이후 변경과 비교할 수 있는 검증 기준이 존재한다.

## STEP 1 — 백업과 버전 기준점

상태: 완료

Action Item:

- [x] 실행 중인 TM 프로세스 확인
- [x] 실제 SQLite DB 수동 백업과 무결성 검사
- [x] 소스 스냅샷 ZIP 생성
- [x] 백업 SHA-256과 논리 테이블 개수 기록
- [x] 사용자 데이터·백업·빌드 결과를 Git에서 제외
- [x] 로컬 Git 저장소와 기준 커밋 생성

주요 체크포인트:

- 기준선 커밋: `a0c6076 chore: record TM 0.1.5 baseline`

완료 조건:

- 문제가 생기면 원본 DB와 소스 기준점으로 돌아갈 수 있다.

## STEP 2 — 로컬 서버 기반

상태: 완료

Action Item:

- [x] Rust `tm-server` crate 추가
- [x] `GET /healthz` 구현
- [x] `GET /readyz` 구현
- [x] 공통 `requestId`와 구조화 오류 구현
- [x] 명시적인 절대 `TM_SERVER_HOME` 없이는 실행 차단
- [x] 임시 DB를 이용한 HTTP 테스트
- [x] 기존 프런트엔드·Rust 전체 회귀 검증

주요 체크포인트:

- 구현 커밋: `b775d4f feat: add local tm-server foundation`

완료 조건:

- TM 데스크톱 앱과 분리된 서버 프로세스가 안전한 로컬 HTTP 경계를 제공한다.

## STEP 3 — OpenAI API 연결 경계

상태: 완료

결정 사항:

- API: OpenAI Responses API
- 기본 모델: `gpt-5.6`, 환경변수로 변경 가능
- 비밀값: 서버 환경변수 `OPENAI_API_KEY`에서만 읽기
- API 응답 저장: `store: false`
- 외부 주소: 공식 `api.openai.com`만 허용

Action Item:

- [x] API 키가 없는 상태를 안전하게 표시하는 상태 API 구현
- [x] 최소 비용 연결 확인용 probe 구현
- [x] 과금 가능한 probe에 명시적 확인 헤더 요구
- [x] API 키가 디버그 출력·HTTP 응답에 나타나지 않는지 테스트
- [x] 비공식 HTTPS 주소로 키가 전달되지 않도록 차단
- [x] 인증 전에는 loopback 외의 주소로 서버 바인딩 차단
- [x] 사용자 직접 API 키 입력과 실제 연결 확인

주요 체크포인트:

- 구현 커밋: `bd95f52 feat: add secure OpenAI connection boundary`
- 상세 절차: [OpenAI 로컬 연결 설정](openai-local-setup.md)

완료 조건:

- 실제 OpenAI 연결이 성공하고 API 키가 코드·Git·로그에 남지 않는다.

## STEP 4 — 클라우드 실행 환경과 최소 배포

상태: 완료

현재 추천안:

- Railway Hobby
- 월 기본 요금은 현재 $5이며 $5의 리소스 사용량이 포함된다.
- 로컬 소스를 CLI로 직접 업로드할 수 있어 GitHub 원격 저장소가 필수는 아니다.
- Dockerfile 원격 빌드, 영구 Volume, 환경변수·sealed secret, 자동 HTTPS를 지원한다.
- 공식 문서: [요금](https://docs.railway.com/pricing/plans), [CLI 배포](https://docs.railway.com/cli/deploying), [Volume](https://docs.railway.com/volumes)

사용자 결정:

- [x] Railway Hobby로 진행할지 확정
- [x] Railway 계정 생성과 결제 수단 등록
- [x] 서버 지역 선택: Southeast Asia Metal, Singapore
- [x] 항상 실행하고 Serverless 모드는 끄기로 결정

권장 기본값:

- 단일 사용자용 Railway Hobby
- Singleton 인스턴스 1개, Railway replica 기능 끄기
- Serverless 끄기
- Restart Policy `Always`
- 영구 Volume 1개
- 인증 구현 전에는 공개 도메인 생성 금지

Codex Action Item:

- [x] Linux 컨테이너 호환성 점검
- [x] 멀티스테이지 Dockerfile 작성
- [x] 빌드 컨텍스트용 `.dockerignore` 작성
- [x] Railway의 `PORT`를 사용하는 cloud-bootstrap 실행 모드 구현
- [x] cloud-bootstrap 모드에서는 AI 호출과 TM 데이터 API 비활성화
- [x] `/readyz` 기반 배포 healthcheck 설정 작성
- [x] 영구 Volume 경로와 `TM_SERVER_HOME` 연결
- [x] Graceful shutdown과 재시작 정책 확인
- [x] 원격 빌드·배포 로그 확인
- [x] 공개 도메인 없이 클라우드 서버와 Volume 동작 확인

검증 기록:

- 2026-07-14 Singapore production 배포와 `/readyz` healthcheck 성공
- 새 이미지 재배포와 동일 이미지 재시작 전후 SQLite 최초 초기화 시각 동일
- 5 GB Volume이 `/var/lib/tm`에서 `Ready`, 공개 도메인 최종 목록은 비어 있음
- `OPENAI_API_KEY`와 사용자 데이터 API 없이 health/readiness만 제공

상세 절차: [Railway Hobby bootstrap 배포](railway-bootstrap-deploy.md)

완료 조건:

- 클라우드 컨테이너가 재시작 후에도 정상 기동한다.
- SQLite 데이터가 재배포 후에도 Volume에 보존된다.
- 인증 전에는 인터넷에서 AI 호출 또는 TM 데이터에 접근할 수 없다.

## STEP 5 — 단일 사용자 인증과 원격 접근 보호

상태: 완료

현재 구현 기본안:

- 본인 1명, 초기 활성 bearer token 1개
- 256-bit 무작위 원문은 사용자 비밀번호 관리자에만 저장
- Railway에는 SHA-256 해시와 만료 시각만 저장
- 권장 만료 90일, 교체 시 이전 토큰 즉시 폐기
- 다중 기기별 등록·폐기는 STEP 11에서 확장

사용자 결정:

- [x] 초기 버전을 본인 한 명만 사용하는 단일 사용자 시스템으로 확정
- [x] 인증 방식 선택: 256-bit 장기 bearer token
- [x] 새 기기 등록과 분실 기기 해제 방식 결정: 초기에는 활성 토큰 1개, 회전 시 이전 토큰 즉시 폐기
- [x] 세션 만료 시간 결정: 90일

Codex Action Item:

- [x] 모든 비공개 API에 인증 미들웨어 적용
- [x] 서버에 원문 인증 토큰을 저장하지 않고 해시만 저장
- [x] 로그인 시도·API 요청 rate limit 적용
- [x] CORS·CSRF·보안 헤더 설정
- [x] 인증 실패 로그에서 민감정보 제거
- [x] 키·토큰 회전과 폐기 절차 작성
- [x] 인증 완료 후에만 HTTPS 공개 도메인 생성

검증 기록:

- 2026-07-16 production 배포 `b49163c3-d882-41c0-9f98-e9e77d0ecae0` 성공
- Railway Volume과 충돌하는 replica 설정을 `multiRegionConfig: null`로 제거하고 singleton 인스턴스 1개로 기동
- `cloud-authenticated`, `/var/lib/tm` Volume `Ready`, SQLite 최초 초기화 시각 유지 확인
- Railway HTTPS 도메인 `https://tm-server-production-5573.up.railway.app` 생성
- 공개 `/healthz`·`/readyz`는 `200`, 토큰 없음·잘못된 토큰·비인증 미등록 경로는 `401` 확인
- 모든 외부 응답의 `no-store`, CSP, HSTS, `nosniff`, frame·referrer 차단 header 확인
- 비밀번호 관리자에 보관한 원문 토큰을 보안 입력해 `/api/v1/auth/status` 원격 인증 `200` 확인

상세 절차: [STEP 5 단일 사용자 인증](single-user-auth-setup.md)

완료 조건:

- 등록된 사용자와 기기만 TM 서버에 접근할 수 있다.
- URL을 아는 것만으로는 어떤 개인 데이터나 AI 호출에도 접근할 수 없다.

## STEP 6 — 데이터 기준 원본과 migration 안전 설계

상태: 완료

Codex 권장 기본안:

- 단일 사용자·singleton 구조에서는 Railway Volume의 SQLite를 유지하고 Postgres 전환은 보류한다.
- STEP 10의 통제된 cutover 전까지 현재 로컬 DB를 기준 원본으로 유지한다.
- 초기에는 양방향 동기화와 dual-write를 구현하지 않는다.
- 실데이터 이동 없이 합성 fixture와 복제본으로 migration을 먼저 검증한다.
- OpenAI와 TM 데이터 route는 이 STEP에서 production에 활성화하지 않는다.

사용자 결정 게이트:

- [x] SQLite Volume singleton 유지 승인
- [x] `로컬 기준 → 일회성 cloud cutover → cloud 기준` 정책 승인
- [x] STEP 6에서는 실데이터를 복사하지 않고 dry-run만 수행하는 범위 승인

Codex TODO:

- [x] 현재 schema, aggregate, 관계, 불변 조건 inventory 작성
- [x] 개인정보·민감정보·운영 metadata 분류표 작성
- [x] schema version, row count, 논리 checksum을 포함한 migration manifest 설계
- [x] 일관된 SQLite snapshot과 무결성 검사 절차 구현
- [x] import 전 preflight와 import 후 비교 검증 구현
- [x] 합성 fixture로 export/import/rollback 반복 테스트
- [x] cutover 실패 시 로컬 DB로 복귀하는 runbook 작성

구현 문서: [데이터 기준 원본과 migration 안전 설계](../architecture/data-authority-and-migration.md)

검증 기록:

- 실제 사용자 DB와 Railway 운영 DB에 접근하거나 데이터를 복사하지 않았다.
- 합성 DB migration 집중 테스트 4개가 모두 통과했다.
- frontend lint·typecheck·17개 테스트와 Rust fmt·clippy가 통과했다.
- 기존 build cache의 Windows 실행 파일 잠금을 분리한 새 target에서 Rust workspace 67개 테스트가 모두 통과했다.

완료 게이트:

- 실데이터를 이동하지 않고도 migration과 rollback이 반복 검증된다.
- 기준 원본이 동시에 두 곳에 존재하지 않는 전환 정책이 문서화된다.

## STEP 7 — 인증된 read-only TM 데이터 API

상태: 완료

Codex 권장 기본안:

- 첫 읽기 범위는 project, task, checklist, tag, note, session, worklog로 제한한다.
- DB 파일, SQL, 파일 경로, backup 원문, 내부 인증·감사 정보는 노출하지 않는다.
- production은 빈 cloud DB로 검증하고 실제 사용자 데이터는 아직 복사하지 않는다.

사용자 결정 게이트:

- [x] 첫 read-only entity 범위를 project, task, checklist, tag, note, session, worklog로 제한
- [x] secret·인증정보·DB/backup/log/파일 경로·첨부 byte를 OpenAI 절대 제외 대상으로 확정

Codex TODO:

- [x] 내부 model과 분리된 versioned DTO·JSON schema 정의
- [x] `tm-core` query만 호출하는 typed GET API 구현
- [x] field allowlist와 민감 필드 redaction 적용
- [x] pagination, filter allowlist, 정렬, 응답 크기 상한 적용
- [x] version/ETag와 일관된 오류 contract 구현
- [x] 인증·rate limit·request ID·metadata-only 감사 조회 기록 연결
- [x] 합성 데이터 기반 contract·권한·회귀 테스트
- [x] production 배포 후 비인증 `401`, 허용 조회 `200`, mutation 부재 확인

계약 문서: [인증된 read-only TM API v1](../architecture/read-only-api-v1.md)

검증 기록:

- 실제 사용자 DB와 Railway 운영 DB를 사용하지 않고 합성 임시 DB만 사용했다.
- `tm-server` 24개 테스트에서 인증, 7개 collection, DTO/JSON Schema 일치, query allowlist, ETag, 405, 413을 확인했다.
- frontend lint·typecheck·17개 테스트와 Rust fmt·workspace clippy·72개 테스트가 모두 통과했다.
- 2026-07-16 Railway production 배포 `d229020e-8d50-47d8-bcc8-782ac5909a49`가 `SUCCESS`로 완료됐다.
- `/var/lib/tm` Volume은 `Ready`이며 이전 배포와 SQLite 최초 초기화 시각이 같아 재배포 중 데이터가 유지됐다.
- 공개 health/readiness는 `200`, 토큰 없음·잘못된 토큰은 `401`을 반환했다.
- 사용자 보안 입력 토큰으로 인증 상태와 빈 cloud DB의 tasks 조회가 `200`을 반환했다.
- 동일 ETag 재조회는 `304`, 인증된 `POST /api/v1/tasks`는 `405 METHOD_NOT_ALLOWED`를 반환했다.
- read-only 응답의 `Cache-Control: no-store`와 HSTS를 확인했고 토큰 원문은 파일·로그에 저장하지 않았다.
- 실제 사용자 데이터 이동과 OpenAI 호출은 수행하지 않았다.

완료 게이트:

- 허용된 필드만 인증된 API에서 읽을 수 있고 원격 mutation은 존재하지 않는다.

## STEP 8 — 통제된 write API와 감사 기록

상태: 완료

Codex 권장 기본안:

- 첫 쓰기 범위는 task·note 생성/수정과 checklist 상태 변경으로 제한한다.
- 영구 삭제, 금전, 외부 전송, 계정·권한 변경은 구현하지 않는다.
- 모든 mutation은 idempotency key와 예상 version을 요구한다.

사용자 결정 게이트:

- [x] 첫 mutation entity·동작 범위 승인
- [x] 삭제·금전·외부 전송·권한 변경 기본 금지 승인

Codex TODO:

- [x] `tm-core` command만 호출하는 typed mutation API 구현
- [x] 입력 길이·개수·형식·상태 전이 validation 적용
- [x] idempotency key 저장과 중복 결과 재사용 구현
- [x] optimistic concurrency와 conflict 응답 구현
- [x] actor, request ID, before/after, 결과를 append-only 감사 이벤트로 기록
- [x] transaction rollback·동시 요청·중복 제출 통합 테스트
- [x] mutation별 향후 승인 정책을 연결할 hook 정의
- [x] production 배포 후 인증·precondition·금지 동작을 비파괴 방식으로 검증

계약 문서: [통제된 TM write API v1](../architecture/controlled-write-api-v1.md)

로컬 검증 기록:

- `STEP 8 수행` 요청을 권장 기본안의 entity 범위와 금지 동작 승인으로 적용했다.
- schema 4에 Task·Note·Checklist 정수 version과 append-only mutation 감사·idempotency ledger를 추가했다.
- `POST` Task·Note, `PATCH` Task·Note·Checklist 완료 상태만 등록하고 삭제·금전·외부 전송·계정·권한 route는 만들지 않았다.
- 모든 mutation은 bearer 인증 외에 `Idempotency-Key`, version precondition, operation 확인을 요구한다.
- 합성 DB에서 동일 key 동시 요청은 실제 변경 1회, stale version은 `409`, 실패는 전체 rollback되는 것을 확인했다.
- 감사·idempotency 행의 update/delete와 오래된 backup을 통한 ledger rewind를 차단했다.
- frontend lint·typecheck·17개 테스트와 Rust fmt·clippy·81개 테스트가 통과했다. 실행 중인 TM이 desktop test binary를 잠가 desktop library 4개와 나머지 workspace를 분리 검증했다.
- 실제 사용자 DB에는 migration이나 mutation을 실행하지 않았다.

Production 검증 기록:

- Railway production 배포 `9e3701df-acb7-4d89-b39a-ab665323a496`가 `SUCCESS`로 완료됐다.
- `/readyz`의 schema 4·WAL·외래키·무결성 내부 검사가 통과하고 `200`을 반환했다.
- 사용자 보안 입력 토큰으로 인증 `200`을 확인했으며 토큰 원문은 파일·로그에 저장하지 않았다.
- Task 생성의 누락 precondition과 Note 생성의 잘못된 operation 확인은 domain mutation 전에 각각 `428`로 거부됐다.
- 존재하지 않는 Task·Note·Checklist의 완전한 수정 요청은 각각 `404`, Task 삭제는 `405 METHOD_NOT_ALLOWED`로 거부됐다.
- 검사 전후 Task·Note는 모두 0건이고 collection ETag도 동일했다. 실제 성공 mutation은 수행하지 않았다.
- 허용 mutation 성공, idempotency replay, stale version 충돌은 합성 로컬 DB에서 검증했다. 실제 HTTPS 성공 mutation smoke test는 STEP 9 staging에서 수행한다.

완료 게이트:

- 허용된 mutation만 실행되며 중복·충돌·실패가 데이터 불일치를 만들지 않는다.
- 모든 원격 변경의 주체와 전후 상태를 추적할 수 있다.

## STEP 9 — 백업·복구·모니터링·비용 안전장치

상태: 완료

Codex 권장 기본안:

- 실데이터 cutover 전에 Volume 외부 백업과 실제 복구 훈련을 완료한다.
- staging과 production을 분리한 뒤 production에 실데이터를 넣는다.
- OpenAI 연결 전부터 요청량·오류율·지연·저장소 사용량을 관측한다.

사용자 결정 게이트:

- [x] 외부 backup 저장소와 필요한 결제 승인
- [x] backup 보존 기간과 허용 가능한 데이터 손실 시간 승인
- [x] Railway·OpenAI 월 비용 경고선과 hard stop 기준 승인

Codex TODO:

- [x] 일관된 SQLite backup과 외부 암호화 저장 구현
- [x] 일·주·월 retention과 자동 무결성 검사 구현
- [x] 빈 환경에서 backup restore 훈련과 checksum 검증
- [x] staging environment와 배포 승격 절차를 구축하고 성공 mutation·idempotency·충돌 smoke test 수행
- [x] API latency·오류율·rate limit·Volume 사용량 관측 기반 구현
- [x] OpenAI 요청별 token·비용 기록 schema와 예산 차단 장치 준비
- [x] 로그 redaction과 request ID 기반 추적 검증
- [x] Railway deployment failure·crash·OOM·usage email/in-app 알림 경로 확인
- [x] Railway CPU·RAM·disk·network monitor는 Hobby에서 사용할 수 없어 Pro 승격 전까지 보류
- [x] OpenAI platform USD 10 monthly budget email alert는 결제수단 등록과 함께 STEP 11에서 설정하도록 이관
- [x] production schema 5 read-only 검증 후 첫 암호화 backup 생성 및 상태 확인
- [x] production 암호화 backup restore drill 수행

완료 게이트:

- Volume이나 배포 환경이 사라져도 검증된 backup으로 복구할 수 있다.
- 비용과 장애를 감지하고 설정한 기준에서 알림 또는 중단할 수 있다.

확정 정책과 복구 절차: [STEP 9 백업·복구·모니터링·비용 안전장치](step9-backup-monitoring-cost.md)

## STEP 10 — 데스크톱 cloud mode와 일회성 cutover

상태: 완료

Codex 권장 기본안:

- 데스크톱에 명시적인 local/cloud mode를 두고 자동 background sync는 만들지 않는다.
- 전환 중 로컬 쓰기를 잠시 멈추고 snapshot을 cloud로 한 번만 import한다.
- 검증 후 cloud DB를 기준 원본으로 전환하고 기존 로컬 DB는 read-only archive로 보존한다.
- 실패하면 maintenance window 안에서 local mode로 복귀한다.

사용자 결정 게이트:

- [x] 실제 TM 데이터를 Railway로 복사하는 것 승인
- [x] cloud DB를 새 기준 원본으로 전환하는 것 승인
- [x] 최대 30분 cutover 중단 시간 승인
- [x] 제외 데이터 없이 전체 이전 승인

Codex TODO:

- [x] 로컬 DB 원문 비노출 inventory와 read-only integrity·foreign key 사전검사
- [x] `tm-cli migration` manifest·dry-run·inspect 명령의 Windows format·test·clippy 통과
- [x] 인증된 HTTPS desktop client와 오류 contract 구현
- [x] 인증 토큰을 OS 보안 저장소에 보관하고 로그·UI 노출 차단
- [x] local/cloud mode 전환과 cloud mode local DB 격리
- [x] 최종 local backup·무결성 검사·write freeze 수행
- [x] migration manifest로 cloud import와 row/checksum 비교
- [x] 기존 TM 기능을 cloud mode에서 회귀 검증
- [x] rollback 훈련 후 cloud 기준 전환과 local archive 생성

완료 게이트:

- 현재 데스크톱 기능이 cloud 기준 DB에서 정상 동작한다.
- local/cloud 양쪽에 서로 다른 최신 데이터가 생기지 않는다.

완료 기록과 cutover 절차: [STEP 10 데스크톱 cloud mode와 일회성 cutover](step10-cloud-cutover.md)

## STEP 11 — Cloud OpenAI와 read-only 오케스트레이터

상태: 진행 중 — 구현·모의 통합 검증 완료, Railway key와 최소 과금 운영 검증 대기

Codex 권장 기본안:

- OpenAI API key는 Railway sealed variable로만 입력하고 DB·응답·로그에 저장하지 않는다.
- Responses API, `store: false`, model allowlist와 요청별 token·비용 상한을 유지한다.
- 첫 오케스트레이터에는 read-only TM 도구만 제공하고 mutation 도구는 등록하지 않는다.
- 전체 DB 대신 필요한 최소 레코드와 요약만 모델에 전달한다.

사용자 결정 게이트:

- [x] OpenAI에 전달 가능한 데이터 범위 승인
- [x] 기본 model과 요청별·월별 비용 상한 승인
- [x] 답변 스타일과 기본 리포트 길이 승인
- [ ] Railway에 OpenAI API key 입력

Codex TODO:

- [x] cloud 전용 OpenAI client와 sealed secret 검증 경계 구현
- [x] 모든 요청을 답변·조회·계획 범위로 제한하는 read-only intent contract 구현
- [x] strict read-only tool schema와 필드·레코드·전체 결과 크기 제한 정의
- [x] 계획과 실행을 분리하고 mutation 도구를 등록하지 않음
- [x] prompt version, model, 합산 token usage, 비용 추정, request ID 기록
- [x] 6회 tool call·무한 루프·60초 timeout·요청/월 예산 초과 차단
- [x] prompt injection 지시와 allowlist 밖 tool output 오염 격리 테스트
- [ ] production 최소 비용 end-to-end probe 수행

완료 게이트:

- AI가 TM 데이터를 최소 범위로 읽고 설명할 수 있지만 어떤 데이터도 변경할 수 없다.

구현 계약과 운영 검증 절차: [STEP 11 read-only AI 오케스트레이터](../architecture/read-only-ai-orchestrator.md)

## STEP 12 — 실행 승인·취소·도구 정책

상태: 대기

권장 권한 등급:

1. 자동 허용: 검색, 조회, 요약, 분석
2. 사전 승인: task·note 생성, 상태 변경
3. 강화 승인: 외부 메시지, 민감정보 공유, 금전 관련 준비
4. 기본 금지: 영구 삭제, 결제 실행, 계정·권한 변경

사용자 결정 게이트:

- [ ] 권한 등급과 각 동작의 배치 승인
- [ ] 첫 실행 도구 1개와 승인 유효 시간 승인
- [ ] 자동 실행을 허용할 read-only 범위 승인

Codex TODO:

- [ ] preview, 영향 범위, 비용, 되돌리기 가능 여부 표시
- [ ] approval 요청·승인·거절·취소·만료 상태 machine 구현
- [ ] 승인 revision과 실행 payload를 해시로 고정
- [ ] 동일 승인·요청의 중복 실행 차단
- [ ] 실행 전 권한과 최신 version 재검증
- [ ] 결과·실패·부분 실행·rollback metadata 감사 기록
- [ ] 첫 low-risk mutation tool을 end-to-end로 검증

완료 게이트:

- AI가 중요한 작업을 사용자 모르게 실행하거나 승인 후 내용을 바꿀 수 없다.

## STEP 13 — 장기 기억·검색·컨텍스트 예산

상태: 대기

Codex 권장 기본안:

- 원본 데이터와 AI용 기억·요약을 별도 schema로 분리한다.
- 초기 retrieval은 SQLite FTS와 구조화 filter를 사용하고 별도 vector 서비스는 보류한다.
- 자동 장기 기억보다 명시적 저장을 먼저 제공한다.
- 요청마다 전체 이력을 보내지 않고 token budget 안에서 관련 항목만 선택한다.

사용자 결정 게이트:

- [ ] 장기 기억으로 저장할 정보 범위 승인
- [ ] 자동 저장과 명시적 저장 정책 승인
- [ ] 보존 기간·삭제 정책과 OpenAI 전송 금지 정보 승인

Codex TODO:

- [ ] memory source, provenance, sensitivity, retention schema 구현
- [ ] 구조화 filter와 FTS 기반 retrieval 구현
- [ ] 일·주·월 요약 계층과 재생성 규칙 구현
- [ ] 요청 종류별 context·token 상한과 truncation 정책 적용
- [ ] 기억 조회·수정·삭제와 감사 이벤트 구현
- [ ] 삭제된 원본의 파생 기억 정리와 retention job 구현
- [ ] 장기 데이터 규모와 비용 회귀 테스트

완료 게이트:

- 데이터가 늘어도 관련 정보만 제한된 비용으로 사용하고 기억의 출처·수정·삭제를 추적할 수 있다.

## STEP 14 — 스케줄러·작업 큐·정기 실행 기반

상태: 대기

Codex 권장 기본안:

- Volume singleton 제약에 맞춰 같은 서버 안의 DB-backed scheduler부터 시작한다.
- job, attempt, lease, idempotency, dead-letter 상태를 SQLite에 기록한다.
- 시각은 UTC로 저장하고 사용자 일정은 `Asia/Seoul`로 계산한다.
- 실제 비서 알림 대신 비용이 없는 내부 검증 job으로 먼저 시험한다.

사용자 결정 게이트:

- [ ] 기본 실행 빈도와 방해 금지 시간 승인
- [ ] 실패 재시도·dead-letter·사용자 알림 기준 승인
- [ ] 서버 재시작 후 놓친 작업 처리 방식 승인

Codex TODO:

- [ ] durable job·attempt·lease schema 구현
- [ ] 중복 claim 방지와 lease 만료 복구 구현
- [ ] exponential backoff와 dead-letter 상태 구현
- [ ] UTC/KST·DST 경계와 missed-run 정책 테스트
- [ ] 배포·재시작·장애 상황의 exactly-once effect 검증
- [ ] queue depth·실패율·지연 모니터링 연결
- [ ] 무과금 내부 정기 작업을 production에서 검증

완료 게이트:

- 서버 재시작과 중복 실행 상황에서도 정기 작업의 효과가 한 번만 안전하게 반영된다.

## STEP 15 — 모바일·원격 클라이언트와 기기 인증

상태: 대기

Codex 권장 기본안:

- 첫 원격 클라이언트는 설치 부담이 낮은 PWA로 시작한다.
- 하나의 공용 장기 토큰 대신 기기별 토큰 해시·만료·폐기 기록으로 확장한다.
- 초기 PWA는 온라인 전용으로 두고 offline mutation은 보류한다.
- CORS는 배포된 PWA origin 하나만 정확히 허용한다.

사용자 결정 게이트:

- [ ] PWA, native mobile 또는 다른 hardware client 선택
- [ ] 기기 등록 승인 방식과 분실 기기 폐기 방식 승인
- [ ] 알림 채널과 offline 사용 범위 승인

Codex TODO:

- [ ] 기기 등록·목록·폐기·만료 API와 감사 기록 구현
- [ ] OS/browser 보안 저장소에 token을 저장하고 UI·로그 노출 차단
- [ ] exact-origin CORS와 CSRF·replay 경계 검증
- [ ] 네트워크 끊김·재시도·중복 제출 처리
- [ ] 작은 화면용 최소 비서 UI 구현
- [ ] 원격 기기 폐기와 token rotation end-to-end 훈련
- [ ] 로컬 PC가 꺼진 상태에서 cloud 접근 검증

완료 게이트:

- 등록된 기기만 cloud TM에 접근하고 분실 기기를 독립적으로 즉시 폐기할 수 있다.

## STEP 16 — 보안 강화·사고 대응·운영 준비

상태: 대기

사용자 결정 게이트:

- [ ] 장애·침해·비용 초과 알림 수신 채널 승인
- [ ] 허용 가능한 복구 시간과 서비스 중단 기준 승인

Codex TODO:

- [ ] 최신 위협 model과 데이터 흐름 검토
- [ ] 의존성 취약점·컨테이너 이미지·secret scan 자동화
- [ ] TLS·header·rate limit·입력 제한·권한 우회 재검증
- [ ] API key·인증 token·기기 token 회전 훈련
- [ ] CSRF·replay·중복 실행·prompt injection 공격 테스트
- [ ] backup restore와 production rollback 모의훈련
- [ ] 사고 시 domain 차단·secret 폐기·복구 runbook 작성
- [ ] 운영 dashboard, SLO, 비용 ceiling, 배포 checklist 확정

완료 게이트:

- 알려진 주요 위험과 장애 시나리오가 테스트되고 중지·폐기·복구 절차가 실제로 작동한다.

## STEP 17 — 첫 실제 AI 비서 기능

상태: 대기

첫 기능 후보:

- 오늘 할 일 우선순위 제안
- 하루 일정 브리핑
- 운동 기록 요약
- 소비 내역 주간 분석

사용자 결정 게이트:

- [ ] 첫 기능 1개와 성공 기준 승인
- [ ] 허용 비용·실행 빈도·자동화 수준 승인
- [ ] 사용할 실제 데이터 범위와 알림 채널 승인

Codex TODO:

- [ ] 기존 API·오케스트레이터·승인·스케줄러만 이용해 최소 기능 구현
- [ ] 기능 전용 권한·prompt·tool·비용 상한 정의
- [ ] staging 검증 후 production 점진 활성화
- [ ] 실제 품질·비용·실패·사용 편의성 측정
- [ ] rollback과 기능 kill switch 검증
- [ ] 측정 결과에 따라 다음 기능 추가 여부 보고

완료 게이트:

- 시스템 기반을 유지한 채 실제 비서 기능 하나가 안전하게 운영되고 품질·비용·편의성을 측정할 수 있다.

## 단계 진행 규칙

각 STEP은 다음 순서로 진행한다.

1. Codex가 해당 단계의 목적과 중요한 결정사항을 설명한다.
2. 사용자는 결제·새 로그인·OTP·비밀값 입력·외부 권한·제품 방향 결정이 필요할 때만 개입한다.
3. Codex가 코드, 클라우드 설정, 배포, 자동 테스트와 운영 검증을 수행한다.
4. 사용자 기기의 비밀값이 필요한 동작만 사용자가 직접 확인한다.
5. Codex가 문서를 갱신하고 로컬 Git 커밋으로 체크포인트를 만든다.
6. 완료 조건을 충족한 뒤에만 다음 STEP을 시작한다.

인증·백업·비용 제한보다 실제 자동 실행 기능을 먼저 공개하지 않는다.

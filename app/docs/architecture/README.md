# TM 아키텍처

`tm-core`가 SQLite와 파일 시스템의 유일한 소유자다. Tauri 앱과 `tm-cli`는 같은 공용 서비스를 호출하며 React는 typed command client를 통해서만 데이터를 읽고 쓴다.

`tm-server`의 기본 `local` 프로필은 localhost 전용 HTTP 경계다. 로컬에서는 liveness/readiness, OpenAI 연결 상태·명시적 probe, 공통 오류·request ID만 제공하며 기존 TM command와 사용자 DB는 아직 원격으로 노출하지 않는다. OpenAI 키는 서버 환경변수에서만 읽고 응답 저장은 끈다.

Railway STEP 4 배포에는 별도의 `cloud-bootstrap` 프로필을 사용한다. 이 프로필은 Railway ID·`PORT`·Volume 경로를 검증한 뒤 `/healthz`와 `/readyz`만 외부 바인딩하고 AI 및 TM 데이터 route를 등록하지 않는다. 공개 도메인은 사용자 인증을 구현하는 STEP 5 전까지 만들지 않는다.

STEP 5의 `cloud-authenticated` 프로필은 공개 health/readiness를 최소 응답으로 축소하고 그 외 모든 경로에 단일 사용자 bearer 인증을 먼저 적용한다. 토큰 원문은 사용자 기기에만 있으며 서버는 SHA-256 해시와 만료 시각만 읽는다. 실패 인증과 정상 API 요청에는 서로 독립적인 in-memory rate limit을 적용하고, CORS 허용 없이 보안·no-store header를 모든 응답에 추가한다. Railway Volume과 함께 singleton 인스턴스 1개로 실행하며 Config as Code에서 replica 기능을 명시적으로 끈다. 이 단계에서도 OpenAI와 TM 사용자 데이터 route는 등록하지 않는다.

STEP 6은 로컬 DB를 STEP 10 전까지 유일한 기준 원본으로 고정한다. `tm-core`의 migration manifest는 schema·migration ledger·무결성·외래키를 검사하고 논리 테이블별 행 개수와 원문을 포함하지 않는 SHA-256을 계산한다. 합성 임시 DB의 online snapshot, read-only preflight, 복구 비교만 수행하며 실제 사용자 DB와 Railway 운영 DB는 이전하지 않는다.

STEP 7의 `/api/v1` 조회 route는 `cloud-authenticated` profile에만 존재하며 `tm-core` query와 외부 전용 DTO allowlist를 사용한다. 삭제·내부·파일·감사 데이터와 허용되지 않은 query를 차단하고 pagination·응답 크기 상한·콘텐츠 ETag를 적용한다. 이 경계는 OpenAI 호출을 하지 않는다.

STEP 8은 Task·Note 생성/수정과 Checklist 완료 상태 변경만 typed command로 허용한다. 모든 mutation은 bearer 인증 외에도 idempotency key, 정수 version precondition, operation 확인을 요구하고 domain 변경·중복 결과·before/after 감사를 한 transaction에 기록한다. 삭제·금전·외부 전송·계정·권한 변경 route는 없다.

STEP 9는 schema 5 append-only AI 비용 원장과 월 hard stop, Railway Bucket으로 client-side 암호화하는 SQLite backup, 일·주·월 보존, 복구 drill, 인증된 운영 상태와 민감정보를 제외한 request telemetry를 추가한다. staging 검증을 통과한 동일 image만 production으로 승격한다.

STEP 10은 데스크톱의 명시적 local/cloud transport, Windows Credential Locker token 경계, 현재 기능 전체의 first-party command allowlist와 maintenance-only database import를 추가한다. 자동 sync나 양방향 write는 허용하지 않으며 source/cloud logical manifest가 완전히 일치한 뒤에만 cloud를 단일 기준 원본으로 전환한다.

STEP 11은 인증된 cloud 서버에 OpenAI Responses API와 read-only 오케스트레이터를 추가한다. 모델은 strict allowlist를 통해 Project·Task·Checklist·Note·Session·Worklog의 최소 필드만 최대 6회 조회할 수 있고 mutation·첨부·backup·인증·감사·비용 원장 도구는 사용할 수 없다. `store: false`, 60초 timeout, 요청별 USD 0.25 예약과 월 USD 20 hard stop을 적용한다.

STEP 12는 기존 7개 자동 read-only 도구에 승인 요청 전용 `propose_task_create`를 추가한다. 제안은 Task를 만들지 않으며 10분 안에 인증된 별도 API에서 locked payload의 revision·SHA-256을 확인하고 1회 승인해야 한다. 승인 후 기존 controlled mutation을 즉시 실행하고 idempotency와 append-only approval audit로 중복·변조·되감기를 막는다.

STEP 13은 원본 TM 데이터와 AI 기억을 schema 7에서 분리한다. 사용자 기억은 명시적 요청과 STEP 12 승인 뒤에만 생성·수정·삭제되며 private·restricted 정보와 금지된 비밀값은 OpenAI로 전달되지 않는다. retrieval은 외부 vector 서비스 없이 SQLite FTS5와 구조화 filter를 사용하고 요청 종류별 최대 6/10/12개, 절대 6,144 bytes 안에서 관련 기억만 전달한다. 일·주·월 요약은 로컬 결정론적 계산이며 출처·보존·삭제를 append-only 이벤트로 추적한다.

STEP 14는 schema 8에 durable job·run·attempt·effect 원장을 추가한다. Railway singleton 서버 안의 worker가 30초마다 due work를 확인하고 5분 lease, 5회 지수형 재시도, dead-letter, 24시간 misfire grace와 coalescing을 적용한다. canary와 기억 정리는 OpenAI를 호출하지 않으며 effect와 실제 변경을 한 SQLite transaction에 기록해 재시작·중복 claim·백업 복원 뒤에도 효과가 한 번만 반영되도록 한다.

STEP 15는 schema 9에 10분짜리 기기 페어링, 90일 기기 세션, 개별·전체 폐기와 append-only 인증 감사를 추가한다. primary bearer token만 기기 승인·폐기를 수행하고 모바일 PWA는 `HttpOnly + Secure + SameSite=Strict` 기기 쿠키, same-origin 검증과 CSRF token으로 제한된 Task·Note·AI 비서·승인 route만 사용한다. PWA는 같은 Railway 서비스에 내장되어 별도 서버가 없으며 service worker는 app shell만 캐시하고 TM 데이터·AI 응답·mutation을 저장하거나 재전송하지 않는다. 백업 복원은 현재 기기 인증 원장을 병합해 폐기된 기기가 되살아나지 않게 한다.

STEP 16은 schema 변경 없이 `normal`·`read-only`·`lockdown` 사고 모드와 독립 AI kill switch를 추가한다. 운영 status는 RPO/RTO, backup freshness/integrity, scheduler, AI 비용 원장과 process lifetime 보안 거절 수를 함께 평가한다. request target/header 제한과 browser security header를 강화하고 private CI에서 SHA-pinned Action, RustSec, Trivy filesystem/image, secret policy scan을 수행한다.

STEP 17은 schema 10의 첫 전용 비서 기능으로 `오늘의 Task AI 리포트`를 추가한다. 최대 20개의 최소 Task fact만 Structured Outputs에 전달하고 반환 ID를 서버가 후보 allowlist와 다시 대조한다. 사용자 수동 호출, 하루 4회, 호출당 USD 0.05, 800 output token, 기능별 kill switch를 적용하며 Windows·모바일에서 결과·비용·latency와 append-only 품질 평가를 공유한다.

개인 캘린더는 schema 11의 `calendar_events`를 Windows와 모바일이 함께 사용한다. 일회성, 매월 특정일, 매월 초일, 매월 말일 규칙은 원본 일정 한 건으로 저장하고 조회하는 달에 occurrence를 계산한다. 변경·삭제는 version precondition과 soft delete를 사용하며, 이 기능 자체는 OpenAI를 호출하거나 결제를 실행하지 않는다.

조회 전용 주식 차트는 schema 12의 `stock_watchlist_items`에 시장·티커·표시 이름만 저장한다. Windows와 모바일이 최대 50개의 관심 종목을 같은 cloud 원본에서 읽고 쓰며, 시세와 차트는 TM origin과 분리된 opaque `data:` sandbox iframe 안의 TradingView 위젯이 직접 가져온다. TM은 가격 데이터를 저장하거나 OpenAI에 관심 종목을 전달하지 않고, 계좌 연결·주문·추천도 제공하지 않는다.

상단 비용 상태는 별도 OpenAI Admin key 없이 TM의 월간 AI 비용 원장을 사용하고, Railway workspace billing usage는 10분 캐시된 공식 GraphQL 조회로 보완한다. Windows와 모바일에는 비용/한도 두 값만 표시하며 외부 provider 장애가 핵심 기능으로 전파되지 않는다.

- [수동 승인 개선 요청 흐름](change-request-workflow.md)
- [데이터 기준 원본과 migration 안전 설계](data-authority-and-migration.md)
- [데스크톱 cloud transport와 cutover 경계](desktop-cloud-transport.md)
- [인증된 read-only TM API v1](read-only-api-v1.md)
- [통제된 TM write API v1](controlled-write-api-v1.md)
- [STEP 11 read-only AI 오케스트레이터](read-only-ai-orchestrator.md)
- [STEP 12 AI 실행 승인 경계](assistant-action-approval.md)
- [STEP 13 장기 기억·검색·컨텍스트 예산](assistant-memory-context.md)
- [TM AI 비서 시스템 구축 로드맵](../operations/tm-ai-assistant-roadmap.md)
- [OpenAI 로컬 연결 설정](../operations/openai-local-setup.md)
- [Railway Hobby bootstrap 배포](../operations/railway-bootstrap-deploy.md)
- [STEP 9 백업·복구·모니터링·비용 안전장치](../operations/step9-backup-monitoring-cost.md)
- [STEP 12 실행 승인 운영 절차](../operations/step12-action-approval.md)
- [STEP 13 기억·검색 운영 절차](../operations/step13-memory-context.md)
- [STEP 14 durable scheduler 운영 절차](../operations/step14-durable-scheduler.md)
- [STEP 15 PWA·기기 인증 운영 절차](../operations/step15-mobile-device-auth.md)
- [STEP 16 위협 모델](../security/step16-threat-model.md)
- [STEP 16 사고 대응·복구 runbook](../operations/step16-incident-response.md)
- [STEP 16 보안 배포 checklist](../operations/step16-deployment-checklist.md)
- [STEP 17 오늘의 Task AI 리포트](today-task-report.md)
- [STEP 17 Task 리포트 운영 절차](../operations/step17-task-report.md)
- [상단 비용 상태 운영 절차](../operations/cost-status.md)
- [개인 캘린더 운영·사용 절차](../operations/personal-calendar.md)
- [조회 전용 주식 차트 운영·사용 절차](../operations/read-only-stock-charts.md)
- [STEP 5 단일 사용자 인증](../operations/single-user-auth-setup.md)

기본 홈은 `C:\Users\tkfk0\Desktop\codex\TM`이다. 테스트는 프로세스별 임시 `TM_HOME`을 사용한다. 모든 저장 시각은 UTC RFC 3339로 기록하고, 사용자 날짜는 `Asia/Seoul`로 계산한다.

release 앱과 CLI는 기본 홈으로 고정된다. debug/test의 `TM_HOME`은 기본 홈 하위 절대경로만 허용하고 fixture는 `TM/dist/test-runs`를 사용한다.

Task+태그+체크리스트, Note+다중 링크, 세션 종료+WorkLog+후속 Task는 각각 하나의 `BEGIN IMMEDIATE` 트랜잭션으로 처리한다. UI command 계층에는 SQL이 없으며 aggregate 저장은 `tm-core` API 한 번만 호출한다.

빌드 출력과 다운로드 캐시는 `TM/dist` 아래에 두며 실제 데이터는 `TM/data`, 백업은 `TM/backups`, 내보내기는 `TM/exports`에 둔다.

도구 버전과 project-local 의존성 위치는 [`toolchain.md`](toolchain.md), 설치 수명주기 결과는 [`installer-validation.md`](installer-validation.md)에 기록한다.

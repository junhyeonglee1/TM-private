# TM 아키텍처

`tm-core`가 SQLite와 파일 시스템의 유일한 소유자다. Tauri 앱과 `tm-cli`는 같은 공용 서비스를 호출하며 React는 typed command client를 통해서만 데이터를 읽고 쓴다.

`tm-server`의 기본 `local` 프로필은 localhost 전용 HTTP 경계다. 로컬에서는 liveness/readiness, OpenAI 연결 상태·명시적 probe, 공통 오류·request ID만 제공하며 기존 TM command와 사용자 DB는 아직 원격으로 노출하지 않는다. OpenAI 키는 서버 환경변수에서만 읽고 응답 저장은 끈다.

Railway STEP 4 배포에는 별도의 `cloud-bootstrap` 프로필을 사용한다. 이 프로필은 Railway ID·`PORT`·Volume 경로를 검증한 뒤 `/healthz`와 `/readyz`만 외부 바인딩하고 AI 및 TM 데이터 route를 등록하지 않는다. 공개 도메인은 사용자 인증을 구현하는 STEP 5 전까지 만들지 않는다.

STEP 5의 `cloud-authenticated` 프로필은 공개 health/readiness를 최소 응답으로 축소하고 그 외 모든 경로에 단일 사용자 bearer 인증을 먼저 적용한다. 토큰 원문은 사용자 기기에만 있으며 서버는 SHA-256 해시와 만료 시각만 읽는다. 실패 인증과 정상 API 요청에는 서로 독립적인 in-memory rate limit을 적용하고, CORS 허용 없이 보안·no-store header를 모든 응답에 추가한다. Railway Volume과 함께 singleton 인스턴스 1개로 실행하며 Config as Code에서 replica 기능을 명시적으로 끈다. 이 단계에서도 OpenAI와 TM 사용자 데이터 route는 등록하지 않는다.

STEP 6은 로컬 DB를 STEP 10 전까지 유일한 기준 원본으로 고정한다. `tm-core`의 migration manifest는 schema·migration ledger·무결성·외래키를 검사하고 논리 테이블별 행 개수와 원문을 포함하지 않는 SHA-256을 계산한다. 합성 임시 DB의 online snapshot, read-only preflight, 복구 비교만 수행하며 실제 사용자 DB와 Railway 운영 DB는 이전하지 않는다.

STEP 7의 `/api/v1` 데이터 route는 `cloud-authenticated` profile에만 존재하며 `tm-core` query와 외부 전용 DTO allowlist를 사용한다. GET 외 method, 삭제·내부·파일·감사 데이터, 허용되지 않은 query를 차단하고 pagination·응답 크기 상한·콘텐츠 ETag를 적용한다. 이 경계는 OpenAI 호출을 하지 않는다.

- [수동 승인 개선 요청 흐름](change-request-workflow.md)
- [데이터 기준 원본과 migration 안전 설계](data-authority-and-migration.md)
- [인증된 read-only TM API v1](read-only-api-v1.md)
- [TM AI 비서 시스템 구축 로드맵](../operations/tm-ai-assistant-roadmap.md)
- [OpenAI 로컬 연결 설정](../operations/openai-local-setup.md)
- [Railway Hobby bootstrap 배포](../operations/railway-bootstrap-deploy.md)
- [STEP 5 단일 사용자 인증](../operations/single-user-auth-setup.md)

기본 홈은 `C:\Users\tkfk0\Desktop\codex\TM`이다. 테스트는 프로세스별 임시 `TM_HOME`을 사용한다. 모든 저장 시각은 UTC RFC 3339로 기록하고, 사용자 날짜는 `Asia/Seoul`로 계산한다.

release 앱과 CLI는 기본 홈으로 고정된다. debug/test의 `TM_HOME`은 기본 홈 하위 절대경로만 허용하고 fixture는 `TM/dist/test-runs`를 사용한다.

Task+태그+체크리스트, Note+다중 링크, 세션 종료+WorkLog+후속 Task는 각각 하나의 `BEGIN IMMEDIATE` 트랜잭션으로 처리한다. UI command 계층에는 SQL이 없으며 aggregate 저장은 `tm-core` API 한 번만 호출한다.

빌드 출력과 다운로드 캐시는 `TM/dist` 아래에 두며 실제 데이터는 `TM/data`, 백업은 `TM/backups`, 내보내기는 `TM/exports`에 둔다.

도구 버전과 project-local 의존성 위치는 [`toolchain.md`](toolchain.md), 설치 수명주기 결과는 [`installer-validation.md`](installer-validation.md)에 기록한다.

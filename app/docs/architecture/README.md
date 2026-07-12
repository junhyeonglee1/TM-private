# TM 아키텍처

`tm-core`가 SQLite와 파일 시스템의 유일한 소유자다. Tauri 앱과 `tm-cli`는 같은 공용 서비스를 호출하며 React는 typed command client를 통해서만 데이터를 읽고 쓴다.

`tm-server`는 향후 클라우드 전환을 위한 localhost 전용 HTTP 경계다. 현재 단계에서는 liveness/readiness와 공통 오류·request ID만 제공하며, 기존 TM command와 사용자 DB는 아직 원격으로 노출하지 않는다.

- [수동 승인 개선 요청 흐름](change-request-workflow.md)

기본 홈은 `C:\Users\tkfk0\Desktop\codex\TM`이다. 테스트는 프로세스별 임시 `TM_HOME`을 사용한다. 모든 저장 시각은 UTC RFC 3339로 기록하고, 사용자 날짜는 `Asia/Seoul`로 계산한다.

release 앱과 CLI는 기본 홈으로 고정된다. debug/test의 `TM_HOME`은 기본 홈 하위 절대경로만 허용하고 fixture는 `TM/dist/test-runs`를 사용한다.

Task+태그+체크리스트, Note+다중 링크, 세션 종료+WorkLog+후속 Task는 각각 하나의 `BEGIN IMMEDIATE` 트랜잭션으로 처리한다. UI command 계층에는 SQL이 없으며 aggregate 저장은 `tm-core` API 한 번만 호출한다.

빌드 출력과 다운로드 캐시는 `TM/dist` 아래에 두며 실제 데이터는 `TM/data`, 백업은 `TM/backups`, 내보내기는 `TM/exports`에 둔다.

도구 버전과 project-local 의존성 위치는 [`toolchain.md`](toolchain.md), 설치 수명주기 결과는 [`installer-validation.md`](installer-validation.md)에 기록한다.

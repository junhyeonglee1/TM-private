# 데이터 안전성 설계

## SQLite 연결

모든 프로세스는 연결마다 `foreign_keys=ON`, WAL, `synchronous=NORMAL`, 15초 `busy_timeout`을 적용한다. 쓰기 서비스는 `BEGIN IMMEDIATE` 트랜잭션을 사용해 앱과 CLI가 동시에 실행되어도 부분 저장을 만들지 않는다.

Task 변경 이력은 DB trigger가 before/after JSON을 생성한다. TaskEvent의 UPDATE와 DELETE는 DB trigger로 차단한다. 확정된 TaskDayEntry도 UPDATE와 DELETE를 차단하며 이월은 이전 planned 행의 `deferred` 전환과 새 날짜 `planned` 삽입을 한 트랜잭션에서 수행한다.

## 백업

- 기존 DB migration 전 온라인 백업
- 정상 앱 시작과 종료 시 강제 온라인 백업
- 종료 백업 실패 시 종료를 중단하고 다음 종료 요청에서 재시도
- 장시간 실행 중 서울 날짜 기준 하루 한 번 자동 백업
- 수동 요청마다 UUIDv7이 포함된 새 파일
- DB 백업 30개, 소스 ZIP 10개 보존
- 소스 ZIP에서 `.env*`, DB/WAL, 로그, 첨부, 키·인증서 가능 파일과 생성 디렉터리 제외
- 복원 후보의 `integrity_check`와 schema version을 확인하고 현재 DB를 다시 백업한 뒤 복원

## 내보내기

JSON과 Markdown은 동일한 read transaction snapshot에서 생성한다. export 파일에는 프로젝트, Task, 체크리스트, 태그, 오늘 기록, 이벤트, 세션, WorkLog, Note, 링크, 첨부 메타데이터와 digest delivery 상태가 포함된다.

## 설치 경계

실행 파일과 설치 패키지는 `TM/dist`에 생성하며 설치·재설치·제거 로직은 외부 `TM/data`, `TM/backups`, `TM/exports`를 소유하거나 삭제하지 않는다.

release 앱과 CLI에는 고정 기본 루트만 컴파일되며 `TM_HOME` override는 포함되지 않는다. NSIS는 `currentUser`, downgrade 차단, WebView2 다운로드 생략, TM 내부 도구 캐시로 구성한다. 신규 설치·동일 버전 재설치·제거에서 외부 데이터 파일 `24`개의 aggregate SHA-256이 유지되고 설치 디렉터리와 HKCU 항목만 제거되는 것을 확인했다. 정확한 결과는 [`installer-validation.md`](installer-validation.md)에 기록한다.

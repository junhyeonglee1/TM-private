# 변경 기록

이 파일에는 사용자에게 보이는 변경 사항을 패치 단위로 기록한다.

## [Unreleased]

## [0.1.5]

### Fixed

- raw Windows release 빌드에 Tauri `custom-protocol`을 강제해 실행 파일이 개발용 localhost 대신 내장 프런트엔드를 열도록 수정했다.
- production feature가 빠진 release 컴파일은 즉시 실패하도록 빌드 방어 조건을 추가했다.

## [0.1.4]

### Changed

- Inbox의 즉흥 Task 입력과 분류 기능을 제거하고, 새 역할을 정하기 전까지 안내만 표시하는 보류 화면으로 바꿨다.
- Task 생성 진입점을 프로젝트 화면으로 모으고 소속·이름·설명·우선순위·선택 마감일만 표시하는 compact 입력 폼으로 정리했다.
- 오늘 화면의 중복 빠른 입력과 프로젝트의 `정리 대기` 그룹을 제거했다.
- Windows x64 앱 실행 파일을 `TM\tm.exe`에서도 바로 실행할 수 있도록 배치한다.

## [0.1.3]

### Added

- 패치 0004: 개선 요청 초안 편집기에 유형별 작성 가이드와 완료 기준 예시를 추가했다.

### Changed

- 가이드 열기와 요청 유형 변경이 사용자가 입력한 초안 내용을 덮어쓰지 않도록 했다.
- 700px 이하에서는 가이드 항목을 한 열로 배치한다.

## [0.1.2]

### Added

- 패치 0003: TM 안에서 개선 요청을 초안으로 기록하고 명시적으로 승인하는 로컬 요청함을 추가했다.
- 승인된 요청 한 건을 원자적으로 claim하고 완료·실패 결과를 기록하는 `tm-cli changes` 명령을 추가했다.
- revision·attempt count·claim key 검증과 append-only before/after 이벤트를 적용해 stale 승인과 중복 처리를 차단했다.

### Changed

- SQLite schema 3에서 schema 2 요청함의 strict CAS와 draft 복귀 revision 규칙을 강화했다.
- 복원 시 완료·취소·실패·처리 시도와 철회된 승인을 비가역 ledger로 보존해 과거 승인 상태가 되살아나지 않게 했다.
- Codex 수동 처리 문서와 앱 안내에 작업공간의 검증된 CLI·프롬프트 경로를 명시했다.

## [0.1.1]

### Added

- 패치 0002: 프로젝트 화면에 master-detail Task 작업공간, 프로젝트 고정 빠른 추가, 상태 그룹과 완료·취소 필터를 추가했다.
- Inbox Task를 프로젝트의 `todo`로 원자적으로 정리하는 분류 동작을 추가했다.

### Changed

- Inbox 빠른 입력은 제목만 수집하고 프로젝트·우선순위는 정리 단계에서 결정하도록 단순화했다.
- 프로젝트 생성 command가 생성된 ID를 반환하며, UI는 새 프로젝트를 즉시 선택한다.
- Tauri `create_task`는 선택적 상태를 검증하고 생략 시 `inbox`를 유지한다.

## [0.1.0]

### Added

- 패치 0001: 프로젝트·Inbox·오늘·히스토리·세션·WorkLog·Note·검색·휴지통 UI를 구현했다.
- Rust 공용 코어에 SQLite migration, FTS5, 불변 TaskEvent, 명시적 이월, 백업·복원, JSON·Markdown 내보내기를 구현했다.
- 동일 코어를 사용하는 `tm-cli`와 원자적 digest delivery claim, DB/소스 백업 명령을 추가했다.
- Windows x64 실행 파일 `tm.exe`, `tm-cli.exe`와 NSIS 설치 패키지를 생성했다.

### Changed

- Task·태그·체크리스트 저장과 Note·다중 링크 생성을 각각 단일 SQLite 트랜잭션으로 묶었다.
- release 앱과 CLI는 고정 TM 루트만 사용하고 테스트 경로·캐시·빌드 결과는 `TM\dist` 아래로 제한했다.
- 종료 백업 실패 시 종료를 중단하고, 소스 ZIP에서 사용자 데이터·로그·비밀 가능 파일을 제외한다.

### Verified

- NSIS 신규 설치·동일 버전 재설치·제거에서 외부 DB·백업·첨부·로그·내보내기 sentinel이 변하지 않음을 확인하고 패치 0001을 `verified`로 전환했다.

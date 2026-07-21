# 변경 기록

이 파일에는 사용자에게 보이는 변경 사항을 패치 단위로 기록한다.

## [Unreleased]

### Added

- localhost 전용 `tm-server` 뼈대와 liveness/readiness, 구조화 오류, request ID를 추가했다.
- 서버 실행 시 명시적인 절대 `TM_SERVER_HOME`을 요구해 실제 사용자 DB를 실수로 여는 경계를 추가했다.
- 서버 환경변수에만 API 키를 보관하는 OpenAI Responses API 연결 경계와 비과금 상태 확인·명시적 최소 probe를 추가했다.
- Railway Hobby용 multi-stage Docker image, Config as Code, Volume entrypoint를 추가했다.
- Railway 환경과 Volume을 검증하고 health/readiness만 제공하는 `cloud-bootstrap` 서버 프로필을 추가했다.
- 서버 시작 로그에 SQLite 최초 초기화 시각을 기록해 Volume 영속성을 민감 데이터 노출 없이 확인할 수 있게 했다.
- Railway 공개 전용 `cloud-authenticated` 프로필과 단일 사용자 bearer token 인증 경계를 추가했다.
- 256-bit 토큰을 화면·파일에 남기지 않고 생성하고 서버용 SHA-256 해시와 만료 시각만 출력하는 PowerShell 도구를 추가했다.
- 비인증 API 은닉, 토큰 만료, 인증 실패·정상 요청 rate limit, CORS 차단과 공통 보안 header를 추가했다.
- schema·migration ledger·무결성·외래키를 검사하고 테이블별 행 개수와 논리 SHA-256을 비교하는 migration manifest와 합성 DB dry-run 경계를 추가했다.
- 인증된 cloud profile에 project·task·checklist·tag·session·worklog·note의 versioned GET API와 DTO allowlist를 추가했다.
- read-only API에 제한된 filter·정렬, pagination, 512KiB 응답 상한, 콘텐츠 ETag와 구조화된 query·method 오류를 추가했다.
- 원문 토큰을 파일이나 명령 기록에 저장하지 않고 production 인증·조회·ETag·mutation 차단을 확인하는 PowerShell 검증 도구를 추가했다.
- Task·Note 생성/수정과 Checklist 완료 상태 변경만 허용하는 인증된 typed write API를 추가했다.
- mutation마다 idempotency key, 정수 version precondition, operation 확인 header와 64KiB body 상한을 요구하는 통제 경계를 추가했다.
- domain 변경, 중복 응답, actor·request ID·before/after를 한 transaction에 기록하는 append-only mutation 감사·idempotency ledger를 추가했다.
- 통제된 write API v1 계약 문서와 JSON Schema를 추가했다.
- 원문 token을 저장하지 않고 production write API의 인증·precondition·금지 동작과 데이터 불변을 확인하는 STEP 8 PowerShell 검증 도구를 추가했다.
- schema 5 append-only OpenAI token·비용 원장과 요청 전 USD 20 월 hard stop을 추가했다.
- SQLite 일관 snapshot을 client-side 암호화해 Railway Bucket으로 전송하고 일 7·주 4·월 12 보존·전체 pack 무결성 검사를 수행하는 작업을 추가했다.
- 활성 DB를 건드리지 않고 최신 외부 backup의 checksum·schema·SQLite 무결성을 검증하는 restore drill을 추가했다.
- 인증된 운영 상태 API와 request ID·route family·status·latency만 남기는 Railway JSON 로그를 추가했다.
- production schema 5와 STEP 11 전 AI route 격리를 과금·mutation 없이 확인하고, AI route 활성화 후에는 USD 10/20 예산 경계도 확인하는 STEP 9 PowerShell 검증 도구를 추가했다.
- row 원문을 노출하지 않고 현재 DB manifest, 일관 snapshot dry-run, 후보 snapshot 검사를 수행하는 `tm-cli migration` 명령과 STEP 10 검증 스크립트를 추가했다.
- Windows Credential Locker와 HTTPS desktop command bridge를 사용하는 명시적 cloud mode를 추가했다.
- schema 5 snapshot을 maintenance mode에서만 검증·import하고 실패 시 자동 rollback하는 STEP 10 cutover 경계를 추가했다.

### Changed

- Railway Volume 서비스가 replica 구성으로 해석되지 않도록 Config as Code의 `multiRegionConfig`를 `null`로 명시했다.
- SQLite schema 4에서 Task·Note·Checklist에 optimistic concurrency용 정수 `version`을 추가하고 read DTO에도 노출했다.
- 현재 mutation ledger와 일치하지 않는 backup 복원을 거부해 감사·idempotency 이력의 rewind를 차단했다.
- 현재 AI 비용 ledger와 일치하지 않는 backup 복원을 거부해 비용 이력의 rewind를 차단했다.
- Windows PowerShell 5.1이 한국어 검증 문구와 기본 결과 경로를 올바르게 처리하도록 production 검증 스크립트를 UTF-8 BOM 형식과 본문 경로 계산 방식으로 고정했다.
- cloud client 설정이 실제 TM home의 `data\cloud-client.json`을 사용하도록 수정하고, 토큰 형식·401 재입력과 운영 snapshot·backup·import route 격리를 저장 전에 검증하도록 강화했다.

### Verified

- Railway Singapore production에서 새 이미지 배포와 동일 이미지 재시작 후에도 `/var/lib/tm`의 SQLite 최초 초기화 시각이 유지됨을 확인했다.
- 인증 구현 전 원격 서버에는 공개 도메인과 OpenAI API 키가 없고 health/readiness route만 존재함을 확인했다.
- Railway production에서 `cloud-authenticated` singleton 배포와 기존 SQLite Volume 보존을 확인했다.
- HTTPS 공개 health/readiness는 `200`, 토큰 없음·잘못된 토큰·비인증 미등록 경로는 `401`이며 공통 보안 header가 적용됨을 확인했다.
- 비밀번호 관리자에 보관한 원문 토큰으로 production `/api/v1/auth/status`가 `200`과 `authenticated: true`를 반환함을 확인했다.
- 합성 SQLite DB에서 migration manifest 원문 비노출, read-only snapshot 비교, 지원하지 않는 schema 거부, rollback 후 논리 일치를 확인했다.
- 합성 cloud DB에서 read-only API 인증·DTO Schema·filter·pagination·ETag·응답 상한·mutation 차단 contract를 확인했다.
- Railway production에서 인증된 tasks 조회 `200`, ETag 재검증 `304`, mutation 요청 `405 METHOD_NOT_ALLOWED`, 빈 cloud DB 유지를 확인했다.
- 합성 DB에서 mutation 중복 제출 1회 실행, stale version 충돌, 실패 rollback, append-only 감사, strict JSON·입력 상한·금지 method를 확인했다.
- Railway production에 schema 4 controlled write API를 배포하고 인증 `200`, precondition `428`, 누락 resource `404`, 삭제 금지 `405`, Task·Note 개수와 ETag 불변을 비파괴 방식으로 확인했다.
- Railway staging schema 5에서 Task mutation·idempotency replay·stale version conflict를 확인하고, 암호화 Bucket snapshot을 빈 위치에 복원해 checksum·schema·SQLite 무결성을 검증했다.
- Railway Hobby에서는 native Volume backup과 CPU·RAM·disk·network monitor가 Pro 전용임을 dashboard와 API 권한 거부로 확인했다. deployment failure·crash·OOM·usage 알림은 email과 in-app으로 활성화되어 있다.
- staging에서 검증한 STEP 9 source를 Railway production deployment `4a0bd0d7-6a3d-4104-9eec-acb16bb78b7c`로 승격했다.
- production schema 5 확인 후 encrypted Bucket backup을 활성화하고 첫 snapshot `ecb2b965` 생성을 확인했다.
- production bearer token을 회전하고 새 token으로 인증·schema 5·원격 backup 성공·STEP 11 전 AI route `404` 격리를 재검증했다.
- production 암호화 backup을 `/tmp`에 복원해 checksum·schema 5·SQLite 무결성·foreign key 검사를 통과했고 활성 DB가 바뀌지 않음을 확인했다.
- production restore drill용 일회용 Railway SSH key와 모든 로컬 key 파일을 폐기하고, 검증 완료 후 staging deployment를 중지하면서 Volume은 보존했다.
- STEP 10 사전 inventory에서 local schema 3 DB의 integrity와 foreign key가 정상이고 Project 2·Task 2·Task event 5, 첨부파일 0개임을 원문 비노출 방식으로 확인했다.
- GitHub Actions Windows run `29812300709`에서 frontend와 Rust workspace test·clippy·release build를 통과했고, 후속 성공 run `29815250852`에서 x64 `tm.exe`·`tm-cli.exe` artifact를 내려받아 SHA-256을 재검증했다.
- production import의 logical SHA-256 `e66d9e33b3dc8fa22551a1914e9dcbd7ac3ac657a77355e37d170da9b4a758ba`가 source와 일치하고 encrypted remote backup이 성공했음을 확인했다.
- 정상 router 복귀 후 인증된 desktop snapshot에서 Project 2개·Task 2개를 확인하고 import route `404`, 최종 `tm-desktop/0.1.5` snapshot 요청 `200`을 확인했다.
- cloud를 단일 기준 원본으로 전환하고 로컬 DB·schema 3 backup·schema 5 snapshot을 2026-10-19까지 read-only archive로 고정했다.

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

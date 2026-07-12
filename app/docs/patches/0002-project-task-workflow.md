# 0002 — 프로젝트별 Task 작업공간 및 Inbox 흐름

- 패치 번호: `0002`
- 상태: `in-progress`

## 목적과 변경 이유

프로젝트를 만든 직후 해당 프로젝트 안에서 Task를 생성·조회·수정할 수 없던 흐름을 보완한다. 프로젝트 화면을 master-detail 작업공간으로 바꾸고, Inbox는 생각을 제목만 빠르게 수집한 뒤 프로젝트의 `todo`로 정리하는 분류 대기함으로 역할을 명확히 한다.

## 변경 파일과 기능 범위

- `app/src/components/TaskPages.tsx`: 프로젝트 목록·상세, 프로젝트 고정 Task 빠른 추가, 상태 그룹, 완료 필터, Inbox 분류 UI
- `app/src/App.tsx`: 프로젝트 생성 ID 반영, 프로젝트 Task 생성 안내, Inbox aggregate 정리 동작
- `app/src/types.ts`, `app/src/lib/api.ts`: 선택적 Task 상태와 프로젝트 ID 반환 계약
- `app/src/lib/mock-transport.ts`: 새 계약과 TaskEvent before/after를 재현하는 로컬 UI transport
- `app/src/styles.css`: 데스크톱 master-detail, 700px 세로 배치, Inbox 분류 및 접근성 스타일
- `app/src/App.test.tsx`, `app/src/lib/api.test.ts`: 프로젝트·Inbox 흐름과 IPC payload 회귀 테스트
- `app/src-tauri/src/commands.rs`: `create_task` 상태 기본값·검증과 `create_project` ID 반환
- `app/package.json`, `app/Cargo.toml`, `app/Cargo.lock`, `app/src-tauri/tauri.conf.json`: 버전 `0.1.1`
- `app/scripts/build-release.ps1`, `app/scripts/build-nsis.ps1`: 설치를 유발할 수 있는 package-manager shim 대신 기존 로컬 CLI를 직접 실행

## DB 및 migration 영향

DB schema와 migration 변경은 없다. 프로젝트 Task 생성은 기존 Task aggregate에 `status=todo`를 전달한다. Inbox 정리는 기존 `update_task_aggregate` 한 번으로 프로젝트와 상태를 함께 변경하므로 태그·체크리스트를 보존하며 TaskEvent에 변경 전·후 값이 남는다.

## 사용자에게 보이는 변화

- 프로젝트 화면 왼쪽에서 프로젝트를 선택하고 오른쪽에서 해당 프로젝트 Task를 관리한다.
- 선택 프로젝트에 제목·우선순위·마감일을 입력해 `todo` Task를 바로 만든다.
- 열린 Task는 `진행 중`, `할 일`, `막힘`, `정리 대기`로 묶이며 완료·취소는 별도 필터에서 본다.
- Task를 누르면 기존 상세 drawer가 열려 설명·상태·프로젝트·태그·체크리스트 등을 수정한다.
- Inbox 입력은 제목만 받고, 각 Task에서 프로젝트를 선택해 `정리 완료`하면 `todo`가 된다.
- 새 프로젝트는 생성 직후 자동 선택된다.

## 검증

| 명령 또는 점검 | 정확한 결과 |
|---|---|
| `node_modules\.bin\eslint.cmd . --max-warnings 0` | 성공, 경고 `0`, exit `0` |
| `node_modules\.bin\tsc.cmd -b --pretty false` | 성공, exit `0` |
| `node_modules\.bin\vitest.cmd run` | `2` files, `11` tests passed, `0` failed |
| `node_modules\.bin\vite.cmd build` | `41` modules; CSS `47.42 kB`(gzip `9.28`), JS `274.25 kB`(gzip `81.88`), 성공 |
| Tauri create_task 상태 계약 테스트 | 기본 `inbox`, 명시적 `todo`, 미지원 상태 거부; `2` passed |
| 1320px UI | 프로젝트 목록 `285px` + 상세 `650px` 2열, 수평 overflow `0` |
| 900px UI | 프로젝트 목록 `245px` + 상세 `568px` 2열, 수평 overflow `0` |
| 700px UI | 목록·상세 `657px` 1열, 수평 overflow `0`, Inbox 빠른 입력 한 줄 유지 |
| 접근성 점검 | 표시 중인 프로젝트 화면 button/input/select 중 접근 이름 누락 `0` |
| 시스템 테마 | 실제 시스템 다크 테마 렌더링 확인; 기본 light 토큰과 dark media query 유지 |
| `cargo fmt --all -- --check` | 성공, exit `0` |
| `cargo clippy --locked --offline --workspace --all-targets -- -D warnings` | 성공, 경고 `0`, exit `0` |
| `cargo test --locked --offline --workspace` | 코어 단위 `2`, 코어 통합 `15`, CLI `7`, Tauri `4`; 합계 `28 passed`, `0 failed` |
| Windows x64 raw release | `tm-desktop 0.1.1`, `tm-cli 0.1.1` offline release build 성공; 두 EXE PE machine `0x8664` |
| `build-nsis.ps1 -Approved` | 로컬 Tauri CLI·NSIS cache만 사용, `TM_0.1.1_x64-setup.exe` 생성, 설치·실행 없음, exit `0` |
| PowerShell 스크립트 parse | `app/scripts` 전체 parse error `0` |
| 소스 경계 검사 | `app/.git` 없음, SQLite·첨부·로그·비밀키 패턴 파일 `0` |

## 산출물 SHA-256

- `TM/dist/release/tm-0.1.1.exe`
  - 크기: `13,552,640` bytes
  - SHA-256: `569F97617D09215FF60CB4705F9F8E437B8192DCCEFE3AFA7BE490EC58E1FA95`
- `TM/dist/release/tm-cli.exe`
  - 크기: `4,466,688` bytes
  - SHA-256: `1893AE6124FE3C3CF0E39B83DBE68B7CFDA7798FD2C5853D9A570CDD55D7D42E`
- `TM/dist/release/TM_0.1.1_x64-setup.exe`
  - 크기: `3,629,704` bytes
  - SHA-256: `9A80296E44F0ED36BB8CAEB49D71FEDB021F2883D58C55BB1E4684A1BBFA134D`

기존 `TM/dist/release/tm.exe`가 PID `9340`으로 실행 중이어서 강제 종료하거나 덮어쓰지 않고 새 데스크톱 EXE를 `tm-0.1.1.exe`로 저장했다. NSIS에는 `0.1.1` 새 빌드가 포함됐다.

## 복구·롤백 방법

앱과 CLI를 종료하고 DB 백업을 먼저 만든 뒤 이 패치의 소스 변경을 `0.1.0` 소스 스냅샷으로 되돌린다. DB migration이 없으므로 데이터베이스 downgrade는 필요하지 않다. `data`, `backups`, `attachments`, `logs`, `exports`는 소스 롤백과 분리해 그대로 보존한다.

## 알려진 제한과 후속 작업

- 프로젝트 설명 수정·보관·삭제 UI와 칸반 drag-and-drop은 포함하지 않는다.
- 실제 `0.1.0 → 0.1.1` 설치·업그레이드·제거 및 외부 데이터 보존 검증은 별도 승인 전까지 실행하지 않는다.
- 위 installer lifecycle 검증이 남아 있으므로 빌드와 테스트가 통과해도 상태를 `verified`로 바꾸지 않는다.
- 현재 실행 중인 `dist/release/tm.exe`는 `0.1.0` 그대로다. 앱을 종료한 뒤 `tm-0.1.1.exe`를 `tm.exe`로 교체하는 작업은 다음 승인 단계에서 수행한다.
- Slack 연결, 메시지 전송, 예약 작업은 변경하지 않는다.

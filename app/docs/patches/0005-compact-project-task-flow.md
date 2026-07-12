# 0005 — compact 프로젝트 Task 생성 흐름

- 패치 번호: `0005`
- 상태: `in-progress`

## 목적과 변경 이유

Inbox의 역할이 명확해질 때까지 즉흥 입력과 분류 기능을 제거하고, Task 생성 위치를 프로젝트 화면 한 곳으로 모은다. 새 Task 입력에 꼭 필요한 정보만 한 줄 중심으로 보여 주어 화면 밀도를 낮추고 빠르게 작성할 수 있게 한다.

## 변경 파일과 기능 범위

- `app/src/components/TaskPages.tsx`: Inbox 보류 화면, 오늘 빠른 입력 제거, 프로젝트 compact Task 입력
- `app/src/App.tsx`: Inbox 동작과 건수 제거, 단순화된 페이지 인터페이스
- `app/src/styles.css`: 보류 화면과 compact 입력의 데스크톱·태블릿·모바일 배치
- `app/src/App.test.tsx`: Inbox 비활성 상태와 프로젝트 Task 생성 회귀 테스트
- `app/package.json`, workspace Cargo manifest·lockfile, `app/src-tauri/tauri.conf.json`: 버전 `0.1.4`

## DB 및 migration 영향

없다. 기존 schema, Task 상태, TaskEvent 규칙은 변경하지 않는다. 프로젝트에서 생성한 Task는 기존과 같이 `todo`로 저장한다.

## 사용자에게 보이는 변화

- Inbox는 입력·분류 동작 없이 향후 기획을 알리는 보류 화면만 표시한다.
- 오늘 화면에서는 새 Task를 만들지 않고 이미 만든 Task의 오늘 계획만 관리한다.
- 프로젝트 Task 입력은 소속 프로젝트, 이름, 설명, 우선순위, 선택 마감일만 표시한다.
- 새 Task의 소속은 현재 선택한 프로젝트로 고정되고, Task를 선택하면 기존 상세 화면에서 전체 필드를 수정할 수 있다.
- 프로젝트의 열린 목록에는 진행 중·할 일·막힘만 표시한다.

## 검증

| 명령 또는 점검 | 정확한 결과 |
|---|---|
| `node_modules/eslint/bin/eslint.js . --max-warnings 0` | 성공, 경고 `0`, exit `0` |
| `node_modules/typescript/bin/tsc -b --pretty false` | 성공, exit `0` |
| `node_modules/vitest/vitest.mjs run` | `2` files, `17` tests passed, `0` failed; Inbox 보류·compact Task 생성 회귀 테스트 포함 |
| `node_modules/vite/bin/vite.js build` | `42` modules; HTML `0.44 kB`(gzip `0.28`), CSS `57.81 kB`(gzip `10.82`), JS `300.60 kB`(gzip `88.04`); 성공 |
| `cargo fmt --all -- --check` | 성공, exit `0` |
| `cargo clippy --locked --offline --workspace --all-targets -- -D warnings` | 기존 TM offline Cargo cache 사용, 성공, 경고 `0`, exit `0` |
| `cargo test --locked --offline --workspace` | 분리 target에서 CLI `11`, core unit `2`, ChangeRequest `12`, core integration `15`, Tauri `4`: 합계 `44` passed, `0` failed |
| 1320px 브라우저 점검 | 프로젝트 master-detail `285px / 650px`, compact 입력 한 줄, 입력 `clientWidth=648`, `scrollWidth=648`, 페이지 수평 overflow `0` |
| 900px 브라우저 점검 | 프로젝트 master-detail `245px / 568px`, 입력 context·이름/설명·옵션의 3단 배치, 입력·페이지 수평 overflow `0` |
| 700px 브라우저 점검 | 프로젝트 목록과 상세 세로 배치, 입력 2열 반응형 배치, 입력·페이지 수평 overflow `0` |
| Inbox 브라우저 점검 | 700px에서 제목 `Inbox`, 안내 `Inbox는 잠시 비워 두었습니다`, main 내부 form `0`, button `0`, 수평 overflow `0` |
| `scripts/build-release.ps1` | 기존 cache만 사용하는 offline Windows x64 `tm.exe`, `tm-cli.exe` release 빌드 성공; NSIS·installer 실행 없음 |

## 산출물 SHA-256

| 산출물 | 바이트 | SHA-256 |
|---|---:|---|
| `dist/release/tm.exe` (`ProductVersion 0.1.4`) | `13,711,360` | `3DAF31C125C2E1E9118FC35F1DBDEA209636DAA77B3899009132899B8044F616` |
| `TM/tm.exe` (`ProductVersion 0.1.4`) | `13,711,360` | `3DAF31C125C2E1E9118FC35F1DBDEA209636DAA77B3899009132899B8044F616` |
| `dist/release/tm-cli.exe` | `4,619,776` | `A44D011777D5EB40D486F28569BFA43A6437C1A4C21475BB4C9AA4EA48F9F53C` |

## 복구·롤백 방법

이 패치에서 변경한 UI·테스트·문서를 되돌리고 버전을 `0.1.3`으로 복원한다. DB migration이 없으므로 사용자 데이터 복원은 필요하지 않다. 루트에 복사한 `tm.exe`는 이전 검증 산출물로 교체한다.

## 알려진 제한과 후속 작업

- 정정: 이 패치에서 생성한 `0.1.4` raw `tm.exe`는 Tauri `custom-protocol` 누락으로 내장 UI 대신 `http://localhost:1420`을 열었다. 해당 산출물은 유효한 release가 아니며 패치 `0006`의 `0.1.5`로 교체한다.
- 기존 DB에 남아 있는 `inbox` 상태 Task는 삭제하거나 변환하지 않으며 현재 프로젝트 열린 그룹에서는 표시하지 않는다.
- Inbox의 후속 역할은 별도 기획 전까지 구현하지 않는다.
- NSIS 설치 패키지와 설치·업그레이드·제거 검증은 이번 패치 범위에 포함하지 않는다.
- Windows NSIS lifecycle 검증 전까지 상태를 `verified`로 바꾸지 않는다.

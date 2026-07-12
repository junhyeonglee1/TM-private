# 0003 — 수동 승인 개선 요청 큐

- 패치 번호: `0003`
- 상태: `in-progress`

## 목적과 변경 이유

사용자가 TM 안에 제품 개선 요청을 기록하고 명시적으로 승인한 뒤, 원할 때 Codex가 한 건을 안전하게 가져가 수정할 수 있는 로컬 수동 승인 흐름을 제공한다. 자동 예약·Git·MCP·OpenAI API 없이 현재 로컬 작업공간과 `tm-cli`만 사용한다.

## 변경 파일과 기능 범위

- `app/crates/tm-core/migrations/0002_change_requests.sql`, `0003_change_request_strict_cas.sql`: 요청함 schema와 기존 schema 2용 strict CAS trigger upgrade
- `app/crates/tm-core/src/model.rs`, `change_request.rs`, `core.rs`, `database.rs`, `backup.rs`, `export.rs`, `lib.rs`: 요청 모델·상태 전이·원자 claim·복원 ledger·내보내기
- `app/crates/tm-core/tests/change_request_integration.rs`, `core_integration.rs`: 동시 claim, stale CAS, schema 1/2 migration, 복원 비후퇴 회귀 테스트
- `app/crates/tm-cli/src/lib.rs`, `main.rs`: `changes prepare|complete|fail|status` 수동 처리 명령
- `app/src-tauri/src/commands.rs`, `lib.rs`: 요청 CRUD·승인·승인 취소·취소·claim 유실 전환 command와 snapshot
- `app/src/components/ChangeRequestsPage.tsx`, `App.tsx`, `types.ts`, `lib/api.ts`, `lib/mock-transport.ts`, `styles.css`: 개선 요청함 UI, 상태별 동작, strict CAS, 반응형 배치
- `app/src/App.test.tsx`, `lib/api.test.ts`: 생성·승인·실패 편집·terminal 읽기 전용·camelCase·모바일 focus 회귀 테스트
- `app/docs/prompts/change-request-processing.md`, `docs/architecture/change-request-workflow.md`: 새 Codex 세션에서도 사용할 수 있는 수동 처리 절차와 권한 경계
- `app/package.json`, workspace Cargo manifest·lockfile, `src-tauri/tauri.conf.json`: 버전 `0.1.2`
- `app/scripts/verify.ps1`: package-manager shim 없이 기존 ESLint·TypeScript·Vitest 실행 파일을 직접 호출

## DB 및 migration 영향

SQLite schema `2`에서 ChangeRequest와 append-only 이벤트를 추가하고, schema `3`에서 모든 사용자 상태 전이에 revision·attempt CAS와 draft 복귀 revision 증가 규칙을 강화한다. 기존 schema `1` 또는 `2` DB는 migration 전에 온라인 백업을 만든다. 승인된 요청의 원자적 claim, claim key 검증과 terminal 상태 불변성을 DB와 Rust 계층에서 함께 강제한다.

## 사용자에게 보이는 변화

- 도구 메뉴의 `개선 요청함`에서 초안을 작성·편집한다.
- 사용자가 승인한 요청만 Codex 처리 대기가 된다.
- 승인 취소, 취소, 실패 후 수정·재승인, 처리 결과 확인을 지원한다.
- 승인만으로 자동 코드 수정은 시작되지 않으며 사용자가 Codex에서 직접 처리를 요청한다.

## 검증

| 명령 또는 점검 | 정확한 결과 |
|---|---|
| `node_modules\.bin\eslint.cmd . --max-warnings 0` | 성공, 경고 `0`, exit `0` |
| `node_modules\.bin\tsc.cmd -b --pretty false` | 성공, exit `0` |
| `node_modules\.bin\vitest.cmd run` | `2` files, `17` tests passed, `0` failed; duration `28.17s` |
| `node_modules\.bin\vite.cmd build` | `42` modules; HTML `0.44 kB`(gzip `0.28`), CSS `55.43 kB`(gzip `10.39`), JS `299.94 kB`(gzip `87.48`); 성공 |
| `cargo fmt --all -- --check` | 성공, exit `0` |
| `cargo clippy --locked --offline --workspace --all-targets -- -D warnings` | 성공, 경고 `0`; 마지막 복원 ledger 보완 후 `tm-core --all-targets`도 재실행 성공 |
| `cargo test --locked --offline --workspace` (`CARGO_TARGET_DIR=dist/test-runs/cargo-final-v012`, jobs `1`) | CLI `11`, core unit `2`, ChangeRequest `12`, core integration `15`, Tauri `4`: 합계 `44` passed, `0` failed; doc tests `0` |
| schema migration | schema `1→2→3`, `2→3` 모두 pre-migration backup 생성 후 성공 |
| 원자 claim·strict CAS | 동시 실행 `12`개 중 claim 성공 `1`; 편집·승인·승인 취소·취소·유실 전환은 revision+attempt 불일치 거부 |
| 복원 비후퇴 | completed/failed/claimed/cancelled, 재승인 attempt, 승인 철회 draft와 전체 이벤트 유지; 누락 FK는 active DB copy 전에 거부하고 DB SHA 불변 |
| 1320px 브라우저 점검 | 수평 overflow `0`; sidebar `248px`, 요청함 page `951.4px`; 접근 이름 없는 main control `0` |
| 900px 브라우저 점검 | 수평 overflow `0`; sidebar off-canvas, 요청함 page `829px`; 카드 action 세로 재배치 정상 |
| 700px 브라우저 점검 | 수평 overflow `0`; 요청함 page `657px`, 카드 본문 `1`열, 접근 이름 없는 control `0`; navigation focus scroll 결함 발견 후 `preventScroll` 적용 및 UI 회귀 테스트 추가 |
| 시스템 테마 | 실제 시스템 dark 렌더링 확인; 기존 light 토큰과 dark media query 유지 |
| 버전·민감정보·경로 감사 | package/Cargo/Tauri 모두 `0.1.2`; `app` 안 DB·WAL·SHM·첨부·로그·`.env`·키·토큰 없음; build/cache/test 출력은 `TM\dist` 내부 |
| `scripts/build-release.ps1` | 기존 cache만 사용하는 offline Windows x64 `tm.exe`, `tm-cli.exe` release 빌드 성공; 설치 프로그램 실행 없음 |
| `scripts/build-nsis.ps1 -Approved` | 로컬 NSIS cache로 `TM_0.1.2_x64-setup.exe` 생성 성공; installer 실행·설치 없음 |

## 산출물 SHA-256

| 산출물 | 바이트 | SHA-256 |
|---|---:|---|
| `dist/release/tm.exe` (`ProductVersion 0.1.2`) | `13,832,704` | `F161E32DC16EAFDAA5E35CB6D5FC8300C6F000D1B6E8F87E51B6E0C3BA724EF1` |
| `dist/release/tm-cli.exe` | `4,619,776` | `AACAEF8FA5D922C53D9EFBEF69E8DF9CCFE88D20A22B35651263C8B8C5B41CE5` |
| `dist/release/TM_0.1.2_x64-setup.exe` (`ProductVersion 0.1.2`) | `3,712,229` | `88D37740F4922A9550E1EC0D83DA20EA7BC4D92961BCAF18BDD070F734F36A84` |

## 복구·롤백 방법

앱과 CLI를 종료하고 최신 DB 백업을 만든다. 소스는 이전 snapshot으로 되돌리되 외부 `data`, `backups`, `attachments`, `logs`, `exports`는 보존한다. schema 3 DB를 이전 schema 실행 파일로 직접 열지 말고, 필요하면 schema 3 앱에서 생성한 pre-migration 또는 수동 백업을 기준으로 복원한다.

## 알려진 제한과 후속 작업

- 예약 작업, MCP, OpenAI API와 Git 연동은 포함하지 않는다.
- 보호된 개선 요청이 참조하는 프로젝트·Task가 선택 백업에 없으면 연결을 임의로 `NULL` 처리하지 않고 복원을 안전하게 거부한다.
- 세 release 산출물은 코드 서명 인증서가 없어 `NotSigned` 상태다.
- 설치·업그레이드·제거 lifecycle 검증 전까지 상태를 `verified`로 바꾸지 않는다.

# 0001 — Local-first foundation

- 패치 번호: `0001`
- 상태: `verified`

## 목적과 변경 이유

개인 Task, 오늘 계획, 작업 세션, WorkLog, Note와 변경 이력을 하나의 로컬 SQLite 데이터베이스에서 안전하게 관리하는 Windows x64 데스크톱 앱을 새로 구축한다. React는 SQL을 실행하지 않고 Rust 공용 코어만 호출한다.

## 변경 파일과 기능 범위

- `app/src`: 한국어 React UI
- `app/src-tauri`: Tauri 2 데스크톱 셸과 command 계층
- `app/crates/tm-core`: 데이터 모델, migration, 서비스, 백업, 검색, export
- `app/crates/tm-cli`: 공용 코어를 사용하는 CLI와 digest delivery claim
- `app/docs`: 아키텍처, 운영 프롬프트와 패치 기록
- `app/scripts`: 설치와 분리된 오프라인 검증, raw x64 빌드, 승인 가드가 있는 NSIS 빌드 절차
- `TM/dist`: pnpm/Cargo cache, 테스트 임시 홈, 프런트엔드와 Windows release 산출물

## DB 및 migration 영향

초기 SQLite schema와 Task·세션·WorkLog·Note 통합 FTS5 색인을 생성한다. 기존 DB가 존재하는 경우 migration 전에 온라인 백업을 만든다. migration은 순차 버전으로 기록하며 데이터 파일은 `app` 밖의 `TM/data`에만 둔다. TaskEvent와 확정 TaskDayEntry, 전송 완료 digest는 DB trigger로 불변성을 강제한다.

## 사용자에게 보이는 변화

프로젝트·Inbox·오늘·히스토리·세션·WorkLog·Note·검색·휴지통·백업/복원·내보내기를 한국어 UI에서 사용할 수 있다. 시스템 다크/라이트 테마와 반응형 레이아웃을 제공한다. `tm-cli`는 digest, health, DB/소스 백업과 export JSON을 제공한다. NSIS installer 검증 전이므로 현재 배포 산출물은 raw EXE 두 개다.

## 검증

| 명령 | 정확한 결과 |
|---|---|
| 작업공간 경로 확인 | `C:\Users\tkfk0\Desktop\codex\TM` |
| 숨김 포함 초기 항목 수 확인 | `0` |
| `rustc --version --verbose` | `rustc 1.97.0`, host `x86_64-pc-windows-msvc` |
| 번들 `node.exe --version` | `v24.14.0` |
| `pnpm --version` | `11.7.0` |
| `rustfmt --version` | `rustfmt 1.9.0-stable` |
| `cargo clippy --version` | `clippy 0.1.97` |
| `pnpm typecheck` | 성공, exit `0` |
| `pnpm lint` | 성공, 경고 `0`, exit `0` |
| `pnpm test` | `2` files, `7` tests passed, `0` failed |
| `pnpm build` | `41` modules, CSS `43.62 kB`(gzip `8.74`), JS `268.54 kB`(gzip `80.46`), 성공 |
| `cargo fmt --all -- --check` | 성공, exit `0` |
| `cargo clippy --locked --offline --workspace --all-targets -- -D warnings` | 성공, 경고 `0`, exit `0` |
| `cargo test --locked --offline --workspace` | 코어 단위 `2`, 코어 통합 `15`, CLI `7`, Tauri `2`; 합계 `26 passed`, `0 failed` |
| 실제 CLI health/digest/backup/export | `WAL`, `shouldSend=[true,false,true,false]`, 최종 `sent`, DB/소스 백업·전체 export 성공 |
| x64 release build | `cargo build --locked --offline --release --target x86_64-pc-windows-msvc ...`; 성공, 최종 경고 `0` |
| PE machine 확인 | `tm.exe`, `tm-cli.exe` 모두 `0x8664` |
| release 고정 경로 검사 | 두 EXE 모두 기본 TM 루트 포함, `TM_HOME` 문자열 없음 |
| 경계 정적 검사 | `app` 금지 파일 `0`, React SQL `0`, 이전 경로 참조 `0`, 비밀 패턴 `0`, `app/.git` 없음 |
| 기본 데이터 경로 오염 검사 | `data`, `backups`, `exports`의 파일 `0`; 모든 기능 검증은 `dist/test-runs` 사용 |
| PowerShell/Tauri 설정 검사 | 스크립트 4개 parse error `0`; `currentUser`, WebView2 `skip`, downgrade 차단, TM 내부 도구 cache |
| NSIS offline bundle | local cache `442`개 파일 사용, `TM_0.1.0_x64-setup.exe` 생성, exit `0` |
| 신규 설치·재설치·제거 | 각 exit `0`; 설치 디렉터리와 HKCU 제거 항목 생성/제거 확인 |
| installer 외부 데이터 보존 | 세 단계 모두 `24`개 파일 aggregate SHA-256 `7ED610152D8AC3969DFD7C8FBD7C6AC7C770857F5CE8F2ADF80E24F00664C1F2` 일치 |
| 설치된 CLI/DB | health `ok`, WAL, schema `1`, integrity `ok`, digest `sent` 유지 |
| WebView2 | `skip`, Runtime `150.0.4078.65` 설치 전후 동일 |
| 검증 데이터 정리 | `dist/test-runs/nsis-lifecycle-20260711-final`로 이동, 기본 데이터 폴더 파일 `0` |

## 산출물 SHA-256

- `TM/dist/release/tm.exe`
  - 크기: `13,553,664` bytes
  - SHA-256: `576EC28DEF763DCF8926D928F414B6D529033070892561C6B0BBCF42650EF31D`
- `TM/dist/release/tm-cli.exe`
  - 크기: `4,466,688` bytes
  - SHA-256: `BF7EDF11527FD1A0FAC69DFDB250CE95636F9164DD169DBE86F069CC6AE85229`
- `TM/dist/release/TM_0.1.0_x64-setup.exe`
  - 크기: `3,621,829` bytes
  - SHA-256: `6A54516A6EFCC270414FAC6164BF09DF643117E29193FA7CCCD21F5C8EF515A1`

두 파일은 Authenticode 서명이 없는 개발 산출물(`NotSigned`)이다.

## 복구·롤백 방법

실행 중인 앱과 CLI를 종료하고 먼저 `tm-cli backup create`로 새 DB 백업을 만든다. 소스는 보존한 이전 ZIP으로 되돌리되 `data`, `backups`, `exports`는 소스 롤백과 분리한다. DB 복원은 현재 DB의 `pre_restore` 백업, 후보 integrity/schema 검사, 연결·이력 검증 순서로 수행한다. downgrade는 NSIS 설정에서 차단한다.

## 알려진 제한과 후속 작업

- NSIS 신규 설치·동일 버전 재설치·제거와 외부 데이터 보존 검증을 완료해 패치 상태를 `verified`로 전환했다.
- 실제 이전 버전 installer가 없어 downgrade 거부는 설정(`allowDowngrades=false`)만 확인했다.
- Slack 연결, 메시지 전송, Codex 예약 작업은 구현 범위 밖의 외부 작업으로 남긴다.
- Git 초기화와 원격 저장소 작업은 수행하지 않는다.
- release EXE는 Authenticode 서명이 없어 배포 시 Windows 출처 경고가 나타날 수 있다.

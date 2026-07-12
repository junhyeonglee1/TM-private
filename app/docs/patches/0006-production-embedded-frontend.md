# 0006 — production 내장 프런트엔드 복구

- 패치 번호: `0006`
- 상태: `in-progress`

## 목적과 변경 이유

`0.1.4` raw Windows 실행 파일이 내장 프런트엔드 대신 개발용 `http://localhost:1420`을 열어 앱을 사용할 수 없는 문제를 수정한다. Tauri production protocol을 release 빌드의 명시적 계약으로 만들고 누락 시 컴파일 단계에서 실패하게 한다.

## 변경 파일과 기능 범위

- `app/src-tauri/Cargo.toml`: 앱의 `custom-protocol` feature를 `tauri/custom-protocol`에 연결
- `app/src-tauri/src/lib.rs`: production feature가 빠진 release 컴파일 차단
- `app/scripts/build-release.ps1`: raw desktop release에 `--features custom-protocol` 강제
- workspace·Tauri·프런트엔드 버전: `0.1.5`
- `app/CHANGELOG.md`, 패치 문서: 잘못된 `0.1.4` 산출물 정정과 검증 기록

## DB 및 migration 영향

없다. schema와 사용자 데이터는 변경하지 않는다. 실제 실행 검증 시 기존 정책에 따라 시작 백업이 생성될 수 있다.

## 사용자에게 보이는 변화

- 최상위 `TM/tm.exe`가 별도 Vite 서버 없이 내장 UI를 연다.
- localhost 연결 실패 화면이 나타나지 않는다.

## 검증

| 명령 또는 점검 | 정확한 결과 |
|---|---|
| ESLint·TypeScript | lint 경고 `0`, typecheck 성공, 각 exit `0` |
| Vitest | `2` files, `17` tests passed, `0` failed |
| Vite production build | `42` modules; HTML `0.44 kB`, CSS `57.81 kB`, JS `300.60 kB`; 성공 |
| Rust fmt·Clippy | fmt 성공; `--features custom-protocol`, `-D warnings` Clippy 성공, 경고 `0` |
| Rust workspace test | CLI `11`, core unit `2`, ChangeRequest `12`, core integration `15`, Tauri `4`: 합계 `44` passed, `0` failed |
| production feature compile guard | feature 없이 `cargo check --release` 실행 시 `TM release builds require the custom-protocol feature`로 의도대로 차단, exit `1` |
| `scripts/build-release.ps1` | Tauri CLI `build --no-bundle --features custom-protocol`; offline Windows x64 raw EXE 성공, NSIS 실행 없음 |
| 내장 asset 검사 | 최신 release build output에 `tauri-codegen-assets` HTML `194` B, CSS `9,810` B, JS `80,561` B 생성; 잘못된 `0.1.4` build output에는 해당 디렉터리 없음 |
| 실제 release 프로세스 검사 | 최상위 `tm.exe` 실행 시 프로세스 `Responding=True`, 시작 백업 생성, 해당 PID의 localhost/TCP 연결 `0`건; 검증 프로세스 종료 후 깨끗한 상태 유지 |

## 산출물 SHA-256

| 산출물 | 바이트 | SHA-256 |
|---|---:|---|
| `TM/tm.exe` (`ProductVersion 0.1.5`) | `13,831,680` | `4D799E06536DBB007F600224B701EDD8F1248ECCFABC21BAEAF2CDD3AAA3ED11` |
| `dist/release/tm.exe` (`ProductVersion 0.1.5`) | `13,831,680` | `4D799E06536DBB007F600224B701EDD8F1248ECCFABC21BAEAF2CDD3AAA3ED11` |
| `dist/release/tm-cli.exe` | `4,619,776` | `495466FC02679AD9FC18B1F2DC2D080CAACF49262056324BE71BFE71281D25A2` |

## 복구·롤백 방법

`custom-protocol` feature와 release compile guard, 빌드 스크립트 인자를 되돌리고 버전을 `0.1.4`로 복원한다. 단, `0.1.4` raw 실행 파일은 localhost 오류가 있으므로 사용자 배포용으로 복원하지 않는다. DB 복원은 필요하지 않다.

## 알려진 제한과 후속 작업

- NSIS 설치 패키지 생성과 설치·재설치·제거 검증은 이번 수정 범위에 포함하지 않는다.
- GUI 자동 실행 환경은 사용자 데스크톱 창 핸들을 제공하지 않아 화면 픽셀 검증은 하지 못했다. 대신 production asset 생성, feature guard, 프로세스 응답, 시작 백업과 localhost 연결 부재를 검증했다.
- 실제 Windows NSIS lifecycle 검증 전까지 상태는 `in-progress`로 유지한다.

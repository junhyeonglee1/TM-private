# TM

TM은 Windows x64용 로컬 우선 개인 Task 관리 프로그램이다. React UI는 Tauri command만 호출하며 SQLite 접근, 시간대 처리, 백업, export와 CLI 기능은 Rust `tm-core`가 담당한다.

## 디렉터리 경계

- 소스·문서·테스트: `TM/app`
- 실제 DB·첨부·로그: `TM/data`
- DB·소스 백업: `TM/backups`
- 사용자 export: `TM/exports`
- 의존성 캐시·빌드·release 산출물: `TM/dist`

기본 데이터 홈은 `C:\Users\tkfk0\Desktop\codex\TM`이며 테스트에서만 `TM_HOME`을 임시 경로로 지정한다.

release 앱과 CLI는 고정 기본 경로만 사용한다. debug/test의 `TM_HOME`은 절대경로이면서 기본 TM 루트 하위인 경우에만 허용하며, 테스트 fixture는 `TM\\dist\\test-runs`를 사용한다.

## 주요 명령

- 전체 검증(설치 없음): `scripts\\verify.ps1`
- Windows x64 검증 artifact 설치: `scripts\\build-release.ps1 -Approved -RunId <Actions run ID> -ExpectedHeadSha <검증 commit SHA>`
  - 해시 검증·잠금 확인·백업 후 `TM\\dist\\release`와 최상위 `TM\\tm.exe`를 함께 갱신한다.
  - 사용자는 항상 `C:\\Users\\tkfk0\\Desktop\\codex\\TM\\tm.exe`를 실행한다.
- 로컬 NSIS 빌드는 Windows 정책상 비활성화되어 있으며, `scripts\\build-nsis.ps1 -Approved`는 안전하게 중단한다.
- DB 백업: `tm-cli backup create`
- 소스 ZIP 스냅샷: `tm-cli backup source`
- 상태 확인: `tm-cli health --json`
- 전체 JSON 출력: `tm-cli export --json`

검증·빌드 스크립트는 기존 `node_modules`, Cargo cache와 lockfile만 사용하며 자동으로 `pnpm install`이나 crate 다운로드를 실행하지 않는다. 의존성 준비는 별도 승인 작업이다.

## 개발 안전 원칙

- Git 초기화와 원격 저장소 작업은 별도 승인 전까지 수행하지 않는다.
- 사용자 데이터와 비밀정보는 `app`에 저장하지 않는다.
- NSIS 도구 준비, 설치 패키지 생성·실행, 설치·재설치·제거, Slack 연결과 예약 작업은 별도 승인 후 수행한다.
- NSIS 설정은 WebView2를 다운로드하지 않는 `skip`, 사용자 단위 설치, downgrade 차단으로 준비되어 있다. WebView2 Runtime이 없는 PC에는 별도 설치가 필요하다.

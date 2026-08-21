# 0009 — Windows 클라우드 자격 증명 콘솔 숨김

## 요청

- 개선 요청 ID: `019f920a-f515-7430-b72a-abcd5ac6a867`
- 요청 제목: `Window 앱 UI 전환시 터미널 창 발생`
- 목표: Windows TM의 탭을 전환할 때 터미널 창이 나타나지 않고 화면이 자연스럽게 전환되어야 한다.

## 원인

클라우드 모드의 desktop command는 Windows Credential Locker에서 운영 토큰을 읽기 위해 `powershell.exe`를 실행한다. 캘린더, 주식, 기기 관리처럼 탭 진입과 동시에 서버 데이터를 읽는 화면에서는 이 자식 프로세스가 Windows 콘솔 창을 잠깐 생성해 화면이 깜박였다.

## 변경

- 자격 증명 조회용 PowerShell 자식 프로세스에 Windows `CREATE_NO_WINDOW` 생성 플래그를 강제했다.
- 표준 입력과 표준 오류 차단, 토큰의 표준 출력 회수, Credential Locker 사용 방식은 그대로 유지했다.
- 보안 정적 검사에서 숨김 실행 플래그가 제거되면 실패하도록 회귀 규칙을 추가했다.
- Windows Rust unit test에서 사용 중인 생성 플래그 값을 검증한다.

## 데이터 영향

- schema와 운영 데이터 변경은 없다.
- 토큰은 계속 Windows Credential Locker에서 요청 시 읽으며 파일에 저장하지 않는다.
- 수정 전에 Railway 운영 DB 백업과 Git source snapshot을 생성했다.

## 검증

- Rust format 통과
- STEP 16 보안 정적 검사 271개 파일 통과
- TypeScript, ESLint, frontend 전체 26개 테스트 통과
- 로컬 Cargo는 Windows 애플리케이션 제어 정책이 생성된 build script를 차단해 실행 불가
- GitHub Actions STEP 10 Windows build run `30096485441` 성공
  - Windows Rust workspace test 성공
  - Windows 실행 파일 빌드 및 artifact 업로드 성공
- GitHub Actions STEP 16 security run `30096488983` 성공
- Windows artifact `tm-step10-windows-x64` manifest 대조 성공
  - `tm.exe`: `d424926a7603b2d8e9be3b166ee30384aebfe03ec6d274165ceed5530969069e`
  - `tm-cli.exe`: `0bcb0832cbc20c2827d911ee6237de10d6b94f1af6f178a04670b0768e5c24c7`

## 복구

소스는 `backups/source/tm-source-pre-change-20260724T131532Z-633e899.zip`으로 복구할 수 있다. 데이터 schema 변경이 없으며 필요하면 구현 전 Railway 원격 백업을 사용한다.

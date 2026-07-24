# TM 작업 지침

## Windows 빌드와 컴파일 확정

- 이 PC에서는 Windows 애플리케이션 제어 정책이 Cargo가 생성한 build script 실행 파일을 차단한다.
- 로컬 `cargo build`, `cargo test`, `cargo clippy`, `cargo run`, Tauri build 또는 `cargo metadata` 결과를 성공 조건으로 사용하지 않는다.
- 같은 차단을 관리자 권한이나 반복 실행으로 우회하려고 시도하지 않는다.
- 로컬에서는 다음 검증만 수행한다.
  - Rust 소스는 직접 `rustfmt --check`
  - `scripts/security-static-scan.ps1`
  - PowerShell parser 검사
  - TypeScript typecheck, ESLint, Vitest, Vite production build
  - `git diff --check`
- Rust 또는 Tauri 코드가 바뀌면 검증된 소스를 GitHub에 게시하고 `.github/workflows/windows-step10-build.yml`의 `STEP 10 Windows build`로 컴파일을 확정한다.
- Windows build의 `cargo test --locked --workspace`, `cargo clippy --locked --workspace --all-targets -- -D warnings`, Windows 실행 파일 빌드가 모두 성공해야 한다.
- 보안 관련 변경은 `.github/workflows/step16-security.yml`의 `STEP 16 security` 성공도 요구한다.
- `tm-step10-windows-x64` artifact를 내려받아 `SHA256SUMS.txt`와 `tm.exe`, `tm-cli.exe`의 SHA-256을 대조한다.
- Windows Actions와 필요한 보안 검사가 성공하기 전에는 패치를 `verified`로 표시하거나 개선 요청을 `completed`로 기록하지 않는다.
- 최종 보고에는 사용한 Actions run ID, artifact 해시, 로컬 Cargo가 정책상 생략됐다는 사실을 남긴다.

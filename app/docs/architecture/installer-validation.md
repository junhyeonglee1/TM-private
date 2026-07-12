# NSIS 설치 수명주기 검증

검증일: `2026-07-11` (`Asia/Seoul` 작업 환경)

## 빌드 경계

- 기존 `%LOCALAPPDATA%\tauri\NSIS` cache `442`개 파일, `7,168,591` bytes를 `TM\dist\build\cargo\.tauri\NSIS`로 복사했다.
- 원본과 복사본의 `makensis.exe` SHA-256 일치를 확인했다.
- Cargo와 pnpm은 offline 환경으로 실행했고 WebView2 install mode는 `skip`이었다.
- target은 `x86_64-pc-windows-msvc`, 설치 범위는 `currentUser`, downgrade는 차단했다.
- `tm-cli.exe`를 Tauri external binary로 NSIS에 포함했다.

첫 sandbox 실행은 `esbuild` 자식 프로세스의 `EPERM`으로 bundle 전에 실패했다. 같은 offline 명령을 허용된 로컬 프로세스로 다시 실행해 성공했으며 추가 다운로드는 없었다.

## 산출물

| 파일 | 크기 | SHA-256 | 비고 |
|---|---:|---|---|
| `tm.exe` | `13,553,664` | `576EC28DEF763DCF8926D928F414B6D529033070892561C6B0BBCF42650EF31D` | raw/bundle metadata 적용 x64 EXE |
| `tm-cli.exe` | `4,466,688` | `BF7EDF11527FD1A0FAC69DFDB250CE95636F9164DD169DBE86F069CC6AE85229` | x64 sidecar |
| `TM_0.1.0_x64-setup.exe` | `3,621,829` | `6A54516A6EFCC270414FAC6164BF09DF643117E29193FA7CCCD21F5C8EF515A1` | x64 payload를 포함한 NSIS installer |

NSIS bootstrap executable 자체의 PE machine은 `0x014C`이고, 설치된 앱과 CLI payload는 `0x8664`다. 세 파일은 Authenticode 서명이 없는 개발 산출물이다.

## 외부 데이터 기준선

- 실제 TM schema `1`, WAL, `integrity_check=ok` DB를 만들었다.
- `tm:evening:2026-07-11` digest를 `sent`로 확정하고 Slack 참조 대신 로컬 sentinel 값을 기록했다.
- `data/attachments`, `data/logs`, `backups/database`, `backups/source`, `exports`에 sentinel을 배치했다.
- DB, backup, lock/WAL 보조 파일과 sentinel을 포함한 파일 수는 `24`개였다.
- 상대 경로·크기·개별 SHA-256을 정렬해 만든 aggregate SHA-256은 `7ED610152D8AC3969DFD7C8FBD7C6AC7C770857F5CE8F2ADF80E24F00664C1F2`였다.

## 신규 설치

- `TM_0.1.0_x64-setup.exe /S`: exit `0`
- 설치 경로: `%LOCALAPPDATA%\TM`
- 설치 파일: `tm.exe`, `tm-cli.exe`, `uninstall.exe`
- HKCU 제거 항목: `TM 0.1.0`, `1`개
- 설치된 CLI health: `ok`, WAL, schema `1`, integrity `ok`
- digest `sent`와 sentinel 참조 유지
- 외부 파일 `24`개와 aggregate SHA-256이 기준선과 일치
- WebView2 Runtime: `150.0.4078.65`, 변경 없음

## 동일 버전 재설치

- installer exit `0`
- 설치 파일 해시와 HKCU 제거 항목 유지
- 설치된 CLI health와 digest 상태 유지
- 외부 파일 수와 aggregate SHA-256이 기준선과 일치

## 제거

- `uninstall.exe /S`: exit `0`
- `%LOCALAPPDATA%\TM`: 제거됨
- HKCU 제거 항목: `0`개
- 외부 DB: WAL, schema `1`, integrity `ok`
- digest `sent` 상태 유지
- 외부 파일 수와 aggregate SHA-256이 기준선과 일치
- WebView2 Runtime 버전 변경 없음

## 검증 데이터 정리

검증용 외부 파일 `24`개는 삭제하지 않고 `TM\dist\test-runs\nsis-lifecycle-20260711-final\external-root`로 이동했다. 이동 후 aggregate SHA-256이 다시 기준선과 일치했고 실제 `TM\data`, `TM\backups`, `TM\exports`의 파일 수는 `0`이다.

실제 이전 버전 installer가 없으므로 downgrade 거부의 동적 검증은 수행하지 않았고, `bundle.windows.allowDowngrades=false` 설정과 동일 버전 재설치를 검증했다.

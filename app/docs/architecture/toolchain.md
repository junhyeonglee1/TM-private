# 개발 도구와 의존성 경계

## 읽기 전용 점검 결과

| 항목 | 확인 버전 | 사용 위치 | 전역 설치/영향 |
|---|---|---|---|
| Rust / Cargo | `1.97.0`, `x86_64-pc-windows-msvc` | 기존 사용자 toolchain | 기존 도구, 재설치 없음 |
| rustfmt / Clippy | `1.9.0-stable` / `0.1.97` | 기존 Rust component | 기존 도구, 재설치 없음 |
| Node.js | `24.14.0` | Codex bundled runtime | 시스템 PATH 변경 없음 |
| pnpm | `11.7.0` | Codex bundled wrapper | 전역 설치 없음 |
| Visual Studio Build Tools | `2022 17.14.37411.7` | 기존 C++ Build Tools | 기존 설치, 변경 없음 |
| MSVC / Windows SDK | `14.44.35207` / `10.0.26100.0` | 기존 VS toolchain | 기존 설치, 변경 없음 |
| WebView2 Runtime | `150.0.4078.65` | 기존 Windows Runtime | 변경 없음; NSIS는 `skip` |
| Tauri CLI | `2.11.4` | `app/node_modules` link, 실제 store는 `TM/dist/cache` | project-local, 전역 설치 없음 |

## 1차 승인으로 준비한 항목

| 항목 | 용도 | 저장 위치 | 시스템 전역 여부 | 예상 영향 | 대체 가능성 |
|---|---|---|---|---|---|
| pnpm 패키지 `333`개 | React/Tauri UI·lint·test·build | `TM/dist/cache/pnpm-*`, `app/node_modules` link | 아니오 | 디스크 cache와 lockfile 생성 | 기존 cache 없이는 불가 |
| Cargo crate `522`개 | Rust core·CLI·Tauri compile | `TM/dist/cache/cargo-home` | 아니오 | 디스크 cache와 `Cargo.lock` 생성 | 기존 cache가 있으면 offline 가능 |
| Tauri CLI `2.11.4` | icon/config/raw app build 및 추후 NSIS orchestration | pnpm project-local store | 아니오 | 시스템 설정 변경 없음 | Cargo CLI 방식도 가능하나 미사용 |

최종 검증과 raw release 빌드는 `--locked --offline`으로 실행했으며 추가 다운로드가 없었다. 검증 스크립트는 `pnpm install`을 포함하지 않는다.

## 2차 승인 결과와 외부 작업 대기

| 항목 | 현재 상태 | 영향·결과 |
|---|---|---|
| NSIS 도구 cache | 기존 cache `442`개 파일을 TM 내부로 복사 | 네트워크 다운로드 없음, `makensis.exe` 해시 일치 |
| NSIS installer | `TM_0.1.0_x64-setup.exe` 생성 | `currentUser`, WebView2 `skip`, CLI sidecar 포함 |
| 설치·재설치·제거 검증 | 완료 | 각 exit `0`, 외부 데이터 aggregate 해시 유지, 설치 흔적 제거 확인 |
| Slack/Codex 예약 | 미연결·미등록 | 외부 서비스 연결·메시지 전송·예약 생성 |

영구 Windows 시스템 설정 변경, Git 초기화, 이전 작업 경로 접근은 수행하지 않았다. 검증 중 생성한 per-user 설치 디렉터리와 HKCU 제거 항목은 제거 완료했다.

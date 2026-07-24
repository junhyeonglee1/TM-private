# 0007 — 클라우드 개선 요청 처리 연결

## 목적

STEP 10 cloud cutover 이후 Railway가 유일한 데이터 기준 원본이 되었지만 기존 `tm-cli changes`는 로컬 DB만 열었다. Windows TM에서 승인한 개선 요청을 Codex가 안전하게 claim하고 처리 결과를 같은 운영 DB에 기록할 수 있도록 클라우드 처리 경계를 추가한다.

## 변경

- `tm-core` desktop command allowlist에 조회·claim·완료·실패 command를 추가했다.
- claim은 기존 SQLite `IMMEDIATE` transaction과 조건부 UPDATE를 그대로 사용한다.
- 완료·실패는 요청 ID, claim key, worker ID가 모두 일치해야 한다.
- 모든 변경 command는 단일 사용자 bearer token과 정확한 confirmation header를 요구한다.
- 모바일 device cookie allowlist에는 새 command를 추가하지 않았다.
- `scripts/invoke-change-request-cloud.ps1`은 기존 cloud configuration과 Windows Credential Locker를 사용하며 토큰을 출력하거나 파일로 저장하지 않는다.
- 처리 프롬프트와 아키텍처 문서를 Railway 기준으로 갱신했다.

## 검증

- Rust unit/integration tests: command 직렬화, 단일 claim, claim identity, confirmation header, 모바일 차단
- PowerShell parser validation
- security static scan, workspace test, Clippy, frontend regression checks
- Railway production 배포 후 `Status` read-only 확인

## 롤백

이 패치를 되돌리고 Railway를 직전 성공 deployment로 재배포한다. 이미 `claimed`인 요청은 자동으로 되돌리지 않으며 TM 개선 요청함에서 `처리 유실 기록`으로 실패 전환한 뒤 사용자가 다시 승인한다.

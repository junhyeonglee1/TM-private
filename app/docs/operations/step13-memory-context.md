# STEP 13 기억·검색 운영 절차

이 절차는 Railway production에서 schema 7과 STEP 13 안전 계약을 확인한다. 실제 기억을 만들거나 OpenAI를 호출하지 않는다.

## 배포 전

1. Windows CI에서 format, clippy, 전체 Rust test, 프런트엔드 검증과 release artifact 생성을 통과시킨다.
2. 격리 SQLite에서 schema 1~6 데이터와 STEP 12 승인 원장이 schema 7 migration 뒤에도 유지되는지 확인한다.
3. `memory_integration` 테스트로 명시적 승인, 민감도 차단, 검색 예산, 요약·retention, 원본 삭제, restore 보호를 확인한다.
4. production token과 OpenAI API key는 소스·로그·결과 파일에 포함하지 않는다.

## 배포

Railway singleton service에 테스트를 통과한 소스를 배포한다. 기존 `/var/lib/tm` Volume을 유지하고 replica와 다중 region을 켜지 않는다. schema migration은 서버 기동 transaction 안에서 실행되며 readiness가 성공하기 전에는 정상 배포로 간주하지 않는다.

## 비과금 읽기 전용 검증

Windows Credential Locker에 저장된 production token을 사용해 다음을 실행한다.

```powershell
Set-Location 'C:\Users\tkfk0\Desktop\codex\TM\app'
& '.\scripts\verify-step13-production.ps1'
```

스크립트는 다음 항목만 확인한다.

- 인증 상태와 DB schema 7 readiness
- prompt `step13-v1`, 자동 read tool 8개
- 명시적 기억만 저장, 모든 변경 승인 필수
- OpenAI 전달 범위 `normal_and_explicitly_allowed_only`
- SQLite FTS5 + 구조화 filter, vector service 미사용
- 컨텍스트 절대 상한 12개·6,144 bytes
- memory 목록 조회와 임의의 불일치 검색을 1개·256 bytes로 제한

결과는 기본적으로 `dist/manual-step13-production-verification/result.json`에 기록한다. token 원문, memory 본문, OpenAI key는 기록하지 않는다. 결과의 `billableAiCallPerformed`와 `productionMemoryMutationPerformed`는 모두 `false`여야 한다.

## 실패 시

- migration/readiness 실패: 새 배포를 승격하지 않고 Railway 배포 로그와 schema backup 상태를 확인한다.
- 계약 불일치: 실제 기억 저장이나 AI 호출을 하지 말고 배포 image와 환경변수만 확인한다.
- 인증 실패: token을 출력하거나 파일에 복사하지 말고 Windows Credential Locker 등록 상태와 Railway token hash·만료만 확인한다.
- rollback이 필요하면 schema 7 이후에 생성된 기억·출처·이벤트 원장을 포함한 최신 backup만 사용한다. 오래된 backup은 원장 되감기 보호로 거부될 수 있다.

정기 summary regeneration과 retention maintenance의 자동 스케줄 연결은 STEP 14에서 수행한다.

## Production 검증 기록

- 2026-07-22 Railway production deployment `4b850b1b-fa4a-4e68-8c9f-8b578af5c098`가 `SUCCESS`로 완료됐다.
- GitHub Actions Windows CI #13은 각 Rust 검증 명령의 fail-fast를 적용한 상태에서 workspace test·clippy, `tm.exe`·`tm-cli.exe` release artifact 생성을 통과했다.
- Docker build에서 workspace 전체 test와 clippy, release `tm-server` build가 통과했다.
- `/readyz`와 인증된 운영 상태에서 schema 7, prompt `step13-v1`, 자동 read tool 8개를 확인했다.
- memory는 명시적 저장만 허용하고 모든 변경에 승인이 필요하며 OpenAI 전달은 `normal_and_explicitly_allowed_only`로 제한됨을 확인했다.
- SQLite FTS5와 구조화 filter, vector service 미사용, 컨텍스트 상한 12개·6,144 bytes를 확인했다.
- memory 목록과 1개·256 bytes 불일치 검색만 실행했다. OpenAI 호출과 production memory mutation은 0회였다.
- production은 singleton 1개, `/var/lib/tm` Volume `READY`, restart policy `Always`, multi-region 비활성 상태를 유지했다.

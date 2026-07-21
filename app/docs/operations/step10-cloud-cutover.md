# STEP 10 데스크톱 cloud mode와 일회성 cutover

## 현재 상태

실데이터 전송과 기준 원본 전환 전의 구현·검증 단계다. 로컬 TM DB와 Railway production DB는 아직 서로 독립적이며, 현재 기준 원본은 로컬 DB다.

2026-07-21 read-only inventory:

| 항목 | 결과 |
| --- | --- |
| 로컬 DB | `C:\Users\tkfk0\Desktop\codex\TM\data\tm.sqlite3` |
| schema | 3; 실제 cutover 전 검증된 앱으로 schema 5 migration 필요 |
| 무결성 | SQLite integrity `ok`, foreign key 위반 0 |
| 사용자 데이터 | Project 2, Task 2, Task event 5 |
| 그 외 이전 대상 | Checklist·Tag·일정·Session·WorkLog·Note·Change Request 모두 0 |
| 첨부파일 | 0개, 0 byte |
| production | schema 5, encrypted backup 및 restore drill 통과; 아직 사용자 실데이터를 넣지 않음 |

inventory에는 제목·본문·파일명 같은 원문을 기록하지 않는다.

## 사용자 결정 게이트

Codex 권장 기본안:

1. 로컬 Project 2개, Task 2개, Task event 5개를 모두 이전한다.
2. cloud 검증이 모두 끝난 시점부터 Railway DB를 유일한 기준 원본으로 전환한다.
3. 30분 maintenance window를 잡고 이 시간에는 TM 쓰기를 중지한다.
4. 현재 Note·WorkLog·첨부파일이 0개이므로 별도 제외 데이터 없이 전체 이전한다.
5. 기존 로컬 DB와 최종 snapshot은 90일 동안 read-only rollback archive로 보존한다.

이 다섯 항목의 사용자 승인 전에는 로컬 DB migration, production maintenance mode, 파일 업로드, DB 교체를 실행하지 않는다.

2026-07-21 사용자 승인: 전체 이전, 검증 후 cloud 단일 원본, 최대 30분 maintenance, 제외 없음, local archive 90일 보존.

## 준비된 비노출 검증 도구

`tm-cli migration`은 row 원문을 출력하지 않고 schema, migration ledger, 테이블별 row count·SHA-256과 전체 logical SHA-256만 다룬다.

```powershell
cargo run --locked --offline -p tm-cli -- migration manifest --json
cargo run --locked --offline -p tm-cli -- migration dry-run --json
cargo run --locked --offline -p tm-cli -- migration inspect --path '<snapshot.sqlite3>' --json
```

현재 Windows Application Control 정책이 이 세션의 `cargo.exe` 실행을 차단하고 있다. 정책을 우회하지 않으며, `scripts/verify-step10-cli.ps1`의 format·test·clippy가 통과하기 전에는 도구를 cutover에 사용하지 않는다.

Railway staging의 Rust 1.97 독립 환경에서 migration CLI format·test·clippy와 server release build를 통과했다. 이후 Docker 검증 범위를 Tauri 데스크톱을 포함한 workspace 전체 test·clippy로 확대했다.

## 구현된 실행 경계

- 명시적 local/cloud Tauri transport와 cloud mode local DB 격리
- Windows Credential Locker token 저장; 설정 파일에는 mode와 HTTPS origin만 저장
- 현재 데스크톱 command allowlist와 strict JSON 계약
- write별 정확한 확인 header와 자동 재시도 금지
- cloud 전체 내보내기의 로컬 파일 materialization
- 정상 router와 분리된 maintenance-only import route
- import 전 schema·migration ledger·integrity·foreign key·logical checksum 검사
- import 직전 backup, 활성화 후 manifest 재검사, 불일치 자동 rollback
- schema 3 보관→schema 5 snapshot→production TLS upload를 수행하는 운영 스크립트

## 실행 순서

1. TM 데스크톱을 종료하고 local write freeze 시각을 기록한다.
2. 현재 schema 3 DB의 pre-migration backup을 만든 뒤 검증된 binary로 schema 5까지 migration한다.
3. schema 5 local manifest와 일관 snapshot을 만들고 서로 logical match인지 확인한다.
4. production을 maintenance mode로 전환하고 직전 encrypted backup을 성공시킨다.
5. local snapshot을 별도 임시 경로로 업로드하고 cloud에서 read-only preflight를 수행한다.
6. source/cloud manifest의 schema, migration ledger, 모든 테이블 row count·checksum, 전체 logical checksum을 비교한다.
7. 일치할 때만 활성 DB 파일을 원자적으로 교체하고 production을 재시작한다.
8. 인증된 read API와 현재 데스크톱 기능의 회귀 검증을 수행한다.
9. 모든 검증 후 cloud write를 열고 cloud를 유일한 기준 원본으로 선언한다.
10. staging은 계속 중지 상태로 두고 local DB는 read-only archive로 보존한다.

## 즉시 중단 조건

- schema 5 migration 또는 pre-migration backup 실패
- SQLite integrity·foreign key 검사 실패
- source/cloud manifest 불일치
- production backup 또는 restore 상태 실패
- 인증·HTTPS·write precondition 회귀 실패
- local/cloud 양쪽에서 write가 가능한 상태 발생

어느 하나라도 발생하면 cloud write를 열지 않고 기존 local 기준 원본으로 복귀한다.

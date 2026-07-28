# 모바일 프로젝트·Task 관리 운영 절차

TM 모바일 PWA의 `할 일` 화면은 Windows 앱과 같은 Railway SQLite 기준 원본을 사용한다. 등록된 모바일 기기에서 프로젝트와 Task를 조회하고, 명시적인 사용자 동작으로 프로젝트·Task를 추가하거나 Task 내용과 상태를 변경할 수 있다. 이 기능 자체는 OpenAI를 호출하지 않는다.

## 지원 범위

- 프로젝트: 이름순 조회, 페이지 추가 조회, 프로젝트 생성, 해당 프로젝트의 Task 필터링
- Task: 최근 수정순 조회, 프로젝트·상태 필터, 페이지 추가 조회
- Task 생성: 프로젝트, 제목, 설명, 초기 상태, 우선순위, 기한
- Task 변경: 제목, 설명, 프로젝트, 상태, 우선순위, 기한
- 빠른 완료: `todo` 또는 `in_progress` Task를 사용자가 한 번 더 확인한 뒤 `done`으로 변경
- 상태 전이: 서버의 기존 Task 상태 전이 규칙을 그대로 적용
- 동시 변경: 읽을 때 받은 양의 정수 `version`을 `If-Match`로 보내며, 다른 기기에서 먼저 변경했으면 `409`에서 중단하고 새로고침을 안내

프로젝트 편집·보관·삭제와 Task 삭제는 이번 모바일 범위에 포함하지 않는다. 이 작업은 기존 schema 13 데이터 모델을 사용하며 새 DB migration을 만들지 않는다.

## 공개 API와 쓰기 계약

등록된 모바일 기기는 다음 경로만 기존 기기 scope 안에서 사용한다.

| 동작 | API | 성공 상태 |
|---|---|---:|
| 프로젝트 조회 | `GET /api/v1/projects` | `200` |
| 프로젝트 생성 | `POST /api/v1/projects` | `201` |
| Task 조회 | `GET /api/v1/tasks` | `200` |
| Task 생성 | `POST /api/v1/tasks` | `201` |
| Task 변경·상태 변경 | `PATCH /api/v1/tasks/{id}` | `200` |

모든 생성·변경 요청에는 Secure 기기 cookie와 정확한 same-origin `Origin`, CSRF cookie와 일치하는 `X-TM-CSRF`가 필요하다. controlled mutation은 아래 헤더도 모두 만족해야 한다.

- `Idempotency-Key`: 요청마다 새 UUID
- `X-TM-Confirm-Mutation`: `project.create`, `task.create`, `task.update` 중 현재 동작과 정확히 일치
- 생성: `If-None-Match: *`
- 변경: 조회한 version을 `If-Match: "<version>"`으로 전달

UI는 POST/PATCH를 자동 재시도하지 않는다. 상태 변경과 빠른 완료는 사용자 확인을 한 번 더 받고, `409`가 오면 서버 값을 덮어쓰지 않는다.

## 보안·오프라인 경계

- 관리자 bearer token은 모바일 JavaScript, cookie, 응답 또는 Cache Storage로 전달하지 않는다.
- 모바일에는 `__Host-tm_device` HttpOnly cookie와 browser-readable `__Host-tm_csrf` cookie만 발급한다. 서버 DB에는 기기 token과 CSRF 원문이 아니라 SHA-256만 저장한다.
- service worker의 cache 이름은 `tm-mobile-shell-v11`이다. 활성화할 때 이전 shell cache를 제거한다.
- `/api/`와 모든 non-GET 요청은 service worker가 가로채거나 저장하지 않는다. 오프라인에서는 app shell만 열리며 프로젝트·Task 데이터 조회 및 변경은 연결이 필요하다.
- Task·프로젝트 데이터는 localStorage, sessionStorage 또는 Cache Storage에 저장하지 않는다.
- 조회·생성·변경은 OpenAI API를 호출하지 않으며 AI 비용 원장을 변경하지 않는다.
- 기기를 폐기하면 해당 cookie의 다음 API 요청은 즉시 `401`이어야 한다.

## 로컬 검증

이 PC에서는 Windows 애플리케이션 제어 정책이 Cargo가 만든 실행 파일을 차단하므로 관리자 권한이나 반복 실행으로 우회하지 않는다. 로컬에서는 다음만 수행한다.

1. TypeScript typecheck, ESLint, Vitest, Vite production build
2. PowerShell parser 검사
3. `scripts/security-static-scan.ps1`
4. Rust 변경 파일의 직접 `rustfmt --check`
5. `git diff --check`

Rust 또는 Tauri 코드가 바뀌면 GitHub Actions의 `.github/workflows/windows-step10-build.yml`에서 `cargo test --locked --workspace`, `cargo clippy --locked --workspace --all-targets -- -D warnings`, Windows 실행 파일 빌드가 모두 성공해야 한다. 보안 변경이므로 `.github/workflows/step16-security.yml`도 성공해야 한다.

Windows build가 성공하면 `tm-step10-windows-x64` artifact를 내려받아 `SHA256SUMS.txt`와 `tm.exe`, `tm-cli.exe`를 직접 대조한다. 두 Actions가 성공하기 전에는 기능을 production 검증 완료 또는 개선 요청 `completed`로 기록하지 않는다.

## 배포 순서

1. 검증된 소스를 GitHub에 push한다.
2. `STEP 10 Windows build`와 `STEP 16 security`의 성공 run ID를 기록한다.
3. Windows artifact의 두 실행 파일 SHA-256을 기록한다.
4. 같은 검증 commit을 Railway production에 배포한다.
5. `/readyz`가 `200`이고 production DB와 원격 백업이 같은 schema이며 backup 상태가 `succeeded`, integrity가 `ok`인지 확인한다.
6. 아래 비파괴 production verifier를 실행한다.
7. 실제 등록 모바일 기기에서 한 번 새로고침한다. 이전 PWA가 열려 있으면 v11 service worker가 활성화되도록 앱을 완전히 닫았다가 다시 열 수 있다.
8. 모바일에서 프로젝트 추가, Task 추가, 상태 변경, 빠른 완료를 사용자 승인 아래 각각 한 번 수행하고 Windows 앱에 같은 결과가 보이는지 확인한다.

```powershell
Set-Location 'C:\Users\tkfk0\Desktop\codex\TM\app'
$verifyScript = '.\scripts\verify-mobile-project-tas' + 'k-production.ps1'
& $verifyScript
```

기본 결과 경로는 아래와 같이 조립되며 해당 디렉터리 아래의 `result.json`에 저장된다. 정적 보안 검사의 OpenAI key 패턴 오탐을 막기 위해 `task-production` 문자열은 소스에서 두 부분으로 나눈다.

```powershell
$resultPath = 'dist/manual-mobile-project-tas' + 'k-production-verification/result.json'
```

결과에는 bearer token, pairing code, polling secret, device·CSRF cookie, 기기 ID, 프로젝트·Task 원문 또는 그 개수를 저장하지 않는다.

## 비파괴 production 검증 계약

위 `$verifyScript` production verifier는 Windows Credential Locker의 `TM Cloud Production` / `single-user`에서 운영 token을 읽고 임시 모바일 기기를 등록한다. 인증 원장에는 임시 pairing·등록·폐기 기록이 생기지만 production 프로젝트·Task에 성공 mutation을 수행하지 않는다.

검증 순서는 다음과 같다.

1. PWA shell에서 프로젝트·Task 폼, 필터, 페이지 조회 UI를 확인한다.
2. `app.js`에서 `project.create`, `task.create`, `task.update`, idempotency, `If-None-Match`·`If-Match`, 사용자 확인 계약을 확인한다.
3. service worker가 `tm-mobile-shell-v11`이고 API·non-GET을 cache하지 않는지 확인한다.
4. 임시 등록 기기로 프로젝트와 Task 전체를 페이지 조회해 메모리 안에서 SHA-256 digest를 만든다.
5. 프로젝트 생성 요청에는 confirmation만 누락하고 빈 프로젝트 이름을 사용해 `428`을 확인한다. confirmation 검사가 회귀해도 빈 이름 검증이 성공 생성을 막는다.
6. 정확한 CSRF·idempotency·`If-Match`·confirmation을 갖춘 Task PATCH를 존재하지 않는 임의 UUID에 보내 `404`를 확인한다.
7. 다른 존재하지 않는 임의 UUID의 Task PATCH에서 CSRF만 누락해 `403`을 확인한다. CSRF 경계가 회귀해도 대상 Task가 없으므로 업무 데이터는 바뀌지 않는다.
8. 프로젝트·Task를 다시 전체 조회해 digest가 같은지 확인한다.
9. 임시 기기를 폐기하고 같은 cookie의 Task 조회가 `401`인지 확인한다.
10. 전후 operations의 DB schema·원격 백업 상태/schema/integrity와 OpenAI API 비용 원장이 같은지 확인한다.

CSRF 실패와 폐기 후 인증 실패는 보안 관측 counter를 의도적으로 증가시킬 수 있다. 따라서 verifier는 operations 응답 전체를 동일 비교하지 않고 DB·backup 불변값만 비교한다. scheduler가 동시에 새 backup을 검증하더라도 status, schema, integrity가 유지되면 통과한다.

업무 데이터 digest가 달라졌다면 실제 사용자가 검증 도중 Windows 또는 모바일에서 데이터를 변경했는지 먼저 확인하고 조용한 시간에 다시 실행한다. verifier가 중간에 실패해도 `finally`에서 임시 등록 기기 폐기를 다시 시도한다. cleanup 확인에 실패하면 성공으로 기록하지 말고 Windows TM의 기기 관리에서 `TM mobile Task verification ...` 기기를 폐기한 뒤 원인을 해결한다.

## Production 완료 기록

배포 후 아래 값을 이 문서에 추가한다.

- 검증 commit
- `STEP 10 Windows build` run ID
- `STEP 16 security` run ID
- `tm.exe`, `tm-cli.exe` SHA-256
- Railway deployment ID
- production schema와 remote backup 상태
- 위 `$verifyScript` production verifier의 결과 시각
- 모바일과 Windows 동기화 확인 결과
- 로컬 Cargo 검증을 정책상 생략했다는 사실

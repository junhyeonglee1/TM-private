# schema 16 지출 AI 분류 배포·검증

schema 16은 기존 schema 15 지출 원장을 유지하면서 AI 분류 이력과 분류 provenance를 추가한다. 운영 전환은 반드시 기능이 꺼진 migration 배포와, 별도 승인을 받은 활성화 배포로 나눈다.

## 안전 조건

1. 공개 저장소에는 실제 거래 파일, 암호, 운영 토큰, OpenAI 키 또는 지출 암호화 키를 넣지 않는다.
2. 로컬 검증과 `STEP 10 Windows build`, `STEP 16 security`가 같은 exact commit에서 성공해야 한다.
3. STEP 10 artifact의 `tm.exe`, `tm-cli.exe`, `tm-office-decryptor.exe`를 `SHA256SUMS.txt`와 대조한다.
4. 1단계 전에 production은 schema 15이고 최신 원격 백업의 integrity와 schema semantics가 유효해야 한다.
5. 1단계와 2단계 모두 같은 clean commit의 exact source archive만 배포한다.
6. 기존 지출 해설 switch `TM_EXPENSE_AI_ENABLED`는 두 단계 모두 `false`로 유지한다.
7. 배포 verifier는 GET 요청만 사용하며 비밀값을 조회하거나 출력하지 않는다.

이 PC에서는 Windows 애플리케이션 제어 정책 때문에 Cargo가 만든 실행 파일을 로컬 성공 조건으로 사용하지 않는다. 관리자 권한으로 우회하지 않는다. 로컬에서는 rustfmt, TypeScript 검사, PowerShell parser, 정적 보안 검사와 `git diff --check`만 실행하고 Rust/Tauri 컴파일은 GitHub Actions에서 확정한다.

## 사전 검증 순서

```powershell
Set-Location 'C:\Users\tkfk0\Desktop\codex\TM'

# 로컬 허용 검사
& '.\app\scripts\security-static-scan.ps1'
git diff --check

# GitHub Actions 성공 후 exact SHA와 run ID를 기록한다.
gh run view <STEP10_RUN_ID> --repo junhyeonglee1/TM-private
gh run view <STEP16_RUN_ID> --repo junhyeonglee1/TM-private
```

Actions 증거에는 workflow 이름·경로, branch, exact commit, conclusion, artifact 이름과 각 파일 SHA-256이 모두 포함되어야 한다.

## 1단계: migration 배포, 분류 비활성

먼저 dry run을 수행한다.

```powershell
& '.\app\scripts\deploy-verified-expense-classification-railway.ps1' `
  -Phase Phase1 `
  -ExpectedHeadSha '<40자리_COMMIT_SHA>' `
  -Step10RunId <STEP10_RUN_ID> `
  -Step16RunId <STEP16_RUN_ID>
```

dry run이 통과하면 실제 1단계를 실행한다.

```powershell
& '.\app\scripts\deploy-verified-expense-classification-railway.ps1' `
  -Phase Phase1 `
  -ExpectedHeadSha '<40자리_COMMIT_SHA>' `
  -Step10RunId <STEP10_RUN_ID> `
  -Step16RunId <STEP16_RUN_ID> `
  -Apply
```

스크립트는 다음을 수행한 뒤 반드시 중단한다.

- `TM_EXPENSE_CLASSIFICATION_AI_ENABLED=false`와 `TM_EXPENSE_AI_ENABLED=false`를 `--skip-deploys`로 고정
- exact source 배포
- schema 16, schema 15 pre-migration backup, schema 16 원격 백업 검증
- classification claimed/staged가 0이고 기능이 꺼져 있는지 검증
- DPAPI로 보호된 1단계 receipt와 단일 사용 승인 상태 저장

1단계가 끝났다고 해서 2단계를 자동으로 실행하지 않는다. receipt를 검토하고 사용자에게 별도 활성화 승인을 받아야 한다.

## 2단계: 별도 승인 후 분류 활성화

승인을 받은 뒤 먼저 dry run으로 1단계 receipt, 동일 source, 현재 비활성 상태를 다시 검증한다.

```powershell
& '.\app\scripts\deploy-verified-expense-classification-railway.ps1' `
  -Phase Phase2 `
  -ExpectedHeadSha '<40자리_COMMIT_SHA>' `
  -Step10RunId <STEP10_RUN_ID> `
  -Step16RunId <STEP16_RUN_ID> `
  -ApproveActivation
```

통과 후 실제 활성화를 실행한다.

```powershell
& '.\app\scripts\deploy-verified-expense-classification-railway.ps1' `
  -Phase Phase2 `
  -ExpectedHeadSha '<40자리_COMMIT_SHA>' `
  -Step10RunId <STEP10_RUN_ID> `
  -Step16RunId <STEP16_RUN_ID> `
  -ApproveActivation `
  -Apply
```

2단계는 1단계 receipt와 exact source를 다시 확인한 뒤 분류 switch만 켜고 같은 source를 재배포한다. 최종 verifier가 schema 16, 최신 원격 백업, `expenseClassificationAiEnabled=true`, `expenseAiEnabled=false`, 열린 classification 작업 0을 확인해야 성공이다. receipt는 한 번만 소비할 수 있다.

## 실패 시 fail-closed 처리

2단계에서 활성화 변수를 쓰기 시작한 뒤 오류가 발생하면 스크립트는 분류 switch를 즉시 `false`로 되돌린다. 활성화 source upload를 시도했다면 성공 여부가 불확실해도 exact source를 비활성 상태로 다시 배포하고 verifier가 `false`를 확인해야 rollback 성공으로 기록한다. 운영 DB를 수동 수정하거나 검증되지 않은 이전 이미지를 강제 배포하지 않는다.

분류 품질, 비용, 개인정보 필터 또는 OpenAI 응답에 이상이 있으면 `TM_EXPENSE_CLASSIFICATION_AI_ENABLED=false`로 분류 호출만 차단한다. 규칙 기반 분류, 거래 조회·수정, 월간 결정론적 요약과 정기지출은 계속 사용할 수 있다. DB·백업·암호화 상태가 비정상이면 기존 incident 절차에 따라 지출 mutation과 AI 호출을 모두 차단한다.

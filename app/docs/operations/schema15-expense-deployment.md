# schema 15 지출 기능 배포·검증

## 성공 조건

배포 완료는 다음을 모두 만족할 때만 선언한다.

1. 로컬 허용 검증(TypeScript, ESLint, Vitest, Vite, 직접 rustfmt, PowerShell parser, 보안 정적 검사, `git diff --check`)이 통과한다.
2. GitHub Actions `STEP 10 Windows build`의 workspace test, Clippy, Windows build가 통과한다.
3. GitHub Actions `STEP 16 security`의 secret/RustSec/Trivy/sidecar audit가 통과하고 `tm-step16-container-provenance` artifact가 검증 commit을 가리킨다.
4. `tm-step10-windows-x64`의 `tm.exe`, `tm-cli.exe`, `tm-office-decryptor.exe`가 `SHA256SUMS.txt`와 일치한다.
5. Railway의 exact deployment ID·production image digest receipt가 검증되고 schema 15, expense crypto readiness와 최신 원격 backup이 정상이다.
6. Windows와 모바일의 읽기 API, 정기지출 CRUD, 캘린더 가상 occurrence, Today 카드가 동기화된다.

이 PC에서는 Windows Code Integrity 정책 때문에 Cargo가 만든 build script 실행 파일이 차단된다. 로컬 Cargo build/test/clippy/run/metadata 또는 Tauri build를 재시도하거나 관리자 권한으로 우회하지 않는다. Rust 확정은 GitHub Actions만 사용한다.

## 배포 순서

### 1. 소스와 lockfile

- 실제 금융 파일 폴더가 `.gitignore`에 포함됐는지 확인한다.
- test fixture는 개인정보가 제거된 합성 파일만 사용한다.
- 새 Rust dependency가 `Cargo.lock`에 반영됐는지 확인한다.
- secret, 원본 파일명, 로컬 경로, 파일 암호가 diff에 없는지 정적 검사한다.

### 2. GitHub Actions와 container provenance

검증된 branch를 push하면 두 workflow가 시작된다. 둘 다 성공하기 전에는 개선 요청을 completed로 표시하지 않는다. 성공한 Windows artifact를 내려받아 다음 세 해시를 대조한다.

```powershell
Get-FileHash -Algorithm SHA256 .\tm.exe, .\tm-cli.exe, .\tm-office-decryptor.exe
Get-Content .\SHA256SUMS.txt
```

로컬 release 폴더에는 canonical 저장소·branch·commit이 일치하는 run만 설치한다.

```powershell
.\scripts\build-release.ps1 -Approved -RunId <STEP 10 run ID> -ExpectedHeadSha <40자리 검증 commit SHA>
```

STEP 16은 현재 checkout의 가변 파일을 직접 build context로 사용하지 않는다. exact commit의 `app` tree를 `git archive`로 만들고 그 archive만 `linux/amd64` 이미지로 build한다. [`container-supply-chain.lock.json`](../../container-supply-chain.lock.json)은 builder/runtime base manifest digest, Rust toolchain, Debian snapshot을 고정하며 [`verify-step16-container-supply-chain.ps1`](../../scripts/verify-step16-container-supply-chain.ps1)이 Dockerfile과 lock의 일치를 build 전후에 확인한다. Debian snapshot timestamp는 Docker `ARG`가 아니므로 Railway service variable로 덮어쓸 수 없다. Runtime의 첫 HTTPS 요청은 pinned builder에서 복사한 CA bundle로 bootstrap한다.

성공한 STEP 16 run에서 `tm-step16-container-provenance`를 내려받아 `ARTIFACT-SHA256SUMS.txt`를 먼저 대조한 뒤 다음을 확인한다.

- `container-provenance.json`의 repository, 40자리 `commitSha`, run ID가 승인한 run과 일치한다.
- `sourceArchiveSha256`, Dockerfile·`.dockerignore`·`Cargo.lock`·supply-chain lock SHA-256이 모두 존재한다.
- 두 base의 index digest와 실제 `linux/amd64` platform manifest digest가 `sha256:` 형식이다.
- Debian snapshot은 `20260731T000000Z`, CI image platform은 `linux/amd64`, revision label은 검증 commit이다.
- `dpkg-packages.tsv`, `image-files.sha256`, `image-inspect.json`, `trivy-image.json`이 artifact checksum에 포함된다.

`ciImage.configDigest`는 STEP 16 runner가 build하고 scan한 로컬 이미지 config digest다. Railway는 이 이미지를 승격하지 않고 exact source archive를 별도로 rebuild한다. 따라서 CI digest와 Railway production digest의 값이 같다고 가정하거나 동일성 증거로 사용하지 않는다. CI artifact는 검증된 source·Dockerfile input과 CI scan 결과를 증명하고, production deployment receipt는 별도의 deployment ID·image digest·source archive SHA-256을 증명한다.

### 3. Railway 암호화 키와 feature control

먼저 승인 commit과 두 Actions run ID를 넣어 dry run을 확인한다.

```powershell
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass `
  -File .\scripts\configure-expense-railway.ps1 `
  -ExpectedHeadSha <40자리 검증 commit SHA> `
  -Step10RunId <STEP 10 run ID> `
  -Step16RunId <STEP 16 run ID>
```

최초 schema 15 도입에서만 다음처럼 명시적으로 새 키 초기화를 승인한다.

```powershell
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass `
  -File .\scripts\configure-expense-railway.ps1 `
  -Apply `
  -InitializeNewKey `
  -ExpectedHeadSha <40자리 검증 commit SHA> `
  -Step10RunId <STEP 10 run ID> `
  -Step16RunId <STEP 16 run ID>
```

스크립트는 production 프로젝트·서비스·HTTPS origin이 코드에 함께 고정되어 있어 다른 환경의 증명으로 키를 설정할 수 없다. 새 256-bit 키는 `-InitializeNewKey`가 있고 다음 읽기 전용 증명 중 하나를 통과할 때만 Windows Credential Locker에 만든 뒤 Railway `TM_EXPENSE_DATA_KEY_V1`에 stdin으로 전달한다.

- schema 14 최초 부트스트랩: database가 정상이고 최신 원격 backup의 schema, integrity, migration ledger, required tables, semantics가 모두 검증됨
- schema 15 최초 부트스트랩: `expenseLedgerEmpty=true`, `expenseKeyInitialized=false`, `expenseKeyInitializationAllowed=true`, `expenseCryptoReady=false`

같은 PC의 동시 실행은 mutex로 거부하며, 키 전송 직전에 운영 상태를 다시 확인한다. 이미 `expenseKeyInitialized=true`이고 `expenseCryptoReady=true`이면 Railway 키는 다시 쓰지 않는다. 이때 로컬 recovery credential이 없으면 스크립트가 중단하며, 저장된 credential이 있어도 기존 production 키와 암호학적으로 일치한다고 간주하지 않는다. 결과의 `recoveryCredentialMatchVerified`는 `false`로 유지한다. initialized 상태에서 crypto가 준비되지 않았으면 이 스크립트는 복구를 시도하지 않고 중단한다. 별도 승인된 challenge 기반 복구 절차와 검증된 backup을 사용해야 하며 `-InitializeNewKey`로 덮어쓰면 안 된다.

키 값은 출력, 클립보드, 결과 JSON에 남지 않는다. stdin 전송을 시도한 뒤 실패하면 원격 반영 여부가 불명확하므로 recovery credential을 보존하고 결과 JSON의 `keyWriteAttempted`·`keyWriteAmbiguous` 상태를 먼저 확인한다. `TM_EXPENSE_AI_ENABLED=true`는 사용자가 버튼을 누른 요청만 허용하며 자동 AI 실행을 만들지 않는다.

키와 feature 변수, `TM_BUILD_COMMIT_SHA`는 모두 `--skip-deploys`로 저장한다. 이 단계는 소스를 배포하지 않으며 결과의 `deploymentTriggered=false`, `sourceDeploymentRequired=true`를 확인한다. schema 14/15 key proof에는 24시간 이내의 snapshot ID·SHA-256이 있는 원격 backup과 backup alert 부재가 포함된다. 결과 JSON은 승인 증거의 snapshot ID·database SHA-256과 30분 만료 `receiptId`를 기록한다. 배포 스크립트는 이 receipt를 필수로 읽고 만료·commit·run ID·backup provenance를 재검증하므로 configure 후 지체하지 않고 다음 단계를 진행한다.

운영 복구 자격 증명 `TM Expense Production Data Key`를 삭제하지 않는다. 키 회전은 기존 데이터를 재암호화하는 별도 migration 없이 실행하면 안 된다.

### 4. Railway migration과 backup

- 두 Actions가 성공한 정확한 commit을 다음 스크립트로 업로드한다. 스크립트는 canonical 저장소·branch·SHA, clean worktree, 두 run, configure receipt와 live schema/backup proof를 재검증한다. 이후 exact commit의 `git archive`를 별도 staging directory에 풀어 Railway에 올리므로 global ignore나 작업 폴더의 숨은 파일이 build context에 들어가지 않는다.

```powershell
.\scripts\deploy-verified-expense-railway.ps1 `
  -Apply `
  -ExpectedHeadSha <40자리 검증 commit SHA> `
  -Step10RunId <STEP 10 run ID> `
  -Step16RunId <STEP 16 run ID> `
  -ConfigurationResultPath .\..\dist\manual-expense-railway-configuration\result.json
```

- `railway up --json`에서 반환된 단일 deployment ID를 사용하며, 동일 message를 목록에서 추정하지 않는다.
- 스크립트가 그 deployment ID의 terminal `SUCCESS`, exact commit metadata, `productionImageDigest`를 확인할 때까지 기다린다. 결과 JSON의 `deploymentWaitRequired=false`를 확인한다.
- `productionImageDigest`는 Railway가 별도로 rebuild한 운영 이미지의 digest다. STEP 16의 `ciImage.configDigest`와 값의 동일성을 요구하지 않고, deployment ID에 결속된 별도 production receipt로 보관한다.
- `/readyz`가 200인지 확인한다.
- 인증된 `/api/v1/ops/status`에서 다음을 확인한다.
  - `database.schemaVersion = 15`
  - `controls.expenseCryptoReady = true`
  - `controls.expenseAiEnabled = true`
  - `controls.incidentMode = normal`
- `database.currentSchemaAppliedAt`과 `localBackup.latestPreMigrationCreatedAt`이 30분 이내로 대응하고 pre-migration backup 크기가 0보다 큰지 확인한다.
- backup job 완료 후 `remoteBackup.schemaVersion = 15`, integrity, migration ledger, required tables, schema semantics가 모두 정상인지 확인한다.
- `remoteBackup.checkedAt`이 24시간 이내이고 snapshot ID·database SHA-256이 있으며 backup 관련 alert가 없는지 확인한다.

마지막으로 deployment result의 정확한 ID와 commit을 읽기 전용 verifier에 전달한다.

```powershell
.\scripts\verify-expense-production.ps1 `
  -ExpectedDeploymentId <Railway deployment ID> `
  -ExpectedHeadSha <40자리 검증 commit SHA> `
  -DeploymentResultPath .\..\dist\manual-expense-railway-deployment\result.json
```

Verifier는 Railway의 현재 active deployment ID·`SUCCESS`·production image digest, server `deploymentProvenance`의 build commit/deployment ID, schema 15와 backup 증거를 함께 대조한다. 검증은 Railway deployment 조회, HTTP GET과 로컬 결과 파일 읽기만 사용하며 production mutation이나 AI 호출을 만들지 않는다.

### 5. API와 UI smoke test

- 월간 summary, cursor 거래, review, 정기지출과 occurrence GET이 200과 `no-store`를 반환하는지 확인한다.
- production verifier는 불변 감사 원장을 오염시키지 않도록 읽기 전용 API와 Windows·모바일 shell만 확인한다.
- 모바일에 파일 선택 또는 import endpoint 호출 경로가 없는지 확인한다.
- 캘린더에서 expense occurrence를 선택하면 캘린더 편집기가 아니라 정기지출 상세로 이동하는지 확인한다.
- Today에 오늘, 7일 이내, 기한 경과, 금액 변동 카드가 AI 호출 없이 표시되는지 확인한다.
- AI 해설 호출은 이 배포 smoke에서 수행하지 않는다. 실제 집계가 생긴 뒤 사용자가 명시적으로 승인한 요청에서만 검증한다.

불변 원장을 오염시키지 않기 위해 production에 합성 거래나 합성 정기지출을 commit하지 않는다. import의 저장·중복·rollback과 mutation 동기화는 Actions의 격리 DB에서 검증하고, production에서는 읽기 전용 검증만 수행한다.

### 6. 실제 파일 미리보기

최종 검증 artifact의 `tm.exe`와 `tm-office-decryptor.exe`를 같은 폴더에 둔다. Windows 지출 화면에서 파일을 선택하고 필요하면 암호를 앱에 직접 입력한다. 다음을 확인한다.

- 원본 파일명과 경로가 cloud 응답, 로그, export에 나타나지 않는다.
- 암호 오류 후 앱이 닫히지 않고 다시 입력할 수 있다.
- 미리보기의 기간, 새 거래, 중복, 정산 후보, 미확인, 거부 건수가 원본과 맞는다.
- 사용자가 `가져오기 확정`을 누르기 전에는 production 원장이 바뀌지 않는다.

실제 production import는 미리보기 확인 후 사용자가 앱에서 별도로 확정한다.

## 장애와 rollback

- `EXPENSE_CRYPTO_NOT_READY`: 배포를 진행하지 말고 Credential Locker의 운영 복구 키와 Railway variable 구성을 확인한다. 새 키를 생성해 덮어쓰지 않는다.
- migration/semantic validation 실패: incident mode를 read-only로 전환하고 pre-migration backup을 보존한다. 실패 원인을 수정한 새 build로 재배포한다.
- STEP 16 provenance 또는 production receipt 불일치: 해당 배포를 verified로 표시하지 않는다. CI image digest와 production image digest를 억지로 같게 만들지 말고, exact source/commit 결속과 각 digest가 속한 build의 deployment ID를 각각 조사한다.
- sidecar hash 불일치: 파일을 실행하지 말고 Actions artifact 전체를 다시 내려받아 SHA-256을 대조한다.
- AI 비용/응답 이상: `TM_EXPENSE_AI_ENABLED=false`로 차단한다. 결정론적 지출 요약과 정기지출은 계속 사용할 수 있다.
- import 중 네트워크 오류: 자동 재시도하지 않는다. 같은 미리보기 session의 idempotent commit 결과를 조회하거나 새 미리보기를 만든다.

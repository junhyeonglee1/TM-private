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

스크립트는 검증된 세 실행 파일과 manifest·provenance를 `TM\\dist\\release`와 최상위 `TM` 경로에 함께 설치한다. 사용자가 실행하는 고정 경로는 `C:\\Users\\tkfk0\\Desktop\\codex\\TM\\tm.exe`다. 실행 중인 TM은 강제 종료하지 않으며, 잠긴 파일이 있으면 어떤 설치 파일도 바꾸기 전에 중단한다.

STEP 16은 현재 checkout의 가변 파일을 직접 build context로 사용하지 않는다. exact commit의 `app` tree를 `git archive`로 만들고 그 archive만 `linux/amd64` 이미지로 build한다. [`container-supply-chain.lock.json`](../../container-supply-chain.lock.json)은 builder/runtime base manifest digest, Rust toolchain, Debian snapshot을 고정하며 [`verify-step16-container-supply-chain.ps1`](../../scripts/verify-step16-container-supply-chain.ps1)이 Dockerfile과 lock의 일치를 build 전후에 확인한다. Debian snapshot timestamp는 Docker `ARG`가 아니므로 Railway service variable로 덮어쓸 수 없다. Runtime의 첫 HTTPS 요청은 pinned builder에서 복사한 CA bundle로 bootstrap한다.

성공한 STEP 16 run에서 `tm-step16-container-provenance`를 내려받아 `ARTIFACT-SHA256SUMS.txt`를 먼저 대조한 뒤 다음을 확인한다.

운영 스크립트는 GitHub CLI의 binary stdout을 원본 ZIP으로 직접 저장하고 REST `artifact.digest`와 ZIP SHA-256을 먼저 대조한다. ZIP은 경로·중복·entry 수·압축률·크기 제한을 검증한 뒤 허용된 파일만 직접 추출한다. `operations-toolchain.lock.json`은 Railway CLI 5.28.1의 exact SHA-256을 고정하고, Git과 GitHub CLI는 고정 설치 경로의 유효한 Authenticode 서명·signer와 실행 시점 해시를 receipt에 결속한다.

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
  -InitializeNewKey `
  -ApprovalReceiptPath .\..\dist\manual-expense-railway-approval\approval.json `
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
  -ApprovalReceiptPath .\..\dist\manual-expense-railway-approval\approval.json `
  -ApprovalNonce <dry-run이 표시한 22자리 nonce> `
  -ExpectedHeadSha <40자리 검증 commit SHA> `
  -Step10RunId <STEP 10 run ID> `
  -Step16RunId <STEP 16 run ID>
```

Dry-run은 production을 변경하지 않고 15분짜리 DPAPI 보호 approval receipt와 일회용 nonce를 만든다. 최초 키 초기화 apply는 같은 Windows 사용자, exact commit, 동일한 STEP 10/16 run attempt와 artifact digest·manifest, 동일한 live backup proof를 다시 검증한 뒤 receipt를 한 번만 소비한다. 모든 production configuration과 source deployment는 `-Force`와 명시적 `-Confirm:$false`를 허용하지 않는다.

스크립트는 production 프로젝트·서비스·HTTPS origin이 코드에 함께 고정되어 있어 다른 환경의 증명으로 키를 설정할 수 없다. 새 256-bit 키는 `-InitializeNewKey`가 있고 다음 읽기 전용 증명 중 하나를 통과할 때만 Windows Credential Locker에 만든 뒤 Railway `TM_EXPENSE_DATA_KEY_V1`에 stdin으로 전달한다.

- schema 14 최초 부트스트랩: database가 정상이고 최신 원격 backup의 schema, integrity, migration ledger, required tables, semantics가 모두 검증됨
- schema 15 최초 부트스트랩: `expenseLedgerEmpty=true`, `expenseKeyInitialized=false`, `expenseKeyInitializationAllowed=true`, `expenseCryptoReady=false`

같은 PC의 configuration과 deployment 동시 실행은 같은 mutex로 거부하며, 모든 apply 경로에서 키 전송 또는 feature 변수 변경 직전에 운영 상태를 다시 확인한다. 이미 `expenseKeyInitialized=true`이고 `expenseCryptoReady=true`이면 Railway 키는 다시 쓰지 않고, active server key·DPAPI 보호 receipt·Windows recovery credential의 domain-separated HMAC fingerprint가 모두 일치할 때만 진행한다. 최초 bootstrap은 `productionFingerprintComparisonDeferred=true`로 기록하고 배포 후 읽기 전용 verifier가 세 값의 일치를 반드시 완결한다. initialized 상태에서 crypto가 준비되지 않았거나 fingerprint가 다르면 이 스크립트는 복구나 덮어쓰기를 시도하지 않고 중단한다.

키 값은 출력, 클립보드, 결과 JSON에 남지 않는다. 제한 시간과 동시 출력 drain이 적용된 stdin 전송을 시도한 뒤 실패하면 recovery credential을 보존하고 결과 JSON의 `keyWriteAttempted`·`keyWriteAmbiguous` 상태를 먼저 확인한다. `TM_EXPENSE_AI_ENABLED`는 승인된 `true` 또는 `false`를 사용하며 production verifier는 live 값이 configuration receipt와 같은지 확인한다. 이 흐름은 자동 AI 실행을 만들지 않는다.

키와 feature 변수, `TM_BUILD_COMMIT_SHA`는 모두 `--skip-deploys`로 저장한다. 최초 configuration은 키의 실제 fingerprint를 `TM_EXPENSE_EXPECTED_KEY_FINGERPRINT`에 결속하고 `TM_EXPENSE_ROLLOUT_MODE=locked`로 고정한다. 이 단계는 소스를 배포하지 않으며 결과의 `deploymentTriggered=false`, `sourceDeploymentRequired=true`를 확인한다. schema 14/15 key proof에는 24시간 이내의 snapshot ID·SHA-256이 있는 원격 backup과 backup alert 부재가 포함된다. 결과 JSON은 승인 증거의 snapshot ID·database SHA-256과 최대 120분짜리 DPAPI 보호 configuration receipt를 기록한다. 새 배포 시작에는 만료되지 않은 receipt가 필요하지만, 이미 증명된 exact locked/enabled deployment를 복구하는 경우에만 만료 receipt를 재사용할 수 있다.

운영 복구 자격 증명 `TM Expense Production Data Key`를 삭제하지 않는다. 키 회전은 기존 데이터를 재암호화하는 별도 migration 없이 실행하면 안 된다.

### 4. Railway 2단계 잠금 migration과 backup

- 두 Actions가 성공한 정확한 commit을 다음 스크립트로 업로드한다. 스크립트는 canonical 저장소·branch·SHA, clean worktree, 두 run, configure receipt와 live schema/backup proof를 재검증한다. 이후 exact commit의 `git archive`를 별도 staging directory에 풀어 Railway에 올리므로 global ignore나 작업 폴더의 숨은 파일이 build context에 들어가지 않는다.

```powershell
.\scripts\deploy-verified-expense-railway.ps1 `
  -Apply `
  -ExpectedHeadSha <40자리 검증 commit SHA> `
  -Step10RunId <STEP 10 run ID> `
  -Step16RunId <STEP 16 run ID> `
  -ConfigurationResultPath .\..\dist\manual-expense-railway-configuration\result.json
```

- 첫 번째 exact-source deployment는 `schema15-expense-lock-<sha12>` message를 사용한다. 이 단계는 schema 15 migration과 pre-migration backup을 수행하지만 암호 probe를 만들지 않으며 모든 지출 API를 닫아 둔다. Railway healthcheck 전용 `/deployment-readyz`만 200이고 일반 `/readyz`는 503이어야 한다.
- 스크립트는 인증된 ops 상태에서 실제 key fingerprint가 configuration receipt와 같고, expected fingerprint match가 true이며, schema 14 pre-migration backup의 SHA-256·크기·integrity·필수 테이블·semantics·시간 상관관계가 모두 맞는지 확인한다.
- 위 증명이 끝난 뒤에만 `TM_EXPENSE_ACTIVATION_FINGERPRINT=<승인 fingerprint>`와 `TM_EXPENSE_ROLLOUT_MODE=enabled`를 `--skip-deploys`로 저장하고, 같은 exact source를 `schema15-expense-activate-<sha12>` message로 두 번째 배포한다. enabled 서버는 actual·expected·activation 세 fingerprint가 같을 때만 probe를 초기화하고 `/readyz`를 200으로 연다.
- 각 `railway up --json`에서 직접 반환된 deployment ID를 사용하며 동일 message만으로 현재 deployment를 추정하지 않는다. 배포 중단 후 재개도 live ops의 exact commit·fingerprint·rollout mode와 Railway deployment ID/message/digest가 모두 일치할 때만 허용한다.
- 스크립트가 두 deployment의 terminal `SUCCESS`, exact commit metadata, 각 `productionImageDigest`를 확인할 때까지 기다린다. 최종 receipt에는 locked/activation deployment ID와 digest를 모두 보관하고 `deploymentWaitRequired=false`를 기록한다.
- `productionImageDigest`는 Railway가 별도로 rebuild한 운영 이미지의 digest다. STEP 16의 `ciImage.configDigest`와 값의 동일성을 요구하지 않고, deployment ID에 결속된 별도 production receipt로 보관한다.
- Railway manifest의 health path는 `/deployment-readyz`다. 최종 활성화 뒤 사용자 readiness인 `/readyz`도 200인지 별도로 확인한다.
- 인증된 `/api/v1/ops/status`에서 다음을 확인한다.
  - `database.schemaVersion = 15`
  - `controls.expenseCryptoReady = true`
  - `controls.expenseRolloutMode = enabled`
  - `controls.expenseExpectedKeyFingerprintMatch = true`
  - `controls.expenseActivationFingerprintMatch = true`
  - `controls.expenseAiEnabled`가 승인된 configuration receipt와 일치
  - `controls.expenseKeyFingerprint`가 Windows recovery credential 및 receipt와 일치
  - `controls.expenseCryptoProbeSha256`가 64자리 SHA-256이고 비밀 키 원문을 포함하지 않음
  - `controls.incidentMode = normal`
- `database.currentSchemaAppliedAt`과 `localBackup.latestPreMigrationCreatedAt`이 30분 이내로 대응하는지 확인한다. pre-migration artifact는 크기뿐 아니라 SHA-256, schema 14, `integrity_check=ok`, schema 14 필수 테이블, schema semantics 검증까지 모두 통과해야 한다.
- backup job 완료 후 `remoteBackup.schemaVersion = 15`, integrity, migration ledger, required tables, schema semantics가 모두 정상인지 확인한다.
- 원격 backup은 `expenseCryptoProbePresent=true`이고 `expenseCryptoProbeSha256`가 live probe hash와 같아야 한다. `snapshotStartedAt`은 activation deployment 시각 이후(시계 정밀도 허용 1초)여야 하며 `checkedAt`보다 늦을 수 없다. 이 결속으로 activation 전 snapshot을 schema 15 최신 backup으로 오인하지 않는다.
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

- `EXPENSE_ROLLOUT_LOCKED`: 첫 단계의 정상 fail-closed 상태다. 지출 API를 사용하지 말고 exact locked deployment proof로 같은 배포 스크립트를 재개한다.
- `EXPENSE_CRYPTO_NOT_READY`: 배포를 진행하지 말고 Credential Locker의 운영 복구 키와 Railway variable 구성을 확인한다. 새 키를 생성해 덮어쓰지 않는다.
- migration/semantic validation 실패: incident mode를 read-only로 전환하고 pre-migration backup을 보존한다. 실패 원인을 수정한 새 build로 재배포한다.
- STEP 16 provenance 또는 production receipt 불일치: 해당 배포를 verified로 표시하지 않는다. CI image digest와 production image digest를 억지로 같게 만들지 말고, exact source/commit 결속과 각 digest가 속한 build의 deployment ID를 각각 조사한다.
- sidecar hash 불일치: 파일을 실행하지 말고 Actions artifact 전체를 다시 내려받아 SHA-256을 대조한다.
- AI 비용/응답 이상: `TM_EXPENSE_AI_ENABLED=false`로 차단한다. 결정론적 지출 요약과 정기지출은 계속 사용할 수 있다.
- import 중 네트워크 오류: 자동 재시도하지 않는다. 같은 미리보기 session의 idempotent commit 결과를 조회하거나 새 미리보기를 만든다.

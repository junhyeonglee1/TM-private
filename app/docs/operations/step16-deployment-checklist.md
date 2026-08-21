# STEP 16 보안 배포 checklist

## 배포 전

- [x] 변경 범위와 migration 호환성 확인
- [x] `git diff --check`와 작업 트리 확인
- [x] frontend lint·typecheck·test 통과
- [x] Rust fmt·workspace test·Clippy 통과
- [x] STEP 16 static secret/workflow policy scan 통과
- [x] RustSec vulnerability 0; upstream unmaintained warning 검토 완료
- [x] Trivy filesystem high/critical finding 0
- [x] `AVD-DS-0002` 단일 예외의 gosu 전환과 2026-10-20 만료일 검토
- [x] production image build와 Trivy image finding 0
- [x] Action reference가 모두 40자리 commit SHA
- [x] DB·backup·token·API key·SSH key가 source/artifact에 없음
- [x] rollback 대상 commit/deployment와 RTO 담당 절차 확인

## 배포 중

- [x] production project/service/environment를 명시
- [x] singleton·Volume `/var/lib/tm`·health path `/readyz` 확인
- [x] build log에 secret·header·body가 없는지 확인
- [x] 새 deployment가 `SUCCESS`가 되기 전 추가 변경 금지

## 배포 후

- [x] `/readyz` 200
- [x] security header·no-CORS·request target limit 통과
- [x] primary 인증과 device scope·CSRF 유지
- [x] `incidentMode=normal`, `aiEnabled=true`
- [x] RPO 24h·RTO 2h·rollback 15m 값 확인
- [x] remote backup `succeeded`, schema 일치, integrity `ok`
- [x] scheduler dead letter 0
- [x] OpenAI $10 경고/TM $20 hard stop 확인
- [x] 실제 OpenAI 호출과 Task·Note mutation 없이 STEP 16 production verification 통과
- [x] 이전 source rollback과 현재 source 재승격 훈련 결과 기록
- [x] 최신 backup restore drill 결과 기록

## 2026-07-22 검증 증거

| 항목 | 결과 |
|---|---|
| 검증 source | `97093a1` |
| Windows build | GitHub Actions `29903708320` 성공 |
| security gate | GitHub Actions `29903711604` 성공; RustSec 0, Trivy filesystem/image high·critical 0 |
| rollback | `17a670b6-524a-44bc-8553-5f8fb013bae4` 성공 후 `702b3853-4b03-4610-8ae3-0fc005d8821d` 재승격, 목표 15분 이내 |
| restore drill | schema 9, SHA-256 `6844b4681ba36c4af43ac066ab91adb26b9c16aa48d410924b8ef400972abe8c`, integrity/foreign key 통과 |
| incident drill | `8e417f95-4d8b-49c8-905b-571ac7560cd3` read-only/AI-off 차단 성공 |
| 정상 복구 | `d7e1375c-b77a-4052-ad66-b10d2f8a54d5` normal/AI-on, operations `healthy` |
| credential cleanup | 임시 Railway SSH key 원격 부재와 로컬 private/public key 부재 확인 |
| 부작용 | OpenAI 호출 0건, Task·Note mutation 0건, 활성 backup Volume 덮어쓰기 없음 |

## 배포 명령

```powershell
railway up --detach --yes --project 7fcb22b5-db34-4e2b-a12a-cbc60391ff5f --service tm-server --environment production
powershell.exe -NoProfile -ExecutionPolicy Bypass -File '.\scripts\verify-step16-production.ps1'
```

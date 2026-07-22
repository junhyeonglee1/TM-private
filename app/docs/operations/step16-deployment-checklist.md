# STEP 16 보안 배포 checklist

## 배포 전

- [ ] 변경 범위와 migration 호환성 확인
- [ ] `git diff --check`와 작업 트리 확인
- [ ] frontend lint·typecheck·test 통과
- [ ] Rust fmt·workspace test·Clippy 통과
- [ ] STEP 16 static secret/workflow policy scan 통과
- [ ] RustSec와 Trivy filesystem high/critical finding 0
- [ ] production image build와 Trivy image finding 0
- [ ] Action reference가 모두 40자리 commit SHA
- [ ] DB·backup·token·API key·SSH key가 source/artifact에 없음
- [ ] rollback 대상 commit/deployment와 RTO 담당 절차 확인

## 배포 중

- [ ] production project/service/environment를 명시
- [ ] singleton·Volume `/var/lib/tm`·health path `/readyz` 확인
- [ ] build log에 secret·header·body가 없는지 확인
- [ ] 새 deployment가 `SUCCESS`가 되기 전 추가 변경 금지

## 배포 후

- [ ] `/readyz` 200
- [ ] security header·no-CORS·request target limit 통과
- [ ] primary 인증과 device scope·CSRF 유지
- [ ] `incidentMode=normal`, `aiEnabled=true`
- [ ] RPO 24h·RTO 2h·rollback 15m 값 확인
- [ ] remote backup `succeeded`, schema 일치, integrity `ok`
- [ ] scheduler dead letter 0
- [ ] OpenAI $10 경고/TM $20 hard stop 확인
- [ ] 실제 OpenAI 호출과 Task·Note mutation 없이 STEP 16 production verification 통과
- [ ] 이전 source rollback과 현재 source 재승격 훈련 결과 기록
- [ ] 최신 backup restore drill 결과 기록

## 배포 명령

```powershell
railway up --detach --yes --project 7fcb22b5-db34-4e2b-a12a-cbc60391ff5f --service tm-server --environment production
powershell.exe -NoProfile -ExecutionPolicy Bypass -File '.\scripts\verify-step16-production.ps1'
```

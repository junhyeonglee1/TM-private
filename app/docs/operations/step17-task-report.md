# STEP 17 Task 리포트 운영 절차

## 기본 정책

- 사용자 호출만 허용한다. 자동 실행과 push 알림은 꺼져 있다.
- Windows와 등록된 모바일 PWA가 같은 production endpoint를 사용한다.
- 하루 최대 4회, 호출당 최대 USD 0.05, 월 전체 hard stop USD 20을 동시에 적용한다.
- 실패한 호출을 client나 server가 자동 재시도하지 않는다.
- 실제 Task를 생성·수정·완료·이월하지 않는다.

## 점진 배포

1. CI의 frontend test/build, Rust workspace test·Clippy, 보안 workflow를 통과한다.
2. `TM_TASK_REPORT_ENABLED`를 unset 또는 `false`로 둔 채 image를 배포한다.
3. `/readyz` schema 10, `/api/v1/ops/status` backup·scheduler·AI budget과 `taskReportEnabled=false`를 확인한다.
4. schema 10 pre-migration backup과 새 remote backup 성공을 확인한다.
5. `TM_TASK_REPORT_ENABLED=true`를 적용해 재배포한다.
6. 확인 header 없는 POST가 `428`이고 latest 조회가 인증된 요청에서만 성공하는지 확인한다.
7. production 실제 Task로 billable 호출을 정확히 한 번 실행한다.
8. 반환 ID가 호출 전 Task 후보 안에 있고 Task·Note 응답이 호출 전후 동일한지 확인한다.
9. token, 추정 비용, latency, prompt version, model, run ID가 기록되는지 확인한다.

## 중지와 복구

품질 저하, OpenAI 오류 증가, 잘못된 ID, 예상 밖 비용, latency 증가가 있으면 먼저 다음을 실행한다.

```powershell
& '.\scripts\set-step17-task-report.ps1' -Enabled false -Apply -Force
```

global AI 사고라면 STEP 16 절차로 `TM_AI_ENABLED=false`를 적용한다. schema 9 image로 rollback하지 않는다. 기능이 꺼져도 기존 보고서와 평가 ledger는 보존되며 backup 대상에 남는다.

## 다음 개선 판단 지표

- `도움 됨` 비율
- 잘못된 structured output 또는 후보 ID 거부율
- 성공/실패율과 p50/p95 latency
- 호출당 평균 input/output token과 평균·최대 비용
- 하루 제한 도달 횟수
- 사용자가 `다시 분석`을 누르는 빈도

초기 실제 1회 검증은 기능 안전성 검증이며 품질 결론을 내리기 위한 표본은 아니다. 최소 1~2주 사용 기록을 모은 뒤 prompt 또는 후보 선정 순서를 조정한다.

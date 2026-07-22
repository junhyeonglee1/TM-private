# STEP 17 오늘의 Task AI 리포트

STEP 17의 첫 실제 비서 기능은 `오늘의 Task AI 리포트`다. 사용자가 Windows `오늘` 화면 또는 모바일 PWA에서 명시적으로 버튼을 누를 때만 OpenAI Responses API를 한 번 호출한다. 자동 스케줄, push, email, Task 변경은 이번 범위에 없다.

## 데이터 경계

서버는 서울 날짜 기준 morning digest를 side effect 없이 미리 본 뒤 다음 순서로 후보를 합친다.

1. 오늘 계획
2. 어제 미완료
3. 3일 이내 마감
4. 막힘
5. 기존 결정론적 추천

중복을 제거한 최대 20개만 사용한다. OpenAI 입력 allowlist는 Task ID·제목·프로젝트 이름·상태·우선순위·마감일·선정 category다. Task 설명, checklist, Note, Worklog, Session, memory, attachment, 인증·backup·감사·비용 원장은 전달하지 않는다. 제목과 프로젝트 이름을 포함한 모든 값은 명령이 아닌 신뢰하지 않는 데이터로 prompt에 선언한다.

## 전용 실행 경계

- endpoint: `POST /api/v1/assistant/task-report`
- 명시적 확인: `x-tm-confirm-ai-call: task-report`
- 조회: `GET /api/v1/assistant/task-reports/latest`
- 평가: `POST /api/v1/assistant/task-reports/{id}/feedback`
- global kill switch: `TM_AI_ENABLED`
- feature kill switch: `TM_TASK_REPORT_ENABLED`
- 1일 시도 상한: 4회
- 1회 사전 예약 상한: 50,000 microUSD, 즉 USD 0.05
- 출력 상한: 800 tokens
- 서버 timeout: 45초
- 자동 재시도: 없음
- OpenAI response 저장: `store: false`
- prompt version: `step17-task-report-v1`

하루 제한 claim과 실행 record 생성은 `BEGIN IMMEDIATE` transaction 하나에서 처리한다. 시작 뒤 실패하거나 서버가 재시작된 시도도 당일 상한에 포함해 과금 폭주를 fail closed한다. 열린 후보가 없으면 OpenAI를 호출하거나 일일 AI 시도를 소비하지 않고 비용 0의 결정론적 결과를 기록한다.

## 결과 검증

Responses API Structured Outputs의 strict JSON Schema로 headline, summary, 1~3개 priority와 최대 3개 alert를 받는다. 서버가 다시 다음 항목을 검증한다.

- priority Task ID가 입력 후보에 실제로 존재한다.
- ID와 rank가 중복되지 않고 rank가 1부터 연속한다.
- 항목 수와 모든 문자열 길이가 서버 상한 이내다.
- headline, summary, reason, next action이 비어 있지 않다.

하나라도 어기면 결과 전체를 저장·표시하지 않고 실패로 기록한다. 보고서에는 Task 변경 기능이나 범용 agent tool이 없다.

## 사용 기록과 품질 평가

schema 10의 `task_report_runs`는 날짜, actor, 성공·실패, 후보 수, model, prompt version, OpenAI response/request ID, 결과 JSON, token, 추정 비용, latency, failure code를 기록한다. `task_report_feedback`은 `도움 됨/도움 안 됨`을 보고서당 한 번 append-only로 기록한다. 원본 prompt와 전체 입력 payload는 저장하지 않는다.

이 두 ledger는 export·backup manifest에 포함된다. 현재 기록과 다른 backup으로 복원해 사용량 또는 평가를 되감으려 하면 restore를 거부한다.

## 배포와 rollback

production cloud profile에서 기능 flag의 기본값은 `false`다. schema 10 image를 먼저 배포해 migration, readiness, backup을 확인한 다음 `TM_TASK_REPORT_ENABLED=true`로 별도 배포한다. schema 10으로 migration한 뒤 schema 9 binary로 되돌리는 rollback은 금지한다. 문제 발생 시 새 binary를 유지한 채 feature flag를 `false`로 내려 기능만 중지하고 roll-forward 수정한다.

스케줄, 정기 지출, 주식은 각자 별도 data selector·prompt·endpoint·비용/권한 정책을 가진 기능으로 같은 경계를 확장한다.

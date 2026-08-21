# Read-only AI 오케스트레이터

STEP 11은 Railway의 `cloud-authenticated` 서버에서 OpenAI Responses API를 호출하되, AI가 TM 데이터를 직접 읽거나 변경하지 못하도록 서버가 모든 도구 실행을 중개한다. API key는 서버 프로세스 환경변수에서만 읽고 SQLite·응답·로그에 기록하지 않는다.

## 운영 계약

| 항목 | 고정값 |
| --- | --- |
| endpoint | `POST /api/v1/assistant/query` |
| 인증 | TM bearer token 필수 |
| 과금 확인 | `X-TM-Confirm-AI-Call: assistant` 필수 |
| model | 기본 `gpt-5.6-terra` |
| prompt version | `calendar-assistant-v1` |
| API | Responses API |
| reasoning | `medium`, `current_turn` |
| 응답 저장 | `store: false` |
| 답변 길이 | 최대 output 2,000 tokens, verbosity `medium` |
| 요청 제한 | message 8 KiB, body 16 KiB |
| 도구 제한 | 한 요청에서 최대 6회, parallel tool call 비활성화 |
| 시간 제한 | 전체 오케스트레이션 60초 |
| 요청별 비용 예약 | USD 0.25 |
| 월 비용 | USD 10 warning, USD 20 hard stop |

`store: false` 흐름에서는 이전 Responses 출력 전체와 `function_call_output`을 다음 요청의 `input`으로 다시 보낸다. reasoning 항목을 버리거나 `previous_response_id`에 의존하지 않는다.

## 데이터 경계

모델이 사용할 수 있는 read 도구는 다음 9개뿐이다.

- `list_projects`
- `list_tasks`
- `list_calendar_occurrences`
- `list_checklist`
- `list_notes`
- `list_sessions`
- `list_worklogs`
- `search_tm`
- `search_memory`

각 도구는 삭제되지 않은 레코드만 최대 20개 반환한다. 캘린더 도구는 한 번에 최대 31일만 조회하며 description을 제외한 occurrence ID·event ID·제목·종류·날짜·시간·반복 정보만 반환한다. 긴 문자열은 필드별 UTF-8 512 bytes에서 잘라내고, 도구 출력 전체가 64 KiB를 넘으면 실패시킨다. 결과는 read-only source와 `untrusted: true`로 표시한다.

허용 필드는 ID, project/session 연결 ID, 제목·설명·본문·목표·결과·차단 요인·다음 행동, 상태·우선순위·완료 여부와 관련 날짜다. 다음 항목은 도구 자체에 존재하지 않는다.

- API key, TM 인증 token과 hash, 만료 정보
- DB 파일·내부 schema·SQL·로컬 경로
- backup, restore 자료와 원격 저장소 credential
- attachment 원본·경로·hash·byte
- mutation 감사 ledger와 AI 비용 ledger 원문
- 외부 web search, MCP, 메시지 발송, 결제, 일정 실행 도구

도구 결과의 제목이나 본문에 명령처럼 보이는 문자열이 있어도 untrusted data로 취급하도록 system instructions에 고정했다. 서버도 모델이 allowlist 밖의 도구 이름이나 strict schema 밖의 인수를 반환하면 실행하지 않는다.

## 읽기와 실행 분리

STEP 11의 오케스트레이터는 조회·검색·요약·분석만 수행한다. 사용자가 생성·수정·삭제·전송·구매·예약을 요청하면 계획을 설명할 수는 있지만 실행할 수 없다. `create_task` 같은 mutation 이름을 모델이 임의로 반환해도 `ASSISTANT_TOOL_NOT_ALLOWED`로 차단되고 TM core의 mutation 함수는 호출되지 않는다.

실제 mutation 도구와 preview·승인·취소·만료·중복 실행 차단은 STEP 12에서 별도 계약으로 추가한다.

## 비용과 실패 처리

서버는 OpenAI 네트워크 요청 전에 USD 0.25를 append-only 비용 원장에 예약한다. 월 USD 20을 넘길 수 있으면 `402 AI_MONTHLY_BUDGET_EXCEEDED`로 네트워크 전에 차단한다. 정상 응답은 모든 Responses 호출의 usage를 합산해 모델별 단가로 정산한다. timeout, 전송 중단, 파싱 실패처럼 과금 여부를 확정할 수 없으면 보수적으로 예약액 전부를 비용으로 남긴다.

HTTP request ID를 사용자가 반복해도 과금 원장이 재사용되지 않도록 각 billable 실행은 별도의 UUID reservation ID를 만든다. 로그에는 HTTP request ID, model, 허용된 도구 이름과 호출 횟수만 남기며 사용자 message, 도구 결과, 답변, token 원문은 기록하지 않는다.

## 배포 순서

1. API key 없이 배포한다.
2. `scripts/verify-step11-production.ps1`로 인증, model, `store: false` 상태, 읽기 전용·비용·도구·timeout 상한, 과금 확인 header, missing-key 차단을 검증한다.
3. Railway sealed variable `OPENAI_API_KEY`를 보안 입력한다. `TM_OPENAI_BASE_URL` override는 production에서 계속 금지한다.
4. 같은 스크립트를 `-ExpectedConfigured $true`로 실행해 key 원문을 노출하지 않고 설정 상태를 확인한다.
5. 명시적 최소 probe와 읽기 전용 assistant 요청을 각각 한 번 실행하고 usage·비용 원장·데이터 불변을 검증한다.

비과금 검증 명령:

```powershell
Set-Location 'C:\Users\tkfk0\Desktop\codex\TM\app'
& '.\scripts\verify-step11-production.ps1'
```

이 스크립트는 운영 token을 Windows Credential Locker의 `TM Cloud Production` / `single-user`에서 읽으며 화면·파일·클립보드에 token을 출력하지 않는다.

Railway secret 보안 입력과 최소 과금 acceptance 명령:

```powershell
Set-Location 'C:\Users\tkfk0\Desktop\codex\TM\app'
& '.\scripts\configure-step11-openai-secret.ps1'
& '.\scripts\verify-step11-production.ps1' -ExpectedConfigured $true
& '.\scripts\invoke-step11-production-acceptance.ps1'
```

secret 입력 스크립트는 키 형식 검증 후 Railway CLI의 stdin으로만 `OPENAI_API_KEY`를 전달하고, 로컬 결과에는 변수 이름과 성공 여부만 기록한다. acceptance 스크립트는 probe 1회와 assistant 질의 1회를 실행한다. 답변 원문은 화면에만 표시하고 결과 파일에는 hash·길이·usage·비용·도구 이름만 남긴다. 호출 전후 Project·Task·Checklist·Note·Session·Worklog의 응답 data hash가 같아야 성공한다.

## 2026-07-21 production 완료 기록

- API key 없는 production deployment `45968361-7cc5-4424-a69a-86c2835fe8d3`에서 missing-key·과금 확인·인증·예산 경계를 비과금으로 검증했다.
- Railway sealed variable 설정 후 deployment `8e9e6e55-5a3a-4896-8860-3cd3bfa6f52c`가 성공했고 key 원문 없이 `configured: true`를 확인했다.
- 실제 probe는 USD 0.000208, read-only assistant는 USD 0.008188로 추정됐다. assistant는 `list_tasks` 1회만 사용했고 input 1,739·output 256 tokens를 사용했다.
- 업무 데이터 SHA-256은 호출 전후 모두 `23c77655c6d26ccde3e07576d4c13514b8af94c8efe3dd27ad8ce9d3129f0163`으로 같았다. 답변 원문은 검증 결과 파일에 저장하지 않았다.
- OpenAI organization에는 월 USD 20 hard limit과 USD 10 email alert를 설정했다. TM 서버 내부의 USD 10 warning·USD 20 선결제 hard stop도 독립적으로 유지한다.

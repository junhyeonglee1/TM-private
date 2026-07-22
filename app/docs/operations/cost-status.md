# 상단 비용 상태

TM은 Windows와 모바일 PWA의 우측 상단에 비용 두 개만 표시한다.

- `API $0.01 / $20`: TM이 이번 달 OpenAI 호출 때 확정한 추정 비용 / TM hard stop
- `Cloud $0.09 / $30`: Railway workspace의 현재 billing period 리소스 사용량 / Railway hard limit

토큰 수, 요청 횟수, 예상 월말 비용, Railway Hobby 최소 구독료는 이 작은 상태 영역에 표시하지 않는다. Railway의 billing period는 달력 월과 다를 수 있으며 정확한 시작·종료일은 badge tooltip에만 둔다.

## 데이터 경계

`GET /api/v1/costs/status`는 인증된 primary admin과 등록된 기기의 읽기만 허용한다. 응답에는 microUSD 정수, 비용 기간, 마지막 갱신 시각만 포함하며 provider credential이나 청구 상세 내역은 포함하지 않는다.

OpenAI 값은 조직 전체 청구 API를 호출하지 않는다. 별도 Admin API key를 만들지 않고 TM의 append-only AI 비용 원장 `committedMicrousd`를 사용하므로, TM 밖에서 같은 OpenAI project/key로 쓴 비용은 포함되지 않는다.

Railway 값은 공식 GraphQL API의 workspace `customer.currentUsage`와 `usageLimit.hardLimit`을 사용한다. 성공 결과는 서버 메모리에 10분 캐시한다. 갱신 실패 후 1분 동안 provider를 다시 호출하지 않으며, 이전 성공값이 있으면 `stale: true`로 반환하고 한 번도 성공하지 못했으면 `Cloud — / $30`으로 표시한다. 비용 상태 실패는 Task·Note·AI 기능을 중단하지 않는다.

## Railway 활성화

필요한 production 변수는 다음과 같다.

| 변수 | 값 | 보안 등급 |
|---|---|---|
| `TM_RAILWAY_API_TOKEN` | 해당 workspace로 제한한 Railway workspace token | sealed secret |
| `TM_RAILWAY_WORKSPACE_ID` | 비용을 조회할 workspace UUID | non-secret |
| `TM_RAILWAY_HARD_LIMIT_USD` | provider limit을 받지 못할 때 쓸 fallback, 현재 `30` | non-secret |

Railway workspace token은 읽기 전용 credential이 아니며 그 workspace의 모든 리소스에 접근할 수 있다. 계정 전체 token은 사용하지 않는다. 이 권한을 승인한 뒤에만 Account Settings의 Tokens에서 대상 workspace를 지정해 새 token을 만들고 다음 보안 입력 스크립트를 실행한다.

```powershell
Set-Location 'C:\Users\tkfk0\Desktop\codex\TM'
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File '.\app\scripts\configure-cost-status-railway.ps1' -Apply
```

스크립트는 non-secret 두 개를 deployment 없이 먼저 저장하고, workspace token은 표준입력으로 sealed variable에 전달한 뒤 deployment를 한 번만 시작한다. token 원문은 결과 파일·콘솔·클립보드에 보존하지 않는다.

## 검증

1. 새 Railway deployment가 `SUCCESS`이고 `/readyz`가 `200`인지 확인한다.
2. primary token으로 `/api/v1/costs/status`를 읽어 API hard limit이 `20000000`, Cloud hard limit이 `30000000`, Cloud `available`이 `true`인지 확인한다.
3. Windows 상단에 `API $x.xx / $20`, `Cloud $x.xx / $30`이 보이는지 확인한다.
4. 등록된 모바일 PWA에서도 같은 값이 보이는지 확인한다.
5. 응답·로그·로컬 결과 파일에 Railway/OpenAI token이 없는지 정적 검사한다.

Railway token이 노출되거나 서버가 침해된 경우 Account Settings에서 token을 즉시 폐기하고 `TM_RAILWAY_API_TOKEN`을 제거한다. 비용 표시만 unavailable로 바뀌며 TM 핵심 기능은 계속 동작한다.

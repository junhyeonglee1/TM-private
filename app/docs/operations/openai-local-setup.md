# OpenAI 로컬 연결 설정

이 절차는 TM 사용자 DB가 아닌 `TM/dist/manual-openai-probe`의 임시 DB로 OpenAI Responses API 연결만 확인한다. 실제 비서 대화나 일정·소비 데이터는 전송하지 않는다.

## 보안 원칙

- API 키는 `tm-server` 프로세스 환경변수 `OPENAI_API_KEY`에만 넣는다.
- 키를 TM 프런트엔드, 모바일 앱, Git, 문서, 로그 또는 채팅에 입력하지 않는다.
- 로컬 단계에서는 `.env` 파일을 자동으로 읽지 않는다.
- 클라우드 배포 단계에서는 같은 환경변수를 클라우드 비밀관리 서비스에서 주입한다.

## 환경변수

| 이름 | 필수 | 기본값 | 용도 |
| --- | --- | --- | --- |
| `TM_SERVER_HOME` | 예 | 없음 | 서버가 사용할 절대 TM 홈 경로 |
| `TM_SERVER_BIND` | 아니요 | `127.0.0.1:8787` | 인증 도입 전에는 loopback 주소만 허용 |
| `OPENAI_API_KEY` | AI 호출 시 | 없음 | 서버 전용 OpenAI API 키 |
| `TM_OPENAI_MODEL` | 아니요 | `gpt-5.6-terra` | Responses API 모델 |
| `TM_OPENAI_BASE_URL` | 아니요 | `https://api.openai.com/v1/` | 공식 OpenAI 주소, 테스트에서는 loopback HTTP만 허용 |
| `TM_OPENAI_TIMEOUT_SECS` | 아니요 | `60` | OpenAI 호출 제한 시간, 1~120초 |
| `TM_OPENAI_MONTHLY_WARNING_USD` | 아니요 | `10` | TM 상태에 경고로 표시할 월 누적 추정 비용 |
| `TM_OPENAI_MONTHLY_HARD_LIMIT_USD` | 아니요 | `20` | 요청 전에 원자적으로 차단할 TM 월 비용 상한 |

## 1. 서버 실행

PowerShell에서 다음 명령을 실행한다. `Read-Host -AsSecureString`을 사용하므로 키가 명령 기록에 남지 않는다.

```powershell
Set-Location 'C:\Users\tkfk0\Desktop\codex\TM\app'
$secret = Read-Host 'OPENAI_API_KEY 입력' -AsSecureString
$env:OPENAI_API_KEY = [System.Net.NetworkCredential]::new('', $secret).Password
$env:TM_SERVER_HOME = (New-Item -ItemType Directory -Force '..\dist\manual-openai-probe').FullName
$env:TM_SERVER_BIND = '127.0.0.1:8787'
$env:TM_OPENAI_MODEL = 'gpt-5.6-terra'
& 'C:\Users\tkfk0\.cargo\bin\cargo.exe' run --locked --offline -p tm-server
```

## 2. 연결 확인

다른 PowerShell 창에서 상태를 먼저 확인한다. 이 요청은 OpenAI를 호출하지 않으므로 비용이 발생하지 않는다.

```powershell
Invoke-RestMethod 'http://127.0.0.1:8787/api/v1/ai/status'
```

`configured`가 `True`, `responseStorage`가 `disabled`인지 확인한다. 그다음 최소 생성 요청을 한 번 보낸다. 이 호출에는 소량의 API 사용료가 발생할 수 있다.

```powershell
Invoke-RestMethod -Method Post `
    -Headers @{ 'x-tm-confirm-ai-call' = 'probe' } `
    'http://127.0.0.1:8787/api/v1/ai/probe'
```

성공하면 `outputText`가 `TM_OPENAI_OK`, `matchedExpectedText`가 `True`, `stored`가 `False`로 표시된다. `usage`에는 실제 입출력 토큰 수가, `estimatedCostMicrousd`와 `budget`에는 비용 추정과 월 잔여 한도가 표시된다. hard limit을 넘는 호출은 OpenAI로 전송되기 전에 `402 AI_MONTHLY_BUDGET_EXCEEDED`로 차단된다.

## 3. 종료

서버 창에서 `Ctrl+C`를 누른 뒤 해당 PowerShell 창을 닫는다. 필요하면 현재 창의 환경변수를 바로 제거한다.

```powershell
Remove-Item Env:OPENAI_API_KEY -ErrorAction SilentlyContinue
```

API 키가 노출됐다고 의심되면 OpenAI 대시보드에서 즉시 폐기하고 새 키를 발급한다.

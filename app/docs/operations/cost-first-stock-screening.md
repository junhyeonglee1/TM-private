# 비용 우선 S&P 500 일일 스크리닝 최종안

## 결정 요약

이 기능은 새로운 유료 데이터 구독이나 별도 Railway 서비스를 만들지 않는다. 기존 `tm-server` 안에서 하루 한 번 정량 계산하고, 계산 결과가 있는 날에만 OpenAI가 짧은 한국어 요약을 한 번 생성한다.

| 항목 | 확정안 | 월 예상 비용 |
|---|---|---:|
| 종목 구성 | DataHub `datasets/s-and-p-500-companies`의 PDDL 데이터 | `$0` |
| 미국 일봉 | Alpaca Basic market data | `$0` |
| 계산·저장 | 기존 Railway service와 SQLite Volume 재사용 | 신규 고정비 `$0` |
| AI 요약 | `gpt-5.4-nano-2026-03-17`, 거래일당 최대 1회 | 약 `$0.05` |
| 기능별 내부 원장 한도 | 월 `$2`에 도달하면 새 OpenAI 예약 중단 | 정상 호출 최대 `$2` |

2026-07-25 공식 공개 가격인 input `$0.20/1M tokens`, cached input `$0.02/1M tokens`, output `$1.25/1M tokens`를 계산 기준으로 고정한다. 22거래일, 호출당 최대 input 10,000 tokens와 output 200 tokens를 가정하면 input 약 `$0.002`, output 약 `$0.00025`, 합계 약 `$0.00225/일`, `$0.0495/월`이다. 이는 여유를 포함해 월 약 `$0.05`로 본다. 캐시 할인은 예상 비용에 포함하지 않는다. 코드의 가격 allowlist는 알려진 모델 이외의 호출을 막지만 OpenAI 가격표 변경을 자동 감지하지는 못한다. 정기 운영 점검과 배포 전 공식 가격 확인으로 allowlist를 갱신하며, 실제 공급자 청구 내역이 최종 기준이다.

TM 전체의 정상 목표는 기존 Railway Hobby `$5` 최소 사용료와 주식 AI 예상 약 `$0.05`를 포함해 월 `$10` 미만이다. 주식 기능은 별도 서버를 늘리지 않으며, 유료 provider fallback·자동 요금제 전환·자동 overage를 금지한다. Alpaca Basic 또는 DataHub를 사용할 수 없으면 기능을 `upstream_unavailable`로 표시하고 멈춘다. 다른 유료 API로 자동 전환하지 않는다.

기존 TM의 전역 OpenAI `$20` hard stop은 다른 비서 기능까지 보호하는 상위 내부 원장 한도다. 주식 기능은 이 한도와 별도로 월 `$2` 원장 한도에서 새 예약을 먼저 차단한다. 다만 TM 원장은 공급자 청구 시스템과 원자적으로 연결된 결제 차단기가 아니다. 예상보다 큰 단일 응답이나 가격표 변경이 있으면 실제 추정 비용을 축소하지 않고 기록하므로 `$2`를 소폭 넘긴 뒤 다음 호출부터 멈출 수 있다. 실제 Railway workspace의 provider hard limit과 OpenAI project budget도 TM 화면 숫자와 별개다. Railway와 OpenAI는 서로의 청구액을 합산 차단할 수 없으므로 `$10`은 정상 운영 목표이지 공급자 간 단일 hard stop은 아니다. 절대 합계가 중요하면 stock AI gate를 계속 끄고 정량 screen만 `$0`로 사용하거나, 각 공급자 한도를 별도로 낮춘다.

## 데이터와 사용 범위

### 구성 종목

- source: [DataHub S&P 500 Companies repository](https://github.com/datasets/s-and-p-500-companies)
- license: [Open Data Commons PDDL 1.0](https://opendatacommons.org/licenses/pddl/1-0/)
- 저장 정보: source URL, upstream revision, SHA-256, 내려받은 시각, license·attribution 문구
- 갱신: 주 1회, 성공한 마지막 snapshot을 유지
- 차단: snapshot이 14일보다 오래되면 새로운 스크린·AI 요약을 만들지 않는다.

화면에는 `S&P 500 구성종목 (DataHub/Wikipedia snapshot)`이라고 표시한다. S&P Global의 승인·제휴를 암시하지 않는다. 구성 종목 수는 고정 500으로 가정하지 않고 snapshot의 각 상장 security/share class를 그대로 처리한다.

### 일봉

- provider: Alpaca Basic
- endpoint 사용: multi-symbol historical bars, `feed=sip`, `adjustment=split`, `currency=USD`
- 범위: 미국 시장 마감 후 15분 이상 지난 일봉만 요청
- 저장: 계산 재현에 필요한 최근 120일 이내, 최소 45거래일의 split-adjusted daily close
- 금지: provider key, 원본 HTTP 응답, 계좌·주문·포트폴리오 데이터 저장

Alpaca의 `$0` 요금과 기술적 API 접근 가능성만으로 TM의 Railway 저장, 동일 사용자 Windows·모바일 표시, 파생 수익률 생성, OpenAI 처리까지의 계약상 사용 범위가 확정되는 것은 아니다. production 데이터 호출을 켜기 전에 Alpaca로부터 다음 사용 형태에 대한 서면 확인을 보관한다.

1. 개인용 TM의 Railway SQLite에 일봉을 일시 저장한다.
2. 같은 사용자의 Windows·모바일 기기에 원시 일봉이 아닌 파생 결과를 표시한다.
3. ticker, 5-session/21-session 수익률, 기준일만 OpenAI processor에 전달한다. 회사명과 원시 가격은 전달하지 않는다.
4. OpenAI `store: false`를 사용하지만 abuse monitoring log가 최대 30일 보존될 수 있음을 알린다.
5. provider 계약 종료 시 보관 자료를 제거하는 정책을 적용한다.

서면 확인 전에는 `TM_STOCK_LICENSE_ACK=false`를 유지한다. 이는 법률 자문을 대신하지 않는 운영 gate다.

## 정량 계산 계약

서버가 정답 숫자를 계산하며 AI는 숫자를 만들거나 수정하지 않는다.

- 기준일 `T`: snapshot membership과 품질 검사를 통과한 최신 확정 미국 거래일
- 5-session: `close(T) / close(T-5) - 1`
- 21-session: `close(T) / close(T-21) - 1`
- 가격: micro-USD 정수
- 임계값 비교: 부동소수점 반올림 없이 정수 교차곱
- 배당: 제외
- split: provider의 split-adjusted close 사용

구간은 겹치지 않게 고정한다.

| 방향 | 구간 |
|---|---|
| 상승 10~20% | `+10% <= r < +20%` |
| 상승 20% 이상 | `r >= +20%` |
| 하락 10~20% | `-20% < r <= -10%` |
| 하락 20% 이상 | `r <= -20%` |

현재일·5-session baseline·21-session baseline 각각 universe의 98% 이상이 정확한 session date를 가져야 성공이다. 97.9%는 실패다. baseline 근처 날짜로 대체하지 않는다. 중복·역행 날짜, 잘못된 통화, 빈 값, 과대 응답은 해당 실행을 성공 처리하지 않는다.

정량 전체 결과가 primary artifact다. 각 결과에는 universe revision/hash, 기준일, baseline 날짜·가격, 수익률, 구간을 동결한다. AI 실패 여부와 관계없이 숫자 목록은 조회할 수 있다.

## 실행과 장애 처리

- scheduler: 미국 장 마감과 SIP 지연을 충분히 지난 매일 `10:30 Asia/Seoul`
- universe 갱신: 같은 scheduler에서 주 1회, 성공한 snapshot만 교체
- 구조: 짧은 DB transaction으로 claim → transaction 밖에서 async provider 호출 → compare-and-set 완료 transaction
- 실행 고정: scheduler claim ID를 screen run ID로 사용하고 claim의 예정 시각에서 확정 시장일을 계산한다. 재시작·재시도는 저장된 시장일과 universe snapshot을 그대로 재사용한다.
- 장중 bar 차단: provider가 더 최신인 당일 누적 daily bar를 반환해도 claim에 고정된 확정 시장일보다 뒤의 session과 bar를 폐기하고, 고정 시장일 자체가 없으면 fail closed한다.
- lease: 30초 heartbeat, 180초 동안 갱신하지 못하면 현재 외부 작업을 취소하고 lease 소유권을 재확인하지 않은 결과는 발행하지 않음
- provider retry: 비용과 중복 side effect를 줄이기 위해 같은 일일 실행 안에서는 자동 재시도하지 않고 429·5xx·timeout을 `upstream_unavailable`로 종료
- dead letter: DB 불변식 위반처럼 사람이 확인해야 하는 내부 오류에만 사용
- AI at-most-once: 호출 전에 비용 reservation과 attempt 시작을 영속화
- AI timeout/crash: `ai_uncertain`, 자동 재시도 금지
- 후보 없음: 정상적인 0건 결과로 저장하고 OpenAI를 호출하지 않음

provider I/O를 SQLite `BEGIN IMMEDIATE` 안에서 수행하지 않는다. 느린 외부 응답이 Task·Note·Calendar 쓰기를 막아서는 안 된다.

## schema 13

schema 12의 `stock_watchlist_items`는 유지하고 다음 테이블을 추가한다.

- `stock_universe_snapshots`
- `stock_universe_members`
- `stock_market_data_batches`
- `stock_market_sessions`
- `stock_daily_bars`
- `stock_screen_runs`
- `stock_screen_results`
- `stock_ai_reports`

기존 `scheduler_jobs.kind`와 `scheduler_effects.job_kind`의 CHECK 제약은 schema 13 migration에서 신규 stock job kind를 허용하도록 table rebuild한다. live, retry, dead-letter row와 foreign key를 보존하고 `PRAGMA foreign_key_check`를 통과해야 한다.

각 수집 batch는 source hash·feed·adjustment·session/bar 수·최초/최종 session·수집 시각을 immutable provenance로 남기고 screen run이 해당 hash를 참조한다. 최신 session/bar 행은 용량을 아끼기 위한 재구성 가능한 mutable cache다. 발행된 screen result에는 실제 기준 가격·날짜·수익률과 source hash를 동결하므로 cache가 갱신돼도 결과가 바뀌지 않는다.

universe snapshot, market-data batch, 발행된 screen run/result, AI report는 export·backup·restore·migration manifest에 포함한다. 현재 복구 원본은 `user_version`뿐 아니라 migration 1..14의 연속 ledger와 schema 14 필수 테이블 manifest를 모두 만족해야 한다. anti-rewind 검증은 현재 운영 원장의 발행 결과·AI 비용·감사 기록을 더 오래된 백업으로 되감지 않게 한다.

## OpenAI 계약과 비용 차단

- model: `gpt-5.4-nano-2026-03-17`
- API: Responses API
- service tier: `default`
- reasoning effort: `none`
- storage: `store: false`
- 최대 후보: 40개
- 정렬: 절대 수익률 내림차순, ticker 오름차순
- 입력: ticker, 5-session/21-session 수익률, 기준일만
- 모델 출력: strict JSON의 입력 후보 내 notable ticker 선택만 허용
- 사용자 출력: 서버가 선택 ticker에 회사명·정확한 수익률을 다시 결합해 중립적 한국어 headline·bullet을 결정적으로 생성
- 금지: 원시 OHLC, 관심 종목, 개인 Task·Calendar, 원인 단정, 예측, 추천, 매수·매도 문구

서버는 출력 ticker가 입력 후보의 중복 없는 부분집합인지 검증하고 모든 문구와 숫자를 원본 정량 결과에서 다시 생성한다. 따라서 모델 자유문장이 원인·추천·외부 종목·변경된 숫자를 넣을 수 없다. 요청 모델과 응답 모델이 pinned snapshot과 다르거나 모델이 가격 allowlist에 없으면 호출 전에 fail closed한다.

한 요청의 reservation은 `$0.01`이다. 같은 `BEGIN IMMEDIATE` transaction에서 다음 두 조건을 모두 확인한다.

1. TM 전역 committed+reserved+신규 reservation이 `$20` 이하
2. `stock_daily_report`의 당월 committed+reserved+신규 reservation이 `$2` 이하

응답 usage가 없으면 reservation 전액을 불확실 비용으로 기록한다. usage로 계산한 실제 추정액이 reservation을 넘으면 그 금액을 `$0.01`로 축소하지 않고 실제 추정액대로 원장에 기록하고 이후 호출을 차단한다. 같은 request ID의 settlement 재호출은 비용·outcome·usage가 모두 일치할 때만 멱등 성공하며, 하나라도 다르면 conflict다. AI가 꺼짐, 예산 소진, 실패, 결과 불확실 상태여도 정량 스크린 결과는 유지한다.

## 기본 비활성 환경변수

최초 schema 13 배포는 다음 값으로 고정한다.

```text
TM_STOCK_SCREEN_ENABLED=false
TM_STOCK_AI_ENABLED=false
TM_STOCK_LICENSE_ACK=false
TM_STOCK_OPENAI_MODEL=gpt-5.4-nano-2026-03-17
```

`TM_AI_ENABLED`는 기존 TM AI master switch다. stock AI가 실행되려면 master와 위 세 stock gate가 모두 허용되어야 한다. 키가 있더라도 gate가 하나라도 닫혀 있으면 Alpaca/OpenAI 호출을 하지 않는다.

## 공개 command와 UI

read-only command:

- `get_latest_stock_screen`
- `list_stock_screen_results({runId,horizon,direction,band,cursor,limit})`

`horizon`은 5 또는 21, `direction`은 `up` 또는 `down`, `band`는 `ten_to_twenty` 또는 `twenty_plus`, `limit`은 최대 50이다. 승인된 모바일 기기는 기존 same-origin·CSRF 경계를 통과해야 한다. read-only command에는 mutation confirmation header를 요구하지 않는다. 수동 `run AI` endpoint와 버튼은 만들지 않는다.

Windows와 모바일의 오늘 화면에는 최신 확정 시장일과 상승·하락 top 3만 표시한다. 주식 화면에서는 5/21 session, 방향, 구간을 필터링하고 50개 단위로 조회한다. 행을 누르면 기존 TradingView 종목만 바꾸며 OpenAI를 호출하지 않는다.

UI는 다음 상태를 서로 구분한다.

- 첫 실행 전
- 최신 성공
- 이전 성공을 보여주는 stale 상태
- coverage 실패
- provider unavailable
- AI off, budget blocked, failed, uncertain
- 정상 0건
- transport error

휴장일·DST·stale 판정은 서버가 한다. UI가 달력으로 추측하지 않는다. offline에서는 현재 세션에 이미 읽은 마지막 결과만 유지하고 cold offline 결과를 보장하지 않는다.

## 운영 상태

`GET /api/v1/ops/status`의 stock block:

- `licenseGate`
- `screenEnabled`
- `aiEnabled`
- `latestExpectedTradingDate`
- `latestSuccess`
- `latestAttempt`
- `coverage`
- `universeAgeDays`
- `nextRunAt`
- `monthlyFeatureCostMicrousd`
- `hardLimitMicrousd` (`2000000`)
- `lastErrorCode`

provider key·응답 body·AI prompt payload는 status와 log에 넣지 않는다.

## 필수 테스트

- migration: 빈 DB와 schema 12 DB를 13으로 열고 pre-migration backup, migration ledger, `foreign_key_check`를 확인
- scheduler rebuild: live·retry·dead-letter job과 effect foreign key를 보존
- 계산: 정확히 `-20`, `-10`, `+10`, `+20%` 경계, 5/21 session, 휴장, DST, split, 중복·누락 bar를 fixture로 검증
- coverage: 97.9% 실패와 98.0% 성공을 별도 검증
- provider mock: multi-symbol pagination, `feed=sip`, `adjustment=split`, 429, 5xx, transport failure, 잘못된 날짜, malformed·oversized payload
- 시장일 고정: 지연 실행과 장중 재시작에서 claim 확정일 뒤의 partial daily bar가 저장·발행되지 않음을 검증
- log: 모든 오류·snapshot·test artifact에 Alpaca key와 원본 authorization header가 없음을 검사
- scheduler 동시성: 느린 provider가 일반 DB write를 막지 않고 lease heartbeat·crash·duplicate claim이 결과를 중복 발행하지 않음을 검증
- AI 비용: 전역/기능 이중 원장 cap, 동시 reservation, 월 경계, unknown model price fail-closed, 실제 추정액 초과 기록, settlement payload 충돌, ambiguous timeout 무재시도
- AI 생략: 후보 0건, coverage 실패, license off, screen off, AI off, budget blocked에서 OpenAI mock 호출이 0회인지 검증
- API: desktop allowlist, 승인 모바일 기기, same-origin·CSRF, read command에 confirmation 불필요, limit 50·엄격한 args
- UI: Windows와 실제 PWA DOM에서 filter, pagination, stale/error/0건, Today top 3, 행 선택 후 기존 chart 변경, 현재 세션 offline 유지
- 복구: export·remote backup·restore에서 migration 1..14 ledger와 schema 14 필수 table manifest, anti-rewind를 확인하고 재구성 가능한 bar cache와 보존 대상 결과를 구분

## 구현·검증 순서

1. schema 13과 deterministic 계산을 synthetic fixture로 구현한다.
2. provider adapter, scheduler claim/heartbeat/finalize, 비용 이중 cap을 mock server로 검증한다.
3. command와 Windows·모바일 UI를 구현한다.
4. export·backup·restore·operations status와 production verifier를 갱신한다.
5. 세 stock gate가 모두 꺼진 commit을 GitHub Actions에서 검증하고 production에 배포한다.
6. 운영 DB와 remote backup이 현재 schema 14, integrity `ok`, `schemaSemanticsValidated=true`이고 stock flags가 모두 꺼져 있음을 확인한다.
7. Alpaca paper account key를 Railway sealed variables로 입력하고 서면 사용 범위 확인 자료를 보관한다.
8. `TM_STOCK_LICENSE_ACK=true`, `TM_STOCK_SCREEN_ENABLED=true`, `TM_STOCK_AI_ENABLED=false`로 7일간 정량 결과만 관찰한다.
9. 98% coverage, 기준일, split 사례, 비용 `$0`을 확인한 뒤 `TM_STOCK_AI_ENABLED=true`로 전환한다.
10. 첫 AI 결과의 숫자 불변성·중립성·사용량을 확인하고 월 `$2` cap을 production에서 재확인한다.

최초 비활성 배포 검증은 provider 호출이나 mobile pairing 없이 다음 명령으로 수행한다.

```powershell
Set-Location 'C:\Users\tkfk0\Desktop\codex\TM'
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File '.\app\scripts\verify-stock-screen-production.ps1'
```

local Windows Cargo는 이 PC의 Code Integrity 정책 때문에 실행하지 않는다. Rust/Tauri 변경은 `rustfmt --check`와 허용된 frontend·PowerShell·정적 검사를 먼저 통과시킨 뒤 GitHub Actions의 `STEP 10 Windows build`에서 `cargo test`, `cargo clippy`, Windows 실행 파일 build를 확정한다. 보안 변경은 `STEP 16 security`도 통과해야 한다. 내려받은 `tm-step10-windows-x64`의 `SHA256SUMS.txt`와 두 실행 파일 해시를 대조하기 전에는 verified 또는 개선 요청 completed로 기록하지 않는다.

## 중단과 rollback

비용, 라이선스, provider 품질, AI 품질 중 하나라도 기준을 벗어나면 먼저 `TM_STOCK_AI_ENABLED=false`, 다음으로 `TM_STOCK_SCREEN_ENABLED=false`를 적용한다. schema downgrade는 하지 않는다. 기존 chart/watchlist와 이미 발행된 정량 결과는 읽기 전용으로 유지한다.

유료 data API 추가, Railway service 추가, model 자동 승급은 별도 사용자 승인 없이는 금지한다.

## 기준 문서

- [OpenAI API pricing](https://developers.openai.com/api/docs/pricing)
- [OpenAI GPT-5.4 nano model](https://developers.openai.com/api/docs/models/gpt-5.4-nano)
- [Alpaca Market Data API](https://docs.alpaca.markets/docs/about-market-data-api)
- [Alpaca historical stock bars](https://docs.alpaca.markets/reference/stockbars)
- [DataHub S&P 500 Companies repository](https://github.com/datasets/s-and-p-500-companies)

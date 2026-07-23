# 조회 전용 주식 차트 운영·사용 절차

TM의 Windows 앱과 모바일 PWA는 같은 Railway SQLite 원본에 관심 종목만 저장한다. 차트와 지연 시세는 TradingView가 브라우저에 직접 제공하며 TM 서버는 가격, 캔들, 거래량을 수집하거나 보관하지 않는다. 차트를 열어도 OpenAI API를 호출하지 않으므로 AI 비용은 발생하지 않는다.

## 지원 범위

- 시장: `KRX`, `NASDAQ`, `NYSE`, `AMEX`
- KRX 티커: 6자리 숫자
- 미국 티커: 영문 대문자, 숫자, `.`, `-`로 구성된 1~10자
- 검색 목록: KRX 유가증권·코스닥과 Nasdaq Trader의 NASDAQ·NYSE·NYSE American 비ETF 종목
- 관심 종목: 최대 50개
- 기본 차트: `NASDAQ:AAPL`
- 차트 설정: 한국어, 서울 시간, 일봉 캔들, 거래량, 기간 선택
- 제외: 계좌 연결, 보유 자산, 매수·매도, 자동매매, 가격 알림, 투자 추천, AI 주가 분석

Windows에서는 왼쪽 `도구`의 `주식`을 연다. 모바일에서는 `/mobile/`에 연결된 승인 기기로 접속해 `주식` 탭을 연다. 시장을 고르고 회사명이나 종목코드를 검색한 뒤 후보를 선택하면 된다. 두 화면은 같은 cloud 관심 종목 목록을 사용한다. 위젯 자체 검색은 저장하지 않고 다른 종목을 임시로 조회하는 용도다.

## 외부 차트와 장애 처리

TradingView는 거래소 정책에 따라 지연 시세를 표시하며 KRX 일부 종목은 위젯 안에서 제한될 수 있다. TM은 제한을 피하기 위한 자동 수집이나 우회를 하지 않는다. 차트가 비어 있으면 화면의 `TradingView 외부 차트에서 확인` 링크를 사용한다.

인터넷이 끊기면 저장된 관심 종목 화면은 유지하고 차트 대신 연결 필요 안내를 표시한다. 다시 온라인이 되면 해당 종목의 iframe을 새로 만든다. TradingView 장애는 TM의 Task, 캘린더, 노트, AI 비서 기능에 영향을 주지 않는다.

## 보안 경계

- TradingView 스크립트는 TM 본문이 아닌 `data:` 격리 문서 iframe에서만 실행한다. `data:` 문서는 TM과 동일 출처가 될 수 없는 고유한 opaque origin이다.
- TradingView가 자기 문서의 쿠키를 읽어 차트를 초기화할 수 있도록 sandbox에 `allow-same-origin`을 사용한다. 이 권한은 opaque `data:` 문서 안에서만 적용되므로 TM origin, 쿠키, 저장소, Tauri IPC에는 접근할 수 없다.
- iframe은 `no-referrer`를 사용하고 격리 문서 자체 CSP에서 TradingView의 검토된 script/frame host만 허용한다.
- TradingView 코드에는 TM bearer token, 기기 쿠키, CSRF 값, Tauri IPC 객체를 전달하지 않는다.
- 모바일 관심 종목 변경은 승인 기기 범위, same-origin, CSRF, 정확한 command confirmation을 모두 통과해야 한다.
- 관심 종목과 차트 데이터는 AI 도구·프롬프트의 입력 후보에 포함하지 않는다.

## 데이터와 복구

schema 12의 `stock_watchlist_items`에는 `symbol`, `market`, `ticker`, `display_name`, 생성·수정 시각만 저장한다. `symbol`은 `MARKET:TICKER` 고유 키이며 동일 종목 저장은 표시 이름을 원자적으로 갱신한다. 삭제는 이미 없는 종목에도 성공하는 멱등 연산이다.

schema 11에서 12로 열기 전에 pre-migration backup을 자동 생성한다. 테이블은 SQLite backup·restore, 전체 JSON/Markdown export, migration manifest, Railway 암호화 원격 백업에 포함된다. schema 12가 적용된 뒤 schema 11 실행 파일로 되돌리지 않고 schema 12에서 roll-forward 수정한다.

## 배포 검증

배포 전 GitHub Actions에서 frontend lint/typecheck/UI test, Rust workspace test/clippy, Windows release build, STEP 16 보안 검사를 통과시킨다. Production에서는 다음을 확인한다.

1. `/readyz`가 ready이고 운영 DB와 원격 백업이 schema 12, integrity `ok`다.
2. `NASDAQ:AAPL`과 `KRX:005930` 관심 종목을 임시 저장하고 Windows bearer command와 임시 승인 모바일 기기 command에서 같은 목록을 읽는다.
3. AAPL과 삼성전자 차트 iframe 또는 외부 TradingView 링크를 확인한다.
4. 테스트 중 추가한 관심 종목과 임시 기기를 정리한다.
5. 관심 종목 작업 전후의 TM AI 비용 원장이 변하지 않았는지 확인한다.

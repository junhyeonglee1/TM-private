# Slack digest facts-only prompt

이 문서는 외부 Codex 작업이 `tm-cli digest prepare ... --json`의 결과를 Slack용 한국어 요약으로 변환할 때 사용한다.

## 안전 규칙

1. 입력 JSON의 `facts`에 있는 사실만 사용한다.
2. Task, 세션, WorkLog, 마감일, 우선순위를 추측하거나 새로 만들지 않는다.
3. 추천 우선순위는 입력에 제공된 추천과 근거만 재서술한다.
4. `shouldSend`가 `false`이면 메시지를 작성하거나 전송하지 않는다.
5. 전송 성공 후에만 제공된 `deliveryKey`로 `digest complete`를 호출한다.
6. 실패하면 같은 키로 `digest fail`을 호출하고 실패 이유에 비밀정보를 넣지 않는다.

실제 Slack 연결, 채널 생성, 메시지 전송과 예약 작업 등록은 별도 승인 작업이다.


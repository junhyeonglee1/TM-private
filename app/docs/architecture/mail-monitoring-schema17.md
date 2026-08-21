# TM 중요 메일 모니터링 (schema 17)

schema 17은 Gmail과 네이버 메일함을 **조회 전용**으로 감시하고, 중요한 메일의 최소 메타데이터만 TM에 보관한다. 자동 회신·발송·삭제·이동·읽음 처리는 구현하지 않는다.

## 신뢰 경계

- Gmail은 OAuth `gmail.readonly` 단일 scope만 허용한다. callback은 state 단일 사용, 10분 만료, PKCE S256을 검증한다.
- Pub/Sub endpoint는 Google OIDC issuer, exact audience, 전용 service account, `email_verified`와 message ID를 검증한다. 알림 본문은 cursor wake-up에만 쓰고 실제 변경은 `history.list`로 조회한다.
- 네이버는 `imap.naver.com:993` TLS에서 `LOGIN`, `EXAMINE`, `UID SEARCH`, `UID FETCH ... BODY.PEEK`, `LOGOUT`만 사용한다. 메일함 변경 명령은 소스와 정적 보안 검사에서 금지한다.
- 원문 HTML·첨부·원격 이미지·인용 대화는 저장하거나 렌더링하지 않는다. 원문은 공급자 HTTPS 웹메일을 외부 브라우저로 연다.
- 이메일은 신뢰할 수 없는 입력이다. OTP·인증번호·비밀번호 재설정·보안 경고는 AI 경계 밖에서 규칙으로만 처리하고 짧은 고정 안내문만 저장한다.

## 저장과 암호화

`mail_accounts`, `mail_credentials`, `mail_sync_state`, `mail_items`, `mail_feedback`, `mail_rules`, `mail_oauth_states`, `mail_webhook_events`, `mail_triage_*`, `mail_reports`, `mail_sync_events`, `mail_mutation_receipts`가 backup manifest에 포함된다.

이메일 주소, 표시 이름, provider message ID, 발신자, 발신 도메인, 제목, 짧은 요약, 행동 문구, OAuth refresh token과 네이버 앱 비밀번호는 `TM_MAIL_DATA_KEY_V1`의 XChaCha20-Poly1305로 암호화한다. 규칙·중복 비교는 HMAC-SHA256 blind index를 사용한다. 키와 평문 인증정보는 DB, 백업, 로그, mutation receipt에 넣지 않는다.

메일 항목은 90일, 일별 요약은 366일 보관한다. 복원 merge는 현재 계정 상태·cursor·사용자 피드백과 AI 완료 상태를 앞으로 합치며 만료된 메일을 다시 살리지 않는다.

## 판정과 비용

Spam·Trash·광고·뉴스레터는 제외하고 계정 보안, 결제 실패·환불, 예약 변경·취소, 기한을 무료 규칙으로 먼저 판정한다. 명확하지 않은 후보만 60초 동안 모아 최대 10건씩 `gpt-5.4-nano-2026-03-17`에 보낸다.

AI 입력은 발신 도메인, 정제·제한된 제목과 짧은 preview, 내부 fact ID뿐이다. URL과 긴 숫자는 가리고 `store:false`, 고정 Structured Output, 도구 없음으로 호출한다. 75점 이상이면서 신뢰도 70 이상만 중요 메일이고, 낮은 신뢰도는 확인 필요다.

- operation: `mail_triage`
- 1회 예약 상한: `$0.01`
- 서울 기준 월 100회
- 메일 전용 월 `$1` hard stop
- 전체 OpenAI 월 `$20` hard stop 내부

AI 실패·timeout·예산 소진 후보는 해당 실패 batch에 결속해 자동 재과금을 막고 기존 규칙 판정을 유지한다.

## UI와 권한

Windows와 모바일은 중요 메일, 확인 필요, 08:00·19:00 요약, 고정 규칙, 연동 상태를 조회한다. 사용자 피드백과 TM 내부 확인 완료만 메일 항목의 TM 상태를 변경하며 공급자 메일함은 바꾸지 않는다.

`TM_MAIL_REPORTS_ENABLED`는 7일 동기화 관찰 중 Today 카드와 08:00·19:00 digest를 별도로 끄는 presentation gate다. 수집·규칙 판정·메일 화면 검증과 분리해 관찰 완료 뒤 켠다.

Gmail OAuth 시작, 네이버 앱 비밀번호 저장, 계정 해제는 운영 토큰이 있는 Windows 관리자만 허용한다. 모바일 기기는 same-origin CSRF, confirmation header, version CAS, 안정적인 idempotency key를 사용해 조회·피드백·확인 완료만 수행한다.

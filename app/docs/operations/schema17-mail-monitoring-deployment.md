# schema 17 메일 모니터링 배포·운영

schema 17은 먼저 모든 메일 gate를 끈 상태로 migration과 backup을 검증한다. Google·네이버 인증정보는 Git, 채팅, 로그, 결과 JSON에 넣지 않는다.

## Gate OFF 배포 조건

1. 로컬 허용 검사와 같은 exact commit의 `STEP 10 Windows build`, `STEP 16 security`가 모두 성공해야 한다.
2. STEP 10 artifact의 `tm.exe`, `tm-cli.exe`, `tm-office-decryptor.exe`를 `SHA256SUMS.txt`와 대조한다.
3. production schema 16과 최신 schema 16 원격 백업의 integrity·semantics가 정상이어야 한다.
4. 다음 변수를 `false`로 유지한 exact source만 먼저 배포한다.

```text
TM_MAIL_ENABLED=false
TM_GMAIL_ENABLED=false
TM_NAVER_MAIL_ENABLED=false
TM_MAIL_AI_ENABLED=false
TM_MAIL_REPORTS_ENABLED=false
```

5. 배포 뒤 `/readyz`와 `/api/v1/ops/status`에서 schema 17, schema 16 pre-migration backup, schema 17 원격 backup, 메일 scheduler queue/retry/dead-letter 0, 메일 AI 비용 `$0/$1`을 읽기 전용으로 확인한다.

이 PC에서는 Windows Code Integrity 정책 때문에 Cargo build/test/clippy를 로컬 성공 근거로 쓰지 않는다. 관리자 권한으로 우회하지 않고 Rust/Tauri는 GitHub Actions로만 확정한다.

## Gmail 준비

Google Cloud에서 Gmail API와 Pub/Sub API를 활성화하고 production OAuth Web client를 만든다. redirect URI는 exact production origin의 `/api/v1/mail/oauth/google/callback`이다. Pub/Sub topic에는 Gmail push service account `gmail-api-push@system.gserviceaccount.com`의 Publisher 권한을 주고, push subscription은 `/api/v1/mail/webhooks/google`을 향하게 한다. subscription 인증은 별도 Google service account와 exact audience를 사용한다.

Railway에는 비밀값을 읽어 출력하지 않는 방식으로 다음을 저장한다.

```text
TM_MAIL_DATA_KEY_V1=<새 256-bit 메일 전용 키>
TM_GMAIL_CLIENT_ID=<OAuth client ID>
TM_GMAIL_CLIENT_SECRET=<OAuth client secret>
TM_GMAIL_OAUTH_REDIRECT_URI=https://<TM origin>/api/v1/mail/oauth/google/callback
TM_GMAIL_PUBSUB_TOPIC=projects/<project>/topics/<topic>
TM_GMAIL_PUBSUB_AUDIENCE=https://<TM origin>/api/v1/mail/webhooks/google
TM_GMAIL_PUBSUB_SERVICE_ACCOUNT=<push 인증 service account email>
```

먼저 `TM_MAIL_ENABLED=true`, `TM_GMAIL_ENABLED=true`, `TM_NAVER_MAIL_ENABLED=false`, `TM_MAIL_AI_ENABLED=false`, `TM_MAIL_REPORTS_ENABLED=false`로 Gmail 규칙 기반 동기화만 활성화한다. Windows TM의 메일 > 연동 상태에서 Gmail 읽기 전용 연결을 누른다. Google 동의 화면에 Gmail 읽기 이외 scope가 보이면 중단한다.

7일 동안 다음을 확인한다.

- 동기화 완전성 99% 이상
- Push 감지 p95 60초 이내, 5분 보정 동기화 정상
- watch 만료 전에 매일 03:20 KST 갱신
- queue/retry/dead-letter 정상, 재인증 필요 0
- provider mailbox 변경 0
- AI 비용 `$0`

이 조건 이후 `TM_MAIL_REPORTS_ENABLED=true`로 Today 카드와 08:00·19:00 요약을 활성화한다. AI는 별도 결정으로 `TM_MAIL_AI_ENABLED=true`를 설정하며, 켜도 월 100회, 메일 `$1`, 전체 `$20` 상한은 그대로 적용된다.

## 네이버 추가

네이버 계정에서 IMAP 사용, 2단계 인증, 애플리케이션 비밀번호를 준비한다. 비밀번호는 채팅에 붙이지 않고 Windows TM의 메일 > 연동 상태에서만 입력한다. 서버는 실제 TLS IMAP 로그인을 먼저 검증한 뒤 암호화해 저장한다.

`TM_NAVER_MAIL_ENABLED=true`로 전환한 뒤 7일 동안 정상 상태 감지 지연 4분 이내, 중복 0, 읽음 상태 변경 0, 재인증 필요 0을 확인한다. 실패하면 네이버 gate만 `false`로 되돌리고 Gmail은 유지한다.

## 사고 대응

- AI 품질·비용 문제: `TM_MAIL_AI_ENABLED=false`
- 네이버 인증·IMAP 문제: `TM_NAVER_MAIL_ENABLED=false`
- Gmail OAuth·Pub/Sub 문제: `TM_GMAIL_ENABLED=false`
- 암호화 키·원장·복원 문제: `TM_MAIL_ENABLED=false`, 필요하면 기존 incident read-only mode

gate를 꺼도 기존 암호화 원장과 피드백은 backup에 남으며 공급자 메일을 변경하지 않는다. 키가 없거나 fingerprint probe가 일치하지 않으면 메일 readiness는 fail-closed한다.

공식 참고: [Gmail push notifications](https://developers.google.com/workspace/gmail/api/guides/push), [Gmail API usage limits](https://developers.google.com/workspace/gmail/api/reference/quota), [Google OAuth 2.0](https://developers.google.com/identity/protocols/oauth2), [Pub/Sub pricing](https://cloud.google.com/pubsub/pricing), [네이버 IMAP 안내](https://help.naver.com/service/30029/contents/21344).

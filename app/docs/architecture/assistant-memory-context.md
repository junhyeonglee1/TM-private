# STEP 13 장기 기억·검색·컨텍스트 예산

STEP 13은 기존 TM 원본 데이터와 AI 비서가 사용하는 장기 기억을 분리한다. 사용자가 명시적으로 저장을 요청한 정보만 기억으로 만들고, 생성·수정·삭제는 STEP 12의 잠금 제안과 별도 승인을 거쳐 실행한다. 기존 Task·Note·Session·Worklog의 의미와 UI는 바꾸지 않는다.

OpenAI Responses 요청은 애플리케이션이 필요한 대화·도구 결과를 선택해 전달하는 구조이며, 이전 응답을 연결해도 이전 입력 토큰은 다시 과금될 수 있다. 따라서 TM은 전체 이력을 매번 전달하지 않고 로컬 검색 결과를 고정 예산 안에서 선택하며 `store: false`를 유지한다. 참고: [OpenAI conversation state](https://developers.openai.com/api/docs/guides/conversation-state)

## 저장 경계

| 항목 | 정책 |
| --- | --- |
| 종류 | preference, goal, routine, constraint, reference, summary |
| 사용자 기억 | 명시적 요청만 허용, 기본 보존은 `until_deleted` |
| 파생 요약 | 서버가 결정론적으로 생성하며 OpenAI를 호출하지 않음 |
| 출처 | task, note, worklog, session, memory 또는 명시적 사용자 입력 |
| 민감도 | normal, private, restricted |
| OpenAI 전달 | `normal`이면서 사용자가 `openaiAllowed=true`로 허용한 기억만 가능 |
| 절대 저장 금지 | 비밀번호, API key, 인증·복구 코드, 전체 카드·계좌·정부 식별번호 |

`assistant_memories`는 본문, 출처, provenance, 민감도, 보존 정책, revision, SHA-256, soft-delete 상태를 보관한다. `assistant_memory_sources`는 원본 연결을 정규화하고 `assistant_memory_events`는 생성·수정·삭제·만료·원본 삭제·재생성을 append-only로 기록한다. schema 7 migration은 기존 STEP 12 승인 원장을 보존하면서 memory create/update/delete operation을 추가한다.

## 승인과 변경

- `memory.create`, `memory.update`, `memory.delete`는 모두 오케스트레이터가 제안만 할 수 있다.
- 실제 변경은 인증된 승인 API에서 10분 안에 revision과 payload SHA-256을 확인한 뒤 한 번만 실행된다.
- update와 delete는 expected revision을 요구해 오래된 승인과 동시 변경을 차단한다.
- idempotency 원장과 append-only 이벤트가 중복 실행과 감사 원장 되감기를 막는다.
- private·restricted 기억은 로컬 저장과 조회만 가능하며 AI 도구 결과에 포함되지 않는다.

## 검색과 컨텍스트 예산

검색은 SQLite FTS5와 종류·민감도·OpenAI 허용 여부 filter를 조합한다. 외부 vector DB나 embedding API는 사용하지 않는다. 후보는 로컬에서 최대 200개까지만 평가하며 실제 반환은 요청 종류별 상한과 직렬화 byte 상한을 동시에 적용한다.

| 요청 종류 | 최대 항목 | 최대 크기 |
| --- | ---: | ---: |
| 일반 답변 | 6 | 2,048 bytes |
| 계획 | 10 | 4,096 bytes |
| 요약 | 12 | 6,144 bytes |

한 assistant 요청에서 memory 검색 도구는 최대 한 번만 실행할 수 있다. 응답 metadata에는 request kind, 허용량, 실제 사용 항목·byte, 추정 token, 생략 수, 검색 횟수와 `vectorServiceUsed=false`를 포함한다. 크기 추정치는 비용 제어용 보수적 값이며 실제 OpenAI 청구 token은 provider usage를 기준으로 기록한다.

## 요약·보존·삭제

- 일간 요약은 90일, 주간 요약은 365일, 월간 요약은 1,095일 보존한다.
- 요약은 정렬된 로컬 기억에서 결정론적으로 계산하고 source hash를 provenance에 기록한다.
- 파생 요약의 민감도는 모든 원본 중 가장 엄격한 값을 사용한다.
- OpenAI 허용은 모든 원본이 `normal`이고 명시적으로 허용된 경우에만 상속한다.
- 연결된 원본이 삭제되거나 없어지면 파생 기억을 soft-delete하고 이벤트를 기록한다.
- 만료된 파생 요약은 maintenance에서 정리한다. 정기 실행 연결은 STEP 14 scheduler 범위다.
- 오래된 backup이 기억·출처·이벤트 원장을 되감으면 restore를 거부한다.

## API와 오케스트레이터

인증된 read-only API:

- `GET /api/v1/assistant/memories`
- `GET /api/v1/assistant/memories/search`
- `GET /api/v1/assistant/memories/{id}`
- `GET /api/v1/assistant/memories/{id}/events`

오케스트레이터에는 기존 조회 도구에 `search_memory`가 추가되며, 제안 도구로 memory create/update/delete가 추가된다. `AI status`는 `step13-v1`, 자동 저장 비활성화, 승인 필수, SQLite FTS5, vector 미사용, 12개·6,144 bytes 절대 상한을 공개한다.

## 검증 기준

- 격리 DB에서 승인 전 무변경, 승인 후 1회 변경, revision 충돌과 감사 이벤트를 확인한다.
- 금지 비밀값과 private·restricted OpenAI 전달을 거부한다.
- 250개 기억에서도 6개·2,048 bytes 검색 상한과 vector 미사용을 확인한다.
- 일·주·월 요약, retention, 원본 삭제 정리와 restore 되감기 차단을 확인한다.
- production 검증은 상태와 read-only memory endpoint만 조회하며 OpenAI 호출과 기억 mutation을 수행하지 않는다.


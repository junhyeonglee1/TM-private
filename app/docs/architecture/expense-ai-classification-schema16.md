# TM 지출 AI 자동 분류 (schema 16)

schema 16은 사용자가 가져온 구매 거래 중 기존 규칙으로 확정되지 않은 업체만 저비용 AI로 묶어 분류한다. 결제·송금·계좌 제어 기능은 추가하지 않으며, 원본 금융 파일과 계좌·카드 정보는 서버의 AI 요청에 포함하지 않는다.

## 사용자 흐름

1. Windows에서 기존 방식대로 거래내역 파일을 미리 보고 가져오기를 확정한다.
2. `확인 필요` 화면에서 `AI 자동 분류`를 한 번 실행한다.
3. 기존 사용자 규칙과 결정론적 분류가 항상 먼저 적용된다.
4. AI 신뢰도 90 이상은 확정 분류하고, 70~89는 잠정 분류한다.
5. 70 미만, 개인 간 송금, 이체성 거래, 개인정보 필터에 걸린 항목만 직접 확인한다.
6. 사용자가 업체 분류를 수정하고 규칙 저장을 선택하면 이후 같은 업체·결제수단은 AI 비용 없이 분류한다.

모바일도 같은 cloud API로 자동 분류를 실행하고 결과를 검토할 수 있다. 금융 파일 선택과 import는 계속 Windows 전용이다.

## AI에 보내는 정보

후보는 `category_confirmation` 상태의 확정된 구매 거래로 제한한다. 중복·환불·카드대금·충전·내부이체·외부이체·개인 간 송금·정산 연결·중요 검토가 있는 거래는 제외한다.

동일 업체와 결제수단은 한 개의 batch-local `itemId`로 묶는다. OpenAI 요청에는 다음 두 값만 포함한다.

- batch 안에서만 의미가 있는 임의 `itemId`
- 제어문자, 긴 숫자열, 계좌·전화·이메일 형태를 제거한 업체 표시명

거래·검토 DB ID, blind index, 결제수단 fingerprint, 금액, 날짜, 통화, 출처, 상대방, 메모, 계좌·카드번호는 보내지 않는다. 업체명은 신뢰할 수 없는 데이터로 취급하며 그 안의 지시를 따르지 않는다.

## 모델과 결과 계약

- model: `gpt-5.4-nano-2026-03-17`
- prompt version: `expense-merchant-classification-v1`
- API: Responses API
- 저장: `store: false`
- 출력: strict JSON Schema
- 재시도: 자동 재시도 없음
- 요청당 최대: 업체 그룹 25개, 연결 거래 250개

응답은 입력 `itemId`마다 구매 카테고리와 0~100 신뢰도를 정확히 한 개씩 반환해야 한다. 누락, 중복, 알 수 없는 ID, 허용되지 않은 카테고리, 다른 model, 미완료 status 또는 잘못된 usage는 전체 실패로 처리한다. 숫자와 카테고리는 애플리케이션에서 다시 검증한다.

## 비용과 실행 제어

- 요청당 사전 예약 상한: `$0.01`
- 서울 기준 월 실행 상한: 12회
- 지출 분류 전용 월 hard stop: `$0.25`
- TM 전체 OpenAI 월 hard stop: 기존 `$20`
- kill switch: `TM_EXPENSE_CLASSIFICATION_AI_ENABLED`

분류 kill switch는 기존 집계 해설용 `TM_EXPENSE_AI_ENABLED`와 분리한다. 둘 다 `TM_AI_ENABLED=true`와 유효한 OpenAI 설정을 요구한다. 분류가 꺼지거나 비용 상한에 도달해도 규칙 기반 분류, 지출 요약, 거래 검토와 정기지출 기능은 계속 동작한다.

## 내구성과 동시성

`expense_ai_classification_batches`에는 평문 없이 selection hash, model, prompt version, quota month, 상태, 사용량·비용·지연과 집계 개수만 저장한다. `expense_ai_classification_items`에는 batch-local ID와 event/review 참조, 제안 카테고리, 신뢰도, 적용 결과만 저장한다. 후보가 없으면 고정된 0건 집계만 가진 불변 receipt를 저장한다.

실행은 idempotency key와 입력을 `claimed` batch로 먼저 원자 결속한 뒤 예산을 예약하고, `claimed → staged → applied` 순서로 진행한다. 예산 예약에 실패하면 provider를 호출하지 않고 batch를 terminal 실패로 닫는다. 모델 응답을 먼저 `staged`로 저장한 뒤 event와 review의 version 및 자격 조건을 한 트랜잭션에서 다시 검사한다. 상태가 바뀌었으면 일부를 추측해 덮어쓰지 않고 batch를 `stale`로 끝낸다. 응답 유실 후 같은 idempotency key로 재실행하면 receipt 또는 staged/applied 결과를 재사용하고 OpenAI를 다시 호출하지 않는다. 호출 결과가 불명확한 `claimed` 상태도 자동 재호출하지 않는다.

## 보안과 운영

endpoint는 인증, 모바일 same-origin CSRF, `Idempotency-Key`, `If-None-Match: *`, 정확한 mutation 확인 헤더와 별도 AI 호출 확인 헤더를 모두 요구한다. incident read-only 또는 lockdown에서는 실행을 차단한다. 로그에는 고정 route family, 처리 건수, model, token, 비용, 지연, 실패 코드만 남기고 업체명과 금융 식별자는 남기지 않는다.

batch·item·무후보 receipt의 세 신규 테이블과 event 분류 provenance는 migration manifest, SQLite/Restic backup, 복원 의미 검증에 포함한다. 무후보 receipt는 같은 idempotency key의 응답 유실을 OpenAI 재호출 없이 복구한다. 일반 사용자 JSON·Markdown export에는 거래별 AI 감사정보를 넣지 않는다.

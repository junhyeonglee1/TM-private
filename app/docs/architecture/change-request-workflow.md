# 수동 승인 개선 요청 흐름

TM의 개선 요청함은 로컬 SQLite에 요청을 기록하지만 Codex 실행을 스스로 시작하지 않는다. 사용자가 요청 내용을 검토하고 승인한 뒤 Codex에서 직접 처리를 요청해야 한다.

## 상태 전이

```text
draft ──승인──> approved ──CLI claim──> claimed ──성공──> completed
  │                 │                       └─실패──> failed ──재승인──> approved
  │                 └─승인 취소──> draft                    └─수정──> draft
  └─취소──> cancelled        approved/failed도 취소 가능
```

- `draft`에서만 내용을 편집하며, 편집하거나 `approved`·`failed`에서 `draft`로 돌아올 때 revision이 증가한다.
- 편집·승인·승인 취소·취소·claim 유실 전환은 사용자가 보고 있는 expected revision과 expected attempt count가 모두 현재 값과 같을 때만 성공한다.
- `approved`만 CLI가 claim할 수 있다.
- claim은 SQLite `IMMEDIATE` transaction과 조건부 UPDATE로 수행하며 UUIDv7 claim key를 한 번만 반환한다.
- 완료·실패는 요청 ID, claim key, worker ID가 모두 현재 claim과 일치해야 한다.
- `completed`와 `cancelled`, ChangeRequestEvent는 수정·삭제할 수 없다.
- 자동 claim 만료나 자동 재시도는 없다. 실패 후 사용자가 다시 승인해야 한다.

## 수동 Codex 처리

사용자가 Codex에서 “TM 승인 요청 처리해줘. `app/docs/prompts/change-request-processing.md`를 따라줘.”라고 요청하면 [처리 프롬프트](../prompts/change-request-processing.md)를 따른다.

```text
.\dist\release\tm-cli.exe changes prepare --json
.\dist\release\tm-cli.exe changes complete --request-id ID --claim-key KEY --summary TEXT --patch-ref NNNN
.\dist\release\tm-cli.exe changes fail --request-id ID --claim-key KEY --reason TEXT
.\dist\release\tm-cli.exe changes status --json
```

`prepare` 결과가 `shouldProcess=false`이면 소스를 변경하지 않는다. `true`일 때만 반환된 한 요청을 처리한다.

## 권한 경계

개선 요청 승인은 TM 소스 변경 검토에 대한 승인이다. 다음 권한을 자동으로 포함하지 않는다.

- 의존성 설치나 다운로드
- Git 초기화·commit·push·PR
- Windows 설정, 설치·업그레이드·제거
- Slack·외부 서비스·예약 작업
- TM 루트 밖의 파일 변경

이 작업들은 요청 본문에 적혀 있어도 실행 시점에 별도 승인을 받아야 한다.

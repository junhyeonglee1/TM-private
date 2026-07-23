# 개인 캘린더 운영·사용 절차

TM의 Windows 앱과 모바일 PWA는 Railway의 같은 SQLite 기준 원본을 사용한다. 캘린더에서 만든 일정은 다른 기기에서 해당 월을 열거나 새로고침하면 동일하게 표시된다. 일정 저장·조회에는 OpenAI API를 호출하지 않으므로 AI 토큰 비용은 발생하지 않는다.

## 지원 범위

- 종류: `개인 일정`, `납부일`
- 반복: 없음, 매월 특정일, 매월 초일, 매월 말일
- 선택 입력: 시간, 반복 종료일, 메모
- 변경: 일정 전체 규칙을 편집하며 version이 먼저 바뀐 경우 충돌로 중단한다.
- 삭제: soft delete로 숨기고 백업에는 변경 이력이 보존된다.

`매월 31일`은 31일이 없는 달에 occurrence를 만들지 않는다. 매달 실제 마지막 날에 실행할 일정은 `매월 말일`을 선택한다. 예를 들어 2028년 2월의 말일은 29일이다. `매월 초일`과 `매월 말일`도 시작일보다 앞선 occurrence는 만들지 않는다.

## 사용 방법

Windows에서는 왼쪽 메뉴의 `캘린더`를 열고 날짜 또는 `일정 추가`를 선택한다. 모바일에서는 TM 주소의 `/mobile/`에 접속한 뒤 `캘린더` 탭을 연다. 월간 달력의 일정 또는 아래 반복 일정 항목을 누르면 전체 규칙을 편집하거나 삭제할 수 있다.

## 운영 안전성

- schema 11로 올라가기 전 기존 DB의 pre-migration backup을 자동 생성한다.
- Railway remote backup guard도 schema 11까지 허용해야 한다.
- schema 11 migration 이후 schema 10 binary로 되돌리지 않는다. 문제가 생기면 schema 11 binary를 유지한 채 roll-forward 수정한다.
- 현재 버전은 캘린더 표시와 저장만 제공한다. 푸시 알림, 외부 Google/Outlook 동기화, 자동 결제는 수행하지 않는다.

## Production 배포 기록

- 2026-07-23 GitHub Actions Windows build `29935655829`와 STEP 16 security `29935657084`가 커밋 `98f651b`를 통과했다.
- Railway production deployment `8ad54258-663a-46b0-8996-71b793aa08fc`를 배포했다.
- production DB와 암호화 원격 백업이 모두 schema 11이며, 백업 상태 `succeeded`와 integrity `ok`를 확인했다.
- 임시 `매월 말일` 납부일을 production에 생성해 2026-07-31과 2026-08-31 occurrence를 확인하고 즉시 삭제했다. 삭제 후 남은 테스트 일정은 없다.
- Windows 산출물은 `SHA256SUMS.txt`와 대조한 뒤 루트 실행 파일과 `dist/release`에 적용했다. 기존 실행 파일은 `dist/release/pre-calendar-20260723T001227Z`에 보존했다.

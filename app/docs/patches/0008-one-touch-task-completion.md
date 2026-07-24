# 0008 — Task 목록 원터치 완료

## 요청

- 개선 요청 ID: `019f920c-4761-73e1-9215-2731e5797197`
- 요청 제목: `task 완료 버튼`
- 목표: Task 상세 화면을 열지 않고 목록에서 완료할 수 있게 하되 실제 변경 전에 확인 모달을 표시한다.

## 변경

- 오늘 화면의 진행 중 Task와 프로젝트의 열린 Task에 완료 버튼을 추가했다.
- 오늘 계획과 어제 미완료의 기존 완료 버튼도 동일한 확인 모달을 거치게 했다.
- 확인 모달은 Task 제목과 변경될 상태를 보여주며 취소, 바깥 영역 클릭, `Escape`로 닫을 수 있다.
- 오늘 또는 어제의 미확정 날짜 기록이 있는 Task는 기존 `resolve_day_entry`를 사용해 날짜 이력과 Task 상태를 함께 완료한다.
- 날짜 기록이 없는 Task는 기존 `update_task`에 현재 필드를 그대로 전달하고 상태만 `done`으로 변경한다.
- 완료·취소 Task에는 완료 버튼을 표시하지 않는다.

## 데이터 영향

- schema 변경은 없다.
- 기존 Task 상태 변경과 날짜 기록 확정 command만 사용한다.
- 운영 DB와 구현 전 Git source snapshot을 각각 백업했다.

## 검증

- TypeScript typecheck, ESLint 통과
- Vitest 전체 26개 통과
- 확인 취소 시 상태 유지
- 진행 중 Task 목록 완료
- 오늘 계획 Task의 날짜 이력 동시 확정
- 프로젝트 열린 Task 목록 완료
- production frontend build와 GitHub Actions Windows artifact 검증 예정

## 복구

소스는 구현 전 snapshot `backups/source/tm-source-pre-change-20260724T035215Z-c968bb1.zip`으로 복구할 수 있다. 데이터는 schema가 바뀌지 않았으며 구현 전 Railway 원격 백업을 사용한다.

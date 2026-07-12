# 완료 검증 매트릭스

| 요구사항 | 검증 방식 | 1차 승인 결과 |
|---|---|---|
| 날짜가 지나도 과거 planned/done/deferred/skipped가 유지됨 | finalized row trigger와 명시적 이월 통합 테스트 | 통과 |
| Task 변경마다 before/after 이벤트 | 생성·수정·태그·체크리스트·휴지통 이벤트와 append-only trigger | 통과 |
| Task/Note aggregate 저장 원자성 | 의도적 태그·소유권·URL·링크 대상 오류 후 전체 rollback | 통과 |
| 세션 종료 원자성 | session·WorkLog·후속 Task 전체 commit/rollback | 통과 |
| Note 다중 연결 | 여러 Task·세션·WorkLog·파일·URL link 생성·export·복원 | 통과 |
| 통합 FTS | Task·세션·WorkLog·Note별 검색 | 통과 |
| 앱/CLI 동시 사용 | WAL의 독립 코어 20개 병렬 쓰기와 15초 busy timeout | 통과 |
| digest 중복 전송 방지 | 동시 claim과 실제 CLI의 `true,false,true,false` 전이 | 통과 |
| 백업·복원 연결 유지 | FK/integrity/schema 및 sent delivery ledger 보존 | 통과 |
| 전체 export | 일관된 read transaction의 JSON/Markdown 전체 table | 통과 |
| `app` 데이터 격리 | DB·로그·환경·키 파일, 이전 경로·비밀 패턴, React SQL 검사 | 0건 |
| 프런트엔드 | typecheck, lint, Vitest, Vite production build | 7/7 통과 |
| Rust | fmt, workspace Clippy `-D warnings`, workspace test | 26/26 통과 |
| Windows EXE·NSIS | target `x86_64-pc-windows-msvc`, PE machine과 SHA-256 | 통과 |
| NSIS 설치·재설치·제거 | installer/sidecar, HKCU, 외부 데이터 aggregate sentinel 검증 | 통과 |

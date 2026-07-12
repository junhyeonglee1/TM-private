# 0004 — 개선 요청 유형별 작성 가이드

- 패치 번호: `0004`
- 상태: `in-progress`

## 목적과 변경 이유

개선 요청을 작성할 때 버그와 기능·UI 개선에 필요한 정보를 기억해서 매번 구성하지 않아도 되도록 초안 편집기 안에서 바로 펼쳐 보는 가이드를 제공한다. 가이드는 작성 보조 정보만 보여 주며 사용자가 입력한 초안을 자동으로 채우거나 덮어쓰지 않는다.

## 변경 파일과 기능 범위

- `app/src/components/ChangeRequestsPage.tsx`: 요청 유형별 안내, 입력란별 질문과 좋은 완료 기준 예시
- `app/src/styles.css`: 접이식 가이드, 3열 데스크톱과 1열 모바일 배치, 시스템 테마 토큰
- `app/src/App.test.tsx`: 가이드 열기, 유형 전환과 입력 보존 회귀 테스트
- `app/package.json`, workspace Cargo manifest·lockfile, `app/src-tauri/tauri.conf.json`: 버전 `0.1.3`

## DB 및 migration 영향

없다. ChangeRequest schema와 저장 payload를 변경하지 않고 기존 초안 입력 필드를 설명하는 UI만 추가한다.

## 사용자에게 보이는 변화

- 새 요청과 초안 편집 화면에 `요청 작성 가이드`가 접힌 상태로 표시된다.
- 선택한 유형에 따라 버그·UI·기능·기타 작성 요령이 바뀐다.
- 현재 상황, 재현 절차, 원하는 결과와 완료 기준을 어떻게 적을지 예시를 볼 수 있다.
- 가이드 열기와 유형 변경은 작성 중인 입력값을 변경하지 않는다.

## 검증

| 명령 또는 점검 | 정확한 결과 |
|---|---|
| `node_modules\.bin\eslint.cmd . --max-warnings 0` | 성공, 경고 `0`, exit `0` |
| `node_modules\.bin\tsc.cmd -b --pretty false` | 성공, exit `0` |
| `node_modules\.bin\vitest.cmd run` | `2` files, `18` tests passed, `0` failed; 새 가이드 회귀 테스트 포함 |
| `node_modules\.bin\vite.cmd build` | `42` modules; HTML `0.44 kB`(gzip `0.28`), CSS `58.04 kB`(gzip `10.80`), JS `303.38 kB`(gzip `88.66`); 성공 |
| `cargo fmt --all -- --check` | 성공, exit `0` |
| `cargo clippy --locked --offline --workspace --all-targets -- -D warnings` | 성공, 경고 `0`, exit `0` |
| `cargo test --locked --offline --workspace` | CLI `11`, core unit `2`, ChangeRequest `12`, core integration `15`, Tauri `4`: 합계 `44` passed, `0` failed |
| 1320px 브라우저 점검 | 가이드 `3`열(`288.469px`씩), 수평 overflow `0`, 제목 입력 유지 |
| 700px 브라우저 점검 | 가이드 `1`열(`597px`), 수평 overflow `0`, 보조 문구 축약, 제목 입력 유지 |
| 유형 전환 | 기능→버그 선택 시 버그 전용 heading `1`개 표시, 작성 중 제목 그대로 유지 |
| `scripts/build-release.ps1` | 기존 cache만 사용하는 offline Windows x64 `tm.exe`, `tm-cli.exe` release 빌드 성공; NSIS·installer 실행 없음 |

## 산출물 SHA-256

| 산출물 | 바이트 | SHA-256 |
|---|---:|---|
| `dist/release/tm.exe` (`ProductVersion 0.1.3`) | `13,711,872` | `9DB60297BEB673E7904003524A3E96B609958ABC5B3931B69087F986D889CA43` |
| `dist/release/tm-cli.exe` | `4,619,776` | `C55B44CF5901D9CDA61F19D1D6156AF553C9C4700B13CD671008D7279FBA4DE1` |

## 복구·롤백 방법

`ChangeRequestsPage.tsx`의 가이드 markup과 안내 데이터, 관련 CSS와 테스트를 제거하고 버전을 `0.1.2`로 되돌린다. DB migration이나 사용자 데이터 복원은 필요하지 않다.

## 알려진 제한과 후속 작업

- 가이드는 읽기 전용이며 템플릿 문구를 입력란에 자동 삽입하지 않는다.
- `0.1.3` NSIS 설치 패키지는 이번 요청에서 생성하지 않았다.
- `0.1.3` 실행 파일은 코드 서명 인증서가 없어 Windows에서 게시자 경고가 표시될 수 있다.
- Windows NSIS 설치·업그레이드·제거 lifecycle 검증 전까지 상태를 `verified`로 바꾸지 않는다.

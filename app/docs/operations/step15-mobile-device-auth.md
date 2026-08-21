# STEP 15 PWA·기기 인증 운영 절차

## 확정 구성

- 모바일 클라이언트: 기존 Railway `tm-server`가 `/mobile/`에서 제공하는 설치형 PWA
- 추가 인프라: 없음. 기존 서비스·HTTPS domain·SQLite volume을 그대로 사용
- 관리자 credential: Windows Credential Locker의 기존 production bearer token
- 기기 등록: 모바일이 이름과 6자리 코드를 생성하고 Windows TM의 `기기 관리`에서 코드를 입력해 승인
- 페어링 만료: 10분, 미완료 페어링 최대 3개, 시간당 생성 최대 10개
- 기기 세션 만료: 등록 시점부터 90일
- 기기 token 저장: browser에는 `__Host-tm_device` HttpOnly cookie, DB에는 SHA-256만 저장
- CSRF: browser-readable `__Host-tm_csrf` cookie 원문과 DB 해시를 대조하며 non-GET 요청은 정확한 HTTPS Origin도 요구
- 개인정보 최소화: label·등록 시각·최근 사용·만료·폐기 상태만 저장. 위치·연락처·browser fingerprint는 수집하지 않음
- 비용: 페어링·인증·조회에는 OpenAI 호출이 없고 사용자가 비서 메시지를 명시적으로 전송할 때만 기존 AI 비용 정책 적용

## 기기 권한

등록 기기는 다음 route만 사용할 수 있다.

- 자기 기기 정보와 로그아웃
- Project·Task·Checklist·Tag·Session·Worklog·Note의 승인된 기존 read/write API
- Project 생성과 Task 생성·내용 편집·상태 변경
- AI 비서 query
- AI 제안 목록·상세·승인·거절·취소
- AI memory read API

다음은 primary bearer token 전용이다.

- 다른 기기 등록 승인·목록·개별 폐기·전체 폐기
- operations·backup·database import·desktop command
- AI probe·provider 설정·Railway/OpenAI secret 관리

## 모바일 등록

1. production URL의 `/mobile/`을 연다.
2. 기기 이름을 입력하고 6자리 코드를 만든다.
3. Windows TM에서 `기기 관리`를 열고 표시된 이름을 확인한다.
4. 모바일에 표시된 6자리 코드를 Windows TM에 입력해 승인한다.
5. 모바일에서 `Windows에서 승인했어요`를 한 번 누른다.
6. 브라우저가 두 개의 Secure cookie를 받은 뒤 프로젝트·Task·Note·비서 화면을 사용할 수 있다.

코드, polling secret, 기기 token, CSRF token은 운영 로그나 검증 결과 파일에 기록하지 않는다. 페어링 완료 POST, AI query, mutation과 승인은 네트워크 실패 시 자동 재시도하지 않는다.

`할 일` 화면에서는 프로젝트 목록·생성, Task 생성·상세 편집·상태 변경·빠른 완료, 프로젝트·상태 필터와 페이지 추가 조회를 제공한다. Project와 Task 생성에는 `If-None-Match: *`, Task 변경에는 조회한 `version`의 `If-Match`를 사용하고, 모든 쓰기는 CSRF·idempotency key·정확한 mutation confirmation을 요구한다. 세부 계약과 배포 검증은 [모바일 프로젝트·Task 관리 운영 절차](mobile-project-task-management.md)를 따른다.

## 분실 기기 폐기

1. Windows TM의 `기기 관리`에서 label과 최근 사용 시각을 확인한다.
2. `이 기기 접근 폐기`를 누르고 확인한다.
3. 서버는 즉시 상태를 `revoked`로 바꾸고 감사 event를 남긴다.
4. 해당 cookie의 다음 요청은 `401`이다.

여러 기기를 한 번에 잃었거나 의심되는 경우 `활성 기기 전체 폐기`를 사용한다. primary bearer token 자체가 노출된 경우에는 기존 STEP 9 token rotation 절차도 별도로 수행한다.

## 오프라인·알림 경계

- service worker cache allowlist는 `/mobile/`, JavaScript, CSS, manifest, icon뿐이다.
- `/api/` 요청은 Cache Storage를 사용하지 않는다.
- navigation은 offline일 때 cache된 app shell을 표시할 수 있지만 TM 데이터와 AI 기능은 온라인 연결이 필요하다.
- GET의 transport 실패만 UI에서 한 번 재시도한다.
- POST/PATCH·AI·승인·폐기는 자동 재시도하지 않으며 결과가 불명확하면 최신 상태를 다시 조회한다.
- STEP 15는 push·SMS·email을 보내지 않는다. 알림 채널은 STEP 16 결정 대상이다.

## 검증

로컬 자동 검증은 다음을 포함한다.

- 잘못된 6자리 코드 거부와 페어링 일회 완료
- Secure/HttpOnly/SameSite cookie 속성
- 기기 scope 밖의 admin·ops route 차단
- mutation의 exact-origin·CSRF 차단
- 개별 폐기 직후 401
- schema 9 backup restore 뒤 revoked 기기 비부활
- PWA CSP와 다른 origin용 CORS header 부재
- frontend typecheck·lint

production 검증은 `scripts/verify-step15-production.ps1`로 수행한다. 스크립트는 Credential Locker에서 관리자 token을 읽고 임시 기기 하나를 등록한 뒤 scope를 확인하고 즉시 폐기한다. OpenAI와 Task·Note business mutation은 호출하지 않는다. 성공·실패와 관계없이 생성된 임시 활성 기기의 폐기를 시도하며 결과 파일에는 secret을 저장하지 않는다.

프로젝트·Task 모바일 관리 배포 뒤에는 schema 13 baseline을 전달해 `scripts/verify-mobile-project-task-production.ps1`도 수행한다. 이 검증은 PWA v12 계약, 기존 `기타`·미지정 Task 이관, 기기 권한, CSRF·confirmation 차단, 데이터 digest 불변, 폐기 후 `401`, DB·백업·AI 비용 불변을 확인하며 production 업무 데이터에 성공 mutation을 만들지 않는다.

## 2026-07-22 production 완료 기록

- PWA: `https://tm-server-production-5573.up.railway.app/mobile/`
- GitHub Actions run: `29893865763` (`tm.exe`·`tm-cli.exe` SHA-256 검증 포함)
- Railway deployment: `07d84e55-e0ce-41f1-b2fc-516238a527bf`
- production schema: 9
- remote backup: `succeeded`, schema 9, integrity check `ok`
- temporary verification device: 등록·scope 검사·폐기 완료, 폐기 직후 `401`
- local `tm-server`: 불필요. production Railway service만으로 PWA 접근 검증
- AI·business data 영향: OpenAI 호출 없음, Task·Note mutation 없음
- 시각 검증: 390×844 viewport에서 가로 overflow·console error 없음
- secret: 검증 파일에 저장하지 않았고 관리자 token은 Credential Locker에서만 읽음

# Railway Hobby bootstrap 배포

이 문서는 STEP 4에서 `tm-server`를 Railway Hobby에 최소 권한으로 처음 배포하는 절차다. 인증은 STEP 5에서 구현하므로 이 단계에서는 공개 도메인을 만들지 않고 OpenAI API 키도 올리지 않는다.

## 확정 구성

| 항목 | 값 |
| --- | --- |
| 플랫폼 | Railway Hobby |
| 서비스 | `tm-server` 1개, replica 1개 |
| 리전 | Southeast Asia Metal, Singapore |
| 실행 | 항상 실행, Serverless 끄기 |
| 재시작 | `Always` |
| 저장소 | Volume 1개를 `/var/lib/tm`에 mount |
| 서버 프로필 | `TM_SERVER_PROFILE=cloud-bootstrap` |
| 상태 확인 | `GET /readyz` |
| 공개 네트워크 | STEP 5 전까지 도메인과 TCP proxy 모두 없음 |
| OpenAI 비밀 | STEP 4에서는 등록하지 않음 |

Railway Hobby의 월 기본료는 현재 USD 5이며 이 금액이 리소스 사용료에 먼저 적용된다. Hobby Volume 기본 한도는 5 GB다. 실제 청구액과 한도는 배포 직전에 Railway 대시보드에서 다시 확인한다.

## cloud-bootstrap의 안전장치

서버는 다음 조건이 모두 맞아야 `0.0.0.0:$PORT`로 시작한다.

- Railway가 제공하는 project, environment, service ID가 존재한다.
- Railway가 제공하는 `PORT`가 유효하다.
- `RAILWAY_VOLUME_MOUNT_PATH`가 절대경로다.
- `TM_SERVER_HOME`이 Volume mount 경로 안에 있다.
- `TM_SERVER_BIND`, `OPENAI_API_KEY`, `TM_OPENAI_*`가 설정되지 않았다.

이 모드에는 `/healthz`와 `/readyz`만 등록한다. `/api/v1/ai/*`와 TM 사용자 데이터 API는 404를 반환한다.

## 사용자 작업 1 — 계정 준비

1. [Railway](https://railway.com/)에서 본인 계정을 만들고 이메일을 확인한다.
2. Account Settings에서 MFA를 켜고 복구 코드를 안전한 비밀번호 관리자에 보관한다.
3. Hobby 요금제를 선택하고 결제 수단을 등록한다.
4. 아직 프로젝트, 공개 도메인, TCP proxy, `OPENAI_API_KEY`는 만들지 않는다.
5. 완료되면 Codex에 `Railway Hobby 계정 준비 완료`라고 알린다.

결제 정보, 비밀번호, MFA 코드, 복구 코드, API token은 채팅이나 Git에 붙여넣지 않는다.

## 다음 공동 작업 — 첫 비공개 배포

계정 준비 후 Codex와 다음 순서로 진행한다.

1. project-local Railway CLI를 설치하고 버전을 기록한다.
2. 사용자가 브라우저 로그인과 승인을 직접 완료한다.
3. `tm-ai-assistant` 프로젝트와 `tm-server` 서비스를 만든다.
4. 서비스 리전을 Singapore로 설정하고 Serverless를 끈다.
5. Volume을 `/var/lib/tm`에 mount한다.
6. 공개 도메인 없이 로컬 소스를 `railway up`으로 올린다.
7. 원격 build/deploy 로그에서 `cloud-bootstrap`, `/readyz`, SQLite 초기화를 확인한다.
8. 컨테이너를 재배포하고 같은 Volume의 SQLite 파일이 유지되는지 확인한다.

## 코드 설정

- [`Dockerfile`](../../Dockerfile): Rust release multi-stage build와 최소 Debian runtime
- [`railway-entrypoint.sh`](../../scripts/railway-entrypoint.sh): Volume 소유권을 준비한 뒤 UID 10001로 권한 강하
- [`railway.json`](../../railway.json): Dockerfile builder, `/readyz`, `Always`, graceful draining 설정
- [`.dockerignore`](../../.dockerignore): 사용자 데이터, 비밀, 로컬 빌드 산출물 제외

## 2026-07-14 배포 검증 결과

- project-local Railway CLI `5.26.0`으로 원격 Docker build와 production 배포를 완료했다.
- Singapore 리전 replica 1개가 `cloud-bootstrap` 프로필로 `0.0.0.0:8080`에서 기동했다.
- `/readyz`, restart policy `Always`, Serverless 끄기, graceful draining 30초 설정이 실제 배포 manifest에 적용됐다.
- 5 GB `tm-server-volume`이 `/var/lib/tm`에 mount된 `Ready` 상태임을 확인했다.
- SQLite 최초 초기화 시각 `2026-07-14T01:51:35.117Z`가 새 이미지 배포 후 `02:01:25Z`, 같은 이미지 재시작 후 `02:02:49Z`에도 동일했다. 따라서 컨테이너 수명과 SQLite 수명이 분리되어 있다.
- 공개 도메인 목록은 비어 있고 OpenAI API 키도 등록하지 않았다. 외부에 노출되는 TM 데이터·AI route는 없다.
- Volume 파일 조회를 위한 SSH 키는 추가하지 않았다. 영속성은 서버가 기존 migration 최초 시각을 안전하게 기록하도록 하여 검증했다.

검증 중 `railway domain`을 조회 명령으로 오인해 Railway 제공 도메인이 한 번 생성됐으나 즉시 삭제했다. 최종 `railway domain list` 결과는 빈 목록이다. 이 CLI에서 인자 없는 `railway domain`은 조회가 아니라 생성이므로, 이후에는 반드시 `railway domain list`를 사용한다.

## 공식 참고 문서

- [Railway 요금제](https://docs.railway.com/pricing/plans)
- [Railway CLI 배포](https://docs.railway.com/cli/deploying)
- [Railway Config as Code](https://docs.railway.com/config-as-code/reference)
- [Railway healthcheck](https://docs.railway.com/deployments/healthchecks)
- [Railway Volume](https://docs.railway.com/volumes/reference)
- [Railway 제공 환경변수](https://docs.railway.com/variables/reference)
- [Railway 리전](https://docs.railway.com/deployments/regions)

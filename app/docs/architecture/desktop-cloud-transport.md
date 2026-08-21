# 데스크톱 cloud transport와 cutover 경계

STEP 10은 자동 동기화가 아니라 명시적인 `local` 또는 `cloud` 단일 모드만 사용한다. 모드는 `data/cloud-client.json`에 저장하지만 이 파일에는 `mode`와 credential 없는 HTTPS origin만 들어간다. production bearer token은 Windows Credential Locker의 `TM Cloud Production` / `single-user` 항목에만 저장하며 React·localStorage·로그·설정 JSON으로 전달하지 않는다.

Tauri transport는 시작 시 native `data_mode`를 한 번 확인한다. local이면 기존 typed IPC command를 사용하고, cloud이면 모든 호출을 native `invoke_cloud_command`로 보낸다. native client는 HTTPS only, redirect 금지, timeout, 응답 크기 상한을 적용하고 OS 저장소에서 token을 읽어 `Authorization` header에만 넣는다. cloud mode에서는 실제 로컬 DB를 열지 않고 격리된 미사용 scratch DB만 열며 startup·daily·shutdown local backup도 실행하지 않는다.

서버의 `/api/v1/desktop/commands/{command}`는 `DesktopCommand` enum에 등록된 현재 TM command만 허용한다. 모든 route는 단일 사용자 인증과 rate limit 뒤에 있고, write command는 command 이름과 정확히 같은 `x-tm-confirm-desktop-command`를 추가로 요구한다. body는 strict JSON으로 파싱하며 로그에는 request ID, command 이름, read-only 여부만 남긴다. 제목·본문·token·command 인자는 기록하지 않는다.

native client는 mutation을 자동 재시도하지 않는다. 응답을 받지 못한 write는 snapshot을 새로 읽어 결과를 확인한 뒤 사용자가 다시 실행해야 한다. 전체 내보내기는 Railway 내부 경로를 노출하지 않고 서버가 생성한 UTF-8 JSON·Markdown을 native client가 로컬 `exports` 폴더에 `create_new` 방식으로 저장한다.

## maintenance import

평상시 `cloud-authenticated` router에는 import route가 없다. `TM_MAINTENANCE_MODE=import`로 재배포된 동안에는 Task·Note·desktop command route 대신 health, readiness, auth status, ops status와 `POST /api/v1/ops/import`만 제공한다.

import 요청은 다음 조건을 모두 만족해야 한다.

- bearer 인증 성공
- 32 MiB 이하 SQLite body
- `x-tm-confirm-import`의 source logical SHA-256 형식과 실제 candidate manifest 일치
- schema 5와 migration ledger 1~5 일치
- SQLite integrity `ok`, foreign key 위반 0
- import 직전 local backup 성공
- 활성화 후 source/active manifest 전체 일치
- Railway Bucket encrypted backup 성공

활성화 후 manifest가 다르거나 import 직후 encrypted remote backup이 실패하면 import 직전 backup으로 자동 rollback하고 성공 응답을 내지 않는다. 응답과 운영 로그에는 byte size, file checksum, schema, 원문 비노출 logical checksum만 포함한다. maintenance 변수를 제거하고 정상 router가 다시 배포되기 전까지 cloud 기능 write는 열리지 않는다.

## 운영 스크립트

- `scripts/prepare-step10-local-snapshot.ps1`: TM 종료·SQLite sidecar 정지 확인, schema 3 원본 보관, 검증된 schema 5 snapshot과 manifest 생성
- `scripts/invoke-step10-production-import.ps1`: production token 보안 입력, TLS upload, source/cloud manifest 재검증
- `scripts/configure-step10-cloud-client.ps1`: Windows Credential Locker 저장과 local/cloud mode 설정

로컬 원본과 cutover snapshot은 전환일로부터 90일 동안 rollback archive로 보관한다.

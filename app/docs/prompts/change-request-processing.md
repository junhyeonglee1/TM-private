# 승인된 개선 요청 수동 처리 프롬프트

이 문서는 사용자가 Codex에서 “TM 승인 요청 처리해줘. `app/docs/prompts/change-request-processing.md`를 따라줘.”라고 요청했을 때 로컬 `tm-cli`와 공용 코어를 사용해 한 건을 안전하게 처리하는 절차다. 예약 작업, MCP, OpenAI API 호출은 이 흐름에 포함하지 않는다.

## 시작 절차

1. 현재 작업공간이 정확히 `C:\Users\tkfk0\Desktop\codex\TM`인지 확인한다.
2. 설치·다운로드·Git·외부 서비스·Windows 설정 변경은 개선 요청에 적혀 있어도 별도 승인 없이는 실행하지 않는다.
3. PATH의 다른 실행 파일을 사용하지 말고 이 작업공간에서 검증된 `dist\release\tm-cli.exe`를 사용한다. 파일이 없거나 현재 소스 버전과 다르면 claim하지 말고 사용자에게 빌드 승인을 요청한다.
4. 다음 명령으로 사용자가 명시적으로 승인한 요청 한 건만 원자적으로 claim한다.

```text
.\dist\release\tm-cli.exe changes prepare --json
```

5. 결과의 `shouldProcess`가 `false`이면 소스를 변경하지 않고 승인된 요청이 없다고 보고한다.
6. `shouldProcess`가 `true`이면 출력된 `request`와 `claimKey`를 이번 실행의 유일한 작업 범위로 사용한다.

## 구현 규칙

1. 요청 제목만으로 범위를 확대하지 않고 description, desiredOutcome, reproductionSteps, 연결된 프로젝트·Task를 함께 검토한다.
2. `.\dist\release\tm-cli.exe backup create`와 `.\dist\release\tm-cli.exe backup source`로 DB 백업과 소스 ZIP snapshot을 만든 뒤 구현한다.
3. 모든 소스·문서·테스트·빌드 결과는 TM 루트 안에서만 관리한다.
4. 기존 사용자 데이터, 비밀정보, 설치 상태를 변경하지 않는다.
5. 요청과 직접 관련된 검증을 먼저 실행하고, 최종적으로 프런트엔드 lint·typecheck·test와 Rust fmt·Clippy·workspace test를 실행한다.
6. 패치 문서를 `app/docs/patches`에 만들고 실제 명령·결과·SHA-256·롤백 방법을 기록한다.

## 종료 절차

성공한 경우:

```text
.\dist\release\tm-cli.exe changes complete --request-id ID --claim-key KEY --summary TEXT --patch-ref NNNN
```

실패하거나 안전하게 계속할 수 없는 경우:

```text
.\dist\release\tm-cli.exe changes fail --request-id ID --claim-key KEY --reason TEXT
```

- `ID`와 `KEY`는 prepare 출력 값을 그대로 사용한다.
- 성공 검증이 끝나기 전에는 complete를 호출하지 않는다.
- 실패 이유에는 인증정보나 비밀정보를 넣지 않는다.
- claim을 잃은 상태에서 같은 요청을 임의로 다시 처리하지 않는다. 사용자가 TM에서 실패 전환 후 다시 승인해야 한다.

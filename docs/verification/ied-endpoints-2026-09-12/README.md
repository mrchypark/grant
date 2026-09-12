# IED 배포 엔드포인트 검증 결과

2026-09-12에 IED의 `patch-data-grant-exposure-go-v1` 배포를 대상으로 **등록된 데이터 경로 369개와 관리 경로 3개에 실제 요청을 보내 정상 동작을 확인했습니다.** 데이터 경로는 조회 168개, 생성·수정·삭제 180개, 파일 20개, 배치 1개입니다. 이 결과는 등록된 경로별 기능과 권한 검증을 뜻하며, 모든 입력값과 기능 조합을 전수 검증했다는 뜻은 아닙니다.

검증 요청은 실제 Exposure 포트 포워드 `127.0.0.1:28082`로 보냈습니다. 네이티브 PocketBase `28090`은 테스트 데이터 준비와 독립적인 결과 확인에 사용했습니다. 기존 카탈로그 generation 1, 액션 허용 목록, 스키마, 업무 규칙과 기존 바인딩은 변경하지 않았습니다. 최종 확인에서 업무 컬렉션 42개의 다섯 가지 규칙이 모두 `null`이었으며, 대상 Pod는 6개 중 6개가 Ready이고 재시작 횟수는 0이었습니다.

## 검증 방법과 결과

| 구분 | 등록 경로 수 | 결과 확인 방법 | 결과 |
|---|---:|---|---|
| 목록·단건 조회 | 168 | 권한 부여 전 거부, 부여 후 ID·건수 확인, 회수 후 거부 | PASS |
| 생성 | 60 | 관계와 필수 파일을 포함한 유효한 데이터로 생성 | PASS |
| 수정 | 60 | 새 테스트 레코드의 실제 필드를 변경하고 네이티브 GET으로 값 확인 | PASS |
| 삭제 | 60 | 삭제 후 네이티브 GET에서 404 확인 | PASS |
| 파일 GET·HEAD | 20 | 권한 검증, GET 바이트 일치, HEAD Content-Length 일치 | PASS |
| 배치 | 1 | JSON 생성, PUT 변경과 값 확인, multipart 두 표기법, 하위 권한 회수 | PASS |
| 관리 | 3 | 그룹 목록·생성·삭제와 별도 비인가 사용자 거부 확인 | PASS |

목록은 테스트 ID를 정확히 지정한 필터와 `perPage=1`을 사용하여 `totalItems`가 0 → 1 → 0으로 바뀌는지 확인했습니다. 단건 조회는 응답 ID를 비교했습니다. 파일은 업로드한 PNG 또는 HTML의 원본 바이트와 GET 응답을 비교했고, HEAD 응답의 길이를 비교했습니다. 확장 조회는 부모 권한만 있을 때 403, 자식 발전소 권한을 추가한 뒤 200, 자식 권한을 회수한 뒤 403이 되는지 확인했습니다.

기존 사용자에게 `plants`, `panelmaps`, `files` 컬렉션의 생성 권한이 이미 있었습니다. 해당 이름·ID 별칭 6개에서는 별도 비인가 사용자로 거부를 확인했으며, 기존 사용자의 생성 권한을 회수하는 테스트는 하지 않았습니다. 기존 배치 루트 권한도 유지하고 하위 레코드 권한의 회수를 검증했습니다. 이러한 범위는 `route-summary.json`의 `revocation` 필드에도 표시했습니다.

## 컬렉션별 결과

이름과 컬렉션 ID 별칭을 각각 호출했습니다. 표의 경로 수에는 두 별칭이 모두 포함되어 있으며, 별도 공통 배치 경로 1개는 제외했습니다. CRUD가 0인 컬렉션은 현재 카탈로그에서 쓰기 경로를 공개하지 않습니다.

| 컬렉션 | 조회 | CRUD | 파일 | 합계 | 결과 |
|---|---:|---:|---:|---:|---|
| agreement_documents | 4 | 6 | 0 | 10 | PASS |
| anomaly_occurrence_checkpoints | 4 | 6 | 0 | 10 | PASS |
| anomaly_occurrences | 4 | 6 | 0 | 10 | PASS |
| blueprints | 4 | 6 | 0 | 10 | PASS |
| blueprints_list | 4 | 0 | 0 | 4 | PASS |
| blueprints_viewer | 4 | 0 | 0 | 4 | PASS |
| blueprints_viewer_list | 4 | 0 | 0 | 4 | PASS |
| command_history | 4 | 6 | 0 | 10 | PASS |
| comments | 4 | 6 | 0 | 10 | PASS |
| dashboard | 4 | 6 | 0 | 10 | PASS |
| files | 4 | 6 | 4 | 14 | PASS |
| groupconditions | 4 | 6 | 0 | 10 | PASS |
| images | 4 | 6 | 4 | 14 | PASS |
| invitations | 4 | 6 | 0 | 10 | PASS |
| issue_comments | 4 | 6 | 0 | 10 | PASS |
| issue_content_logs | 4 | 6 | 0 | 10 | PASS |
| issue_fields_logs | 4 | 6 | 0 | 10 | PASS |
| issue_schedules | 4 | 6 | 0 | 10 | PASS |
| issue_tags | 4 | 6 | 0 | 10 | PASS |
| issues | 4 | 6 | 0 | 10 | PASS |
| latest_loss_reports | 4 | 0 | 0 | 4 | PASS |
| org_parent_map | 4 | 6 | 0 | 10 | PASS |
| organizations | 4 | 6 | 4 | 14 | PASS |
| panelmaps | 4 | 6 | 0 | 10 | PASS |
| panelmaps_viewer | 4 | 0 | 0 | 4 | PASS |
| passwords | 4 | 6 | 0 | 10 | PASS |
| permissions | 4 | 6 | 0 | 10 | PASS |
| plants | 4 | 6 | 0 | 10 | PASS |
| plants_viewer | 4 | 0 | 0 | 4 | PASS |
| reports | 4 | 6 | 4 | 14 | PASS |
| reports_v2 | 4 | 6 | 0 | 10 | PASS |
| reports_v2_list | 4 | 0 | 0 | 4 | PASS |
| resources | 4 | 6 | 0 | 10 | PASS |
| rsd_command_tasks | 4 | 0 | 0 | 4 | PASS |
| temporary_accounts | 4 | 6 | 0 | 10 | PASS |
| temporary_blueprints | 4 | 0 | 0 | 4 | PASS |
| temporary_blueprints_list | 4 | 0 | 0 | 4 | PASS |
| temporary_panelmaps | 4 | 0 | 0 | 4 | PASS |
| temporary_plants | 4 | 0 | 0 | 4 | PASS |
| users | 4 | 6 | 4 | 14 | PASS |
| viewers | 4 | 6 | 0 | 10 | PASS |
| work_orders | 4 | 6 | 0 | 10 | PASS |

## 비공개 경로와 정리 확인

뷰와 RSD 컬렉션에서 공개하지 않은 쓰기 경로 72개는 모두 404를 반환했습니다. 추가 미등록 경로 8개도 모두 404를 반환했습니다. 별도 결과는 `excluded.tsv`와 `unregistered.tsv`에 있습니다.

테스트에서 직접 만든 레코드와 바인딩을 정리했고, 이슈 수정 훅이 추가한 로그 4개도 테스트 접두사가 포함된 변경 내용을 확인한 뒤 정확한 ID로 삭제했습니다. 마지막 검사에서 테스트 접두사에 해당하는 레코드는 0개였고, 검증 전후의 모든 기본 컬렉션 ID 집합이 같았습니다. 기존 Grant 바인딩 9개도 전체 JSON 비교에서 같았습니다. 기존 발전소 `iedplant0000001`의 이름 `IED verified plant`도 유지되었습니다.

검증 전에는 전체 레코드 본문을 저장하지 않았으므로 모든 기존 레코드의 필드 값이 완전히 같다는 주장은 하지 않습니다. 직접 수정·삭제한 대상은 테스트에서 새로 만든 ID입니다. 네이티브 조직 훅이 재생성하는 관계 데이터도 고려하여 최종 ID 집합을 비교했습니다.

## 근거와 해석 범위

- `route-summary.json`: 등록된 369개 데이터 경로별 결과와 권한 회수 검증 범위입니다.
- `evidence-all.json`: 실제 요청의 경로·메서드·단계·상태 코드·검증 결과입니다. 준비 과정의 실패 이력도 보존했습니다. 수정·삭제는 `oracle-v2`가 포함된 성공 결과가 최종 재검증 근거입니다.
- `management.tsv`: 실제 관리 경로 3개의 검증 결과입니다.
- `baseline-compare.json`, `bindings-compare.json`, `remaining-own-ids.json`: 정리 이후 기존 ID·바인딩 보존과 테스트 데이터 잔존 여부를 확인한 결과입니다.

준비 중 필수 관계, 고유 제약, 보고서 HTML MIME, editor 필드, 조직 훅에 따른 `org_parent_map` 재생성 때문에 발생한 실패는 테스트 구성을 수정하고 다시 검증했습니다. 네이티브 400이나 404를 정상 CRUD 성공으로 계산하지 않았습니다. 최종 경로별 성공 결과와 과거 준비 실패는 구별해서 읽어야 합니다.

이번 검증은 입력값의 모든 조합, 대규모 페이지 처리, 모든 잘못된 요청, 원자적 롤백 시나리오와 동시성을 전수 검사하지 않았습니다. 운영 중인 모든 사용자의 권한 또는 모든 기존 업무 데이터 내용에 대한 검증도 포함하지 않습니다.

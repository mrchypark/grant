# certd 권한 모델

이 문서는 현재 코드와 스키마가 기대하는 조직, 그룹, 발전 설비 권한 전개 구조를 정리한다.

## 기본 엔티티

- `organization`
  - 사업자 조직의 기본 단위다.
  - 발전사업자, 총판, 대리점, 시공사, 유지보수사를 모두 포함한다.
- `group`
  - 조직 내부 팀 단위다.
  - 외부 조직 관계 표현에는 쓰지 않는다.
- `asset`
  - 글로벌 권한 모델의 canonical 발전 설비 리소스다.
  - 하나의 `owner_organization_id`에 귀속된다.
  - `created_by_organization_id`로 최초 생성 조직을 기록한다.
- `plant`
  - 한국 RPS 호환 계층의 발전소 표현이다.
  - 글로벌 신규 권한의 canonical 리소스는 `asset`이다.

## 권한 규칙

- `organization.*`
  - 조직 스코프 권한이다.
  - 예: `organization.plant.create`, `organization.credential.use`
  - 전개 테이블이 아니라 런타임 직접 평가로 처리한다.
- `plant.*`
  - 한국 RPS 호환 경로의 특정 발전소 권한이다.
  - 예: `plant.profile.read`, `plant.sync.execute`
  - 전개 결과를 `effective_*` 테이블로 조회한다.
  - 호환 API는 먼저 `plant.*` 권한을 확인하고, 없으면 `asset_profiles(profile_type = 'kr_rps', external_id = facility_code)`로 `asset_id`를 찾아 대응되는 `asset.*` 권한을 확인한다.
  - 주요 호환 mapping은 `plant.sync.execute -> asset.sync.execute`, `plant.telemetry.read -> asset.generation.read`, `plant.rec.read -> asset.certificate.read`, `plant.rate.read -> asset.rate.read`다.
  - `plant.*` binding은 대응 `asset.*` action으로 자동 전개하지 않는다. 브레이킹 스키마 기준에서 canonical 권한은 별도 `asset` resource binding으로 부여한다.
- `asset.*`
  - 글로벌 발전 설비 권한이다.
  - 예: `asset.profile.read`, `asset.sync.execute`, `asset.generation.read`
  - 전개 결과를 `effective_*_asset_*` 테이블로 조회한다.
  - registry account, credential, data source 사용은 asset 권한 위에 추가 실행 조건으로 검증한다.
  - 저장형 인증서를 사용하는 KR RPS sync는 `asset.sync.execute`와 별도로 `asset.credential.use` 및 활성 KR RPS credential 존재를 확인한다.
  - 대상 KR RPS registry project가 이미 registry account에 묶여 있으면, 단순히 실행 조직의 활성 인증서가 있는 것만으로는 부족하다. `credential_bindings`가 해당 registry account에 연결된 credential이어야 한다.

## 목록 노출 규칙

- asset 목록은 `effective_user_asset_visibility`를 본다.
- 한국 RPS 호환 plant 목록은 `effective_user_plant_visibility`와 `effective_user_asset_visibility -> asset_profiles(profile_type = 'kr_rps')`를 합쳐 본다.
- asset visibility를 만드는 read 권한은 `asset.profile.read`다.
- 현재 1차 구현 기준으로 visibility를 만드는 read 권한은 `plant.profile.read` 하나다.
- `asset.document.read`나 `plant.document.read` 같은 다른 read 권한만으로는 목록에 자동 노출되지 않는다.

즉, 문서 접근 권한이 있더라도 `asset.profile.read` 또는 호환 경로의 `plant.profile.read`가 없으면 목록에서 발전소가 보이지 않을 수 있다.

## 전개 테이블

권한 모델의 핵심은 아래 전개 결과에 있다.

- `effective_organization_asset_actions`
  - 조직 기준 asset action 전개
- `effective_user_asset_actions`
  - 사용자 기준 asset action 전개
- `effective_user_asset_visibility`
  - 사용자 기준 asset 목록 노출 전개
- `effective_organization_plant_actions`
  - 조직 기준 한국 RPS 호환 plant action 전개
- `effective_user_plant_actions`
  - 사용자 기준 한국 RPS 호환 plant action 전개
- `effective_user_plant_visibility`
  - 사용자 기준 한국 RPS 호환 plant 목록 노출 전개

테이블 정의와 제약은 [database-schema.md](./database-schema.md)를 기준으로 본다.

## 코드와의 연결

- write/create/deactivate 같은 조직 변경 작업은 voter 라우트에서만 허용된다.
- read 경로는 전개 결과를 이용해 asset/plant 조회와 문서 목록 노출을 결정한다.
- 한국 RPS 수집/생성 경로는 `plants` 호환 행과 함께 `assets`, `asset_profiles`, `registry_projects`, `data_sources`, `generation_data`, `rate_data`, `energy_certificates`를 같이 갱신한다.
- 한국 RPS adapter는 canonical `generation_data.value`의 의미를 `metric_type = gross_generation_kwh`로 명시한다. `value`는 `generation_records.generation_kwh`/원천 `GELEC_QTY`에 대응하며, `self_consumption_kwh`와 `supply_kwh`는 현재 보조 metadata 및 한국 호환 테이블에 남긴다.
- 한국 RPS full sync는 canonical `generation_data`, `rate_data`, `energy_certificates`도 해당 source/project 범위에서 replace한다.
- 저장형 crawl task token 발급/소비와 ephemeral crawl 실행은 route layer에만 의존하지 않고 service method 내부에서 `created_by_user_id` 기준 scope 권한을 다시 확인한다. bounded full sync처럼 실행 불가능한 scope shape은 token 발급 및 소비 전에 거부한다.
- ephemeral crawl은 signed challenge의 `facility_code`를 권위 있는 scope로 사용한다. 요청 body의 `facilityCode`가 challenge scope와 다르거나, facility 없는 challenge를 요청 body facility로 재바인딩하려 하면 service layer에서 거부한다. facility가 생략됐지만 단일 실행 가능 plant가 명확하면 service가 해석한 facility scope를 token 저장과 sync 실행에 그대로 전달한다.
- ephemeral crawl 실행 요청자는 challenge를 만든 사용자와 같아야 한다. 라우트는 `ctx.user_id`를 service로 전달하고, service는 challenge creator와 requester 불일치를 거부한다.
- 저장형 인증서 등록도 challenge를 만든 사용자와 실제 등록 요청자가 같아야 한다. 저장형 인증서는 registry account/organization credential이므로 facility-scoped stored challenge는 생성 시점부터 거부하고, facility scope는 ephemeral sync 또는 crawl task token scope에서만 사용한다.
- collector가 최종 facility를 반환한 뒤에는 그 resolved facility를 다시 권한 검사한다. 기존 plant면 `plant.sync.execute`가 필요하고, 저장형 인증서 경로는 추가로 `asset.credential.use`와 registry account credential binding을 요구한다. 새 plant면 `organization.plant.create`와 `organization.credential.use`가 모두 필요하다.
- 법적 조직 identity는 request payload나 self-signed 인증서 subject만으로 bootstrap하지 않는다. 조직에 이미 연결된 business number 또는 별도 검증된 조직 business number가 있어야 저장형 인증서 등록이 가능하다.
- 인증서 proof는 운영 기본값에서 X.509 trust store 기준 체인 검증을 통과해야 한다. `CERTD_CERT_TRUST_BUNDLE_PATH`가 설정되면 해당 PEM CA bundle을 trust anchor로 사용하고, 없으면 시스템 기본 trust store를 사용한다. 요청 bundle은 leaf 외 intermediate DER 목록을 `certificateChainDerBase64`로 전달할 수 있고, 서버는 이 chain을 trust store 검증에 사용한다. `CERTD_ALLOW_UNTRUSTED_CERTIFICATES=true`는 local/test fixture 전용 예외다.
- sync가 새 plant/asset을 최초 materialize하면, route에서 검증된 initiating user 기준으로 owner/creator plant 및 asset binding을 bootstrap한다. 이 경로가 없으면 첫 수집 직후 생성자가 자신이 만든 asset을 다시 읽거나 재수집할 수 없다.
- sync 성공 materialization은 DB transaction 안에서 plant, canonical asset projection, 한국 RPS 호환 projection, document rows, sync item, sync run 완료 상태를 함께 커밋한다. 객체 저장소 upload는 transaction 전에 수행하고 stale object 삭제는 commit 이후에만 수행한다.
- 기존 owner plant를 delegated sync로 처음 수집할 때 owner registry account가 아직 확인되지 않았다면, 실행 조직의 `business_no`로 owner asset의 registry account를 만들지 않는다. registry project는 account 미확인 상태로 남기고, 명시적 owner/registry binding이 생긴 뒤 credential binding을 연결한다.
- delegated stored sync는 owner asset의 registry account/credential binding을 요구한다. registry account가 아직 NULL인 project에서는 owner 조직 credential만 fallback으로 허용하고, operator 조직의 별도 인증서는 사용할 수 없다.
- 신규 plant 생성 시에는 한국 RPS 호환 `plant.*` bootstrap binding과 함께 owner/creator/user에 대응되는 `asset.*` binding을 생성한다.
- learner 노드는 read/authz 보조 경로로 쓰일 수 있지만, write/admin surface는 열지 않는다.

## 문서 갱신 규칙

아래가 바뀌면 이 문서와 [database-schema.md](./database-schema.md)를 함께 갱신해야 한다.

- 조직/그룹/발전 설비 관계 의미
- canonical key 의미
- `effective_*` 전개 방식
- asset/plant visibility 생성 규칙

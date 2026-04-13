# Certificate Gateway Design

목표는 `certd`에 사용자 로그인 대체가 아닌 `공동인증서 기반 수집 실행 게이트웨이`를 추가하는 것이다. 이 게이트웨이는 공동인증서를 검증 가능한 작업 권한으로 바꾸고, 웹은 `JS SDK`, 네이티브는 `desktop agent`, 서버는 기존 Rust `certd`가 맡는다.

핵심 원칙은 다음과 같다.

- 공동인증서 플로우는 사용자 로그인 세션이 아니다.
- 웹은 브라우저 JS만으로 인증서를 직접 다루지 않는다.
- 비밀번호와 개인키 처리는 항상 네이티브 agent가 맡는다.
- 서버는 challenge 발급, 서명 검증, 인증서 상태 확인, 저장형 암호화 보관, 수집 실행 권한 부여만 맡는다.
- `ephemeral`은 한 번 인증하고 즉시 수집을 실행한 뒤 종료한다.
- `stored`는 검증된 인증서를 암호화 저장하고, 이후에는 짧은 수명의 `crawl task token`으로 수집을 실행한다.
- 저장형 암호화는 "새 저장 비밀번호"가 아니라 `per-certificate DEK + KMS-style key wrapping`으로 처리한다.

## 왜 로그인 시스템이 아닌가

이 플로우의 목표는 사용자 식별이나 일반 API 세션 관리가 아니다. 목표는 공동인증서를 이용해 RPS 사이트 수집을 합법적으로 실행하는 것이다.

따라서 결과물은 다음 둘 중 하나뿐이다.

- 즉시 실행형 수집
- 저장된 인증서를 이용한 제한된 수집 실행 권한

즉 공동인증서는 `AuthN for user session`이 아니라 `proof of authority for crawl execution`으로 취급한다.

## 시스템 구성

### Web JS SDK

웹 SDK는 오케스트레이션만 담당한다.

- `stored` 또는 `ephemeral` 전략 선택
- 서버에 challenge 요청
- 사용자에게 인증서 선택/비밀번호 입력 UI 제공
- 로컬 `desktop agent` 호출
- agent가 생성한 서명 결과를 서버에 제출
- `ephemeral`이면 즉시 실행 API 호출
- `stored`면 인증서 등록 완료 또는 저장된 인증서 기반 작업 토큰 발급 호출

SDK는 브라우저 안에서 인증서 파일, 키 저장소, 개인키 복호화, PKCS#7 서명을 직접 수행하지 않는다.

### Desktop Agent

`desktop agent`는 Windows/macOS 공통 Go 패키지로 제공한다.

역할:

- 로컬 인증서 탐색
- 인증서 메타데이터 반환
- 비밀번호로 로컬 인증서 열기
- challenge 서명
- `stored` 경로일 때만 저장용 인증서 bundle 추출
- 비밀번호와 평문 key material을 메모리에서만 사용 후 폐기

웹과의 통신 방식은 1차 구현에서 loopback HTTP가 가장 단순하다.

예상 로컬 API:

- `GET /healthz`
- `GET /certificates`
- `POST /sign`

### certd Certificate Gateway

Rust `certd` 안에 별도 서비스가 아니라 새로운 기능 경계로 넣는다.

역할:

- challenge 발급
- 제출된 서명 검증
- 인증서 fingerprint, subject, business number 추출
- 인증서 만료/상태 확인
- `stored`일 때 암호화 저장
- `stored`일 때 이후 1회성 `crawl task token` 발급
- `ephemeral`일 때 즉시 수집 실행

이 설계는 이미 존재하는 다음 자산을 재사용한다.

- 인증서 저장/복호화 유틸
- NPKI signer
- 도메인/수집 서비스
- 조직/권한 모델

## 두 가지 실행 경로

### 1. Ephemeral

`ephemeral`은 "지금 이 인증서로 한 번 실행하고 끝내기" 흐름이다.

흐름:

1. 웹 SDK가 challenge를 요청한다.
2. SDK가 사용자로부터 인증서 선택과 비밀번호를 받는다.
3. SDK가 `desktop agent`에 challenge와 비밀번호를 전달한다.
4. agent가 로컬 인증서를 열고 challenge에 서명한다.
5. SDK가 서명 결과와 원하는 수집 입력을 서버에 제출한다.
6. 서버가 검증 성공 시 즉시 수집을 시작한다.
7. 작업이 시작되면 이 인증 플로우는 종료된다.

특징:

- 서버는 인증서를 장기 저장하지 않는다.
- 서버는 사용자 원래 인증서 비밀번호를 저장하지 않는다.
- 별도 일반 세션 토큰을 만들지 않는다.
- 결과는 `crawl_id`와 작업 상태만 반환한다.

### 2. Stored

`stored`는 "인증서를 한 번 등록하고 이후에는 저장된 인증서를 이용해 반복 실행" 흐름이다.

흐름:

1. 웹 SDK가 challenge를 요청한다.
2. SDK가 사용자에게 인증서 선택과 비밀번호를 받는다.
3. SDK가 `desktop agent`에 challenge와 비밀번호를 전달한다.
4. agent가 challenge에 서명하고 저장용 인증서 bundle도 준비한다.
5. SDK가 서명 결과와 bundle을 서버에 제출한다.
6. 서버가 검증 후 bundle을 암호화 저장한다.
7. 이후 실행 시 서버는 저장된 인증서를 복호화해 수집에 사용한다.
8. 반복 실행은 1회성 `crawl task token`을 통해 시작한다.

특징:

- 사용자 원래 인증서 비밀번호는 저장하지 않는다.
- 저장된 것은 암호화된 bundle과 wrapped DEK뿐이다.
- 반복 실행은 일반 세션이 아니라 짧은 수명의 작업 권한으로 제한한다.

## 책임 분리

가장 중요한 분리는 "비밀번호 입력 위치"와 "비밀번호 사용 주체"를 나누는 것이다.

- 웹 UI / SDK
  - 비밀번호를 입력받을 수 있다.
  - 비밀번호를 로컬 agent에 전달할 수 있다.
- desktop agent
  - 비밀번호로 인증서를 실제로 연다.
  - 서명을 실제로 수행한다.
- certd
  - 비밀번호를 신뢰 경계 안에서 장기 보관하지 않는다.
  - 검증과 실행 권한 제어만 맡는다.

즉 "비밀번호 입력은 웹에서 가능하지만, 비밀번호로 서명하는 주체는 agent"가 된다.

## API 설계

조직 문맥은 기존 `certd`와 동일하게 `organizationId` 기준으로 묶는다.

### Server API

권장 1차 API는 다음과 같다.

- `POST /api/v1/organizations/{organizationId}/cert-gateway/challenges`
- `POST /api/v1/organizations/{organizationId}/cert-gateway/ephemeral-executions`
- `POST /api/v1/organizations/{organizationId}/certificates`
- `GET /api/v1/organizations/{organizationId}/certificates`
- `DELETE /api/v1/organizations/{organizationId}/certificates/{certificateId}`
- `POST /api/v1/organizations/{organizationId}/certificates/{certificateId}/crawl-tokens`
- `POST /api/v1/organizations/{organizationId}/crawls`

#### `POST /cert-gateway/challenges`

입력:

- `mode`: `stored | ephemeral`
- `provider`: `npki`
- 선택적 `facilityCode`

출력:

- `challengeId`
- `nonce`
- `payload`
- `expiresAt`

#### `POST /cert-gateway/ephemeral-executions`

입력:

- `challengeId`
- `challengePayload`
- `challengeSignatureBase64`
- `bundle`

동작:

- challenge 일치 여부 확인
- `signCertDerBase64`의 공개키로 `challengePayload` 서명 검증
- 인증서 상태 검증
- business number 등 식별자 추출
- 즉시 수집 실행

출력:

- `crawlId`
- `status`

이 경로는 작업 시작까지 한 번에 끝낸다. 별도 일반 토큰은 만들지 않는다.

#### `POST /certificates`

입력:

- `challengeId`
- `challengePayload`
- `challengeSignatureBase64`
- `bundle`

동작:

- challenge 검증
- `signCertDerBase64`의 공개키로 `challengePayload` 서명 검증
- 인증서 식별자 추출
- bundle 암호화 저장

출력:

- `certificateId`
- `subject`
- `businessNo`
- `createdAt`

#### `POST /certificates/{certificateId}/crawl-tokens`

입력:

- 수집 스코프
  - 예: `facilityCode`, `startMonth`, `endMonth`, `mode`

출력:

- `crawlTaskToken`
- `expiresAt`

이 토큰은 짧은 TTL, 1회 사용, 수집 시작 전용이다.

#### `POST /crawls`

입력:

- `Authorization: Bearer {crawlTaskToken}`
- 실제 수집 입력

출력:

- `crawlId`
- `status`

### Desktop Agent API

1차 구현은 loopback HTTP를 권장한다.

- `GET http://127.0.0.1:{port}/healthz`
- `GET http://127.0.0.1:{port}/certificates`
- `POST http://127.0.0.1:{port}/sign`

`POST /sign` 입력:

- `challenge`
- 인증서 선택 정보
- 비밀번호
- `mode`

출력:

- `signedPayload`
- `certificateChain`
- `certificateBundle` (`stored`일 때만)

## 데이터 모델

기존 `certificates` 테이블을 확장하고, challenge 및 작업 토큰 테이블을 추가한다.

### certificates

권장 컬럼:

- `id`
- `organization_id`
- `provider`
- `business_no`
- `common_name`
- `fingerprint_sha256`
- `issuer_dn`
- `subject_dn`
- `not_before`
- `not_after`
- `encrypted_blob`
- `blob_nonce`
- `wrapped_dek`
- `kms_key_id`
- `active`
- `last_used_at`
- `created_at`
- `updated_at`

`encrypted_blob`는 평문 bundle 전체를 랜덤 `DEK`로 암호화한 결과다.

### cert_gateway_challenges

권장 컬럼:

- `id`
- `organization_id`
- `mode`
- `nonce`
- `payload_hash`
- `facility_code`
- `expires_at`
- `consumed_at`
- `created_by_user_id`
- `created_at`

challenge는 짧은 수명과 1회 사용을 전제로 한다.

### crawl_task_tokens

권장 컬럼:

- `id`
- `organization_id`
- `certificate_id`
- `token_hash`
- `scope_json`
- `expires_at`
- `used_at`
- `created_by_user_id`
- `created_at`

토큰 원문은 저장하지 않고 해시만 저장한다.

## 키 관리

저장형에서 "새 랜덤 비밀번호를 만들어 저장"하는 방식은 사용하지 않는다. 대신 인증서마다 랜덤 `DEK`를 생성하고, 그 `DEK`를 KMS-style provider로 감싼다.

### KMS 인터페이스

Rust 내부 인터페이스:

```rust
trait KmsProvider {
    fn active_key_id(&self) -> &str;
    fn wrap(&self, dek: &[u8]) -> anyhow::Result<Vec<u8>>;
    fn unwrap(&self, key_id: &str, wrapped: &[u8]) -> anyhow::Result<Vec<u8>>;
}
```

### 1차 구현: EnvKmsProvider

첫 구현은 실제 cloud KMS가 아니라 환경변수 기반 provider다. 하지만 외부에서 볼 때는 KMS와 같은 계약으로 취급한다.

예상 환경변수:

- `CERTD_KMS_ACTIVE_KEY_ID`
- `CERTD_KMS_MASTER_KEYS`

예시:

```bash
CERTD_KMS_ACTIVE_KEY_ID=local-dev-v1
CERTD_KMS_MASTER_KEYS=local-dev-v1:BASE64_KEY,local-dev-v2:BASE64_KEY
```

규칙:

- 쓰기는 항상 `active_key_id`
- 읽기는 row에 저장된 `kms_key_id`로 복호화
- 나중에 AWS KMS, GCP KMS, Vault로 교체해도 상위 저장 로직은 유지

## 인증서 저장 방식

`stored` 경로 저장 절차는 다음과 같다.

1. 서버가 랜덤 `DEK`를 생성한다.
2. 평문 인증서 bundle을 `DEK`로 암호화한다.
3. `DEK`를 active master key로 wrap한다.
4. `encrypted_blob`, `blob_nonce`, `wrapped_dek`, `kms_key_id`를 DB에 저장한다.

복호화 절차는 다음과 같다.

1. row의 `kms_key_id`로 wrapping key를 선택한다.
2. `wrapped_dek`를 unwrap한다.
3. 메모리에서만 bundle을 복호화한다.
4. 수집 실행 후 평문을 폐기한다.

원래 인증서 비밀번호는 서버에 저장하지 않는다.

## 수집 권한 모델

공동인증서 검증이 곧 전체 API 권한을 의미하지는 않는다. 인증서는 수집 실행 권한만 좁게 부여한다.

권장 규칙:

- `ephemeral`
  - 검증 성공 직후 요청된 수집 1건만 실행 가능
- `stored`
  - 등록 이후에도 각 실행은 별도의 짧은 `crawl task token`이 필요
- `crawl task token`
  - 1회 사용
  - TTL 1분 내외
  - 허용된 organization / certificate / facility / month range만 사용 가능

즉 저장형도 일반 로그인 세션처럼 계속 쓰는 토큰을 주지 않는다.

## 보안 규칙

- challenge TTL은 2분 내외
- challenge는 1회 사용
- `crawl task token` TTL은 1분 내외
- `crawl task token`은 1회 사용
- 토큰 원문은 DB에 저장하지 않고 해시만 저장
- 서버 로그에 비밀번호, 평문 bundle, 개인키, 토큰 원문을 남기지 않음
- `ephemeral` 경로는 인증서 평문을 영속 저장하지 않음
- `stored` 경로도 사용자 원래 인증서 비밀번호를 저장하지 않음
- 인증서 만료/폐기/사업자번호 mismatch는 명시적 에러 코드로 반환

## 비목표

- 공동인증서를 사용자 로그인 세션으로 사용하지 않는다.
- 브라우저 JS가 개인키를 직접 다루지 않는다.
- 일반 사용자 세션 토큰이나 refresh token 체계를 추가하지 않는다.
- 1차 구현에서 mobile companion까지 동시에 구현하지 않는다.
- 1차 구현에서 cloud KMS에 직접 의존하지 않는다.

## 단계적 구현 방향

1차 구현은 다음 순서를 권장한다.

1. `certd` 내 challenge / stored / ephemeral API 추가
2. `EnvKmsProvider`와 DB 확장
3. `desktop agent` 최소 loopback HTTP 구현
4. `JS SDK` 최소 오케스트레이션 구현
5. 로컬 E2E와 실패 케이스 검증

이 순서면 서버 경계와 보안 모델을 먼저 고정한 뒤, 클라이언트 패키지를 얹을 수 있다.

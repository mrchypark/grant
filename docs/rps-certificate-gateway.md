# Certificate Gateway

이 문서는 `certd`의 공동인증서 기반 작업 실행 경계를 설명한다. 이 경계는 사용자 로그인 대체가 아니라, `rps.energy.or.kr` 수집 작업을 안전하게 실행하기 위한 별도 capability layer다.

## 목표

- `ephemeral`
  - 공동인증서를 서버에 저장하지 않고 지금 1회성 수집만 실행
- `stored`
  - 공동인증서를 암호화 저장하고 이후 반복 수집에 재사용
- 브라우저는 JS SDK만 사용
- 실제 인증서 접근과 비밀번호 처리는 local desktop agent에서 수행

## 구성 요소

- `crates/infra`
  - challenge 발급, proof 검증, encrypted certificate 저장, 1회용 crawl task token 발급
- `crates/api`
  - certificate gateway HTTP routes 노출
- `desktop-agent`
  - loopback HTTP server
  - `/healthz`, `/certificates`, `/sign`
- `packages/certd-web-sdk`
  - 브라우저 앱에서 사용하는 orchestration SDK

## KMS 환경변수

저장형 인증서는 "새 비밀번호"를 두지 않고 per-certificate DEK로 암호화한다.

```bash
export CERTD_KMS_ACTIVE_KEY_ID=local-dev-v1
export CERTD_KMS_MASTER_KEYS=local-dev-v1:BASE64_KEY_MATERIAL,local-dev-v2:BASE64_KEY_MATERIAL
```

- `CERTD_KMS_ACTIVE_KEY_ID`
  - 새로 저장할 때 사용할 wrapping key id
- `CERTD_KMS_MASTER_KEYS`
  - `key-id:base64-key` 목록
  - 읽기 시에는 row에 저장된 `kms_key_id`로 알맞은 key를 선택한다

## 서버 API

공통 조직 경로 prefix:

```text
/api/v1/organizations/{organizationId}
```

### Challenge 발급

```http
POST /cert-gateway/challenges
Content-Type: application/json

{
  "mode": "stored" | "ephemeral",
  "facilityCode": "FAC-001",
  "ttlSeconds": 120
}
```

응답:

```json
{
  "challengeId": "challenge-1",
  "mode": "ephemeral",
  "nonce": "nonce-1",
  "payload": "cert-gateway:...",
  "expiresAt": "2026-04-09T00:00:00Z"
}
```

### Ephemeral 실행

```http
POST /cert-gateway/ephemeral-executions
Content-Type: application/json

{
  "challengeId": "challenge-1",
  "challengePayload": "cert-gateway:...",
  "bundle": {
    "businessNo": "2768603291",
    "password": "secret-password",
    "commonName": "Test Certificate",
    "signCertDerBase64": "AQID",
    "signPriKeyBase64": "BAUG",
    "kmCertDerBase64": "BwgJ",
    "kmPriKeyBase64": "CgsM"
  },
  "facilityCode": "FAC-001",
  "startMonth": "2026-01",
  "endMonth": "2026-03",
  "mode": "delta"
}
```

성공 시 `202 Accepted`와 `SyncRun`을 반환한다.

### Stored 등록

```http
POST /certificates
Content-Type: application/json

{
  "challengeId": "challenge-2",
  "challengePayload": "cert-gateway:...",
  "bundle": {
    "businessNo": "2768603291",
    "password": "secret-password",
    "commonName": "Stored Certificate",
    "signCertDerBase64": "AQID",
    "signPriKeyBase64": "BAUG",
    "kmCertDerBase64": "BwgJ",
    "kmPriKeyBase64": "CgsM"
  }
}
```

성공 시 `201 Created`와 저장된 인증서 메타데이터를 반환한다.

### 1회용 crawl task token 발급

```http
POST /certificates/{certificateId}/crawl-tokens
Content-Type: application/json

{
  "facilityCode": "FAC-003",
  "startMonth": "2026-01",
  "endMonth": "2026-04",
  "mode": "full",
  "ttlSeconds": 60
}
```

응답:

```json
{
  "crawlTaskToken": "opaque-token",
  "expiresAt": "2026-04-09T00:01:00Z"
}
```

### crawl 시작

```http
POST /crawls
Authorization: Bearer <crawlTaskToken>
```

- token은 아주 짧은 TTL이다
- 1회 사용 후 재사용은 거절된다

## Desktop Agent 로컬 API

기본 bind는 loopback only다.

```text
GET  /healthz
GET  /certificates
POST /sign
```

### `GET /healthz`

응답:

```text
ok
```

### `GET /certificates`

응답:

```json
{
  "items": [
    {
      "id": "cert-1",
      "commonName": "Test Certificate",
      "businessNo": "2768603291",
      "notAfter": "2027-04-09T00:00:00Z"
    }
  ]
}
```

### `POST /sign`

요청:

```json
{
  "certificateId": "cert-1",
  "challenge": "cert-gateway:...",
  "password": "secret-password"
}
```

현재 스캐폴드 구현 상태:

- `challenge` 또는 `password`가 비어 있으면 `400`
- 정상 요청은 아직 플랫폼별 인증서 접근 구현이 없어서 `501 Not Implemented`

## JS SDK 사용 예시

```ts
import { CertdWebSdk } from "@certd/web-sdk";

const sdk = new CertdWebSdk({
  gatewayBaseUrl: "https://certd.example.com",
  agentBaseUrl: "http://127.0.0.1:8765",
});
```

### Ephemeral

```ts
const run = await sdk.runEphemeralExecution("org-1", {
  certificateId: "cert-1",
  password: "secret-password",
  facilityCode: "FAC-001",
  startMonth: "2026-01",
  endMonth: "2026-03",
  mode: "delta",
});
```

### Stored

```ts
const cert = await sdk.registerStoredCertificate("org-1", {
  certificateId: "cert-2",
  password: "secret-password",
  facilityCode: "FAC-002",
});
```

SDK가 내부적으로 수행하는 순서는 다음과 같다.

1. local agent `/healthz` 확인
2. gateway challenge 생성
3. local agent `/sign` 호출
4. `ephemeral`이면 즉시 실행, `stored`면 등록 요청

## 현재 구현 상태

- Rust gateway route/service는 테스트로 검증된 스켈레톤이 있다
- stored certificate는 env-backed KMS provider로 wrap된 DEK를 저장한다
- desktop agent는 loopback HTTP contract만 구현되어 있다
- 실제 Windows/macOS 인증서 저장소 접근과 서명은 아직 구현되지 않았다
- JS SDK는 브라우저 orchestration skeleton이며 agent와 gateway 계약을 감싼다

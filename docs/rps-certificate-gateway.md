# Certificate Gateway

이 문서는 `certd`의 공동인증서 기반 작업 실행 경계를 설명한다. 이 경계는 사용자 로그인 대체가 아니라, `rps.energy.or.kr` 수집 작업을 안전하게 실행하기 위한 별도 capability layer다.

## 목표

- `ephemeral`
  - 공동인증서를 서버에 저장하지 않고 지금 1회성 수집만 실행
- `stored`
  - 공동인증서를 암호화 저장하고 이후 반복 수집에 재사용
- 브라우저는 JS SDK만 사용
- 실제 인증서 접근과 비밀번호 처리는 local desktop agent의 native approval modal에서 수행

## 구성 요소

- `crates/infra`
  - challenge 발급, proof 검증, encrypted certificate 저장, 1회용 crawl task token 발급
- `crates/api`
  - certificate gateway HTTP routes 노출
- `desktop-agent`
  - loopback HTTP server
  - `/healthz`, `/sessions/open`, `/certificates`, `/sign`
  - `/approve-ephemeral-run`, `/approve-stored-registration`
- `packages/certd-web-sdk`
  - 고수준 제품 API + 저수준 raw client

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

Desktop agent는 인증서 탐색 루트를 환경변수로 덮어쓸 수 있다.

```bash
export DESKTOP_AGENT_NPKI_DIRS="$HOME/NPKI:$HOME/Documents/NPKI"
```

loopback agent는 허용 origin과 local session secret도 환경변수로 제어할 수 있다.

```bash
export DESKTOP_AGENT_ALLOWED_ORIGINS="http://127.0.0.1:4173,http://localhost:4173"
export DESKTOP_AGENT_GATEWAY_BASE_URL="https://certd.example.com"
export DESKTOP_AGENT_SESSION_SECRET="$(openssl rand -base64 32)"
export DESKTOP_AGENT_DISABLE_TRAY=1
```

- `DESKTOP_AGENT_ALLOWED_ORIGINS`
  - 브라우저에서 agent를 호출할 수 있는 allowlist origin
- `DESKTOP_AGENT_GATEWAY_BASE_URL`
  - local agent가 certificate bundle과 password를 전달할 단일 trusted certd backend base URL
  - 브라우저가 보낸 `gatewayBaseUrl`는 이 값과 일치할 때만 허용된다
- `DESKTOP_AGENT_SESSION_SECRET`
  - local session token HMAC에 쓰는 32-byte 이상 secret
  - 비워 두면 사용자 config 디렉터리에 device-local secret을 생성한다
- `DESKTOP_AGENT_DISABLE_TRAY`
  - macOS 기본 tray 실행을 끄고 기존 headless HTTP agent로 실행한다

- macOS 기본 탐색 경로
  - `~/NPKI`
  - `~/Documents/NPKI`
  - `~/Library/Preferences/NPKI`
- Windows 기본 탐색 경로
  - `%USERPROFILE%\\NPKI`
  - `%USERPROFILE%\\Documents\\NPKI`
  - `%LOCALAPPDATA%\\NPKI`
  - `%APPDATA%\\NPKI`

USB에 인증서가 있을 때는 USB 루트를 직접 지정하면 된다.

```bash
export DESKTOP_AGENT_NPKI_DIRS="/Volumes/Lexar/NPKI"
```

agent는 지정된 루트 아래를 재귀적으로 훑어서 아래 4개 파일이 같은 디렉터리에 있는 곳을 인증서 bundle로 취급한다.

```text
signCert.der
signPri.key
kmCert.der
kmPri.key
```

대표적인 실제 레이아웃 예시는 다음 중 하나다.

```text
/Volumes/Lexar/NPKI/<issuer>/<type>/<subject>/signCert.der
/Volumes/Lexar/NPKI/<issuer>/<type>/<subject>/signPri.key
/Volumes/Lexar/NPKI/<issuer>/<type>/<subject>/kmCert.der
/Volumes/Lexar/NPKI/<issuer>/<type>/<subject>/kmPri.key
```

또는

```text
/Volumes/Lexar/NPKI/yessign/USER/cn=<subject>/...
/Volumes/Lexar/NPKI/crosscert/USER/cn=<subject>/...
```

macOS에서 removable volume이 `Operation not permitted`로 보이면, Codex/Terminal 앱에 Files and Folders 또는 Full Disk Access 권한이 없어서 USB 내부를 읽지 못하는 상태일 수 있다.

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
  "challengeSignatureBase64": "BASE64_RSA_SHA256_SIGNATURE",
  "bundle": {
    "businessNo": "2768603291",
    "password": "secret-password",
    "commonName": "Test Certificate",
    "signCertDerBase64": "AQID",
    "signPriKeyBase64": "BAUG",
    "kmCertDerBase64": "BwgJ",
    "kmPriKeyBase64": "CgsM"
  }
}
```

성공 시 `202 Accepted`와 `SyncRun`을 반환한다. 기본 제품형 high-level SDK 호출은 여전히 이 스코프를 생략해도 되지만, multi-plant 조직처럼 서버가 단일 대상을 자동 결정할 수 없는 경우를 위해 `facilityCode`, `startMonth`, `endMonth`, `mode`를 선택적으로 agent/raw client 경로로 전달할 수 있다.

### Stored 등록

```http
POST /certificates
Content-Type: application/json

{
  "challengeId": "challenge-2",
  "challengePayload": "cert-gateway:...",
  "challengeSignatureBase64": "BASE64_RSA_SHA256_SIGNATURE",
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

`challengeSignatureBase64`는 local desktop agent가 `challengePayload` 원문 바이트를 공동인증서 sign key로 RSA/SHA-256 서명한 값이다. 서버는 `bundle.signCertDerBase64`의 공개키로 이 값을 검증한 뒤에만 challenge를 consume 한다.

성공 시 `201 Created`와 저장된 인증서 메타데이터를 반환한다.

### 1회용 crawl task token 발급

```http
POST /certificates/{certificateId}/crawl-tokens
Content-Type: application/json

{
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
POST /sessions/open
GET  /certificates
POST /sign
POST /approve-ephemeral-run
POST /approve-stored-registration
```

### `GET /healthz`

응답:

```text
ok
```

### `GET /certificates`

`X-Agent-Session` 헤더가 필요하다. session은 `POST /sessions/open`으로 먼저 연다.

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

`X-Agent-Session` 헤더가 필요하다.

요청:

```json
{
  "certificateId": "cert-1",
  "challenge": "cert-gateway:...",
  "password": "secret-password"
}
```

현재 구현 상태:

- 허용 origin만 CORS 허용
- `/sessions/open`으로 짧은 local session token 발급
- `/certificates`, `/sign`는 session 없으면 거절
- 제품형 high-level SDK는 `/approve-ephemeral-run`, `/approve-stored-registration` 사용
- 실제 NPKI USB/파일 시스템 탐색과 비밀번호 검증 구현 포함
- macOS에서는 기본적으로 메뉴바(tray) 앱으로 실행되고 `Open Example Page`, `Quit certd Agent` 메뉴 제공
- native approval modal은 현재 macOS AppKit(`NSAlert + certificate picker + secure password field`) 구현, Windows는 후속 구현 대상

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
await sdk.assertAvailable();

const run = await sdk.runEphemeral("org-1", {
  businessNo: "2768603291",
});
```

### Stored

```ts
const cert = await sdk.registerStored("org-1", {
  businessNo: "2768603291",
});

const stored = await sdk.listStoredCertificates("org-1");
const run = await sdk.runStored("org-1", {
  certificateId: stored[0].id,
});
```

SDK가 내부적으로 수행하는 순서는 다음과 같다.

1. local agent `/healthz` 확인
2. local agent `/sessions/open`으로 local session 확보
3. local agent `approve-*` endpoint 호출
4. agent가 native approval modal로 인증서 선택과 비밀번호 입력 처리
5. agent가 gateway challenge 생성, 로컬 sign, gateway 실행까지 처리

## 로컬 검증

Go desktop agent 빌드:

```bash
export PATH=/opt/homebrew/bin:$PATH
cd desktop-agent
go build -o ./bin/desktop-agent ./cmd/desktop-agent
```

USB 인증서를 직접 읽도록 agent 실행:

```bash
export DESKTOP_AGENT_ADDR=127.0.0.1:8765
export DESKTOP_AGENT_NPKI_DIRS="/Volumes/Lexar/NPKI"
export DESKTOP_AGENT_ALLOWED_ORIGINS="http://127.0.0.1:4173,http://localhost:4173"
./bin/desktop-agent
```

기본 확인:

```bash
curl -i http://127.0.0.1:8765/healthz
SESSION_TOKEN=$(curl -sS -X POST http://127.0.0.1:8765/sessions/open \
  -H 'Origin: http://127.0.0.1:4173' | jq -r .sessionToken)
curl http://127.0.0.1:8765/certificates \
  -H 'Origin: http://127.0.0.1:4173' \
  -H "X-Agent-Session: ${SESSION_TOKEN}"
```

JS 예시 페이지 실행:

```bash
cd packages/certd-web-sdk
npm install
npm run example
```

정적 빌드 검증만 하려면:

```bash
npm run example:build
python3 -m http.server 4173 --directory ./examples/browser/dist/example
```

## 현재 구현 상태

- Rust gateway route/service는 테스트로 검증된 스켈레톤이 있다
- stored certificate는 env-backed KMS provider로 wrap된 DEK를 저장한다
- desktop agent는 loopback HTTP contract만 구현되어 있다
- 실제 Windows/macOS 인증서 저장소 접근과 서명은 아직 구현되지 않았다
- JS SDK는 브라우저 orchestration skeleton이며 agent와 gateway 계약을 감싼다

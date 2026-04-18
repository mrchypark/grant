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
- `packages/web-sdk`
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
export DESKTOP_AGENT_NPKI_DIRS="$HOME/NPKI:$HOME/GPKI:$HOME/Documents/NPKI:$HOME/Documents/GPKI"
```

loopback agent는 허용 origin과 local session secret도 환경변수로 제어할 수 있다.

```bash
export DESKTOP_AGENT_ALLOWED_ORIGINS="http://127.0.0.1:4173,http://localhost:4173"
export DESKTOP_AGENT_GATEWAY_BASE_URL="https://certd.example.com"
export DESKTOP_AGENT_SESSION_SECRET="$(openssl rand -base64 32)"
```

- `DESKTOP_AGENT_ALLOWED_ORIGINS`
  - 브라우저에서 agent를 호출할 수 있는 allowlist origin
- `DESKTOP_AGENT_GATEWAY_BASE_URL`
  - local agent가 certificate bundle과 password를 전달할 단일 trusted certd backend base URL
  - 브라우저가 보낸 `gatewayBaseUrl`는 이 값과 일치할 때만 허용된다
- `DESKTOP_AGENT_SESSION_SECRET`
  - local session token HMAC에 쓰는 32-byte 이상 secret
  - 비워 두면 사용자 config 디렉터리에 device-local secret을 생성한다

- macOS 기본 탐색 경로
  - `~/NPKI`
  - `~/GPKI`
  - `~/Documents/NPKI`
  - `~/Documents/GPKI`
  - `~/Library/Preferences/NPKI`
  - `~/Library/Preferences/GPKI`
- Windows 기본 탐색 경로
  - `%USERPROFILE%\\NPKI`
  - `%USERPROFILE%\\GPKI`
  - `%USERPROFILE%\\Documents\\NPKI`
  - `%USERPROFILE%\\Documents\\GPKI`
  - `%USERPROFILE%\\AppData\\LocalLow\\NPKI` (브라우저/낮은 무결성 플러그인 설치 경유)
  - `%USERPROFILE%\\AppData\\LocalLow\\GPKI`
  - `%ProgramFiles%\\NPKI`, `%ProgramFiles%\\GPKI` (64bit 기관 설치기)
  - `%ProgramFiles(x86)%\\NPKI`, `%ProgramFiles(x86)%\\GPKI` (32bit 에뮬레이션 설치기)
  - Windows AutoDiscover는 현재 mount된 고정/이동식 드라이브를 `GetLogicalDrives` + `GetDriveType` 으로 훑어 `<드라이브>:\\NPKI`, `<드라이브>:\\GPKI`, 그리고 드라이브 루트의 `.p12/.pfx` 파일 경로를 직접 추가한다. CD-ROM/네트워크 드라이브는 hang 방지를 위해 제외한다.

macOS와 Windows desktop agent 모두 tray 앱으로 상주 실행된다 (macOS 메뉴바, Windows 시스템 트레이). tray 없는 headless 실행 모드는 더 이상 지원하지 않는다. 표시 문자열은 [`internal/localization`](../desktop-agent/internal/localization/) 의 `ko`/`en` 카피를 사용한다. 트레이 아이콘은 [`desktop-agent/assets/`](../desktop-agent/assets/) 패키지의 `tray-icon.png`(macOS)/`tray-icon.ico`(Windows) 로 각 플랫폼별로 번들한다. Windows 빌드는 `-H=windowsgui` 링커 플래그로 콘솔 없이 실행되며 로컬에서는 `make build`, CI는 [`.github/workflows/desktop-agent.yml`](../.github/workflows/desktop-agent.yml) 의 ubuntu/macos/windows 매트릭스에서 자동 빌드한다.

USB에 인증서가 있을 때는 USB 루트를 직접 지정하면 된다.

```bash
export DESKTOP_AGENT_NPKI_DIRS="/Volumes/Lexar/NPKI:/Volumes/Lexar/GPKI"
```

agent는 지정된 루트 아래를 재귀적으로 훑어서 아래 4개 파일이 같은 디렉터리에 있는 곳을 인증서 bundle로 취급한다. `NPKI`와 `GPKI`는 같은 bundle source로 다뤄지고, 목록과 서명 요청의 `certificateId`는 `bundle:` 접두사를 가진 source-aware id를 사용한다.

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

`GET /certificates`의 bundle metadata는 다음 필드를 포함한다.

- `id`
  - `bundle:/absolute/path/to/cert-dir` 또는 `pkcs12:/absolute/path/to/file.p12` 형태
- `sourceKind`
  - `bundle` 또는 `pkcs12`
- `storageKind`
  - `npki`, `gpki`, `pkcs12`, 또는 커스텀 bundle root/alias에서는 `unknown`
- `displayPath`
  - 사용자에게 보이는 축약 경로. API/docs/tests에서는 플랫폼과 무관하게 `NPKI/yessign/...`처럼 forward-slash 스타일로 표시한다.
- `issuerName`
  - 인증서 발급자 표시명
- `hasKmMaterial`
  - bundle에 KM 파일이 포함되어 있는지 여부

macOS native approval modal은 목록 라벨에서 `로컬 NPKI`, `이동식디스크 GPKI`, `이동식디스크 PKCS#12`처럼 소스와 저장 매체를 함께 보여주고, 마지막에 `displayPath`를 붙여 같은 CN이 여러 개여도 구분되도록 한다.

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
      "id": "bundle:/Volumes/Lexar/NPKI/yessign/USER/cn=test",
      "commonName": "Test Certificate",
      "businessNo": "2768603291",
      "notAfter": "2027-04-09T00:00:00Z",
      "sourceKind": "bundle",
      "storageKind": "npki",
      "displayPath": "NPKI/yessign/USER/cn=test",
      "issuerName": "Test Certificate",
      "hasKmMaterial": true
    },
    {
      "id": "pkcs12:/Volumes/Lexar/protected.p12",
      "commonName": "Protected PKCS12 Certificate",
      "businessNo": "2768603291",
      "notAfter": "2027-04-09T00:00:00Z",
      "sourceKind": "pkcs12",
      "storageKind": "pkcs12",
      "displayPath": "protected.p12",
      "issuerName": "Test Certificate",
      "hasKmMaterial": false
    }
  ]
}
```

### `POST /sign`

`X-Agent-Session` 헤더가 필요하다.

요청:

```json
{
  "certificateId": "bundle:/Volumes/Lexar/NPKI/yessign/USER/cn=test",
  "challenge": "cert-gateway:...",
  "password": "secret-password"
}
```

현재 구현 상태:

- 허용 origin만 CORS 허용
- `/sessions/open`으로 짧은 local session token 발급
- `/certificates`, `/sign`는 session 없으면 거절
- 제품형 high-level SDK는 `/approve-ephemeral-run`, `/approve-stored-registration` 사용
- 실제 NPKI/GPKI USB/파일 시스템 탐색과 비밀번호 검증 구현 포함
- `.p12`/`.pfx` PKCS#12 파일도 탐색, 목록, 서명, stored registration/ephemeral execution까지 지원
- macOS에서는 기본적으로 메뉴바(tray) 앱으로 실행되고 `Open Example Page`, `Quit desktop agent` 메뉴 제공
- native approval modal
  - macOS: AppKit `NSAlert` + `NSPopUpButton`(인증서 선택) + `NSSecureTextField`(비밀번호) + `NSAlertStyleWarning`
    - 인증서 목록 라벨에 source/storage/path 메타데이터를 함께 보여준다
  - Windows: Win32 `DialogBoxIndirectParamW` 인메모리 DLGTEMPLATE로 경고 아이콘(`IDI_WARNING`) + 12pt 볼드 Segoe UI 헤딩 + ComboBox + `EM_SETCUEBANNER`로 플레이스홀더가 걸린 `ES_PASSWORD` Edit + Approve/Cancel을 단일 모달에 묶는다. cgo 없이 `golang.org/x/sys/windows` syscall만 사용하고, 모달 foreground 쟁취는 `AllowSetForegroundWindow` + `AttachThreadInput` 조합으로 처리한다
  - 공통 파이프라인(직렬화 `acquireApprovalModal`, 컨텍스트 취소, 언어별 카피 주입)은 [`internal/approval/prompter.go`](../desktop-agent/internal/approval/prompter.go) 에 모여 있고 각 플랫폼은 `init()`에서 네이티브 호출을 모달 hook에 바인딩만 한다
  - Windows 바이너리는 [`internal/approval/app.manifest`](../desktop-agent/internal/approval/app.manifest)를 `.syso` 리소스로 임베드해 ComCtl32 v6(모던 비주얼 스타일 + cue banner)와 Per-Monitor DPI v2를 활성화한다. 매니페스트 변경 후에는 `go generate ./internal/approval` 로 `.syso`를 재생성한다
- approval 모달 표시 문자열은 [`internal/localization`](../desktop-agent/internal/localization/) 의 `ApprovalTitle`/`ApprovalInfoFormat`/`Approve`/`Cancel`/`CertificateLabel`/`PasswordLabel`/`PasswordPlaceholder` 카피를 사용해 시스템 언어에 따라 ko/en으로 분기된다

## JS SDK 사용 예시

```ts
import { WebSdk } from "@conalog/web-sdk";

const sdk = new WebSdk({
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

### HTML5 파일 찾기 도구

브라우저가 로컬 인증서 저장소를 직접 읽을 수 없는 환경에서는 사용자가 NPKI 폴더나 4개 파일을 직접 선택해야 한다.

```ts
import {
  configureHtml5CertificateInput,
  prepareHtml5CertificateBundle,
} from "@conalog/web-sdk";

const input = document.getElementById("cert-files") as HTMLInputElement;
configureHtml5CertificateInput(input, { mode: "directory" });

const prepared = await prepareHtml5CertificateBundle(input.files ?? [], {
  businessNo: "2768603291",
  commonName: "사업자 범용 인증서",
  password: "secret-password",
});

console.log(prepared.bundle.signCertDerBase64);
```

브라우저 안에서 `signPri.key` 복호화와 challenge 서명까지 끝내려면, desktop agent와 같은 Go NPKI 로직을 WebAssembly로 감싼 고수준 API를 사용할 수 있다. SDK는 이 경로를 두 가지 보안 모델로 나눠 제공한다.

```ts
const proof = await sdk.createSignedChallengeProof("org-1", {
  certificateFiles: input.files ?? [],
  password: "secret-password",
  mode: "ephemeral",
  facilityCode: "FAC-001",
});
```

`createSignedChallengeProof()`는 사용자가 파일을 직접 고른 뒤 브라우저가 Go/WASM signer를 로드해서 다음을 수행한다.

1. gateway challenge 생성
2. `signPri.key`를 비밀번호로 복호화
3. challenge payload를 `SHA256withRSA`로 서명
4. `challengeId`, `challengePayload`, `challengeSignatureBase64`, `signCertDerBase64`만 caller에 반환

이 경로는 개인키/비밀번호를 gateway로 보내지 않는다. 브라우저 전용 서명 proof를 직접 소비하는 backend가 있을 때 적합하다.

gateway가 인증서 번들을 escrow 받아야 하는 경우에는 명시적 opt-in 메서드를 사용한다.

```ts
const run = await sdk.runEphemeralWithEscrowedBrowserCertificate("org-1", {
  certificateFiles: input.files ?? [],
  password: "secret-password",
  facilityCode: "FAC-001",
  mode: "full",
  escrow: true,
});
```

이 escrow 경로는 challenge 서명 뒤에 gateway `ephemeral-executions` 또는 `stored-certificates` endpoint로 인증서 번들과 비밀번호를 전송한다. 기본 메서드 `runEphemeralWithBrowserCertificate()` / `registerStoredWithBrowserCertificate()`는 `escrow: true` 없이는 실패하도록 막아 둔다.

브라우저 보안 제약 때문에 OS 인증서 저장소 자동 탐색은 여전히 불가능하고, 파일 선택은 사용자가 직접 해야 한다.

배포 시 browser global bundle과 함께 `npki_signer.wasm`, `wasm_exec.js`를 같은 디렉터리에 두면 된다. WASM asset은 same-origin `https:` 또는 loopback `http:`에서만 로드되고, gateway URL도 `https:` 또는 loopback `http:`만 허용한다.

```html
<script src="/dist/browser/certd-web-sdk.global.js"></script>
<script>
  window.WebSdkBrowser.configureHtml5CertificateInput(
    document.getElementById("cert-files"),
    { mode: "directory" },
  );
</script>
```

desktop agent 경로에서 SDK가 내부적으로 수행하는 순서는 다음과 같다.

1. local agent `/healthz` 확인
2. local agent `/sessions/open`으로 local session 확보
3. local agent `approve-*` endpoint 호출
4. agent가 native approval modal로 인증서 선택과 비밀번호 입력 처리
   - 예: `로컬 NPKI | NPKI/yessign/...`, `이동식디스크 PKCS#12 | protected.p12`
5. agent가 gateway challenge 생성, 로컬 sign, gateway 실행까지 처리

브라우저 단독 흐름은 위 agent 단계를 생략하고, 선택된 인증서 파일만으로 challenge 서명과 proof 반환 또는 escrow gateway 호출을 수행한다.

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
export DESKTOP_AGENT_NPKI_DIRS="/Volumes/Lexar/NPKI:/Volumes/Lexar/GPKI"
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
cd packages/web-sdk
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

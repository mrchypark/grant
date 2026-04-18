# Korean Certificate Source Expansion Design

목표는 `desktop-agent`가 한국 환경에서 실제로 많이 쓰이는 공동인증서 저장 형태를 더 넓게 탐색하고, 그 결과를 현재 `cert-gateway` stored/ephemeral flow에 그대로 연결하는 것이다.

이번 변경에서 지원 범위는 다음과 같다.

- `NPKI` 4파일 번들 경로
- `GPKI` 4파일 번들 경로
- 루트 명칭과 디렉터리 깊이가 약간 달라도 `signCert.der`, `signPri.key`, `kmCert.der`, `kmPri.key` 4개가 같이 있으면 후보로 인정
- `.p12`, `.pfx` 기반 `PKCS#12` 파일 탐색
- 로컬 기본 경로와 이동식디스크 루트 모두 탐색

제외 범위는 다음과 같다.

- 브라우저 인증서 저장소 직접 연동
- 스마트인증/클라우드 인증서
- HSM, 보안토큰, 하드웨어 키스토어
- `PKCS#7` 단독 파일을 사용자 선택 대상으로 노출하는 기능

권장 구조는 기존 `desktop-agent/internal/npki`를 "한국 인증서 저장소 전체" 성격의 계층으로 일반화하는 것이다.

- 공통 service는 인증서 메타데이터 목록과 `certificateId` 기반 서명 요청만 다룬다.
- 실제 탐색과 서명은 source adapter가 담당한다.
- 초기 source adapter는 두 개다.
  - `bundle`: `NPKI`/`GPKI`/기타 4파일 세트
  - `pkcs12`: `.p12`/`.pfx`
- 이동식디스크 탐색은 source별 로직이 아니라 공통 root discovery에서 처리한다.

이 구조를 택하는 이유는 세 가지다.

1. 현재 코드는 `NPKI`라는 이름 아래 실제로는 "탐색 + 메타데이터 추출 + 서명"을 모두 같이 갖고 있어서, 지원 포맷이 늘어나면 급격히 복잡해진다.
2. `PKCS#12`는 파일 단위 소스이고 `NPKI`/`GPKI`는 디렉터리 단위 소스라서, 같은 인터페이스 아래에 어댑터로 분리하는 편이 자연스럽다.
3. 이후 브라우저 저장소나 스마트인증을 붙일 때도 `ListCertificates`/`Sign` 계약을 유지한 채 adapter만 추가할 수 있다.

식별자는 더 이상 "bundle directory path" 하나로 고정하지 않는다. source-aware id를 사용한다.

- `bundle:/absolute/path/to/cert-dir`
- `pkcs12:/absolute/path/to/cert-file.p12`

이렇게 해야 같은 CN을 가진 인증서가 여러 개 있어도 실제 소스 타입과 파일 위치를 다시 역참조할 수 있다.

메타데이터는 기존 값에 source 구분용 필드를 추가한다.

- `id`
- `commonName`
- `businessNo`
- `notAfter`
- `sourceKind`
  - `bundle` | `pkcs12`
- `storageKind`
  - `npki` | `gpki` | `pkcs12` | `unknown`
- `displayPath`
  - 승인 모달과 브라우저 예제에서 사람이 식별할 수 있는 축약 경로
- `issuerName`
- `hasKmMaterial`

이 메타데이터 확장이 필요한 이유는 다음과 같다.

- `PKCS#12`와 `NPKI`가 같은 CN을 공유할 수 있다.
- 이동식디스크와 로컬 파일이 동시에 있으면 어느 저장소인지 구분이 필요하다.
- `PKCS#12`에서는 KM material이 없을 수 있으므로, 실행 가능 범위를 미리 표현해야 한다.

서명과 proof bundle 생성 규칙은 다음과 같다.

- `bundle` source
  - 지금과 동일하게 sign cert/key로 challenge를 서명한다.
  - sign/KM 네 파일을 그대로 `ProofBundle`로 변환한다.
- `pkcs12` source
  - `.p12`/`.pfx`에서 sign certificate와 private key를 추출한다.
  - 가능하면 체인 또는 bag에서 KM material을 추출한다.
  - KM material이 없더라도 sign cert/key 기반 challenge proof는 생성한다.

여기서 중요한 설계 결정은 `ProofBundle`의 KM 필드를 optional로 완화하는 것이다.

- 한국 현장에서는 `.p12/.pfx` 하나로 배포되는 경우가 많다.
- 지금처럼 KM 쌍을 항상 필수로 두면 `PKCS#12` E2E 지원이 구조적으로 막힌다.
- 따라서 agent, SDK, API, infra service 모두 `KM` 필드가 비어 있어도 proof 등록과 이후 실행이 가능한지 검토하고, 필요 시 계약을 완화한다.

이동식디스크 탐색 규칙은 다음과 같다.

- macOS: `/Volumes/*`
- Windows: removable drive 루트
- 각 루트에서 다음을 모두 탐색한다.
  - `NPKI`
  - `GPKI`
  - 루트 이하 `.p12`, `.pfx`

탐색은 여전히 best-effort여야 한다.

- 접근 권한이 없는 볼륨은 skip
- 손상된 인증서 파일은 skip
- 일부 소스 실패가 전체 `/certificates` 실패로 이어지지 않도록 한다

API 호환 전략은 다음과 같다.

- `GET /certificates`
  - 기존 필드 유지
  - 메타데이터 필드 확장
- `POST /sign`
  - 요청 계약 유지
  - 응답의 `bundle`은 KM 필드 optional 허용 방향으로 확장
- `approve-ephemeral-run`, `approve-stored-registration`
  - 계약 유지
  - 내부적으로는 richer metadata를 사용해 기본 선택과 표시 문자열만 개선

검증 범위는 세 단계로 나눈다.

1. `desktop-agent` 단위 테스트
   - bundle/gpki/pkcs12/discovery/source-aware id
2. `desktop-agent` HTTP API 테스트
   - 메타데이터 확장 응답과 approval flow
3. 서버/SDK 테스트
   - KM optional bundle 처리와 E2E flow 회귀

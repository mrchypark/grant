# Rust Native Collector Design

**Goal:** Rust 서비스가 공동인증서 로그인과 RPS 수집을 직접 수행한다.

## 범위

- 제거 대상
  - 이전 collector 구현과 Rust 외 빌드 의존성
- 유지 대상
  - 현재 Rust HTTP API 계약
  - `hiqlite` 테이블 스키마
  - PDF 저장소 추상화

## 설계

### 1. 인증서/서명 레이어

Rust `infra` 아래에 NPKI signer 모듈을 둔다.

- `signPri.key` ASN.1 파싱
- NPKI SEED-CBC 복호화
- `vidRandom` 추출
- PKCS#7 signedData 생성
- PKCS#7 envelopedData 생성

구현 우선순위:

1. NPKI 개인키 복호화
2. `LoginPayload`와 동일한 payload 생성
3. 기존 Go 테스트 벡터와 동등성 검증

### 2. RPS 세션/수집 레이어

Rust `infra`에 `RpsCollector`를 추가한다.

- kcert 로그인 페이지 파싱
- `CertLoginEwsCheck.do` 제출
- 반환 폼 제출
- `P_ENTE_ID` 추출
- 설비 목록 / 발전량 / 이용률 / REC / 첨부문서 조회
- PDF 다운로드

### 3. 서비스 레이어 치환

현재 service 경로를 collector 직접 호출 형태로 정리한다.

- `full`: 전체 기간 + PDF 저장
- `delta`: 마지막 월부터 현재 월까지 재수집, PDF 생략

### 4. 제거 기준

아래가 모두 통과하면 Go를 제거한다.

- 인증서 업로드 후 `mode=full` 성공
- `mode=delta` 성공
- `generation/rates/rec/documents` 결과가 현재 기준과 동일
- PDF 다운로드 성공
- Docker 이미지가 Rust 단일 빌드로 생성

## 위험요소

- PKCS#7 생성 포맷이 사이트 요구와 조금만 달라도 로그인 실패 가능
- SEED/NPKI ASN.1 복호화는 테스트 벡터 없이 구현하면 쉽게 깨짐
- delta 응답에서 `null`/빈 배열 처리 차이를 그대로 맞춰야 함

## 성공 기준

- `README`에서 외부 collector 빌드 단계가 사라진다.
- Dockerfile이 Rust 단일 빌드다.
- Rust E2E만으로 수집과 backup 검증이 가능하다.

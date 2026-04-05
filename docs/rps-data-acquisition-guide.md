# RPS 데이터 수집 정리

기준일: `2026-03-30`

이 문서는 현재 워크스페이스에서 확인한 RPS 실데이터와 현재 동작하는 Go live 수집기 기준으로 다음 항목을 정리한다.

1. 인증서 저장 방식
2. 인증서 로그인 후 확보 가능한 데이터
3. 발전량 전체 기간 수집 방식과 인접 발전 평균 수집 방식
4. PDF 저장 위치와 분류 체계

## 1. 인증서 저장

서버 방식 기준으로 필요한 인증서 파일은 아래 4개다.

- `signCert.der`
- `signPri.key`
- `kmCert.der`
- `kmPri.key`

권장 저장 경로:

```text
/Users/cypark/Documents/work/cert/secrets/cert/
```

현재 확인된 파일:

- [signCert.der](/Users/cypark/Documents/work/cert/secrets/cert/signCert.der)
- [signPri.key](/Users/cypark/Documents/work/cert/secrets/cert/signPri.key)
- [kmCert.der](/Users/cypark/Documents/work/cert/secrets/cert/kmCert.der)
- [kmPri.key](/Users/cypark/Documents/work/cert/secrets/cert/kmPri.key)

운영 원칙:

- 인증서 비밀번호와 사이트 로그인 비밀번호는 코드나 문서에 저장하지 않는다.
- 비밀번호는 환경변수나 외부 시크릿 저장소에서 주입한다.
- `signPri.key`를 복호화한 평문 개인키는 디스크에 영구 저장하지 않는다.
- `downloads/`, `tmp/`, `secrets/`는 산출물/민감정보 디렉터리로 취급한다.

## 2. 인증서 로그인 후 확보한 데이터

현재 검증된 대상 설비:

- 설비코드: `PV15-R-090795`
- 발전소명: `커널로그1호 태양광발전소`
- 사업자번호: `2768603291`
- 설비확인서 발급일: `2024-11-12`
- REC 개시일: `2024-09-05`
- 계약번호: `5001058115`
- 위치: `부산광역시 기장군 장안읍 명례리 907-4`
- 설치용량: `221.615 kW`

현재 live 수집 기준 확보 범위:

- `RPS 대상설비 목록`
- `발전량 확인`
- `태양광 이용률 팝업`
- `수수료 납부 및 REC 발급`
- `준공검사 필증 / 개발행위 제출 상세`
- `신규 신청서 전체 첨부파일 22건`

현재 live 출력은 `2024-09 ~ 2026-02` 전체 기간 `18건` 발전량과 `18건` REC 발급 이력을 포함한다.

### 발전량 확인

실제 확인된 기간은 `2025-09 ~ 2026-02`이며, 6건 합계는 `122,483 kWh`다.

| 발전년월 | 발전량(kWh) | 공급전력량(kWh) | 신청기한 | GELEC_ID |
| --- | ---: | ---: | --- | ---: |
| 2026-02 | 21,036 | 21,036 | 2026-05-29 | 18651219 |
| 2026-01 | 23,260 | 23,260 | 2026-05-04 | 18319998 |
| 2025-12 | 19,170 | 19,170 | 2026-03-31 | 18035610 |
| 2025-11 | 22,014 | 22,014 | 2026-03-02 | 17811623 |
| 2025-10 | 15,890 | 15,890 | 2026-01-29 | 17529061 |
| 2025-09 | 21,113 | 21,113 | 2025-12-29 | 17318080 |

### 태양광 이용률 팝업

`O2_02_02_026_cstpop.do` 기준 필드 의미:

- `ONE_RATE`: 본인 발전소 이용률
- `TWO_RATE`: 전국 평균
- `FOUR_RATE`: 인근지역 평균
- `THREE_RATE`: 인근지역 최고
- `CONTROL=N`: 해당 월은 다음 달에 공개

확인된 값:

| 발전년월 | 본인 | 전국 평균 | 인근 평균 | 인근 최고 | 비고 |
| --- | ---: | ---: | ---: | ---: | --- |
| 2026-02 | - | - | - | - | `CONTROL=N`, 아직 미공개 |
| 2026-01 | 14.1 | 12.1 | 13.7 | 40.4 | 조회 가능 |
| 2025-12 | 11.6 | 9.8 | 10.5 | 15.0 | 조회 가능 |
| 2025-11 | 13.8 | 12.6 | 12.9 | 17.4 | 조회 가능 |
| 2025-10 | 9.6 | 10.7 | 9.4 | 11.7 | 조회 가능 |
| 2025-09 | 13.2 | 12.5 | 12.9 | 16.2 | 조회 가능 |

현재 확보 구간 기준 평균:

- 본인 평균 이용률: `12.46`
- 전국 평균 이용률: `11.54`
- 인근 평균 이용률: `11.88`

### REC 발급

실제 확인된 기간은 `2025-09 ~ 2026-02`이며, 합계는 `REC 183`, 발급수수료 `10,065원`, 미납 합계 `0원`이다.

| 발전년월 | APP_NO | 공급전력량 | REC | 수수료 | 발급일 | 유효기간 |
| --- | ---: | ---: | ---: | ---: | --- | --- |
| 2026-02 | 12154225 | 21,036 | 31 | 1,705원 | 2026-03-25 | 2029-03-25 |
| 2026-01 | 11949166 | 23,260 | 35 | 1,925원 | 2026-02-25 | 2029-02-25 |
| 2025-12 | 11756015 | 19,170 | 29 | 1,595원 | 2026-01-26 | 2029-01-26 |
| 2025-11 | 11537296 | 22,014 | 33 | 1,815원 | 2025-12-26 | 2028-12-26 |
| 2025-10 | 11326193 | 15,890 | 24 | 1,320원 | 2025-11-25 | 2028-11-25 |
| 2025-09 | 11144213 | 21,113 | 31 | 1,705원 | 2025-10-27 | 2028-10-27 |

## 3. 전체 기간 발전량과 인접 발전 평균

### 발전량 전체 기간

검증된 화면은 [CST_O2_O2_02_02_010_cst.do.html](/Users/cypark/Documents/work/cert/tmp/rps_crawl/CST_O2_O2_02_02_010_cst.do.html) 이다.

조회 패키지:

- `O2_02_02_010_S`
- 엔드포인트: `POST /comm/SP_RPS_G2.do`

주요 파라미터:

- `P_ENTE_ID=175858`
- `P_SDT=YYYY-MM`
- `P_EDT=YYYY-MM`
- `P_MNG_ORG=%`
- `P_STAT_INFO=%`
- `P_POPLNT_TERM=''`
- `P_PKG_NM=O2_02_02_010_S`
- `P_PAGE_YN=Y`
- `P_SNUMB`
- `P_PSIZE`

현재 확인된 UI 제약:

- 화면 기본값은 `2025-09 ~ 2026-03`
- 프론트엔드에서는 보통 12개월 이내 범위를 전제로 움직인다

권장 수집 방식:

1. `REC 개시일` 또는 `설비확인서 발급일`을 시작점으로 잡는다.
2. `12개월 이하` 구간으로 나눠 `O2_02_02_010_S`를 반복 호출한다.
3. 결과를 `GELEC_ID` 기준으로 합친다.
4. 각 `GELEC_ID`별로 이용률 팝업 패키지를 붙여 인접 평균을 수집한다.

### 인접 발전 평균

검증된 팝업은 `O2_02_02_026_cstpop.do`다.

조회 패키지:

- `O2_02_02_026_S`

수집 방식:

1. `O2_02_02_010_S`에서 각 월의 `GELEC_ID`를 수집한다.
2. `GELEC_ID`별로 `O2_02_02_026_S`를 호출한다.
3. `FOUR_RATE`를 인접지역 평균으로 저장한다.
4. `CONTROL=N`이면 아직 미공개 상태로 기록한다.

권장 저장 구조:

```json
{
  "month": "2026-01",
  "gelec_id": "18319998",
  "generation_kwh": 23260,
  "supply_kwh": 23260,
  "self_rate": 14.1,
  "national_rate": 12.1,
  "nearby_avg_rate": 13.7,
  "nearby_max_rate": 40.4,
  "published": true
}
```

## 4. PDF 저장과 분류 체계

### 저장 위치

현재 신규 신청서 전체 첨부는 아래에 저장했다.

- [rps_application_202409110041](/Users/cypark/Documents/work/cert/downloads/rps_application_202409110041)

준공검사/개발행위 제출 화면에서 바로 내려받은 일부 3건은 아래에 저장했다.

- [rps_o2_04_02_011](/Users/cypark/Documents/work/cert/downloads/rps_o2_04_02_011)

### 분류 체계

Go 산출물 기준 카테고리는 아래 8개다.

| 카테고리 ID | 의미 | 대표 파일 |
| --- | --- | --- |
| `plant_info` | 발전소 기본 식별 서류 | 사업자등록증 |
| `permit` | 인허가/검사/운전개시 | 발전사업허가증, 사용전검사증 |
| `site_land` | 부지/건축물/토지 관련 | 토지대장, 건축물대장, 공장등록증명서 |
| `drawings` | 설계/배치/결선 도면 | 단선결선도, 설치도면, 건축물 현황도 |
| `photos` | 현장/내부 사진 | 공사사진대장, 건축물 내부사진 |
| `equipment` | 설비 구성 및 인증서 | 구조안전확인서, 모듈/인버터 KS 인증서 |
| `finance` | 비용/정산 근거 | 견적서, 세금계산서 |
| `development` | 개발행위 관련 서류 | 준공검사 접수증, 준공필증, 허가서 |

원본 RPS 첨부 `fileClass` 기준 목록은 아래와 같다.

#### 발전소정보

- [1.사업자등록증(대표자변경)-(주)대성엄정에너지_조성천.pdf](/Users/cypark/Documents/work/cert/downloads/rps_application_202409110041/1.사업자등록증(대표자변경)-(주)대성엄정에너지_조성천.pdf)
- [2.발전사업(변경)허가증_대성부산1호. (주)대성엄정에너지. 조성천.pdf](/Users/cypark/Documents/work/cert/downloads/rps_application_202409110041/2.발전사업(변경)허가증_대성부산1호.%20(주)대성엄정에너지.%20조성천.pdf)
- [3.사용전검사확인증_대성부산1호.pdf](/Users/cypark/Documents/work/cert/downloads/rps_application_202409110041/3.사용전검사확인증_대성부산1호.pdf)
- [4.단선결선도_대성부산1호 태양광발전소.pdf](/Users/cypark/Documents/work/cert/downloads/rps_application_202409110041/4.단선결선도_대성부산1호%20태양광발전소.pdf)
- [5.상업운전개시확인서_대성부산1호.pdf](/Users/cypark/Documents/work/cert/downloads/rps_application_202409110041/5.상업운전개시확인서_대성부산1호.pdf)

#### 발전방식

- [6.공사계획(변경)신고필증(대성부산1호).pdf](/Users/cypark/Documents/work/cert/downloads/rps_application_202409110041/6.공사계획(변경)신고필증(대성부산1호).pdf)
- [7.토지대장 및 지적도 등본_명례리907-4.pdf](/Users/cypark/Documents/work/cert/downloads/rps_application_202409110041/7.토지대장%20및%20지적도%20등본_명례리907-4.pdf)
- [8.건축물대장 및 건축물 현황도_명례리 907-4.pdf](/Users/cypark/Documents/work/cert/downloads/rps_application_202409110041/8.건축물대장%20및%20건축물%20현황도_명례리%20907-4.pdf)
- [공장등록증명서_콜링씨앤디.pdf](/Users/cypark/Documents/work/cert/downloads/rps_application_202409110041/공장등록증명서_콜링씨앤디.pdf)
- [건축물 내부사진.pdf](/Users/cypark/Documents/work/cert/downloads/rps_application_202409110041/건축물%20내부사진.pdf)
- [콜링씨앤디_세금계산서.pdf](/Users/cypark/Documents/work/cert/downloads/rps_application_202409110041/콜링씨앤디_세금계산서.pdf)

#### 설비구성

- [9.공사사진대장_대성부산1호.pdf](/Users/cypark/Documents/work/cert/downloads/rps_application_202409110041/9.공사사진대장_대성부산1호.pdf)
- [10.구조안전확인서 및 검토서.pdf](/Users/cypark/Documents/work/cert/downloads/rps_application_202409110041/10.구조안전확인서%20및%20검토서.pdf)
- [14.대성부산1호 태양광발전소(221.615kW)-모 JAM72D42-635LB, 인 SUN2000-60KTL-MOA-(24.08.28) - 복사본.pdf](/Users/cypark/Documents/work/cert/downloads/rps_application_202409110041/14.대성부산1호%20태양광발전소(221.615kW)-모%20JAM72D42-635LB,%20인%20SUN2000-60KTL-MOA-(24.08.28)%20-%20복사본.pdf)
- [모듈_KS인증서_대성부산1호.pdf](/Users/cypark/Documents/work/cert/downloads/rps_application_202409110041/모듈_KS인증서_대성부산1호.pdf)
- [대성부산1호 태양광발전소 인버터 KS인증서(PV-LGTI-1-0234) (20).pdf](/Users/cypark/Documents/work/cert/downloads/rps_application_202409110041/대성부산1호%20태양광발전소%20인버터%20KS인증서(PV-LGTI-1-0234)%20(20).pdf)
- [대성부산1호 태양광발전소(접속함).pdf](/Users/cypark/Documents/work/cert/downloads/rps_application_202409110041/대성부산1호%20태양광발전소(접속함).pdf)

#### 소요자금

- [15.견적서_대성부산1호.pdf](/Users/cypark/Documents/work/cert/downloads/rps_application_202409110041/15.견적서_대성부산1호.pdf)

#### 개발행위

- [19.개발행위준공검사 신청서[명례리 907-4번지]_접수확인날인.pdf](/Users/cypark/Documents/work/cert/downloads/rps_application_202409110041/19.개발행위준공검사%20신청서[명례리%20907-4번지]_접수확인날인.pdf)
- [20.개발행위허가준공필증_대성부산1호 태양광발전소.pdf](/Users/cypark/Documents/work/cert/downloads/rps_application_202409110041/20.개발행위허가준공필증_대성부산1호%20태양광발전소.pdf)
- [20.개발행위허가준공알림공문_대성부산1호 태양광발전소 .pdf](/Users/cypark/Documents/work/cert/downloads/rps_application_202409110041/20.개발행위허가준공알림공문_대성부산1호%20태양광발전소%20.pdf)
- [19.개발행위(변경)허가서_대성부산1호. (주)대성엄정에너지.조성천.pdf](/Users/cypark/Documents/work/cert/downloads/rps_application_202409110041/19.개발행위(변경)허가서_대성부산1호.%20(주)대성엄정에너지.조성천.pdf)

## 구현 상태 메모

- `onerec.kmos.kr` 서버 로그인과 REC 조회는 현재 Go로 구현되어 있다.
- `rps.energy.or.kr` 실데이터는 현재 Go live 수집기로 직접 정리할 수 있다.
- 현재 Go 산출물은 `발전량`, `이용률`, `REC 발급`, `신청서 첨부파일 전체`를 JSON/PDF로 구조화한다.
- 출력 CLI는 `go run ./cmd/rpssync -live` 이고, 기본 산출 위치는 `output/<설비코드>/latest` 다.
- 서버 모드는 `go run ./cmd/server` 이고, PocketBase를 저장소로 두고 Huma API를 `/api/v1` 아래에 붙인다.
- 서버는 인증서 4종과 인증서 비밀번호를 AES-GCM 마스터키로 암호화해 저장한다.
- 서버 수집 결과는 `businesses -> plants -> sync_runs -> generation/rates/rec/documents` 구조로 적재된다.
- 문서 API는 각 PDF에 대해 카테고리, SHA-256, PocketBase 파일 다운로드 URL을 함께 반환한다.

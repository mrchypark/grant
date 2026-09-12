# 외부 Application 연동 설계와 로드맵

상태: 설계만 기록하며 구현과 배포는 보류한다. 2026-09-12 결정에 따라 현재 지원 범위는 **Kubernetes(K8s)에 직접 배포하는 내부 서비스 Provider**로 한정한다. 운영자가 Provider·액션·HTTP catalog를 등록하고 Exposure와 Grant의 연결을 관리한다. 외부 개발자에게 현재 서비스 키나 Provider 관리 권한을 배포하는 절차는 제공하지 않는다.

이 문서는 기존 API, 스키마, canonical key 및 배포 계약을 변경하지 않는다. 현재 지원 동작은 [catalog coverage](catalog-coverage.md), 연결 계약은 [HTTP capabilities](http-capabilities.md)를 따른다.

## 외부에 제공할 모델

외부 개발자는 Application 하나를 등록하는 흐름을 사용한다. 기존 서비스를 호출하는 앱은 소비자이며, 자신이 제공하는 API도 연결하려는 앱은 선택적으로 제공자 기능을 신청한다. Provider는 액션·리소스·HTTP 계약을 구분하는 내부 개념으로 유지한다. 제공자 기능이 필요한 경우 플랫폼이 내부 Provider와 catalog 및 PEP 연결을 관리하는 설계를 검토한다. Application과 Provider를 물리적으로 합치거나 기존 `patch-data.*` 액션 ID를 변경하기로 결정한 것은 아니다.

향후 단일 등록 흐름이 IdP client 생성과 Grant Application 생성을 조정하고, 두 항목을 `client_id`로 연결한다. IdP는 자격 증명, redirect URI, 인증 flow와 계정 인증을 소유한다. Grant는 앱 소유 조직, 요청 액션, 동의와 내부 Provider 연결을 소유한다. 로그인만 필요한 앱은 IdP 등록만 사용할 수 있다. 현재 Grant가 IdP client를 자동 생성한다는 의미는 아니며, 이 통합 등록 기능도 이번 작업에서는 구현하지 않는다.

제공자 기능으로 정의하는 자기 서비스의 액션과, 소비자로서 다른 Provider에 요청하는 액션은 별도로 관리한다. 자기 액션을 등록할 수 있다는 사실이 다른 Provider의 액션을 사용할 권한을 뜻하지 않으며, 하나의 Application 등록 흐름에서도 두 권한 집합을 구분한다.

계정은 여러 앱에서 공유하는 IdP identity와 Grant의 내부 사용자로 연결한다. 앱마다 별도 계정을 생성하지 않는다. 운영 인증은 reid/Rauthy 연동을 목표로 하며, PocketBase/Patch Auth는 현재 통합 예제로 구분한다. 현재 구현된 adapter 설정은 `rauthy`와 `pocketbase`다. reid라는 별도 adapter나 Rauthy와 같은 프로토콜이 구현되었다고 가정하지 않는다. 실제 인증 계약은 [권한 모델](authorization-model.md)을 따른다.

앱 소유 조직은 개발·배포 책임을 지는 조직이고, 리소스 조직은 접근 권한을 부여하는 조직이다. 향후 여러 고객 조직이 같은 앱을 사용하려면 앱 등록과 조직별 설치·허용 상태를 구분해야 한다. 현재는 Application의 조직과 선택한 조직이 일치해야 하며, PocketBase 모드는 서비스마다 고정 ApplicationID를 사용한다. 새 앱 등록만으로 다중 앱·다중 조직 연동이 완료되지는 않는다.

접근 허용은 사용자·조직·Provider·Application의 활성 상태, 사용자 RBAC, 앱에 허용한 액션, 현재 사용자 동의의 교집합으로 결정한다. 앱 등록·소유권·동의만으로 리소스 권한을 부여하지 않는다. 기존 trusted auto-approve 정책을 외부 앱에 자동 적용하지 않는다. 조직을 넘는 권한은 대상 조직과 agreement·binding 정책에 따라 명시적으로 부여해야 한다.

## 향후 연결 방식

우선 검토할 방식은 플랫폼이 운영하는 Exposure를 통한 외부 API 연결이다. 외부 backend의 고정 origin, 경로와 메서드, 액션·리소스 추출, 본문·응답 한도와 철회 동작을 검토한다. backend에 직접 접근하여 검사를 우회할 수 없도록 신뢰 경계도 정해야 한다.

mTLS 등 서비스별 인증, 자격 증명 회전·철회, origin 검증과 tenant 격리는 향후 설계 항목이다. SDK는 검증된 계약의 사용을 돕는 수단으로 검토한다. SDK가 제출한 subject나 resource 주장을 무조건 신뢰하지 않으며, mTLS도 요청 evidence와 권한 검사를 대신하지 않는다. 이러한 외부용 기능은 아직 지원한다고 선언하지 않는다.

제안하는 mTLS 구성에서는 Exposure가 플랫폼이 발급한 client 인증서와 개인키를 보유하고, 인증서 identity를 연결 대상 Provider 범위로 제한한다. Provider에는 신뢰할 CA와 예상 identity 및 SDK·Ingress 설정을 전달하며 Exposure의 개인키는 전달하지 않는다. Provider의 TLS 종료 지점(SDK가 구성하는 TLS 서버 또는 Ingress)은 인증서 체인·만료·identity를 검증하고, Exposure는 backend의 HTTPS 서버 인증서와 hostname을 검증한다. Provider는 인증서 회전·철회 정책의 적용과 미인증 직접 요청의 거부를 보장해야 한다. mTLS는 연결한 프록시를 인증하며, 최종 사용자의 권한은 Grant·Exposure의 별도 판정으로 확인한다. 이 구성은 향후 설계이며 현재 구현된 외부 연결 계약이 아니다.

외부 개발자가 Exposure를 직접 운영하는 방식은 선택적인 후속 검토 대상으로 둔다. 운영 주체가 생성하는 evidence의 신뢰 범위, catalog 배포·철회 및 backend 보호 검증을 먼저 정해야 한다. 외부 조직에 Grant learner와 권한 replica를 배포하는 방식은 보류한다. 내부 learner의 전체 권한 복제·fence 계약을 외부에 그대로 확장하지 않는다.

## 단계별 완료 기준

| 단계 | 범위 | 완료 기준 |
|---|---|---|
| 현재 | K8s에 직접 배포하는 내부 Provider | 등록 액션과 활성 catalog의 일치, 실제 요청의 허용·거부·철회, 비공개 경로 차단을 검증하고 기존 내부 배포 절차를 유지한다. |
| 설계 확정 | 외부 Application과 조직별 사용 모델 | 소비자·선택적 제공자 흐름, IdP client 생성과 Grant 등록의 연결·실패 복구, 앱 소유 조직과 리소스 조직, identity/client 매핑, 동의·철회와 책임 경계를 문서로 확정한다. |
| 제한된 외부 연동 | 플랫폼 운영 Exposure | 서비스별 인증·회전·철회, backend 우회 방지, 잘못된 origin·tenant·evidence 거부, 권한 철회 및 장애 시 접근 거부를 실제 테스트로 증명한 뒤 지원 여부를 결정한다. |
| 개발자 도구 | SDK와 등록 경험 | 동일 계정의 여러 앱 사용, 조직별 허용 액션, 동의 축소·앱 폐기 및 기존 namespace 보존을 검증한다. SDK 없이도 같은 서버 검증을 적용한다. |
| 후속 검토 | 외부 자체 운영 Exposure | 별도의 신뢰 경계와 운영 검증을 확정한 뒤 검토한다. 외부 learner 복제는 기본 범위에 포함하지 않는다. |

미래 단계마다 별도 구현 결정을 내린다. 관계·제약 또는 canonical key의 의미를 변경할 때는 [database-schema.md](database-schema.md)의 릴레이션·구조의 이유와 이전 계획을 함께 갱신한다. 이 로드맵 자체는 현재 데이터베이스 의미를 변경하지 않는다.

## 현재 구현 근거

- [Application과 Provider 모델](../internal/grant/models.go), [관계와 PEP 제약](../internal/grant/store_schema.sql)
- [조직과 Application의 인증 매핑](../internal/grant/service_decisions.go)
- [액션 등록](../internal/grant/service_catalog.go), [catalog와 PEP 운영](../internal/grant/service_capabilities.go)
- [내부 learner와 배포 구성](runtime-modes.md)

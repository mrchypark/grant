# TrailBase Auth Integration Plan

`TrailBase = AuthN`, `certd + hiqlite = AuthZ + domain` 구조로 연동한다. 두 서비스는 별도 바이너리/별도 프로세스로 배포한다.

## 목표

- TrailBase 기본 auth UI로 로그인 가능
- `certd`가 TrailBase bearer token으로 사용자 식별 가능
- local `organization` 권한 모델을 그대로 유지
- custom UI 없이도 1차 E2E 가능

## 단계

### 1. TrailBase 운영 스켈레톤 추가

- `docker-compose` 또는 `k8s`에 TrailBase 추가
- TrailBase data dir와 config 경로 정리
- 기본 admin/bootstrap 절차 문서화

산출물:

- TrailBase 실행 설정
- 로컬 개발 실행 문서

현재 코드 기준으로 2~8 단계는 완료됐다. 남아 있는 건 배포 설정과 실제 TrailBase 서비스 연결이다.

### 2. TrailBase auth bridge endpoint 추가

- TrailBase custom endpoint `/api/certd/introspect`
- bearer token 검증
- `trailbase_user_id`, `email`, `display_name`, `verified`, `active`, `expires_at` 반환
- 내부 호출 보호 수단 추가

산출물:

- TrailBase 확장 코드
- introspection contract 문서

### 3. certd auth 경로 단일화

- 헤더 기반 테스트용 컨텍스트 resolver 제거
- TrailBase bearer token만 허용
- `CERTD_TRAILBASE_INTERNAL_SECRET` 필수화

산출물:

- TrailBase bearer-only middleware
- 내부 introspection client

### 4. user identity 매핑 스키마 추가

- `user_identities` 테이블 추가
- `provider = trailbase`
- `(provider, external_user_id)` unique

산출물:

- schema migration
- domain model

### 5. TrailBase token introspection 연동

- `certd`에서 TrailBase 검증 endpoint 호출
- 응답 모델 정의
- 짧은 메모리 캐시 추가

산출물:

- TrailBase auth client
- token cache

### 6. shadow user upsert

- introspection 결과에서 local user 찾기
- 없으면 생성
- 있으면 `last_seen_at`, email/display_name snapshot 갱신

산출물:

- `resolve_or_create_user_from_trailbase()`

### 7. organization context 결합

- `X-Organization-Id` 유지
- local membership 확인
- principal 구성

산출물:

- authenticated principal type
- request context extractor

### 8. 기존 authorization 경로 연결

- `organization.*` 평가
- `plant.*` 평가
- `effective_user_plant_visibility`
- `effective_user_plant_actions`

산출물:

- 기존 API를 TrailBase token 기반으로 구동

### 9. TrailBase E2E

- TrailBase에서 사용자 생성
- 로그인 후 token 획득
- `certd` API 호출
- plant list/detail/sync 실행 확인

산출물:

- E2E 테스트 절차 또는 자동화 스크립트

## 비목표

- 조직/권한 데이터를 TrailBase로 이동하지 않는다
- 1차에서는 custom auth UI를 만들지 않는다
- 1차에서는 OAuth/MFA를 필수로 구현하지 않는다

## 위험 요소

- TrailBase 검증 endpoint 설계 미완성
- token cache TTL이 너무 길면 무효화 반영이 늦어짐
- TrailBase와 certd의 user lifecycle 불일치

## 남은 작업

1. 실제 TrailBase 서비스 기동 설정 추가
2. `trailbase-auth-bridge` 실행 방법 문서화
3. `certd`와 TrailBase를 함께 띄우는 배포 스크립트 또는 매니페스트 작성
4. 실제 TrailBase 로그인 토큰으로 `certd` E2E 검증

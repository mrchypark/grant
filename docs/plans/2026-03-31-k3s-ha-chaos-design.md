# K3s HA Chaos Design

**Goal:** `certd`를 K3s 계열 환경에 HA로 배포하고, pod/node 장애와 backup/restore 시나리오를 실제로 검증한다.

## 선택한 접근

로컬 검증은 `k3d`를 사용해 K3s와 동일한 운영 모델을 재현한다.

- 클러스터: `k3d` 1 server + 3 agents
- 앱: `certd` 3 replicas
- 메타데이터: `hiqlite` 3노드
- 문서 저장: K8s에서는 `ARTIFACTS_BACKEND=s3`로 MinIO 공유 스토리지 사용
- backup: `hiqlite` S3 backup을 MinIO에 저장

## 대안과 판단

1. `kind + local filesystem`
- 간단하지만 K3s와 다르고, 멀티 replica에서 PDF 일관성이 깨진다.

2. `k3d + MinIO shared object storage`
- 추천안이다.
- K3s 동작과 가깝고, 모든 pod가 동일한 PDF 객체를 읽을 수 있다.

3. 실제 `k3s` 로컬 설치
- 가장 실제와 가깝지만 로컬 준비 비용이 높고 자동화가 무겁다.

## 설계 핵심

- `certd`는 StatefulSet 3 replicas로 배포한다.
- `hiqlite`는 기존 StatefulSet 내부 구성을 그대로 사용하되 anti-affinity와 PDB를 둔다.
- PDF/object storage와 backup은 MinIO를 사용한다.
- MinIO 버킷은 `cert-artifacts`와 `cert-backups` 두 개로 분리한다.
- chaos는 별도 프레임워크 대신 우선 `kubectl delete pod`, `kubectl drain`, `scale down/up`으로 검증한다.

## 검증 시나리오

1. fresh cluster 배포
2. 인증서 업로드
3. `mode=full` sync
4. 요약/문서 다운로드 확인
5. `certd` pod 1개 강제 삭제
6. `mode=delta` sync 재확인
7. worker node drain으로 reschedule 확인
8. backup 생성
9. 전체 StatefulSet scale down
10. `HQL_BACKUP_RESTORE`로 restore 수행
11. scale up 후 데이터/문서 재검증

## 성공 기준

- `mode=full`과 `mode=delta`가 K3s 환경에서 성공한다.
- pod 장애 이후에도 API와 문서 다운로드가 정상이다.
- backup 객체가 MinIO에 생성된다.
- restore 후 발전소 요약, REC, 문서 다운로드가 복구된다.

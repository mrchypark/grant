# K3d HA Topology Design

목표는 `cert` 레포의 현재 단일 `Deployment` 기반 배포를, 실제 검증하려는 `hiqlite` 운영 토폴로지로 바꾸는 것이다.

권장 구조는 다음과 같다.

- `certd-voter`: `StatefulSet` 3개
- 저장소: `emptyDir`
- 식별자: `HQL_NODE_ID_FROM=k8s`
- 네트워킹: `headless Service` + `publishNotReadyAddresses: true`
- `certd-learner-{1,2,3}`: `Deployment` 3개
- learner는 `split` 런타임과 `CERTD_LOCAL_NODE_ROLE=learner` 사용
- artifact 저장과 hiqlite backup은 모두 MinIO 사용

이 구조를 택하는 이유는 두 가지다.

1. `StatefulSet`이 안정적인 pod 이름과 ordinal을 제공해서 `hiqlite` voter quorum의 `node_id`를 고정할 수 있다.
2. learner는 quorum에 들어가지 않으므로 `Deployment`로 분리해 로컬 read replica 역할만 맡기면 된다.

대안은 두 가지였지만 채택하지 않는다.

- 단일 `Deployment` + restore-on-start:
  - 구현은 단순하지만 quorum/leader/learner 검증이 불가능하다.
- `Deployment` 3개 voter:
  - pod identity가 불안정해서 `hiqlite` Raft membership 전제와 맞지 않는다.

데이터 흐름은 다음과 같다.

- voter는 embedded hiqlite node로 quorum을 구성한다.
- learner는 local embedded learner + remote quorum client를 같이 든다.
- 일반 read는 local learner SQLite에서 처리한다.
- write와 strong read는 quorum으로 전달한다.
- artifact 저장과 backup object는 MinIO bucket에 저장된다.

장애 모델은 이렇게 본다.

- voter 1개 장애: quorum 유지, 재선출/재합류로 복구
- learner 장애: read replica만 영향, write는 무영향
- MinIO 장애: artifact 다운로드/backup에 영향, DB quorum과는 분리
- 전체 voter 중단: MinIO backup에서 restore 후 재가동

검증 범위는 다음과 같다.

- `kustomize build deploy/k8s`
- 스크립트 `bash -n`
- 이후 실제 클러스터에서는 bootstrap, learner join, chaos, backup/restore 순서로 확인

# Grant Backup / Restore

> Historical Hiqlite guide. Commands and source references apply only to the [c716877 legacy checkout](https://github.com/Conalog/grant/tree/c716877e7b970b7d38392b377f64384936992884), not the current native Go runtime. See [current recovery](../../backup-restore.md).

Grant의 복구 단위는 Grant 권한 데이터와 Rauthy identity 데이터를 함께 고정한 recovery generation이다. 두 시스템은 모두 3노드 hiqlite `StatefulSet`과 `emptyDir`을 사용한다. PVC 재부착은 복구 수단이 아니며, 전체 손실은 외부 S3 백업 쌍으로 복구한다.

## 운영 불변조건

- `grant-voter`와 `rauthy`는 각각 voter 3개, `podManagementPolicy: Parallel`, `PodDisruptionBudget.minAvailable: 2`로 실행한다.
- production overlay는 hostname 기준 hard anti-affinity와 `DoNotSchedule` topology spread를 요구한다. 3개 failure domain의 실제 수용량이 없으면 배포하지 않는다.
- 두 데이터 디렉터리는 `emptyDir`이다. 한 Pod 손실은 남은 quorum에서 재가입하지만, 세 Pod 동시 손실은 S3 복구가 필요하다.
- production lifecycle mode의 기본값은 `steady`다. 빈 Pod는 직접 ordinal peer가 cluster-ready일 때만 시작한다. production은 `fresh`를 거부하고 `restore`는 recovery state와 이름이 같은 restore authorization Secret이 있을 때만 ordinal 0을 연다. mode 파일이 없거나 잘못되면 즉시 실패한다.
- restore는 외부 Kubernetes operator 스크립트로만 수행한다. HTTP restore API는 제공하지 않는다.
- Grant와 Rauthy는 서로 다른 bucket, IAM principal, encryption keyring, recovery generation을 사용한다.

hiqlite는 bucket root에 `backup_node_<leader-id>_<unix-seconds>.sqlite` 이름으로 저장하며 workload prefix를 지원하지 않는다. 같은 bucket을 공유하면 같은 초의 이름 충돌과 retention 교차 삭제가 가능하므로 별도 bucket이 필수다. 환경이나 cluster generation이 바뀌어도 새 bucket과 새 `BACKUP_GENERATION`을 사용해 이전 retention job과 분리한다.

## Production 입력

`deploy/overlays/production`은 다음 외부 리소스를 생성하지 않는다. 값이 없으면 Pod가 fail closed한다.

- `grant-production-config`, `grant-production-backup`, `grant-production-secrets`
- `rauthy-production-config`, `rauthy-production-backup`, `rauthy-production-secrets`
- operator 소유 `grant-recovery-state`
- 별도 관리되는 production Ingress/TLS와 실제 digest로 치환한 Grant release image. 저장소의 `REQUIRED_PRODUCTION_DIGEST`는 admission-invalid marker이므로 그대로 배포할 수 없다.
- 공식 Rauthy 0.36.0 digest에서 빌드한 `grant-rauthy-launcher` multi-architecture image의 실제 manifest digest. 저장소의 launcher digest token은 publish 전 fail-closed marker이며 그대로 배포할 수 없다.
- private GHCR package를 읽는 namespace Secret `rauthy-launcher-pull`. `deploy/overlays/production/rauthy-launcher-pull-secret.example.yaml`은 형식만 보여주며 실제 `.dockerconfigjson`은 외부 secret manager에서 공급한다.
- workload별 materializer와 restore operator AWS profile의 `iam:GetRole` 권한. `deploy/overlays/production/recovery-role-resolution-policy.example.json`을 각 workload account와 실제 role path에 맞춰 별도로 적용한다.

형식만 보여주는 파일은 `deploy/overlays/production/*.example.yaml`에 있다. 저장소에 실제 credential을 넣지 않는다.

두 workload가 각각 요구하는 backup 값은 다음과 같다.

- `HQL_BACKUP_CRON`, `HQL_BACKUP_KEEP_DAYS`, `HQL_BACKUP_KEEP_DAYS_LOCAL`
- `HQL_S3_URL`, `HQL_S3_BUCKET`, `HQL_S3_REGION`, `HQL_S3_PATH_STYLE`
- `HQL_S3_KEY`, `HQL_S3_SECRET`
- `ENC_KEYS`, `ENC_KEY_ACTIVE`, `KEYRING_GENERATION`

`ENC_KEYS`는 현재 active key만이 아니라 retention 기간의 모든 백업을 복호화할 수 있는 versioned keyring이어야 한다. Grant keyring과 Rauthy keyring을 별도로 escrow하고, 삭제·회전 이력을 recovery record와 함께 보존한다. record는 `KEYRING_GENERATION`과 `ENC_KEYS` 원문 SHA-256을 모두 고정하며 restore operator는 ordinal 0을 기동하기 전에 실제 Secret을 decode해 같은 digest인지 확인한다. S3 principal은 workload와 역할별 최소 권한으로 분리한다. 평상시 backup writer, object-version materializer, restore reader는 모두 다른 principal이어야 한다.

STS `assumed-role/<role-name>/<session-name>` ARN에는 IAM role path가 들어 있지 않으므로 문자열 조합으로 static role ARN을 만들지 않는다. recovery pair 생성과 전체 복구 스크립트는 assumed-role principal을 비교하기 전에 해당 workload profile로 `iam:GetRole`을 호출하고 AWS가 반환한 exact `Role.Arn`으로만 비교한다. role name은 최대 64자, session name은 2~64자와 AWS 허용 문자 집합을 만족해야 하며 lookup 실패, account/partition/name 불일치, 잘못된 path 추정은 모두 fail closed한다. static IAM role ARN은 입력 그대로 exact 비교한다. materializer profile과 restore-reader profile에는 그 workload의 writer, materializer, reader role만 Resource로 제한한 `iam:GetRole`을 부여한다. Grant와 Rauthy가 다른 account라면 policy도 분리하며 cross-account role을 한 profile에서 해석한다고 가정하지 않는다.

Grant에 주입하는 Rauthy 검증 API key는 `Clients` 그룹의 `read` 권한만 부여하며, master provisioning verify가 `GET /auth/v1/clients/{client_id}`로 예약된 client 설정을 확인할 때만 사용한다. client 생성·수정과 client secret 회전은 별도 운영자/IaC credential로 수행하고 그 credential은 Grant에 주입하지 않는다. 검증 key의 bootstrap Secret, 현재 key 이름, 만료, 회전·폐기 기록은 외부 secret manager에서 관리하며, 새 read-only key로 Grant 검증 호출을 확인한 뒤 이전 key를 폐기한다.

## 백업 완료 판정

cron 또는 backup 요청 성공은 local snapshot 생성 요청을 받았다는 뜻이며 S3 업로드 완료 증거가 아니다. 운영 모니터는 다음을 확인한다.

1. 예상 cron 이후 새 object version을 S3에서 polling한다.
2. object 크기, version ID, ciphertext SHA-256을 기록한다.
3. Grant와 Rauthy object를 동일한 write-fence 구간에서 recovery generation으로 승인한다.
4. 고정 image와 escrow keyring을 사용하는 격리 namespace restore가 semantic check를 통과해야 production restore 승인 상태가 된다.

`scripts/materialize-s3-restore.sh`는 exact source `versionId`가 보일 때까지 polling한다. 따라서 비동기 업로드가 아직 끝나지 않았으면 승인하지 않는다.

## Recovery pair 생성

`scripts/create-recovery-pair.sh`는 미리 적용된 traffic/write fence와 서명된 fence evidence를 검증하고, 두 source version을 workload별 bucket의 새 key로 materialize한다. namespace에는 `grant.cypark.dev/recovery-write-fence=<recovery-id>` label이 있어야 하며 public Service endpoint는 비어 있고 모든 CronJob은 suspend 상태여야 한다.

Fence signer는 direct/headless writer, 관리 작업과 외부 queue consumer가 모두 중지된 구간의 namespace UID, 두 source version, 시작/종료 시각을 JSON에 기록하고 detached OpenSSL signature를 만든다. `snapshots.grant`와 `snapshots.rauthy`에는 각각 `snapshotId`, `sourceKey`, `sourceVersionId`, 실제 snapshot `createdAt`을 넣는다. `createdAt`과 source object의 `LastModified`가 모두 서명 구간 안에 있어야 한다. 비동기 업로드의 `LastModified`만으로는 fence 뒤 snapshot 생성을 증명하지 못하므로 승인하지 않는다. `release.cronJobsToResume`에는 fence 전 active였고 완료 후 재개할 CronJob 이름만 넣는다. 단순 환경변수 assertion은 승인 증거로 인정하지 않는다.

필수 metadata:

- recovery ID와 generation
- workload별 backup generation
- source bucket, object key, version ID, ciphertext SHA-256
- exact image digest, schema ID, encryption key ID, complete keyring digest/generation
- workload별 backup writer, materializer, restore reader ARN
- isolated drill ID와 성공 상태
- Object Lock retention 종료 시각

materialized key는 `restore/<workload>/<recovery-id>/<sha256>.enc`이다. bucket versioning과 S3 Object Lock `COMPLIANCE`가 필수이며 retention은 최소 24시간 뒤여야 한다. materializer는 AWS CLI paginator로 exact-key prefix의 모든 `Versions`와 `DeleteMarkers`를 읽고 `Key`가 정확히 같은 history가 하나라도 있으면 재사용을 거부한다. 첫 upload도 `If-None-Match: *` 조건으로 경쟁 overwrite를 막는다. HQL은 `versionId`를 직접 읽지 못하므로 첫 version을 만든 직후 그 exact key에 대한 `PutObject`, `DeleteObject`, `DeleteObjectVersion` explicit Deny bucket-policy 문을 설치한다. 별도 explicit Deny는 평상시 backup writer의 `GetObject`/`GetObjectVersion`도 이 restore key에서 차단한다. upload 직후와 정책 설치 뒤 history가 새 version 정확히 하나이고 delete marker가 없는지 확인한 다음 current version, exact retention, ciphertext SHA를 다시 확인한다. 이 절차는 조건부 PUT과 정책 설치 사이의 관측 가능한 경쟁을 fail closed하지만 외부 분산 lock은 아니다. materialization 동안 해당 bucket policy를 변경하거나 Object Lock governance를 우회할 principal을 동시에 실행하지 않아야 하며, 실패한 시도에서 생성된 locked version은 같은 deterministic key로 재시도하지 않고 새 recovery ID로 다시 승인한다. restore 시 one-shot Secret이 전용 reader의 `HQL_S3_KEY`/`HQL_S3_SECRET`을 Pod에 공급한다. Grant shell wrapper와 Rauthy static launcher는 평상시 writer credential을 해당 1회 실행에서만 덮어쓴다. operator가 검증한 reader ARN과 실제 Pod reader가 같은 record 값에 묶인다.

## Rauthy launcher image

공식 Rauthy 0.36.0 image는 distroless이고 `/bin/sh`가 없으므로 main container에서 shell restore wrapper를 실행하지 않는다. `deploy/rauthy-launcher`의 stdlib-only Go binary를 `CGO_ENABLED=0`으로 빌드해 digest-pinned 공식 image에 복사한다. launcher는 Secret 값을 인자나 로그에 출력하지 않으며 `/app/rauthy` child에만 환경변수로 전달한다. 종료 신호는 child에 전달하고 child exit code를 보존한다. Secret이 없으면 일반 Rauthy 시작 경로로 동작한다.

복구를 처음 소비하기 전에 launcher는 `/app/data/.rauthy-restore-state.json`을 `pending`으로 원자 기록한다. recovery ID는 32자 이하 lowercase DNS label이어야 하며 `s3:restore/rauthy/<recovery-id>/<sha256>.enc` 형식, lifecycle `restore` mode를 모두 검증한다. child가 hiqlite `/ready`에 연속 3회 성공하고 그 사이 종료하지 않은 뒤에만 상태를 `completed`로 원자 교체한다. `pending` 상태에서 container가 재시작되면 자동 replay하지 않고 실패한다. Secret이 삭제됐어도 `completed` recovery ID와 lifecycle ID가 같으면 일반 시작할 수 있다. `restore`인데 authorization과 같은 completed 상태가 모두 없으면 실패한다. 운영자는 불완전한 복구를 같은 Pod에서 재개하지 않고 전체 recovery checkpoint와 immutable object를 재검증한 뒤 ordinal 0을 명시적으로 re-arm한다.

Grant의 `/gate/restore-once.sh`도 authorization을 검증한 뒤 `/var/lib/grant/hiqlite/.restore-consumed`를 `pending`, recovery ID, exact immutable restore directive로 원자 교체하고 child를 시작한다. 로컬 `/healthz`와 `/readyz`가 제한 시간 안에 연속 3회 모두 성공하고 child가 계속 실행 중일 때만 같은 파일을 `completed`로 원자 교체한다. child가 readiness 전에 종료되거나 신호를 받으면 exit status를 보존하고 `pending`을 유지하며, timeout은 child를 종료한 뒤 124로 실패한다. 같은 recovery ID와 exact directive의 stale `pending`은 authorization이 유지된 동안 재시도하고, 같은 binding의 `completed`만 restore 환경변수를 제거한 일반 시작으로 replay를 억제한다. supervisor는 HUP/INT/TERM을 child에 전달하고 grace 종료 뒤에도 남은 child를 회수한다. 기본 readiness 제한은 300초, probe 간격은 2초이며 image에는 로컬 HTTP probe용 `curl`이 포함된다.

Grant의 restore child는 해당 프로세스가 종료될 때까지 전용 reader credential을 유지하지만 supervisor parent와 readiness probe에는 이를 남기지 않는다. 정상 `fresh`/`steady` 시작과 `completed` replay는 항상 Pod의 평상시 backup writer credential을 사용한다. 복구에 사용한 ordinal 0이 계속 실행 중이면 단계 6의 `partition: 0` restart로 no-restore template을 적용하기 전에는 writer credential이 복귀하지 않으므로, 그 restart와 readiness를 확인하기 전에는 backup CronJob이나 direct writer를 재개하지 않는다.

PR의 `rauthy-launcher-image` workflow는 read-only 권한으로 unit/race/vet, 실제 `/app/rauthy --help` 실행 계약과 amd64/arm64 no-push build를 검사한다. PR에서는 launcher digest marker를 source/contract 검증에 한해 허용하며 production 정적 검증의 기본 동작은 계속 fail closed다. PR 또는 feature ref에는 `packages:write`를 부여하지 않는다. merge 후 production publish만 `main`의 `rauthy-launcher-image`를 정확한 main HEAD SHA와 confirmation `publish-rauthy-launcher-production`으로 실행한다. 보호 환경 `rauthy-launcher-production-publish`는 `main`만 허용하는 custom deployment branch policy로 제한하며 workflow도 dispatch ref, 입력 SHA, checkout HEAD가 모두 같은지 재검증한다. 현재 private organization plan은 required reviewer 설정을 거부하므로 reviewer 승인을 보호 수단으로 주장하거나 의존하지 않는다.

contract와 publish build는 BuildKit `v0.30.0` multi-architecture index digest를 고정한다. publish는 `provenance: mode=max,version=v1`과 digest로 고정한 `docker/buildkit-syft-scanner`가 만든 registry-portable OCI attestation을 권위 있는 공급망 증거로 사용한다. push 뒤 tag의 exact digest, linux/amd64·linux/arm64 manifest와 연결된 attestation manifest를 확인하고, 각 in-toto layer blob을 pull 전용 registry bearer로 받아 descriptor size와 SHA-256을 검증한다. 이어 전체 in-toto envelope의 type·predicate type·platform manifest subject binding과 SLSA v1 predicate의 BuildKit build type, source label/VCS revision, config/context path, resolved dependencies, LLB/completeness 및 GitHub run builder와 SPDX predicate의 version·document ID·namespace·packages를 확인한 뒤에만 digest를 summary에 기록한다. 새 build의 builder는 현재 run/attempt와 exact 일치해야 한다. 같은 GitHub run의 후속 attempt에서 commit tag가 이미 존재하면 overwrite하거나 다시 build하지 않고 이전 attempt의 builder인지 확인한 뒤 기존 digest를 같은 verifier로 재검증하여 실패한 검증 단계만 안전하게 복구한다. 다른 run이 만든 기존 tag나 현재/future attempt builder는 fail closed다.

raw GHCR/GitHub credential은 token 교환 직후 제거하며 Docker child, argv, 로그에 전달하지 않는다. 현재 private organization plan에서 지원되지 않는 GitHub repository signed attestation은 생성했다고 주장하지 않으며 workflow도 해당 권한이나 action에 의존하지 않는다. 결과 summary의 `sha256:` digest를 받은 운영자는 새 `feature/pin-rauthy-launcher-<main-short-sha>` branch에서 `deploy/k8s/rauthy-statefulset.yaml`의 단일 marker를 치환하고 정적 검증을 통과시키는 후속 PR을 만든다. workflow는 repository contents를 쓰거나 branch/PR을 자동 생성하지 않는다. package visibility를 public으로 바꾸거나 mutable tag만 record/StatefulSet에 사용하면 안 된다.

GitHub Environment `rauthy-launcher-production-publish`에는 custom deployment branch policy로 `main`만 허용한다. 이 private organization plan에서는 required reviewer를 사용할 수 없으므로 workflow의 main-only 조건, exact SHA 일치 검사, 명시적 confirmation을 publication 보호 경계로 유지한다. GHCR package는 이 repository에 연결하고 Actions access를 이 repository로 제한한다. cluster pull credential은 read-only `read:packages` token만 사용한다. 로컬/CI script는 `GHCR_USERNAME`과 `GHCR_TOKEN`이 없으면 derivative image 사용을 거부하고, `--password-stdin`으로 만든 임시 Docker config를 명령별 `docker --config`에만 사용해 기존 Docker context를 보존하며 `rauthy-launcher-pull` Secret을 생성한다. credential 값은 argv나 로그에 넣지 않는다.

이 exact-key Deny는 Object Lock 만료 뒤에도 자동으로 사라지지 않는다. 보존 정책에 따라 restore record와 승인을 폐기한 뒤 bucket-policy의 해당 `immutablePolicySid`를 별도 변경관리로 제거해야 하며, materializer나 restore workflow가 자동으로 제거하지 않는다.

```bash
export RECOVERY_ID=2026-07-14-prod-01
export RECOVERY_GENERATION=prod-generation-42
export GRANT_BACKUP_GENERATION=grant-prod-g42
export RAUTHY_BACKUP_GENERATION=rauthy-prod-g42
export GRANT_KEYRING_GENERATION=grant-keyring-g42
export RAUTHY_KEYRING_GENERATION=rauthy-keyring-g42
export GRANT_KEYRING_SHA256='<sha256-of-exact-ENC_KEYS-bytes>'
export RAUTHY_KEYRING_SHA256='<sha256-of-exact-ENC_KEYS-bytes>'
export GRANT_WRITER_PRINCIPAL_ARN='arn:aws:iam::...:role/grant-backup-writer'
export RAUTHY_WRITER_PRINCIPAL_ARN='arn:aws:iam::...:role/rauthy-backup-writer'
export GRANT_RESTORE_READER_PRINCIPAL_ARN='arn:aws:iam::...:role/grant-restore-reader'
export RAUTHY_RESTORE_READER_PRINCIPAL_ARN='arn:aws:iam::...:role/rauthy-restore-reader'
export RESTORE_RETAIN_UNTIL=2027-07-14T00:00:00Z
export ISOLATED_DRILL_ID=pending
export ISOLATED_DRILL_PASSED=false
export FENCE_EVIDENCE=/secure/fence-evidence.json
export FENCE_SIGNATURE=/secure/fence-evidence.sig
export FENCE_PUBLIC_KEY=/secure/fence-signer-public.pem
export RECOVERY_FENCE_PUBLIC_KEY_SHA256='<pinned-public-key-DER-sha256>'
export GRANT_AWS_PROFILE=grant-materializer
export RAUTHY_AWS_PROFILE=rauthy-materializer

# GRANT_*와 RAUTHY_* source bucket/key/version/SHA/image/schema/key ID를 설정한다.
scripts/create-recovery-pair.sh --namespace grant --output recovery-pair.json
```

최초 record는 production 승인이 아니다. 보호된 `s3-recovery-drill` workflow는 `main` ref에서만 실행되고, UID와 recovery ID annotation으로 고정된 drill namespace에서 저장소의 고정 복구 및 semantic 경로를 실행한다. Secret으로 임의 셸을 받지 않는다. 성공하면 canonical approved JSON 전체에 전용 private key로 detached signature를 만들며 production operator는 별도 배포된 public key로 이를 검증한다. JSON의 `passed` 값을 직접 수정한 파일은 승인으로 인정되지 않는다.

복구 record는 credential을 포함하지 않지만 운영 metadata이므로 변경 이력과 승인을 보호한다. PR CI는 `--dry-run`만 실행한다.

## 전체 클러스터 복구

production destructive restore는 먼저 동일한 recovery pair를 사용한 격리 namespace live drill이 통과해야 한다. 그 후 다음 명령으로 exact recovery ID를 다시 확인한다.

```bash
export GRANT_AWS_PROFILE=grant-recovery-reader
export RAUTHY_AWS_PROFILE=rauthy-recovery-reader
export RECOVERY_SEMANTIC_BEARER_TOKEN=...
export RECOVERY_SEMANTIC_APP_AUTH_KEY=...
export RECOVERY_APPROVAL_PUBLIC_KEY_SHA256='<pinned-public-key-DER-sha256>'
export RECOVERY_OPERATOR_GIT_COMMIT='<approved-workflow-gitCommit>'

scripts/restore-full-cluster.sh \
  --record recovery-pair.json \
  --approval-signature recovery-pair.sig \
  --approval-public-key /secure/recovery-approval-public.pem \
  --semantic-config semantic-config.json \
  --namespace grant \
  --expected-namespace-uid '<immutable-namespace-uid>' \
  --confirm-recovery-id 2026-07-14-prod-01
```

스크립트의 단계는 다음과 같다.

1. 두 public Service를 fence하고, immutable current version·Object Lock·SHA·image digest를 다시 검증한다.
2. 아직 복구 데이터가 생성되기 전에만 두 StatefulSet을 0으로 만든다.
3. Rauthy ordinal 0에 recovery ID가 결합된 `rauthy-restore-once` Secret을 연결하고 `restore` mode에서 기동한다. static launcher는 `emptyDir`에 `pending`을 먼저 기록하고 Rauthy/hiqlite ready가 확인된 뒤에만 `completed`로 바꾼다. Grant의 one-shot wrapper도 directive가 정확히 `s3:restore/grant/<recovery-id>/<lowercase-sha256>.enc`인지 확인해 live backup, 다른 workload 또는 다른 recovery object를 거부하고, health/readiness 확인 뒤에만 `completed`로 바꾼다. Rauthy의 stale `pending`은 명시적 re-arm 전까지 fail closed하고 Grant의 같은-ID stale `pending`은 authorization이 유지된 동안 재시도한다.
4. ordinal 0 Ready와 S3 정책/version 재검증 직후 Secret을 삭제한다. 새 Pod는 authorization이 없어 init에서 막히며, 재시도는 operator의 명시적 re-arm이 필요하다.
5. Rauthy ordinal 0의 one-shot Secret을 삭제하고 checkpoint의 Rauthy lifecycle을 `steady`로 CAS 전환한 뒤 ordinal 0 Ready를 다시 확인한다. 이 Secret 부재·mode 전환·seed readiness가 모두 확인된 상태에서만 `partition: 1` no-restore template과 replicas 3 확장을 적용한다. fresh `emptyDir` peer는 restore authorization이나 로컬 completed marker가 없으므로 `restore` mode로 기동하지 않는다. 명시적 ordinal 0 re-arm은 확장 전에만 lifecycle을 다시 `restore`로 바꾸고, 검증·Secret 삭제 뒤 `steady`로 되돌린다. Grant wrapper는 peer-ready 상태에서 restore directive 없이 일반 join이 가능하지만 같은 one-shot Secret 삭제와 no-restore template 순서를 유지한다. 각 Pod readiness와 native membership의 config가 joint consensus가 아닌 정확히 하나의 voter set `1,2,3`인지 확인한다.
6. lifecycle mode가 `steady`로 유지되는지 확인하고 `partition: 0`으로 ordinal 0만 quorum-preserving restart한다. restore env와 Secret이 모두 사라졌는지 확인한다.
7. Grant에 같은 1→3 절차를 적용한다. learner가 있다면 voter migration과 복구가 끝난 후에만 재가입시킨다. schema migration은 voter만 수행한다.
8. 각 Pod의 native Raft metrics에서 voter IDs `1,2,3`, 같은 leader/term/applied index를 확인한다. Rauthy는 SQLite와 cache Raft를 모두 검사한다. semantic 검사 동안 일반 `rauthy` Service는 계속 fence 상태이며, recovery-only ClusterIP Service와 일시적인 Grant 내부 base URL만 사용한다. 고정 data-only semantic config의 Rauthy health와 Grant application-authz allow/deny 결과가 서명된 drill evidence와 일치하고 내부 route가 원복된 후에만 일반 Service selector를 원복한다.

checkpoint는 operator 소유 `grant-recovery-state` ConfigMap에 recovery ID, generation, phase, checksum, ControllerRevision과 함께 남는다. namespace-scoped Lease가 동시 실행을 막고 ConfigMap은 `resourceVersion` CAS로 정확히 다음 phase로만 교체된다. 알 수 없거나 비어 있는 phase, 건너뛰기, 역행은 모두 거부한다. crash 뒤 남은 Lease는 checkpoint를 확인한 운영자만 수동 삭제한다.

`unfenced`는 terminal audit 상태이며 자동으로 다음 recovery에 넘기지 않는다. 외부 direct writer가 복구되고 namespace의 보호된 `grant.cypark.dev/recovery-direct-writers-released=<recovery-id>` annotation이 설정된 뒤, 같은 서명 record와 `--release-completed`를 다시 사용한다. release는 signed `cronJobsToResume`만 재개하고 namespace write-fence label을 제거한 다음 ConfigMap을 `resourceVersion` CAS로 `recovery-id: none`에 넘긴다. 마지막 recovery ID/generation/checksum/release 시각은 ConfigMap audit field와 namespace annotation에 보존된다.

```bash
scripts/restore-full-cluster.sh \
  --record recovery-pair.json --approval-signature recovery-pair.sig \
  --approval-public-key /secure/recovery-approval-public.pem \
  --namespace grant --expected-namespace-uid '<immutable-namespace-uid>' \
  --confirm-recovery-id 2026-07-14-prod-01 --release-completed
```

live drill은 `RESTORE_STOP_AFTER_PHASE=<phase>`로 checkpoint 직후 operator 중단을 주입할 수 있다. one-shot Secret 삭제 후 아직 peer가 없는 `*-ordinal0-restored` 구간에서 ordinal 0까지 손실했다면 자동 재적용하지 않는다. 같은 immutable record를 다시 확인한 뒤 `--rearm-ordinal0 rauthy` 또는 `--rearm-ordinal0 grant`를 명시해야 한다. 이미 두 peer가 catch-up한 `*-voters-caught-up` 이후에는 re-arm하지 않고 partition cleanup을 재개한다.

## Lifecycle gate와 검증

`deploy/k8s/lifecycle-gate.sh`는 aggregate Service나 Ingress가 아니라 각 StatefulSet ordinal DNS의 cluster-readiness endpoint를 확인한다. production은 local-only `fresh`를 거부하며 restore ID가 state와 Secret에서 일치해야 한다. Rauthy main container는 shell이 아니라 derivative image의 `/usr/local/bin/rauthy-launcher`를 실행한다. production workload ServiceAccount는 API token을 mount하지 않고 recovery state를 수정할 RBAC도 받지 않는다.

정적 검증:

```bash
sh scripts/verify-ephemeral-k8s.sh static
sh scripts/verify-docs.sh
```

수동 live drill은 GitHub Actions `s3-recovery-drill` workflow의 보호된 environment와 전용 namespace에서 실행한다. `RECOVERY_SEMANTIC_CONFIG_JSON_B64`는 조직, action, resource, 예상 allow/deny만 담는 data-only JSON이며 bearer와 `GRANT_APP_AUTH_KEY`는 별도 Secret이다. signing private key, namespace-scoped kubeconfig, workload별 read-only S3 reader를 사용한다. 다음 장애 지점을 모두 반복한다.

- all-empty `steady`
- ordinal 0 restore 전·후 재시작
- one-shot Secret 삭제 직후 재시작
- `partition: 1` scale-up 중 operator 중단
- voter catch-up 후 `partition: 0` cleanup 중단
- checkpoint별 재실행

정적 CI와 dry-run은 실제 S3 복호화, target Kubernetes controller 동작, Rauthy/Grant semantic data를 증명하지 못한다. 실제 provider live drill이 production readiness의 필수 조건이다.

## 마이그레이션과 제한

기존 PVC 기반 Rauthy StatefulSet의 `volumeClaimTemplates`는 immutable field다. in-place patch하지 않는다. breaking 배포 전 recovery pair와 격리 restore를 승인하고, traffic을 fence한 뒤 기존 StatefulSet과 Pod를 삭제·재생성하여 no-PVC manifest를 적용하고 전체 restore 절차를 수행한다. 이전 PVC는 새 cluster semantic 검증과 rollback 기간이 끝날 때까지 별도 보존하고 자동으로 삭제하지 않는다.

Grant와 Rauthy backup은 분산 transaction이 아니다. write fence가 없는 두 최신 object를 임의로 조합하면 identity와 authorization 시점이 달라질 수 있다. recovery generation 밖의 조합은 금지한다.

RPO 입력은 backup cron 간격, S3 업로드 지연, pair 승인 주기다. RTO 입력은 object polling/materialization, ordinal 0 restore, 두 cluster의 voter catch-up, cleanup rollout, semantic 검증 시간이다. 각 live drill에서 이 시간을 기록해 목표와 비교한다.

# Local development

The server and manifest digest executable use Go 1.27. Use `go test ./...`, `go vet ./...`, and `CGO_ENABLED=0 go build ./cmd/...` from the repository root. Rhiza embeds SQLite through its native Go API; no Rust compiler or cgo toolchain is needed.

For a local single voter, select a new directory and provide distinct master/application keys plus the identity provider configuration:

```sh
export GRANT_DATA_DIR="$PWD/.local/grant-rhiza"
export GRANT_BACKUP_DIR="$PWD/.local/grant-backups"
export GRANT_NODE_ID=grant-local
export GRANT_CLUSTER_ID=grant-local
export GRANT_BIND_ADDR=127.0.0.1:8080
export GRANT_PEER_ADDR=127.0.0.1:8100
# Load GRANT_MASTER_KEY and GRANT_APP_AUTH_KEY from private caller-owned files.
# Configure GRANT_RAUTHY_BASE_URL and the Rauthy client credentials, or select
# GRANT_AUTH_PROVIDER=pocketbase with its origin and configured application ID.
go run ./cmd/grant
```

The command does not convert an existing Hiqlite store. Follow [backup-restore.md](backup-restore.md) before reusing existing authority data.

This single-voter command is not the embedded Rhiza Operator configuration. The optional Operator overlay is a fresh, native three-voter logical cluster and rejects these `GRANT_*` storage aliases; see [rhiza-operator.md](rhiza-operator.md).

`sh scripts/verify-go-three-voter.sh` creates an isolated local Docker Versity Gateway container with its POSIX backend and three local Grant processes. It verifies S3 authentication, replicated writes, strong reads, quorum survival and restart with the original WAL. It then checks private Operator authentication, rejection of a same-ID empty-WAL replacement, a fresh learner's health/readiness and business gates, graceful restart during admission, certified promotion, and writes from the promoted voter. A separate fresh three-voter logical restore verifies admission readiness, occupied-target rejection and recovery. This fixture calls the native management API directly; it does not qualify Kubernetes Operator disaster recovery or production fencing. It never invokes kubectl. `KEEP_GRANT_FIXTURE=1` retains its own temporary logs for diagnosis; it never cleans unrelated containers or files.

`sh scripts/verify-go-cli-contract.sh` builds both executables with cgo disabled and checks actual process output, including initialization and the V1 manifest digest contract.

The disposable Kubernetes overlay remains a three-voter topology. Use a dedicated kubeconfig and explicit local context when running `scripts/ci-local.sh`. Never point it at an existing production context. Its object-store fixture credentials are local test values, not operational credentials. Both fixtures pin the official [Versity Gateway v1.8.0 release](https://github.com/versity/versitygw/releases/tag/v1.8.0) image by digest. The overlay creates the bucket directory with a POSIX init container on the gateway's shared `emptyDir`; it needs no S3 client image. Bootstrap checks public image availability before applying workloads and waits at most two minutes for the object store before waiting on Rauthy or Grant.

Compute the V1 authority manifest digest with:

```sh
go run ./cmd/grant-patch-data-manifest-digest < manifest.json
```

The receipt-only V2 authority snapshot receiver and exporter have been removed. Runtime logical backup and restore remain separate supported operations.

Historical Rust commands are preserved in [the previous development guide](history/hiqlite/local-development.md).

For local patch-auth, set `GRANT_AUTH_PROVIDER=pocketbase`,
`GRANT_POCKETBASE_BASE_URL=http://127.0.0.1:8091`,
`GRANT_POCKETBASE_COLLECTION=users`, and `GRANT_POCKETBASE_APPLICATION_ID`.
The configured origin/collection is the only authentication authority; domain
membership is provisioned separately in Grant. Use a fresh Grant database after
the scoped identity schema change. The previous Patch Data authentication mode
and environment aliases are removed; business V1 imports remain available with
explicit `issuer` and `collectionId` in every imported user.

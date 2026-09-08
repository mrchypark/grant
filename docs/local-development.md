# Local development

The runtime and all three executables use Go 1.27. Use `go test ./...`, `go vet ./...`, and `CGO_ENABLED=0 go build ./cmd/...` from the repository root. Rhiza embeds SQLite through its native Go API; no Rust compiler or cgo toolchain is needed.

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
# GRANT_AUTH_PROVIDER=patch-data with its origin and configured application ID.
go run ./cmd/grant
```

The command does not convert an existing Hiqlite store. Follow [backup-restore.md](backup-restore.md) before reusing existing authority data.

`sh scripts/verify-go-three-voter.sh` creates an isolated local Docker MinIO container and three local Grant processes. It verifies a replicated write, strong reads on every voter, writes with one voter down, and recovery into the same node directory. It then restores a populated logical snapshot into a separate fresh three-voter prefix, checks that readiness waits for all admissions, and verifies restored quorum recovery. It never invokes kubectl. `KEEP_GRANT_FIXTURE=1` retains its own temporary logs for diagnosis; it never cleans unrelated containers or files.

`sh scripts/verify-go-cli-contract.sh` builds all three executables with cgo disabled and checks actual process output, including initialization. It requires the local `sqlite3` CLI and `jq`; its frozen SQLite fixture verifies the exporter independently of the Go hash helpers.

The disposable Kubernetes overlay remains a three-voter topology. Use a dedicated kubeconfig and explicit local context when running `scripts/ci-local.sh`. Never point it at an existing production context. Its object-store fixture credentials are local test values, not operational credentials. The MinIO server/client image versions are fixed to published [upstream releases](https://github.com/minio/minio/releases).

The two artifact tools preserve the existing binary names:

```sh
go run ./cmd/grant-patch-data-manifest-digest < manifest.json
go run ./cmd/grant-patch-data-authority-snapshot \
  --database frozen-pocketbase.sqlite --source-revision REVISION \
  --source-ledger-digest sha256:LOWERCASE_HEX_DIGEST --generation 1 \
  --produced-at 2026-09-08T00:00:00Z --output-dir new-snapshot-bundle
```

The exporter rejects SQLite WAL/SHM sidecars and existing output directories. Publication uses an atomic no-replace rename. Tests use temporary SQLite files and do not access live PocketBase data.

Historical Rust commands are preserved in [the previous development guide](history/hiqlite/local-development.md).

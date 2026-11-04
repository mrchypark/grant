# Disposable independent verification

Run from a Grant checkout:

```sh
sh scripts/verify-independent-environment.sh
```

The default `all` phase runs ordinary Go tests, then the real process/object-archive
DR harness for S3, GCS and Azure Blob. It creates no persistent environment.
No Kubernetes context, consumer namespace, consumer credentials or live IdP is
required. It does not call kubectl or apply a deployment.

## Phases and prerequisites

| Command | Scope | Requirements |
| --- | --- | --- |
| `sh scripts/verify-independent-environment.sh all` | Unit phase, then all three DR providers; default | Requirements from both rows below |
| `sh scripts/verify-independent-environment.sh unit` | `CGO_ENABLED=0 GOMAXPROCS=2 go test -v -mod=readonly -p=1 -timeout=10m ./...` | POSIX sh, Git, Go version from go.mod, ps/lsof, normal system tools, network for uncached public Go modules/toolchain |
| `sh scripts/verify-independent-environment.sh dr` | `sh scripts/verify-independent-object-dr.sh` | Same Go tools, Docker daemon on a local Unix socket, at least 4 GiB free, public Docker Hub access for uncached pinned images |

The wrapper supports Linux/macOS AMD64 and ARM64 hosts. Other hosts return exit 77,
`SKIP/UNVERIFIED`, not PASS. A missing tool, unavailable provider, failed assertion
or cleanup failure is a nonzero result. All three provider receipts are mandatory
for DR success; there is no cloud fallback. Unit success alone does not qualify DR:
the opt-in integration tests report SKIP outside the harness.

Both phases use a restricted child environment without assigning HOME or
CODEX_HOME. Go settings are explicit with `GOENV=off`; Docker uses a temporary,
empty `DOCKER_CONFIG`. The wrapper accepts an explicitly local Unix `DOCKER_HOST`
or reads the caller's Docker context once to locate the local socket, then passes
only that socket to the DR runner. An ambient non-Unix `DOCKER_HOST` is rejected.
Known Dory contexts/sockets are rejected before pulling or creating a container.
A Unix socket alone does not establish native-daemon isolation. The supported
qualification target is the owned Ubuntu GitHub runner's native Moby daemon;
current local Dory remains UNVERIFIED and must not be retried or bypassed.
Provider endpoints are isolated loopback fixtures with fake credentials. Cloud
environment variables and consumer kubeconfig are not passed to runtime children.
Public module/image downloads are dependencies, not access to consumer data.
Shared Go caches and downloaded images may remain; no cache pruning is performed.

## Fixtures, versions and evidence

Go tests use their own HTTP IdP mocks (including Goauthy) and embedded Rhiza
fixtures. The DR fixture's IdP stub rejects external authentication; its authority
checks use owned internal users. Neither controls a consumer IdP. The DR runner
builds the actual Grant executable and uses actual provider adapters/SDKs with:

| Provider | Official emulator release | Immutable multi-platform image index |
| --- | --- | --- |
| S3 | Floci 2.3.0 | `floci/floci@sha256:e0b292df4dc9559b7b1bf4f51bdda4bb8e269276236438cd03cae661260ea8a9` |
| GCS | Floci-GCP 0.11.0 | `floci/floci-gcp@sha256:3defb6e4b037c9ea0555e719c38ea5840159c15145a1872dc651b0c6bc9b2bbb` |
| Azure Blob | Floci-AZ 0.15.0 | `floci/floci-az@sha256:6a48e35207f041f05aeede4ecba6291e0b26a0f18675a82d6465ddb8fbef00ce` |

Docker selects the daemon's platform; image inspection and executable build
metadata belong to each run's evidence. A local ARM64 pass is not native AMD64
image qualification. The existing CI container checks supply separate coverage.

This work starts at published Grant v0.4.1, Git
`2ec29f6e4bcedace1564bb42e0473bec4f1b5bfd`, Rhiza v0.21.2. Each invocation records
HEAD, dirty status, a hash of the tracked Git diff, hashes of the explicit approved
harness files, Go/Rhiza versions and host. Deleted tracked files are legitimate
diff inputs; arbitrary untracked files and symlink targets are not traversed for
hashes. Source diffs are not copied into uploaded evidence, because a dirty diff
could contain credentials. Go metadata and execution use the same sanitized
GOENV/GOWORK settings and the toolchain selected by go.mod.
Dirty source is not an exact released artifact; the DR runner also preserves
harness source and built-executable metadata. Earlier one-off Rhiza v0.22.1 DR
results are historical evidence, not a pass for this source. Independently tested
owner-eligibility and learner-authz branches are not merged into this environment.

Evidence paths are printed under `${TMPDIR:-/tmp}`:
`grant-independent-environment.*` holds wrapper receipts and phase logs;
`grant-independent-evidence.*` holds DR build/runtime/provider/cleanup evidence.
Keep both directories for review. The fixtures are alive only while their phase
runs: serial provider containers, Grant processes, temporary DataDirs and archives
are cleaned on completion or failure. Cleanup targets only owned resources; it
does not stop other workloads, prune shared storage, or restart Docker. Inspect
cleanup receipts before asserting that an interrupted run left no resources.
Unit execution records named SKIP results in `unit.log` and `unit-skips.txt`.
Its process observer records owned descendant PIDs, start identities and executable
paths. Interrupted cleanup rechecks those identities before bounded TERM/KILL,
never signals a reused PID, and verifies the runtime tree has no open files before
removing it. Ambiguous cleanup preserves the tree and returns failure. The
separate CI race command retains its existing CGO-enabled behavior.

Allow minutes on a warm machine and longer for cold builds/image downloads. The
unit command has a 10-minute per-package timeout; DR has bounded child/test
deadlines in addition to build time. The required two recovery cycles for each
provider include at least 100 seconds of restored-main observation per cycle;
these assertions must pass remotely before any successful qualification claim.
A disk watchdog
protects the DR runner's 4 GiB reserve. No broad local race run is added: the
existing native CI job retains its full race suite.

## CI and limits

The existing [e2e-local workflow](../.github/workflows/e2e-local.yml) retains native
AMD64 build/race/security checks and the disposable runner-local vind bootstrap.
It adds one mandatory DR phase before build/vet/race, with a 30-minute step timeout,
and uploads wrapper/DR evidence immediately afterward even on failure. The overall
verification job allows 60 minutes, retaining the existing tests and their semantic
deadlines. The verification job has read-only repository
permissions. No new workflow, publication, consumer secret or persistent cluster
is added. The existing pinned Rauthy compatibility image remains a test dependency;
it does not control another project's live IdP or data.

DR coverage is clean-close init/main, HTTP writes, restart, complete owned DataDir
loss, archive restore, and authority continuity. This is emulator evidence, not
production-cloud parity, abrupt crash recovery, competing-writer fencing, managed
Operator v1alpha2, Kubernetes NoPVC recovery, or full live-IdP authentication.
The IED mixed integration environment is excluded. A source change requires new
qualification; historical PR 8 CI and feature-branch tests do not qualify a new
integrated artifact. External issuer login/client readiness remains separate.

The initial local Dory run failed closed before emulator startup because Docker
rewrote a requested loopback binding to a wildcard binding. No current all-three
runtime pass is claimed for this harness. Native Linux CI must establish it; do
not weaken the binding guard or substitute the historical v0.22.1 results.

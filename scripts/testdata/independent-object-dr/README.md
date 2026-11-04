# Independent Grant object DR fixture

Run `scripts/verify-independent-object-dr.sh` from any working directory. Requires
POSIX sh, Git, Go matching go.mod, and a running local Docker Unix socket. Go module
and pinned-image downloads are allowed. No cluster, cloud account, external IdP,
other project runtime, or permanent service is required. The current checkout is
tested: this harness does not update Rhiza or qualify a published container image.

The runner builds the real cmd/grant and a root-module Go test executable with
CGO disabled, two Go workers and serial package builds. Normal `go test` skips
integration with an explicit UNVERIFIED message; the runner requires completion
receipts from S3, GCS and Azure and fails on any missing result or cleanup error.

Each provider gets a fresh 0700 directory, random identity/bucket/prefix, fake
credentials, one labeled container and loopback-only ephemeral ports. Inherited
cloud credentials, proxies, kubeconfig and user configuration are not passed to
Grant, Docker or provider fixtures. The 4 GiB reserve watchdog samples once per
second during builds, pulls and tests. Provider setup, CLI calls, readiness,
shutdown, helper checks and the entire integration executable have deadlines.

The real process sequence is init CLI, main readiness, clean stop, authority seed
through existing Grant service fixtures, main HTTP write, clean restart, complete
DataDir removal, init CLI restore, main HTTP comparison, then authority verification
without reseeding. Assertions cover custom role/action and active user binding,
transferred owner, exact ownership ETag, positive and negative internal-user
authorization, and former-owner mutation returning 403. Authority snapshots also
run before loss. Startup's ordinary Bootstrap cannot recreate these custom
post-transfer assertions. Main and the authority helper never open a DataDir
concurrently. This is clean sequential DR, not concurrent-writer fencing.

Official release pins (verified against the Docker Hub version-tag metadata):

| Provider | Release | Immutable multi-platform index |
| --- | --- | --- |
| S3 | floci/floci 2.3.0 | sha256:e0b292df4dc9559b7b1bf4f51bdda4bb8e269276236438cd03cae661260ea8a9 |
| GCS | floci/floci-gcp 0.11.0 | sha256:3defb6e4b037c9ea0555e719c38ea5840159c15145a1872dc651b0c6bc9b2bbb |
| Azure | floci/floci-az 0.15.0 | sha256:6a48e35207f041f05aeede4ecba6291e0b26a0f18675a82d6465ddb8fbef00ce |

Provenance endpoints are `https://hub.docker.com/v2/repositories/floci/NAME/tags/VERSION`
with NAME and VERSION from the table. Sources are the corresponding
`https://github.com/floci-io/NAME` repositories. Never replace pins with latest.

Evidence path is printed immediately. It retains the source SHA, dirty-diff hash and
status, copies of harness sources, Go binary build metadata including exact SDK
modules, image/container inspection, phase logs, ownership ETag, HTTP rows and
per-provider PASS/cleanup receipts. Runtime executables, fake credentials, data,
containers and anonymous image volumes are removed. Shared images/caches and
pre-existing processes are not pruned or stopped. Logs are the requested output;
remove their printed directory yourself after review.

A Docker-compatible service that rewrites 127.0.0.1 bindings to wildcard bindings
is unsupported: the runner rejects it before container start, reports UNVERIFIED,
and cleans up. It does not weaken isolation or fall back to live services.

Passing these emulator cases establishes neither cloud parity, external fencing,
partition/crash behavior, HA/Operator/Kubernetes deployment nor image rollout
qualification. The independent functional environment is disposable, not always-on.

The runner never assigns HOME. Go uses GOENV=off and explicit build paths; Docker
uses a private DOCKER_CONFIG. Ambient non-Unix DOCKER_HOST values are rejected
before any Docker command. Raw dirty diffs are hashed in disposable private work
and deleted immediately; evidence contains only the hash, status and approved
harness copies, not a complete source diff.

Run `sh scripts/testdata/independent-object-dr/guards-test.sh` for small disk,
endpoint and native process-cleanup regressions without Docker. Main process
receipts record PID, exact executable path, start identity and command. Failure
fallback validates current identity and executable before TERM/KILL, rejects
stale/foreign identities, and makes required fallback a failing verification run.
Unknown process state preserves work and reports failure rather than deleting a
potentially live executable. Container absence requires a successful daemon list.

Container creation first proves the fresh exact name and unique owner label are
both absent, then records create admission before sending POST. If create commits
but the CLI response/CID is lost, cleanup reconciles fresh exact name and label
lookups. At most one matching CID is accepted, and its inspected name/label must
match before recording or deleting it. The shell fallback also works from the
owner/admission receipts when no CID receipt exists. Pre-existing collisions,
foreign identities, ambiguity and daemon errors fail without deleting a target;
an unknown create is never converted into a successful provider test.

A Unix socket alone is not proof of an independent native daemon. Metadata
preflight rejects known Dory/bridge/proxy contexts and socket identifiers before
contacting the daemon. Only recognized default or Docker Desktop endpoint shapes
proceed to read-only native platform/OS identity checks. Unknown contexts, sockets
or identities fail UNVERIFIED and require owner review; there is no bypass flag.
The real runner's Dory rejection is tested with a metadata-only mock, and direct
opt-in Go tests also reject proxy/unknown socket and server identities. Runtime
proof must come from an owned normal native daemon (such as ephemeral GitHub CI),
not a shared-cluster Docker bridge. Do not retry local Dory runtime operations.

The additional `gcs-scale` scenario uses its own fresh GCS emulator, independent
of the unchanged S3/GCS/Azure basic cases. It creates six random-prefix providers
and six active enforcers through Grant HTTP POST provider / PUT enforcer APIs,
then requires two sequential full-DataDir-loss init/main restore cycles. SQL and
Grant PEP reads check all six active providers/enforcers before loss and after each
cycle, alongside the unchanged custom role/binding/owner-ETag/allow-deny oracle.
Cycle-numbered init exit, main startup/readiness/clean-close, HTTP and authority
helper Open/Close logs prevent the second handoff from overwriting the first.
The runner requires all four scenario receipts. No maintenance token is cleared,
no timeout is relaxed, and no consumer database, configuration or secret is used.
This is a synthetic regression motivated by reported consumer behavior; it is not
an owned reproduction or a defect classification until native CI executes it.

Evidence correction for the motivating consumer report: the oldwriter Pod was
gone, but an independently preserved graceful process exit is NOT available.
Reported UTC timeline: init exited 0 at 14:26:33; main began at 14:26:34;
context cancellation appeared at 14:27:35 (+61s from main start); unresolved
ownership appeared at 14:27:58 (+84s), before the 150s probe budget. First-main
termination metadata/events are absent. These facts do not establish what caused
cancellation or prove clean oldwriter shutdown. Do not substitute probe-budget
changes for an owned reproduction.

Future phase-2 execution must independently capture its own oldwriter graceful
shutdown request and observed exit, restore/init and main stdout and stderr,
process exit codes and signals, UTC start/termination/cancellation timestamps,
and failure evidence before fixture cleanup. Current combined-output/exit logs
are not claimed to satisfy that complete future capture requirement. This
requirement is pending phase-2 implementation/review and native-CI execution;
there is still no owned reproduction of the consumer failure.

Phase-2 capture implementation: init/restore and authority helper commands retain
separate stdout/stderr plus process start, exit code, signal and UTC metadata.
Their PID/executable/start-identity receipts are persisted before Wait, just like
main receipts; the shell fallback reconciles both owned grant and verify.test
executables. A partial/unverifiable receipt preserves private runtime work and
fails verification. HTTP receipts retain synthetic ACK status and response data,
and helper logs retain timestamped PEP readback and authorization decisions.

Each recovery has a separate >=100-second diagnostic observation window after
readiness, with repeated readiness and provider readback checks. Readiness still
fails at 45 seconds. On readiness failure, diagnostic observation continues for a
bounded 100 seconds without converting late readiness into acceptance. Within
observation, failed readiness/readback marks failure but does not terminate main;
natural exit or the observation deadline ends the diagnostic window. Explicit
external cancellation/disk-safety interruption remains an abort condition and is
recorded as incomplete diagnostics. Natural exit and harness SIGTERM/SIGKILL are
distinguished. Graceful shutdown is proven only by successful SIGTERM delivery and
observed exit code zero without forced kill, not by Pod absence or readiness.

These phase-2 code changes have only been statically edited/formatted under the
local disk hold. Compilation, regression execution, reviewer recheck and native
CI runtime proof remain required. Prior logs do not validate the new phase-2 code.

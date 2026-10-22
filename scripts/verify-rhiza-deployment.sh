#!/bin/sh
set -eu
root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
command -v kubectl >/dev/null
for overlay in ephemeral production ied; do
  rendered=$(kubectl kustomize "$root/deploy/overlays/$overlay")
  printf '%s\n' "$rendered" | grep -q 'protocol: UDP'
  printf '%s\n' "$rendered" | grep -q 'name: grant-rhiza-storage'
  printf '%s\n' "$rendered" | grep -q 'replicas: 3'
  public_service=$(printf '%s\n' "$rendered" | awk '
    function emit() { if (service && grant) printf "%s", document }
    /^---$/ { emit(); document=""; service=0; grant=0; next }
    { document=document $0 "\n" }
    /^kind: Service$/ { service=1 }
    /^  name: grant$/ { grant=1 }
    END { emit() }')
  [ -n "$public_service" ] || { echo "missing public Grant service: $overlay" >&2; exit 1; }
  printf '%s\n' "$public_service" | grep -q 'targetPort: 8080'
  if printf '%s\n' "$public_service" | grep -Eq '8100|8200|hiq-api|name: raft'; then
    echo "public Grant service exposes a private or obsolete runtime port: $overlay" >&2
    exit 1
  fi
done
operator=$(kubectl kustomize "$root/deploy/overlays/rhiza-operator")
printf '%s\n' "$operator" | grep -q 'name: grant-rhiza-config'
printf '%s\n' "$operator" | grep -q 'RHIZA_BIND_ADDR: 0.0.0.0:8101'
printf '%s\n' "$operator" | grep -q 'RHIZA_CLUSTER_ID: grant-rhiza-v019'
printf '%s\n' "$operator" | grep -q 'RHIZA_ENABLE_RECONFIGURATION: "true"'
printf '%s\n' "$operator" | grep -q 'name: grant-rhiza-operator-credentials'
printf '%s\n' "$operator" | grep -q 'key: peer_tokens'
printf '%s\n' "$operator" | grep -q 'name: peer-quic'
printf '%s\n' "$operator" | grep -q 'type: OnDelete'
printf '%s\n' "$operator" | grep -q 'name: grant-rhiza-observe'
printf '%s\n' "$operator" | grep -q 'automatic: false'
printf '%s\n' "$operator" | grep -q 'name: rhizarecoveries.rhiza.mrchypark.dev'
printf '%s\n' "$operator" | grep -q 'name: rhizaclusters.rhiza.mrchypark.dev'
printf '%s\n' "$operator" | grep -q 'name: rhizafences.rhiza.mrchypark.dev'
printf '%s\n' "$operator" | grep -q 'image: ghcr.io/mrchypark/rhiza-operator:v0.19.0@sha256:'
printf '%s\n' "$operator" | grep -q 'port: 8101'
printf '%s\n' "$operator" | grep -q 'rhiza.mrchypark.dev/automatic-learner'
if printf '%s\n' "$operator" | grep -Eq 'GRANT_(CLUSTER_ID|PEER_ADDR|DATA_DIR):|RHIZA_AUTOMATIC_RECOVERY|RHIZA_FENCER_BACKEND|image: .*:latest'; then
  echo 'Rhiza Operator overlay retains a forbidden legacy alias or automatic/latest setting' >&2
  exit 1
fi
if grep -q 'restore-once\|HQL_' "$root/deploy/k8s/app-statefulset.yaml"; then
  echo 'native Grant must not run the legacy Hiqlite restore wrapper' >&2
  exit 1
fi
printf '%s\n' 'Native Rhiza deployment manifests verified'

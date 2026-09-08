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
if grep -q 'restore-once\|HQL_' "$root/deploy/k8s/app-statefulset.yaml"; then
  echo 'native Grant must not run the legacy Hiqlite restore wrapper' >&2
  exit 1
fi
printf '%s\n' 'Native Rhiza deployment manifests verified'

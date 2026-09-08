#!/bin/sh
set -eu
root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
command -v kubectl >/dev/null
for overlay in ephemeral production ied; do
  rendered=$(kubectl kustomize "$root/deploy/overlays/$overlay")
  printf '%s\n' "$rendered" | grep -q 'protocol: UDP'
  printf '%s\n' "$rendered" | grep -q 'name: grant-rhiza-storage'
  printf '%s\n' "$rendered" | grep -q 'replicas: 3'
done
if grep -q 'restore-once\|HQL_' "$root/deploy/k8s/app-statefulset.yaml"; then
  echo 'native Grant must not run the legacy Hiqlite restore wrapper' >&2
  exit 1
fi
printf '%s\n' 'Native Rhiza deployment manifests verified'

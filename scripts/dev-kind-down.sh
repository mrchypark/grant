#!/usr/bin/env bash
set -euo pipefail

CLUSTER_NAME="${CLUSTER_NAME:-certd-dev}"
kind delete cluster --name "${CLUSTER_NAME}"

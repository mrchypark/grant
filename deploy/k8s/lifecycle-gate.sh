#!/bin/sh
set -eu

MODE_FILE=${LIFECYCLE_MODE_FILE:-/lifecycle/mode}
RECOVERY_ID_FILE=${LIFECYCLE_RECOVERY_ID_FILE:-/lifecycle/recovery-id}
RESTORE_ID_FILE=${LIFECYCLE_RESTORE_ID_FILE:-/restore/recovery-id}
ALLOW_LOCAL_FRESH=${LIFECYCLE_ALLOW_LOCAL_FRESH:-false}
HEADLESS_SERVICE=${LIFECYCLE_HEADLESS_SERVICE:?LIFECYCLE_HEADLESS_SERVICE is required}
REPLICAS=${LIFECYCLE_REPLICAS:-3}
PEER_PORT=${LIFECYCLE_PEER_PORT:?LIFECYCLE_PEER_PORT is required}
PEER_PATH=${LIFECYCLE_PEER_PATH:?LIFECYCLE_PEER_PATH is required}
RETRY_SECONDS=${LIFECYCLE_RETRY_SECONDS:-3}

POD_HOSTNAME=$(hostname)
ordinal=${POD_HOSTNAME##*-}
case "${ordinal}" in
  ''|*[!0-9]*)
    echo "lifecycle gate cannot derive StatefulSet ordinal from ${POD_HOSTNAME}" >&2
    exit 1
    ;;
esac

read_mode() {
  [ -r "${MODE_FILE}" ] || { echo "lifecycle mode file is missing" >&2; return 1; }
  mode=$(sed -n '1{s/[[:space:]]//g;p;}' "${MODE_FILE}")
  case "${mode}" in steady|fresh|restore) ;; *) echo "invalid lifecycle mode: ${mode:-empty}" >&2; return 1 ;; esac
  printf '%s\n' "${mode}"
}

restore_authorized() {
  [ -r "${RECOVERY_ID_FILE}" ] && [ -r "${RESTORE_ID_FILE}" ] || return 1
  lifecycle_id=$(sed -n '1{s/[[:space:]]//g;p;}' "${RECOVERY_ID_FILE}")
  restore_id=$(sed -n '1{s/[[:space:]]//g;p;}' "${RESTORE_ID_FILE}")
  [ -n "${lifecycle_id}" ] && [ "${lifecycle_id}" = "${restore_id}" ]
}

peer_ready() {
  peer=0
  while [ "${peer}" -lt "${REPLICAS}" ]; do
    if [ "${peer}" -ne "${ordinal}" ]; then
      url="http://${POD_HOSTNAME%-*}-${peer}.${HEADLESS_SERVICE}:${PEER_PORT}${PEER_PATH}"
      if wget -q -T 2 -O /dev/null "${url}" 2>/dev/null; then
        echo "lifecycle gate found ready direct peer ${url}"
        return 0
      fi
    fi
    peer=$((peer + 1))
  done
  return 1
}

while :; do
  mode=$(read_mode)
  case "${mode}:${ordinal}" in
    fresh:0)
      [ "${ALLOW_LOCAL_FRESH}" = true ] || {
        echo "production lifecycle gate rejects fresh bootstrap; use an approved restore" >&2
        exit 1
      }
      echo "lifecycle gate allowing ordinal 0 in explicitly local fresh mode"
      exit 0
      ;;
    restore:0)
      if restore_authorized; then
        echo "lifecycle gate allowing ordinal 0 for the matching restore authorization"
        exit 0
      fi
      ;;
  esac
  if peer_ready; then
    exit 0
  fi
  echo "lifecycle gate blocked: mode=${mode}, ordinal=${ordinal}, no direct ready peer" >&2
  sleep "${RETRY_SECONDS}"
done

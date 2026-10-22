#!/bin/sh
set -eu

if [ "$#" -lt 1 ]; then
  printf '%s\n' 'usage: cached-go-module.sh TOOL_PATH [args...]' >&2
  exit 2
fi

TOOL_PATH=$1
shift
TOOL_NAME=$(basename "${TOOL_PATH}")
SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname "${TOOL_PATH}")" && pwd)
SRC_DIR=${SCRIPT_DIR}/${TOOL_NAME}-src
CACHE_ROOT=${CODEX_GO_SCRIPT_CACHE:-${XDG_CACHE_HOME:-${HOME}/.cache}/codex-go-scripts}
BUILD_PKG=${CODEX_GO_BUILD_PKG:-.}
PLATFORM=$(go env GOOS GOARCH | tr '\n' '-' | sed 's/-$//')

[ -f "${SRC_DIR}/go.mod" ] || { printf 'missing Go module: %s/go.mod\n' "${SRC_DIR}" >&2; exit 1; }
KEY=$(
  cd "${SRC_DIR}" && {
    go env GOVERSION GOOS GOARCH GOWORK GOFLAGS CGO_ENABLED GOEXPERIMENT CC CXX
    printf 'BUILD_PKG=%s\n' "${BUILD_PKG}"
    if command -v shasum >/dev/null 2>&1; then
      find . -type d \( \( -name '.*' ! -name . \) -o -name vendor \) -prune -o -type f -exec shasum -a 256 {} + | sort
    else
      find . -type d \( \( -name '.*' ! -name . \) -o -name vendor \) -prune -o -type f -exec sha256sum {} + | sort
    fi
  } | if command -v shasum >/dev/null 2>&1; then
    shasum -a 256
  else
    sha256sum
  fi | awk '{print $1}'
)
[ -n "${KEY}" ] || { printf '%s\n' 'failed to compute cache key' >&2; exit 1; }
BIN_DIR=${CACHE_ROOT}/${TOOL_NAME}/${PLATFORM}/${KEY}
BIN=${BIN_DIR}/${TOOL_NAME}

if [ "${CACHED_GO_REBUILD:-0}" = 1 ] || [ ! -x "${BIN}" ]; then
  mkdir -p "${BIN_DIR}"
  [ "${CACHED_GO_DEBUG:-0}" != 1 ] || printf 'building %s\n' "${BIN}" >&2
  TMP=${BIN}.tmp.$$
  rm -f "${TMP}"
  trap 'rm -f "${TMP}"' 0 1 2 3 15
  (cd "${SRC_DIR}" && go build -trimpath -o "${TMP}" "${BUILD_PKG}")
  mv "${TMP}" "${BIN}"
  trap - 0 1 2 3 15
fi

exec "${BIN}" "$@"

#!/bin/sh
set -eu

ROOT_DIR=$(CDPATH='' cd -P "$(dirname "$0")/.." && pwd)
TMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/grant-docs.XXXXXX")
cleanup() { rm -rf "${TMP_DIR}"; }
trap cleanup 0 HUP INT TERM

FAILURES="${TMP_DIR}/failures"
: >"${FAILURES}"

find "${ROOT_DIR}" -type f -name '*.md' \
  ! -path '*/target/*' ! -path '*/.git/*' | while IFS= read -r file; do
  grep -oE '\]\([^)]*\)' "${file}" 2>/dev/null \
    | sed 's/^](//;s/)$//' \
    | while IFS= read -r target; do
      target=$(printf '%s' "${target}" | sed 's/^<//;s/>$//;s/[[:space:]]\+"[^"]*"$//')
      case "${target}" in
        ''|'#'*|http://*|https://*|mailto:*|data:*|app://*) continue ;;
      esac
      target=${target%%#*}
      target=${target%%\?*}
      case "${target}" in
        /*) resolved="${ROOT_DIR}${target}" ;;
        *) resolved="$(dirname "${file}")/${target}" ;;
      esac
      [ -e "${resolved}" ] || printf '%s: missing link target %s\n' "${file#"${ROOT_DIR}"/}" "${target}" >>"${FAILURES}"
    done
done

SCHEMA_SOURCE="${ROOT_DIR}/internal/grant/store_schema.sql"
SCHEMA_DOC="${ROOT_DIR}/docs/database-schema.md"
[ -r "${SCHEMA_SOURCE}" ] || printf 'missing canonical schema source: %s\n' "${SCHEMA_SOURCE}" >>"${FAILURES}"
[ -r "${SCHEMA_DOC}" ] || printf 'missing schema documentation: %s\n' "${SCHEMA_DOC}" >>"${FAILURES}"

if [ -r "${SCHEMA_SOURCE}" ] && [ -r "${SCHEMA_DOC}" ]; then
  grep -q '(../internal/grant/store_schema.sql)' "${SCHEMA_DOC}" \
    || printf 'docs/database-schema.md must link to ../internal/grant/store_schema.sql\n' >>"${FAILURES}"

  sed -n 's/.*CREATE TABLE IF NOT EXISTS[[:space:]]\{1,\}\([A-Za-z0-9_]*\).*/\1/p' "${SCHEMA_SOURCE}" \
    | sort -u >"${TMP_DIR}/source-tables"
  # shellcheck disable=SC2016 # Markdown backticks are literal delimiters.
  grep -oE '`[A-Za-z0-9_]+`' "${SCHEMA_DOC}" \
    | tr -d '`' \
    | sort -u >"${TMP_DIR}/documented-tables"

  comm -23 "${TMP_DIR}/source-tables" "${TMP_DIR}/documented-tables" \
    | while IFS= read -r table; do
      [ -z "${table}" ] || printf 'docs/database-schema.md is missing table inventory entry: %s\n' "${table}" >>"${FAILURES}"
    done
fi

if [ -s "${FAILURES}" ]; then
  cat "${FAILURES}" >&2
  exit 1
fi

echo "Markdown links and database schema inventory verified"

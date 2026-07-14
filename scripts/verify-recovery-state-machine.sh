#!/bin/sh
set -eu

ROOT_DIR=$(CDPATH='' cd -P "$(dirname "$0")/.." && pwd)
# shellcheck source=recovery-state-machine.sh
# shellcheck disable=SC1091
. "${ROOT_DIR}/scripts/recovery-state-machine.sh"

phases='materialized
rauthy-restore-armed
rauthy-ordinal0-restored
rauthy-voters-caught-up
rauthy-cleanup-complete
grant-restore-armed
grant-ordinal0-restored
grant-voters-caught-up
cleanup-complete
semantic-verified
unfenced'

previous=
printf '%s\n' "${phases}" | while IFS= read -r phase; do
  recovery_phase_known "${phase}"
  if [ -n "${previous}" ]; then
    recovery_assert_transition "${previous}" "${phase}"
  fi
  previous=${phase}
done

for invalid in '' typo RESTORE 'grant-ordinal0-restored '; do
  if recovery_phase_known "${invalid}"; then
    echo "invalid phase was accepted: '${invalid}'" >&2
    exit 1
  fi
done

for invalid_transition in \
  'materialized materialized' \
  'rauthy-ordinal0-restored materialized' \
  'grant-restore-armed grant-voters-caught-up' \
  'unfenced materialized'; do
  # Intentional split of the two-word transition test vector.
  # shellcheck disable=SC2086
  set -- ${invalid_transition}
  if recovery_assert_transition "$1" "$2" >/dev/null 2>&1; then
    echo "invalid transition was accepted: ${invalid_transition}" >&2
    exit 1
  fi
done

echo "recovery transition table verified"

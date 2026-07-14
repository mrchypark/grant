#!/bin/sh

# This file is sourced by restore-full-cluster.sh and its transition tests.

recovery_phase_known() {
  case "$1" in
    materialized|rauthy-restore-armed|rauthy-ordinal0-restored|rauthy-voters-caught-up|rauthy-cleanup-complete|grant-restore-armed|grant-ordinal0-restored|grant-voters-caught-up|cleanup-complete|semantic-verified|unfenced) return 0 ;;
    *) return 1 ;;
  esac
}

recovery_phase_next() {
  case "$1" in
    materialized) echo rauthy-restore-armed ;;
    rauthy-restore-armed) echo rauthy-ordinal0-restored ;;
    rauthy-ordinal0-restored) echo rauthy-voters-caught-up ;;
    rauthy-voters-caught-up) echo rauthy-cleanup-complete ;;
    rauthy-cleanup-complete) echo grant-restore-armed ;;
    grant-restore-armed) echo grant-ordinal0-restored ;;
    grant-ordinal0-restored) echo grant-voters-caught-up ;;
    grant-voters-caught-up) echo cleanup-complete ;;
    cleanup-complete) echo semantic-verified ;;
    semantic-verified) echo unfenced ;;
    unfenced) echo '' ;;
    *) return 1 ;;
  esac
}

recovery_phase_rank() {
  case "$1" in
    materialized) echo 10 ;; rauthy-restore-armed) echo 20 ;; rauthy-ordinal0-restored) echo 30 ;;
    rauthy-voters-caught-up) echo 40 ;; rauthy-cleanup-complete) echo 50 ;; grant-restore-armed) echo 60 ;;
    grant-ordinal0-restored) echo 70 ;; grant-voters-caught-up) echo 80 ;; cleanup-complete) echo 90 ;;
    semantic-verified) echo 100 ;; unfenced) echo 110 ;; *) return 1 ;;
  esac
}

recovery_assert_transition() {
  current=$1
  requested=$2
  recovery_phase_known "${current}" || {
    echo "unknown recovery phase: ${current}" >&2
    return 1
  }
  recovery_phase_known "${requested}" || {
    echo "unknown recovery phase: ${requested}" >&2
    return 1
  }
  expected=$(recovery_phase_next "${current}")
  [ "${requested}" = "${expected}" ] || {
    echo "invalid recovery transition: ${current} -> ${requested}; expected ${expected:-terminal}" >&2
    return 1
  }
}

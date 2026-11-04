# Sourced only by the independent runner and its isolated regression fixtures.
independent_unix_endpoint() { case "$1" in unix:///*) return 0;; *) return 1;; esac; }
independent_df_valid() {
 awk 'NR>1 {if ($4 !~ /^[0-9]+$/) {bad=1; next}; seen++; if ($4 < 4194304) bad=1} END {exit (bad || seen<1)}'
}
independent_space() {
 disk=$(df -Pk "$@") || return 1
 printf '%s\n' "$disk" | independent_df_valid
}
independent_process_identity() {
 LC_ALL=C ps -p "$1" -o lstart= || return 1
 LC_ALL=C ps -p "$1" -o args= || return 1
}
# Return 0: original process absent, 1: fallback used, 2: state unverifiable.
independent_reap_receipt() {
 receipt=$1; owned_root=$2
 recorded_pid=$(sed -n '1p' "$receipt"); recorded_bin=$(sed -n '2p' "$receipt")
 recorded_start=$(sed -n '3p' "$receipt"); recorded_args=$(sed -n '4p' "$receipt")
 case "$recorded_pid" in ''|*[!0-9]*) return 2;; esac
 case "$recorded_bin" in "$owned_root/grant"|"$owned_root/verify.test") ;; *) return 2;; esac
 [ -n "$recorded_start" ] || return 2
 if ! kill -0 "$recorded_pid" 2>/dev/null; then return 0; fi
 case "$recorded_args" in "$recorded_bin"|"$recorded_bin "*) ;; *) return 2;; esac
 used=0; rounds=0
 while :; do
  if ! kill -0 "$recorded_pid" 2>/dev/null; then return "$used"; fi
  current=$(independent_process_identity "$recorded_pid") || return 2
  current_start=$(printf '%s\n' "$current" | sed -n '1p' | sed 's/^[[:space:]]*//')
  current_args=$(printf '%s\n' "$current" | sed -n '2p' | sed 's/^[[:space:]]*//')
  # A reused PID belongs to someone else: original process is gone, do not signal.
  [ "$current_start" = "$recorded_start" ] || return "$used"
  [ "$current_args" = "$recorded_args" ] || return 2
  [ -f "$recorded_bin" ] && [ ! -L "$recorded_bin" ] || return 2
  if [ -d /proc ]; then
   actual_binary=$(readlink "/proc/$recorded_pid/exe") || return 2
  else
   actual_binary=$(LC_ALL=C ps -p "$recorded_pid" -o comm= | sed 's/^[[:space:]]*//') || return 2
  fi
  [ "$actual_binary" = "$recorded_bin" ] || return 2
  used=1
  if [ "$rounds" -eq 0 ]; then kill -TERM "$recorded_pid" 2>/dev/null || return 2; fi
  if [ "$rounds" -eq 10 ]; then kill -KILL "$recorded_pid" 2>/dev/null || return 2; fi
  [ "$rounds" -lt 15 ] || return 2
  rounds=$((rounds+1)); sleep 1
 done
}
# A successful exact name+label list is required; never mutate on ambiguity.
independent_discover_container() {
 owner=$1
 labels=$(docker_local ps -aq --no-trunc --filter "label=grant.independent.owner=$owner") || return 2
 names=$(docker_local ps -aq --no-trunc --filter "name=^/$owner$") || return 2
 if [ -z "$labels" ] && [ -z "$names" ]; then return 0; fi
 [ "$labels" = "$names" ] && [ "${#labels}" -eq 64 ] || return 2
 case "$labels" in *[!0-9a-f]*) return 2;; esac
 printf '%s\n' "$labels"
}
# Metadata-only gate: do not contact known bridge/proxy daemons, even for info.
independent_native_metadata() {
 context_id=$1; socket_id=$2
 identity=$(printf '%s %s\n' "$context_id" "$socket_id" | tr '[:upper:]' '[:lower:]')
 case "$identity" in *dory*|*bridge*|*proxy*|*kube*|*k8s*|*rancher*|*colima*|*podman*) return 1;; esac
 case "$context_id:$socket_id" in
  default:unix:///var/run/docker.sock) return 0;;
  desktop-linux:unix://*/.docker/run/docker.sock) return 0;;
  *) return 1;;
 esac
}
independent_native_server() {
 case "$1" in 'Docker Engine - Community'|'Docker Engine - Enterprise'|'Docker Engine'|'Moby Engine'|'Docker Desktop'|'Docker Desktop '*) ;;
  *) return 1;; esac
 case "$2" in Ubuntu*|Debian*|'Docker Desktop'*|'Alpine Linux'*|'Fedora Linux'*|CentOS*|'Rocky Linux'*|'Amazon Linux'*) return 0;; *) return 1;; esac
}

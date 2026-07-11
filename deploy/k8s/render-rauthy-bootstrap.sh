#!/bin/sh
set -eu

template_dir="${1:-/bootstrap-template}"
output_dir="${2:-/bootstrap}"
client_secret="${GRANT_RAUTHY_CLIENT_SECRET:?missing GRANT_RAUTHY_CLIENT_SECRET}"
admin_key="${GRANT_RAUTHY_ADMIN_API_KEY:?missing GRANT_RAUTHY_ADMIN_API_KEY}"
admin_name="${admin_key%%\$*}"
admin_secret="${admin_key#*\$}"

[ "$admin_name" != "$admin_key" ] || { echo "GRANT_RAUTHY_ADMIN_API_KEY must contain name\$secret" >&2; exit 1; }

case "$client_secret" in
  *[!A-Za-z0-9]*|'') echo "GRANT_RAUTHY_CLIENT_SECRET must be alphanumeric" >&2; exit 1 ;;
esac
case "$admin_name" in
  *[!A-Za-z0-9_/-]*|'') echo "GRANT_RAUTHY_ADMIN_API_KEY has an invalid name" >&2; exit 1 ;;
esac
case "$admin_secret" in
  *[!A-Za-z0-9]*|'') echo "GRANT_RAUTHY_ADMIN_API_KEY must contain name\$alphanumeric-secret" >&2; exit 1 ;;
esac
[ "${#client_secret}" -ge 64 ] || { echo "GRANT_RAUTHY_CLIENT_SECRET must be at least 64 characters" >&2; exit 1; }
[ "${#admin_name}" -ge 2 ] && [ "${#admin_name}" -le 24 ] || { echo "GRANT_RAUTHY_ADMIN_API_KEY name must be 2-24 characters" >&2; exit 1; }
[ "${#admin_secret}" -ge 64 ] || { echo "GRANT_RAUTHY_ADMIN_API_KEY secret must be at least 64 characters" >&2; exit 1; }

umask 077
mkdir -p "$output_dir"
clients_tmp="$output_dir/.clients.json.tmp"
api_keys_tmp="$output_dir/.api_keys.json.tmp"
trap 'rm -f "$clients_tmp" "$api_keys_tmp"' 0 HUP INT TERM
sed "s|__GRANT_RAUTHY_CLIENT_SECRET__|$client_secret|g" \
  "$template_dir/clients.json" >"$clients_tmp"
sed \
  -e "s|__GRANT_RAUTHY_ADMIN_API_KEY_NAME__|$admin_name|g" \
  -e "s|__GRANT_RAUTHY_ADMIN_API_KEY_SECRET__|$admin_secret|g" \
  "$template_dir/api_keys.json" >"$api_keys_tmp"
if [ "$(id -u)" -eq 0 ]; then
  chown 10001:10001 "$clients_tmp" "$api_keys_tmp"
fi
chmod 0400 "$clients_tmp" "$api_keys_tmp"
mv -f "$clients_tmp" "$output_dir/clients.json"
mv -f "$api_keys_tmp" "$output_dir/api_keys.json"
trap - 0 HUP INT TERM

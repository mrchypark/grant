#!/bin/sh
# Executable-level stdout and cgo-free SQLite checks (including package init).
set -eu
root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
for tool in go sqlite3 jq; do command -v "$tool" >/dev/null; done
task_dir=$(mktemp -d "${TMPDIR:-/tmp}/grant-cli-contract.XXXXXX")
trap 'rm -rf "$task_dir"' EXIT HUP INT TERM
(cd "$root" && CGO_ENABLED=0 go build -o "$task_dir/" ./cmd/...)
if "$task_dir/grant" invalid-command >"$task_dir/server.out" 2>"$task_dir/server.err"; then
 echo 'server accepted unknown command' >&2; exit 1
fi
[ ! -s "$task_dir/server.out" ] && [ -s "$task_dir/server.err" ]
printf '%s' '{"manifestVersion":"grant.patch-data-authority.v1","generation":"1","ledgerDigest":"x","users":[],"organizations":[],"memberships":[],"hierarchyEdges":[],"plantBindings":[],"applicationAssignments":[]}' \
 | "$task_dir/grant-patch-data-manifest-digest" >"$task_dir/digest.out" 2>"$task_dir/digest.err"
[ ! -s "$task_dir/digest.err" ]
[ "$(wc -l < "$task_dir/digest.out" | tr -d ' ')" = 1 ]
[ "$(wc -c < "$task_dir/digest.out" | tr -d ' ')" = 65 ]
case "$(cat "$task_dir/digest.out")" in *[!a-f0-9]*) echo 'digest stdout contamination' >&2; exit 1 ;; esac
sqlite3 "$task_dir/source.db" <<'SQL'
CREATE TABLE organizations(id TEXT,owner TEXT);
CREATE TABLE users(id TEXT,deleted TEXT,organizations TEXT);
CREATE TABLE viewers(id TEXT,deleted TEXT,organizations TEXT);
CREATE TABLE temporary_accounts(id TEXT,expired TEXT,organization TEXT);
CREATE TABLE service_principals(id TEXT,enabled INTEGER);
CREATE TABLE org_parent_map(organization TEXT,parent TEXT);
CREATE TABLE plants(id TEXT,organization TEXT,isArchive INTEGER);
CREATE TABLE permissions(plant TEXT,users TEXT,viewers TEXT,temporary TEXT);
INSERT INTO organizations VALUES('o','u');
INSERT INTO users VALUES('u','','["o"]');
INSERT INTO plants VALUES('p','o',0);
SQL
"$task_dir/grant-patch-data-authority-snapshot" --database "$task_dir/source.db" \
 --source-revision r1 --source-ledger-digest sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa \
 --generation 1 --produced-at 2026-09-08T00:00:00Z --output-dir "$task_dir/bundle" \
 >"$task_dir/export.out" 2>"$task_dir/export.err"
[ ! -s "$task_dir/export.out" ] && [ ! -s "$task_dir/export.err" ]
jq -e '.sourceWatermark=="sha256:f3c71c19fa99588541db89a6aac38912c8876e1c6987c96e020b7e72069b506f" and .snapshotDigest=="sha256:d7bb40927cf19e06e515bfdd562092031865531575ead14c455d68c8d89fa06b"' "$task_dir/bundle/snapshot.json" >/dev/null
printf '%s\n' 'All three cgo-free executable stdout contracts and frozen SQLite export passed'

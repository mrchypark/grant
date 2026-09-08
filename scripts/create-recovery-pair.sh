#!/bin/sh
# Retained as an explicit failure for callers of the retired Hiqlite command.
printf '%s\n' 'Grant Hiqlite recovery-pair creation (including dry runs) is retired. Use docs/backup-restore.md for native Rhiza recovery. Rauthy-only S3 materialization remains available through scripts/materialize-s3-restore.sh rauthy.' >&2
exit 2

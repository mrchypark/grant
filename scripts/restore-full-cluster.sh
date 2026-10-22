#!/bin/sh
# Retained as an explicit failure for callers of the retired Hiqlite command.
printf '%s\n' 'Grant Hiqlite full-cluster recovery (including dry runs) is retired. Use docs/backup-restore.md for native Rhiza recovery; use the immutable c716877 checkout for historical Hiqlite procedures.' >&2
exit 2

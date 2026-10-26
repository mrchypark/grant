#!/bin/sh
# Independent jq/OpenSSL oracle; do not use Grant's Go hashing implementation.
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
file="$root/internal/grant/testdata/organization-context-vectors.json"
sha() { openssl dgst -sha256 -r | awk '{print "sha256:" $1}'; }
for policy in omitted required optional; do
 contract=$(jq -cSj --arg p "$policy" '.[] | select(.policy==$p) | .generation.capabilities[0] | del(.contractDigest)' "$file" | sha)
 expected=$(jq -r --arg p "$policy" '.[] | select(.policy==$p) | .generation.capabilities[0].contractDigest' "$file")
 test "$contract" = "$expected"
 catalog=$(jq -cj --arg p "$policy" '.[] | select(.policy==$p) | .generation' "$file" | sha)
 expected=$(jq -r --arg p "$policy" '.[] | select(.policy==$p) | .catalogDigest' "$file")
 test "$catalog" = "$expected"
 printf '%s %s %s\n' "$policy" "$contract" "$catalog"
done

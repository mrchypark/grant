# Local native API authorization work

The source task is archived Codex task `[L] grant` (`01a07e72-b82a-70c3-9fcb-dcf639cd6172`). Its latest user corrections govern this work: keep PocketBase/Patch Data unchanged, implement authorization in Grant and the generic Exposure sidecar, define native API rights in the catalog, and add generic Grant/Exposure capabilities where a catalog alone is insufficient. Do not add another adapting service or put Grant checks inside PocketBase. Work is local; older IED deployment discussion is not a current deployment instruction.

The continuation requested remaining collection support after record-filtered lists. It did not reduce the original objective to read-only mappings, and it did not request a persistent local development deployment. Base mutation/file mappings and bounded batches have since been implemented and verified; realtime and relationship expansion remain separate incomplete families.

Keep four states distinct:

- The checked-in catalog manifest/constructor defines mappings.
- CLI JSON is a generated staging payload, not an activated database.
- A Grant SQLite/Rhiza dataset can already contain registered actions and an activated catalog even when all processes are stopped.
- A running service is a separate runtime state; stopping it does not erase the preserved database.

Read-only inspection of retained local run `patch-auth-e2e.SvCea0` found both `single/sqlite.db` (voter) and `data-pdp/sqlite.db` (learner) already contained active Patch Data generation 3, 140 capabilities and 35 actions. Its digest was `sha256:a0e57ae9705f4f15b08fd8f6596da5b6bc15a13b9a5bc8189dc9e58c7036bb76`. These are retained synthetic integration datasets, not proof of deployment or an unregistered schema. Subsequent tests create their own isolated data and record exact coverage in `catalog-coverage.md`; do not replace an intended dataset with a new empty database or silently reinterpret a test run as a durable deployment.

The next local checkpoint adds the generic request-array batch contract to the full catalog: explicit root authority plus every child, declared PUT upsert pairs, original-byte forwarding and one fenced Grant decision. This expands the catalog beyond per-record CRUD; it does not by itself complete relationship expansion or realtime subscription authorization. Preserve exact verification evidence for each completed family instead of calling all API support complete from route counts alone.

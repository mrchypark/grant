# HTTP capability catalog coverage

Scope: Grant and Exposure only. The audited business service is Patch Data `05f897b`, account service is Patch Auth `0151ad6`, and native API reference is PocketBase 0.40.3. Those services are unchanged by this catalog work. Route transport support does not by itself establish authority over the returned business records.

## Runtime coverage

| API family | Contract status | Publication requirement |
|---|---|---|
| Single-record JSON CRUD | Implemented | Explicit action/resource mapping, including create targets and mutable relation fields |
| Lists and searches | Actual Grant/learner/PocketBase integration passed for the representative catalog | Exact record authority IDs, row key, safe scalar fields, filter/sort parameters, bounded fexpr ACL |
| Multipart and opaque uploads | Actual Grant-authorized multipart upload passed; generic transport also tested separately | Exact media types and action/resource mapping; body bytes are preserved |
| Files, Range, conditional reads | Actual Grant-authorized exact-byte download passed; Range/conditional/HEAD verified with a controlled PDP | Record-to-file authority mapping and explicit filename segment bound |
| Relation expand and cross-record filter/sort | Pending, not published | Related resource checks; root-record permission does not cover related records |
| Batch and upsert | Pending, not published | All nested operations authorized before one unchanged atomic upstream request; create/update distinction must be explicit |
| Realtime subscriptions and events | Pending, not published | Principal-bound subscription scope, changes, revocation and stream lifetime checks |
| PocketBase account APIs | Separate Patch Auth service | Explicit account-service catalog; business profile collections are not login endpoints |
| System/admin APIs | Private, not automatically published | Explicit independent administrative contract; no collection wildcard |

List filtering uses an upstream ACL filter, not response post-filtering, so native pagination and total counts remain meaningful. Under the configured service token PocketBase may bypass its own user list/view rules; Grant/Exposure therefore cannot rely on those rules to repair an overly broad catalog. The fexpr limit is 200 expressions, including the client filter. Overflow is an error, never a truncated permission set.

### Verification evidence

The fresh local wrapper run `patch-auth-e2e.lrIQXj` passed on 2026-09-09 using actual Grant, its fenced learner, Exposure and native PocketBase. It verified multipart upload through an explicit `file.write` collection binding, exact-byte download through a `file.read` record binding, scoped lists through collection-name and collection-ID aliases, intersection with a client OR filter, native pagination, and revocation producing an empty HTTP 200 list. The existing account, management and failed-fence checks also passed. These are isolated synthetic fixtures; they do not establish missing authority mappings for other business collections or prove a deployed rollout.

A separate native PocketBase suite with a controlled PDP verified Range, conditional responses, HEAD and denied multipart requests. Those results test transport behavior; they are not an actual-Grant authorization proof for every Range or conditional request. Batch, realtime and related-record expansion remain pending as listed above.

## Business collection inventory and mapping status

The fixed schema contains 35 non-system collections: 27 base collections and 8 views. The three former account collections are base business profiles in the extracted service. System collections are excluded. Both concrete collection names and IDs must map to the same operator-chosen canonical resource; a wildcard collection variable must not expose arbitrary collections.

Only the existing `plants` → `plant` / `patch-data.plant.read` mapping is established for the representative read fixture. Every other mapping remains an explicit operator input. In particular, a view's name or a relation to a plant does not establish inherited authority.

| Collection name | Stable collection ID | Runtime type | Authority mapping |
|---|---|---|---|
| `users` | `_pb_users_auth_` | base | Operator mapping required |
| `plants` | `30hrtar148uy66c` | base | Existing plant/read mapping; representative fixture |
| `panelmaps` | `l17r68r7ddzqcij` | base | Operator mapping required |
| `organizations` | `1l9cjra913llae4` | base | Operator mapping required |
| `viewers` | `kovp4j74yv9jr2v` | base | Operator mapping required |
| `invitations` | `lrjsosb8mul77k0` | base | Operator mapping required |
| `images` | `w8l3o9jedinck7g` | base | Operator mapping required |
| `dashboard` | `i95ea9ateuaxwqg` | base | Operator mapping required |
| `comments` | `lrsdfyc753nlnz7` | base | Operator mapping required |
| `permissions` | `pt20y45muocfut5` | base | Operator mapping required |
| `groupconditions` | `1t0dxj7x1r6vvo0` | base | Operator mapping required |
| `passwords` | `8ngtkucypa7xlaj` | base | Operator mapping required |
| `temporary_accounts` | `d57i4dxnnp7l8hh` | base | Operator mapping required |
| `issue_comments` | `snv8bayo8ma6hw0` | base | Operator mapping required |
| `issue_content_logs` | `misvzvyhp0fe7g4` | base | Operator mapping required |
| `issue_fields_logs` | `v5q8qxup8hbxg21` | base | Operator mapping required |
| `issue_tags` | `i10pcc82twxf6pd` | base | Operator mapping required |
| `issues` | `hqrb76fyop74r06` | base | Operator mapping required |
| `work_orders` | `yzruwgkyotkp02h` | base | Operator mapping required |
| `command_history` | `ogq6rbg3df1g496` | base | Operator mapping required |
| `issue_schedules` | `kyzupz6cwz00swl` | base | Operator mapping required |
| `reports` | `r0thjxht4e4dz2m` | base | Operator mapping required |
| `files` | `pbc_3446931122` | base | Operator mapping required |
| `blueprints` | `pbc_1114506494` | base | Operator mapping required |
| `resources` | `pbc_2337082678` | base | Operator mapping required |
| `reports_v2` | `pbc_3451676670` | base | Operator mapping required |
| `org_parent_map` | `35180llsus0m925` | base | Operator mapping required |
| `plants_viewer` | `6260lz1eo0w2vcr` | view | Operator mapping required |
| `panelmaps_viewer` | `8hamrn2tnq9j6c6` | view | Operator mapping required |
| `temporary_panelmaps` | `zkvh97egvg137hs` | view | Operator mapping required |
| `temporary_plants` | `fiqvrx9bli2qvag` | view | Operator mapping required |
| `temporary_blueprints` | `pbc_405412299` | view | Operator mapping required |
| `blueprints_viewer` | `pbc_1337089469` | view | Operator mapping required |
| `latest_loss_reports` | `pbc_1915755548` | view | Operator mapping required |
| `reports_v2_list` | `pbc_2089487142` | view | Operator mapping required |

## Required operator inputs

For each published API, provide the fixed provider, collection name and ID aliases, action IDs, canonical resource type, and resource-ID extraction. Creation and collection management require their own explicit authority target. Declare which root fields are safe scalar filter/sort fields; relation paths, view-to-source identity mappings, nested JSON fields and expansion require separate semantics. Supply media types and byte limits for uploads/files, and the complete permitted request/response header and status policy.

The representative plants catalog fixture is a bounded example, not automatic enrollment of all collections. Catalog generation and activation remain explicit and digest-bound. A schema snapshot identifies API structure; it does not supply missing business authorization decisions.

Primary API evidence: [PocketBase record CRUD](https://github.com/pocketbase/pocketbase/blob/v0.40.3/apis/record_crud.go), [batch API](https://github.com/pocketbase/pocketbase/blob/v0.40.3/apis/batch.go), [realtime API](https://github.com/pocketbase/pocketbase/blob/v0.40.3/apis/realtime.go), and [filter expression limit](https://github.com/pocketbase/pocketbase/blob/v0.40.3/tools/search/provider.go).

Scoped list queries only admit the declared filter/sort roles and explicitly mapped numeric page/page-size or boolean skip-total controls. Arbitrary text or scalar passthrough is unsupported. Native `fields` projection, expansion and relational query options therefore remain unpublished. This provides record-level result filtering, not field-level authorization or automatic discovery of safe upstream parameter semantics.

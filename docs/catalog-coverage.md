# HTTP capability catalog coverage

Scope: Grant and Exposure with the reconciled Patch Data `8155f8a` business schema and Patch Auth `bc973ff` security extraction; native API reference is PocketBase 0.40.3. The two PocketBase services remain on work branches. Route transport support does not by itself establish authority over the returned business records.

## Runtime coverage

| API family | Contract status | Publication requirement |
|---|---|---|
| Single-record JSON CRUD | Explicit mutations for 30 base collections; RSD tasks remain read-only | Separate create collection target and update/delete record target; authority includes the whole native operation and its side effects |
| Lists and searches | Actual Grant/learner/PocketBase integration passed for the representative catalog | Exact record authority IDs, row key, safe scalar fields, filter/sort parameters, bounded fexpr ACL |
| Multipart and opaque uploads | Actual Grant-authorized multipart upload passed; generic transport also tested separately | Exact media types and action/resource mapping; body bytes are preserved |
| Files, Range, conditional reads | Actual Grant-authorized exact-byte download passed; Range/conditional/HEAD verified with a controlled PDP | Record-to-file authority mapping and explicit filename segment bound |
| Relation expand | Explicit graph and buffered post-response authorization implemented | Root and every related read checked in one fenced snapshot before bytes are released |
| Cross-record filter/sort | Pending, not published | Separate query semantics; expansion does not permit arbitrary joins |
| Batch and upsert | Generic bounded request-array contract implemented | Explicit batch root and all child actions, declared PUT create/update pair, one fenced decision before unchanged native atomic request |
| Realtime subscriptions and events | Pending, not published | Principal-bound subscription scope, changes, revocation and stream lifetime checks |
| PocketBase account APIs | Separate Patch Auth service | Explicit account-service catalog; business profile collections are not login endpoints |
| System/admin APIs | Private, not automatically published | Explicit independent administrative contract; no collection wildcard |

List filtering uses an upstream ACL filter, not response post-filtering, so native pagination and total counts remain meaningful. Under the configured service token PocketBase may bypass its own user list/view rules; Grant/Exposure therefore cannot rely on those rules to repair an overly broad catalog. The fexpr limit is 200 expressions, including the client filter. Overflow is an error, never a truncated permission set.

### Verification evidence

The expansion run `patch-auth-e2e.xBjuEy` passed on 2026-09-09 using the 35-node, 68-edge response graph. It verified both collection aliases, nested and array relations, scoped list counts/pages, empty lists, revocation during the upstream fetch, native 404 behavior and unchanged permitted response bytes. Every fetched expanded response was withheld until its post-response decision passed. This is representative native coverage plus generic graph validation; it does not claim positive fixtures for every graph edge or realtime support.

The subsequent batch run `patch-auth-e2e.74577o` passed on 2026-09-09 with 323 routes and 122 actions. It verified root-plus-child permission requirements, JSON and multipart batch forwarding with the exact original body and one upstream request, explicit-ID PUT upsert checks, native atomic rollback, and revocation denying a whole batch. Grant unit tests additionally reject missing/reordered upsert checks, inconsistent original paths/IDs/body digests, forged child capabilities, query injection and more than 20 operations. Relationship expansion and realtime are not covered by this result.

The full native collection run `patch-auth-e2e.4VMTgp` passed on 2026-09-09 with 322 routes and 121 registered actions. It exercised denied base mutations across aliases, explicit create/update/delete grants, byte-preserved native JSON and multipart changes, independent file-read rights, and a real 62-character `org_parent_map` detail ID through both aliases. Native `If-Match` returned 412 for a mismatched ETag; `download=1/0/TRUE/FALSE` retained native behavior. Exposure returned `Cache-Control: no-store` for authorized responses so file caching does not bypass subsequent revocation checks. Grant tests additionally cover every generated mutation action's exact-resource grant/revocation and a 200-action replacement whose failed final insert preserves prior actions and role edges, followed by successful retry.

The fresh actual-Grant/PocketBase wrapper run `patch-auth-e2e.SvCea0` passed on 2026-09-09. It checked deny/empty scope for all 34 newly mapped collections, then explicit positive grants for three base collections and six views, including revocation, name/ID aliases and native list pagination/counts. Existing plant, upload/download, account and management checks also passed. This does not claim positive native fixtures for every collection: the generated catalog's all-35 stage/activation, independent grant and revocation behavior is additionally covered by Grant store/evaluator tests. No deployed rollout is implied.

The fresh local wrapper run `patch-auth-e2e.lrIQXj` passed on 2026-09-09 using actual Grant, its fenced learner, Exposure and native PocketBase. It verified multipart upload through an explicit `file.write` collection binding, exact-byte download through a `file.read` record binding, scoped lists through collection-name and collection-ID aliases, intersection with a client OR filter, native pagination, and revocation producing an empty HTTP 200 list. The existing account, management and failed-fence checks also passed. These are isolated synthetic fixtures; they do not establish missing authority mappings for other business collections or prove a deployed rollout.

A separate native PocketBase suite with a controlled PDP verified Range, conditional responses, HEAD and denied multipart requests. Those results test transport behavior; they are not an actual-Grant authorization proof for every Range or conditional request. That older suite did not cover batch, realtime or related-record expansion.

## Business collection inventory and mapping status

The reconciled schema contains 42 non-system collections: 31 base collections and 11 views. The three former account collections are base business profiles in the extracted service. System collections are excluded. Both concrete collection names and IDs must map to the same operator-chosen canonical resource; a wildcard collection variable must not expose arbitrary collections.

All 42 collections have explicit list and single-record GET mappings in [patch_data_collections.json](../internal/grant/patch_data_collections.json). `go run ./cmd/patch-data-catalog` produces 369 concrete routes and 138 action definitions: 168 list/detail routes, 180 create/update/delete routes for 30 base collections, and 20 file GET/HEAD routes for five file-bearing collections, plus one batch route. Both name and stable-ID aliases use the same authority. `-read-only` retains the 168-route, 42-action read catalog. `plants` preserves its existing resource type; every other collection has an independent resource type equal to its collection name. A view's relation or matching source ID does not establish inherited authority. These mappings do not migrate legacy PocketBase rules or automatically grant new rights to existing plant readers.

| Collection name | Stable collection ID | Runtime type | Authority mapping |
|---|---|---|---|
| `users` | `_pb_users_auth_` | base | `users` / `patch-data.users.read` |
| `plants` | `30hrtar148uy66c` | base | `plant` / `patch-data.plant.read` |
| `panelmaps` | `l17r68r7ddzqcij` | base | `panelmaps` / `patch-data.panelmaps.read` |
| `organizations` | `1l9cjra913llae4` | base | `organizations` / `patch-data.organizations.read` |
| `viewers` | `kovp4j74yv9jr2v` | base | `viewers` / `patch-data.viewers.read` |
| `invitations` | `lrjsosb8mul77k0` | base | `invitations` / `patch-data.invitations.read` |
| `images` | `w8l3o9jedinck7g` | base | `images` / `patch-data.images.read` |
| `dashboard` | `i95ea9ateuaxwqg` | base | `dashboard` / `patch-data.dashboard.read` |
| `comments` | `lrsdfyc753nlnz7` | base | `comments` / `patch-data.comments.read` |
| `permissions` | `pt20y45muocfut5` | base | `permissions` / `patch-data.permissions.read` |
| `groupconditions` | `1t0dxj7x1r6vvo0` | base | `groupconditions` / `patch-data.groupconditions.read` |
| `passwords` | `8ngtkucypa7xlaj` | base | `passwords` / `patch-data.passwords.read` |
| `temporary_accounts` | `d57i4dxnnp7l8hh` | base | `temporary_accounts` / `patch-data.temporary_accounts.read` |
| `issue_comments` | `snv8bayo8ma6hw0` | base | `issue_comments` / `patch-data.issue_comments.read` |
| `issue_content_logs` | `misvzvyhp0fe7g4` | base | `issue_content_logs` / `patch-data.issue_content_logs.read` |
| `issue_fields_logs` | `v5q8qxup8hbxg21` | base | `issue_fields_logs` / `patch-data.issue_fields_logs.read` |
| `issue_tags` | `i10pcc82twxf6pd` | base | `issue_tags` / `patch-data.issue_tags.read` |
| `issues` | `hqrb76fyop74r06` | base | `issues` / `patch-data.issues.read` |
| `work_orders` | `yzruwgkyotkp02h` | base | `work_orders` / `patch-data.work_orders.read` |
| `command_history` | `ogq6rbg3df1g496` | base | `command_history` / `patch-data.command_history.read` |
| `issue_schedules` | `kyzupz6cwz00swl` | base | `issue_schedules` / `patch-data.issue_schedules.read` |
| `reports` | `r0thjxht4e4dz2m` | base | `reports` / `patch-data.reports.read` |
| `files` | `pbc_3446931122` | base | `files` / `patch-data.files.read` |
| `blueprints` | `pbc_1114506494` | base | `blueprints` / `patch-data.blueprints.read` |
| `resources` | `pbc_2337082678` | base | `resources` / `patch-data.resources.read` |
| `reports_v2` | `pbc_3451676670` | base | `reports_v2` / `patch-data.reports_v2.read` |
| `org_parent_map` | `35180llsus0m925` | base | `org_parent_map` / `patch-data.org_parent_map.read` |
| `plants_viewer` | `6260lz1eo0w2vcr` | view | `plants_viewer` / `patch-data.plants_viewer.read` |
| `panelmaps_viewer` | `8hamrn2tnq9j6c6` | view | `panelmaps_viewer` / `patch-data.panelmaps_viewer.read` |
| `temporary_panelmaps` | `zkvh97egvg137hs` | view | `temporary_panelmaps` / `patch-data.temporary_panelmaps.read` |
| `temporary_plants` | `fiqvrx9bli2qvag` | view | `temporary_plants` / `patch-data.temporary_plants.read` |
| `temporary_blueprints` | `pbc_405412299` | view | `temporary_blueprints` / `patch-data.temporary_blueprints.read` |
| `blueprints_viewer` | `pbc_1337089469` | view | `blueprints_viewer` / `patch-data.blueprints_viewer.read` |
| `latest_loss_reports` | `pbc_1915755548` | view | `latest_loss_reports` / `patch-data.latest_loss_reports.read` |
| `reports_v2_list` | `pbc_2089487142` | view | `reports_v2_list` / `patch-data.reports_v2_list.read` |

| `blueprints_list` | `pbc_1185224127` | view | `blueprints_list` / `patch-data.blueprints_list.read` |
| `rsd_command_tasks` | `pbc_4011293327` | base | `rsd_command_tasks` / `patch-data.rsd_command_tasks.read` |
| `agreement_documents` | `pbc_165310787` | base | `agreement_documents` / `patch-data.agreement_documents.read` |
| `anomaly_occurrences` | `anomalyoccur001` | base | `anomaly_occurrences` / `patch-data.anomaly_occurrences.read` |
| `blueprints_viewer_list` | `pbc_1337089470` | view | `blueprints_viewer_list` / `patch-data.blueprints_viewer_list.read` |
| `temporary_blueprints_list` | `pbc_405412300` | view | `temporary_blueprints_list` / `patch-data.temporary_blueprints_list.read` |
| `anomaly_occurrence_checkpoints` | `projcheckpoint1` | base | `anomaly_occurrence_checkpoints` / `patch-data.anomaly_occurrence_checkpoints.read` |

## Generate, register and activate

Run from the Grant repository. Generation is explicit; use the actual latest staged generation for the compare-and-swap precondition (omit `-previous-generation` only for a new provider catalog).

```sh
go run ./cmd/patch-data-catalog -generation 2 -previous-generation 1 > catalog.json
go run ./cmd/patch-data-catalog -actions > actions.json
jq '{actions: .}' actions.json > actions-request.json
```

For an existing provider, save its complete current action array as `existing-actions.json`, then use `go run ./cmd/patch-data-catalog -actions -existing-actions existing-actions.json > merged-actions.json` and `jq '{actions: .}' merged-actions.json > actions-request.json`. The merge retains unrelated actions and existing read-action metadata, including either previously stored `plant.read` or `patch-data.plant.read` code. New action IDs use the provider prefix, with short codes such as `panelmaps.read`. Review the full replacement payload before registration; the merge cannot discover actions absent from its supplied snapshot.

The command only emits files; it makes no network calls and does not grant or activate anything. The provider is fixed to `patch-data`. Register the action definitions through `PUT /api/v1/admin/providers/patch-data/actions` with the `{actions:[...]}` body. This endpoint replaces the provider action set: merge any existing custom action definitions with the generated definitions before submitting to an existing provider. Likewise, the generated stage replaces the active route set when activated; preserve separately reviewed custom routes in a combined generation and recompute its contract/catalog digests if they must remain available. If an old route overlaps a generated route, review its action/resource contract before replacement; blindly appending both is rejected. Do not replace unrelated active routes without that review.

For example, reading a `panelmaps` record requires a role containing `patch-data.panelmaps.read`, an admitted application containing that action, and a binding with `providerId:"patch-data", resourceType:"panelmaps", resourceId:"<actual-record-id>"`. Create the role with `POST /api/v1/orgs/{org}/roles`, set its `{actionIds:["patch-data.panelmaps.read"]}` through `PUT /api/v1/orgs/{org}/roles/{role}/actions`, and admit the action through `PUT /api/v1/orgs/{org}/apps/{app}/actions` under the existing application trust/consent workflow. These PUT endpoints replace their respective action sets, so preserve existing entries. Bind a permitted subject with:

```json
{"id":"panelmap-reader-binding","subjectType":"user","subjectId":"<grant-user-id>","subjectOrganizationId":"<org>","providerId":"patch-data","resourceType":"panelmaps","resourceId":"<actual-record-id>","roleId":"<role>","agreementId":null,"state":"active"}
```

Submit that body to `POST /api/v1/orgs/{org}/bindings`. Membership, application admission, agreements and consent remain enforced by the existing evaluator. A list needs the same exact record grants as GET; the list's constant collection-name target does not grant all rows. With no matching record grants an admitted application receives an empty list, and a detail read is denied. A view requires its own binding using the view's returned `id`, even when that ID equals a source row ID. Revoke the binding through the existing binding lifecycle; the next authorized list reflects the revoked scope.

Stage `catalog.json` through `POST /api/v1/admin/providers/patch-data/http-capability-generations`, register the actual PEP application at `PUT /api/v1/admin/providers/patch-data/pep-clients/{pep}`, then activate via `POST /api/v1/admin/providers/patch-data/http-capability-active` with the current active generation/digest as `expectedGeneration`/`expectedCatalogDigest` and the output generation/digest as `targetGeneration`/`targetCatalogDigest`. Configure Exposure with the resulting catalog digest. Current active and latest staged generations can differ; do not substitute one for the other. No production registration or activation is implied by generating this artifact.

## Mutation and file authority

For a base collection with resource type `R` and collection name `C`, the catalog declares the following independent rights. A role or application must explicitly contain the requested action; possessing a read action or a grant in another namespace never implies these rights.

| Native operation | Action | Resource type / ID |
|---|---|---|
| POST create | `patch-data.R.create` | `collection` / `C` |
| PATCH update, including native file upload/removal | `patch-data.R.update` | `R` / path record ID |
| DELETE | `patch-data.R.delete` | `R` / path record ID |
| GET or HEAD record file | `patch-data.R.file.read` | `R` / path record ID |

These high-risk mutation actions authorize the complete native operation, including mutable relation fields, implicit relation cleanup, cascaded deletes, and existing PocketBase hook effects. They do **not** separately check the authority of each related or affected record, impose field allowlists, or inherit permissions from a relation. In the unchanged source, organization writes maintain `org_parent_map`, issue updates may append issue logs, plant writes invoke geocoding hooks, and relation cascade settings can delete dependent records. Grant these actions only to subjects permitted to perform the full operation. Updating business `permissions`, `users` or organization rows does not create Grant memberships, roles or grants. Existing issue logging uses the native service principal; this catalog does not rewrite it into the end-user identity.

POST/PATCH accept bounded native JSON or multipart using the existing `opaque` transport and exact `application/json` / `multipart/form-data` content types. Exposure hashes and forwards original bytes; PocketBase validates their syntax and fields. Non-file collections have a 1 MiB request bound; `users`, `organizations`, `images`, `reports` and `files` have a 128 MiB request bound. JSON responses remain bounded at 1 MiB. The seven source file fields are `users.avatar`, `organizations.logo/icon/homeBanner`, `images.image`, `reports.file`, and `files.file`; their native validation limits still apply. Uploads use create/update rights, while downloading any file on the record requires the separate `.file.read` right. File bytes are bounded at 128 MiB, filename segments at 1024 decoded bytes, with Range/conditional headers and bounded `download`/`thumb` query parameters.

Configure **both** Exposure request and response maxima to at least 128 MiB when loading the full catalog. A smaller response maximum rejects the catalog, and a smaller request maximum rejects file-collection mutation policies even for small payloads. These are explicit supported transport limits, not unlimited native file support.

`org_parent_map` is a base collection materialized by existing organization hooks. Its native IDs use `lower(hex(organizationID || ':' || parentID))` (62 characters for two 15-character IDs), so its detail/mutation extractor uses bounded `asciiToken`; other collection record IDs retain `pocketbaseId`. It receives explicit whole-record mutation rights like other bases, but source hooks may rebuild its rows. Views remain read-only.

## Read authority and remaining API inputs

The read actions are marked high risk because they authorize the entire native record response, including business profile data and any sensitive fields exposed by the service token. In particular, `passwords` may contain business password values; `temporary_accounts`, `users`, `permissions` and invitation records can contain sensitive access or identity metadata. Grant these explicit actions/record bindings only to principals allowed to receive the whole record. The scalar query allowlist does not redact response fields, implement field authorization, or permit login through business profiles. No role, organization membership, or action is inferred from those records.

Scalar filter/sort fields are pinned to the audited schema snapshot `1785485804_collections_snapshot.js` after its auth-to-base field removal. The checked-in manifest excludes hidden fields, relations, JSON, file fields, password values, temporary links and removed auth fields. View fields are only their declared scalar output fields; no source relation lookup is inferred. `plants` retains `created,id,isArchive,name,updated`. The source snapshot supplies structure and row identity, while the independent resource namespaces here are explicit new policy mappings. Changes to the source schema require reviewing the manifest before publication.

For each published API, provide the fixed provider, collection name and ID aliases, action IDs, canonical resource type, and resource-ID extraction. Creation and collection management require their own explicit authority target. Declare which root fields are safe scalar filter/sort fields; relation paths, view-to-source identity mappings, nested JSON fields and expansion require separate semantics. Supply media types and byte limits for uploads/files, and the complete permitted request/response header and status policy.

The generated catalog publishes list/detail reads, base create/update/delete, and record-file GET/HEAD. Realtime, cross-record filters and native field projection remain unpublished. Catalog generation and activation remain explicit and digest-bound.

Primary API evidence: [PocketBase record CRUD](https://github.com/pocketbase/pocketbase/blob/v0.40.3/apis/record_crud.go), [batch API](https://github.com/pocketbase/pocketbase/blob/v0.40.3/apis/batch.go), [realtime API](https://github.com/pocketbase/pocketbase/blob/v0.40.3/apis/realtime.go), and [filter expression limit](https://github.com/pocketbase/pocketbase/blob/v0.40.3/tools/search/provider.go).

Scoped list queries only admit the declared filter/sort roles and explicitly mapped numeric page/page-size or boolean skip-total controls. Arbitrary text or scalar passthrough is unsupported. Native `fields` projection and cross-record filtering remain unpublished; declared `expand` paths use the separate response graph. This provides record-level result filtering, not field-level authorization or automatic discovery of safe upstream parameter semantics.

## Latest main source reconciliation for IED development

The catalog now follows the business schema from Patch Data main `af6b560`,
reconciled with the auth extraction branch. It contains 42 collections and 74
explicit relation edges. Added namespaces are `blueprints_list`,
`temporary_blueprints_list`, `blueprints_viewer_list`, `rsd_command_tasks`,
`anomaly_occurrences`, `anomaly_occurrence_checkpoints`, and `agreement_documents`.
The three blueprint list collections remain independent view authorities.

`rsd_command_tasks` is explicitly read-only in the manifest despite being a base
collection. Its native API rules include business state-transition constraints
that a service superuser bypasses. Create/update/delete and batch upsert routes
are therefore not published for that collection. No replacement hook was added.
All other published whole-record mutation semantics remain as stated above.

MFA challenge and send-window state moved to Patch Auth and is excluded from this
business catalog. Historical native verification runs above used the older
35-collection snapshot; the reconciled catalog has its own stage/activation and
all-namespace authority tests and requires fresh native integration evidence.
Patch Data and Patch Auth remain on their work branches for deployment; only
Grant and Exposure are merged to main, per the latest user instruction.

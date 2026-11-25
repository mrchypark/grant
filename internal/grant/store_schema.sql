                CREATE TABLE IF NOT EXISTS users (
                    id TEXT PRIMARY KEY,
                    email TEXT,
                    display_name TEXT NOT NULL,
                    active INTEGER NOT NULL DEFAULT 1,
                    created_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS user_identities (
                    id TEXT PRIMARY KEY,
                    user_id TEXT NOT NULL,
                    provider TEXT NOT NULL,
                    issuer TEXT NOT NULL DEFAULT '',
                    collection_id TEXT NOT NULL DEFAULT '',
                    external_user_id TEXT NOT NULL,
                    email_snapshot TEXT,
                    display_name_snapshot TEXT,
                    last_seen_at TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    CHECK ((issuer = '' AND collection_id = '' AND provider <> 'pocketbase') OR (issuer <> '' AND collection_id <> '' AND provider = 'pocketbase'))
                );
                CREATE TABLE IF NOT EXISTS auth_identity_cache (
                    token_hash TEXT PRIMARY KEY,
                    provider TEXT NOT NULL,
                    external_user_id TEXT NOT NULL,
                    client_id TEXT NOT NULL,
                    email TEXT,
                    display_name TEXT NOT NULL,
                    active INTEGER NOT NULL,
                    expires_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS parties (
                    id TEXT PRIMARY KEY,
                    business_no TEXT NOT NULL UNIQUE,
                    legal_name TEXT NOT NULL,
                    active INTEGER NOT NULL DEFAULT 1,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS organizations (
                    id TEXT PRIMARY KEY,
                    business_no TEXT,
                    name TEXT NOT NULL,
                    inactivated_at TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS organization_party_links (
                    organization_id TEXT NOT NULL,
                    party_id TEXT NOT NULL,
                    relation_type TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    PRIMARY KEY (organization_id, party_id, relation_type)
                );
                CREATE TABLE IF NOT EXISTS organization_memberships (
                    id TEXT PRIMARY KEY,
                    organization_id TEXT NOT NULL,
                    user_id TEXT NOT NULL,
                    membership_role TEXT NOT NULL,
                    status TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    UNIQUE (organization_id, user_id)
                );
                CREATE TABLE IF NOT EXISTS groups (
                    id TEXT PRIMARY KEY,
                    organization_id TEXT NOT NULL,
                    name TEXT NOT NULL,
                    description TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    UNIQUE (organization_id, name)
                );
                CREATE TABLE IF NOT EXISTS group_memberships (
                    group_id TEXT NOT NULL,
                    organization_membership_id TEXT NOT NULL,
                    PRIMARY KEY (group_id, organization_membership_id)
                );
                CREATE TABLE IF NOT EXISTS agreements (
                    id TEXT PRIMARY KEY,
                    agreement_type TEXT NOT NULL,
                    grantor_organization_id TEXT NOT NULL,
                    grantee_organization_id TEXT NOT NULL,
                    status TEXT NOT NULL,
                    title TEXT NOT NULL,
                    reference_no TEXT,
                    starts_at TEXT,
                    ends_at TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS service_providers (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    description TEXT,
                    active INTEGER NOT NULL DEFAULT 1,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS actions (
                    id TEXT PRIMARY KEY,
                    provider_id TEXT NOT NULL DEFAULT 'grant',
                    code TEXT NOT NULL,
                    name TEXT NOT NULL,
                    description TEXT,
                    risk_level TEXT NOT NULL,
                    visibility_granting INTEGER NOT NULL DEFAULT 0,
                    grantable_to_applications INTEGER NOT NULL DEFAULT 0,
                    consent_label TEXT,
                    consent_description TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS roles (
                    id TEXT PRIMARY KEY,
                    organization_id TEXT,
                    code TEXT NOT NULL,
                    name TEXT NOT NULL,
                    description TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    UNIQUE (organization_id, code)
                );
                CREATE TABLE IF NOT EXISTS role_actions (
                    role_id TEXT NOT NULL,
                    action_id TEXT NOT NULL,
                    PRIMARY KEY (role_id, action_id)
                );
                CREATE TABLE IF NOT EXISTS role_action_field_views (
                    role_id TEXT NOT NULL,
                    action_id TEXT NOT NULL,
                    resource_type TEXT NOT NULL,
                    schema_key TEXT NOT NULL,
                    field_pointer TEXT NOT NULL,
                    view_mode TEXT NOT NULL
                        CONSTRAINT role_action_field_view_mode
                        CHECK (view_mode IN ('real', 'anonymized')),
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    PRIMARY KEY (
                        role_id,
                        action_id,
                        resource_type,
                        schema_key,
                        field_pointer
                    ),
                    FOREIGN KEY (role_id) REFERENCES roles(id),
                    FOREIGN KEY (action_id) REFERENCES actions(id),
                    FOREIGN KEY (role_id, action_id)
                        REFERENCES role_actions(role_id, action_id)
                );
                CREATE TABLE IF NOT EXISTS field_view_policy_audit_outbox (
                    event_id TEXT NOT NULL PRIMARY KEY,
                    idempotency_key TEXT NOT NULL UNIQUE,
                    request_hash TEXT NOT NULL,
                    organization_id TEXT NOT NULL,
                    actor_user_id TEXT NOT NULL,
                    role_id TEXT NOT NULL,
                    action_id TEXT NOT NULL,
                    resource_type TEXT NOT NULL,
                    schema_key TEXT NOT NULL,
                    before_json TEXT NOT NULL,
                    after_json TEXT NOT NULL,
                    created_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS role_bindings (
                    id TEXT PRIMARY KEY,
                    subject_type TEXT NOT NULL,
                    subject_id TEXT NOT NULL,
                    subject_organization_id TEXT NOT NULL,
                    provider_id TEXT NOT NULL DEFAULT 'grant',
                    resource_type TEXT NOT NULL,
                    resource_id TEXT NOT NULL,
                    role_id TEXT NOT NULL,
                    agreement_id TEXT,
                    state TEXT NOT NULL,
                    created_by_user_id TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS applications (
                    id TEXT PRIMARY KEY,
                    organization_id TEXT NOT NULL,
                    slug TEXT NOT NULL UNIQUE,
                    name TEXT NOT NULL,
                    description TEXT,
                    application_type TEXT NOT NULL,
                    app_type TEXT NOT NULL,
                    first_party INTEGER NOT NULL DEFAULT 0,
                    consent_policy TEXT NOT NULL,
                    active INTEGER NOT NULL DEFAULT 1,
                    provisioning_status TEXT NOT NULL DEFAULT 'unprovisioned',
                    rauthy_client_id TEXT,
                    security_mutation_token TEXT,
                    security_mutation_expires_at TEXT,
                    created_by_user_id TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS application_redirect_uris (
                    application_id TEXT NOT NULL,
                    redirect_uri TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    PRIMARY KEY (application_id, redirect_uri)
                );
                CREATE TABLE IF NOT EXISTS application_actions (
                    application_id TEXT NOT NULL,
                    action_id TEXT NOT NULL,
                    PRIMARY KEY (application_id, action_id)
                );
                CREATE TABLE IF NOT EXISTS application_grants (
                    id TEXT PRIMARY KEY,
                    user_id TEXT NOT NULL,
                    application_id TEXT NOT NULL,
                    organization_id TEXT NOT NULL,
                    status TEXT NOT NULL,
                    consented_at TEXT NOT NULL,
                    revoked_at TEXT,
                    revoked_by_user_id TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS application_grant_actions (
                    application_grant_id TEXT NOT NULL,
                    action_id TEXT NOT NULL,
                    PRIMARY KEY (application_grant_id, action_id)
                );
                CREATE TABLE IF NOT EXISTS application_consent_assertions (
                    id TEXT PRIMARY KEY,
                    valid INTEGER NOT NULL
                        CONSTRAINT application_consent_assertion_valid CHECK (valid = 1)
                );
                CREATE TABLE IF NOT EXISTS patch_data_authority_imports (
                    provider_id TEXT PRIMARY KEY,
                    generation TEXT NOT NULL,
                    ledger_digest TEXT NOT NULL,
                    foundation_digest TEXT NOT NULL,
                    applied_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS organization_hierarchy_edges (
                    source_provider TEXT NOT NULL,
                    parent_organization_id TEXT NOT NULL,
                    child_organization_id TEXT NOT NULL,
                    source_generation TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    PRIMARY KEY (source_provider, parent_organization_id, child_organization_id),
                    UNIQUE (source_provider, child_organization_id)
                );
                CREATE TABLE IF NOT EXISTS patch_data_authority_import_entities (
                    entity_type TEXT NOT NULL,
                    entity_id TEXT NOT NULL,
                    source_generation TEXT NOT NULL,
                    PRIMARY KEY (entity_type, entity_id)
                );
                CREATE TABLE IF NOT EXISTS provider_http_capability_sets (
                    provider_id TEXT NOT NULL,
                    generation INTEGER NOT NULL CHECK (generation > 0),
                    catalog_digest TEXT NOT NULL,
                    canonical_json TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    PRIMARY KEY (provider_id, generation),
                    UNIQUE (provider_id, catalog_digest),
                    FOREIGN KEY (provider_id) REFERENCES service_providers(id)
                );
                CREATE TABLE IF NOT EXISTS provider_http_capability_active (
                    provider_id TEXT PRIMARY KEY,
                    generation INTEGER NOT NULL,
                    catalog_digest TEXT NOT NULL,
                    activated_at TEXT NOT NULL,
                    activated_by TEXT NOT NULL,
                    FOREIGN KEY (provider_id, generation)
                        REFERENCES provider_http_capability_sets(provider_id, generation)
                );
                CREATE TABLE IF NOT EXISTS provider_http_capability_activations (
                    activation_receipt_id TEXT NOT NULL UNIQUE,
                    provider_id TEXT NOT NULL,
                    activation_sequence INTEGER NOT NULL,
                    from_generation INTEGER,
                    to_generation INTEGER NOT NULL,
                    catalog_digest TEXT NOT NULL,
                    activated_at TEXT NOT NULL,
                    activated_by TEXT NOT NULL,
                    PRIMARY KEY (provider_id, activation_sequence)
                );
                CREATE TABLE IF NOT EXISTS provider_pep_clients (
                    pep_client_id TEXT PRIMARY KEY,
                    application_id TEXT NOT NULL UNIQUE,
                    provider_id TEXT NOT NULL,
                    lifecycle TEXT NOT NULL CHECK (lifecycle IN ('active', 'disabled', 'deleted')),
                    valid_until TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    FOREIGN KEY (application_id) REFERENCES applications(id),
                    FOREIGN KEY (provider_id) REFERENCES service_providers(id)
                );
                CREATE TABLE IF NOT EXISTS provider_pep_client_receipts (
                    receipt_id TEXT PRIMARY KEY,
                    pep_client_id TEXT NOT NULL,
                    application_id TEXT NOT NULL,
                    provider_id TEXT NOT NULL,
                    lifecycle TEXT NOT NULL,
                    valid_until TEXT,
                    changed_at TEXT NOT NULL,
                    changed_by TEXT NOT NULL
                );
ALTER TABLE patch_data_authority_imports ADD COLUMN manifest_version TEXT NOT NULL DEFAULT 'grant.patch-data-authority.v1';
CREATE TABLE grant_import_payloads (import_id TEXT NOT NULL, kind TEXT NOT NULL, ordinal INTEGER NOT NULL, payload TEXT NOT NULL, PRIMARY KEY(import_id,kind,ordinal));

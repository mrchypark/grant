mod support;

use domain::{Action, ActionRiskLevel, ServiceProvider};
use infra::{config::ReadConsistency, database::Database};
use support::{NameRow, test_runtime};

type TableRow = NameRow;

const AUTHORIZATION_TABLES: &[&str] = &[
    "user_identities",
    "parties",
    "organizations",
    "organization_party_links",
    "organization_memberships",
    "groups",
    "group_memberships",
    "agreements",
    "service_providers",
    "actions",
    "roles",
    "role_actions",
    "role_bindings",
    "applications",
    "application_redirect_uris",
    "application_actions",
    "application_grants",
    "application_grant_actions",
];

const REMOVED_GRANT_TABLES: &[&str] = &[
    "agreement_documents",
    "assets",
    "asset_profiles",
    "registries",
    "registry_accounts",
    "registry_projects",
    "data_sources",
    "generation_data",
    "rate_data",
    "rate_data_old_real_value",
    "energy_certificates",
    "auth_credentials",
    "certificates",
    "cert_gateway_challenges",
    "crawl_task_tokens",
    "credential_bindings",
    "country_capabilities",
    "sync_runs",
    "sync_run_items",
    "effective_organization_asset_actions",
    "effective_user_asset_actions",
    "effective_user_asset_visibility",
    "effective_organization_plant_actions",
    "effective_user_plant_actions",
    "effective_user_plant_visibility",
    "plants",
    "generation_records",
    "rate_records",
    "rec_records",
    "documents",
];

#[tokio::test]
async fn ensure_schema_creates_current_authorization_tables() {
    let (runtime, _db, _dir) = support::ensured_database().await;
    let names = support::table_names(&runtime).await;

    for &table in AUTHORIZATION_TABLES {
        assert!(names.contains(table), "missing table {table}");
    }

    assert!(!names.contains("businesses"));
    assert!(!names.contains("business_memberships"));
    assert!(!names.contains("organization_edges"));
    assert!(!names.contains("organization_closure"));
    for &removed_table in REMOVED_GRANT_TABLES {
        assert!(
            !names.contains(removed_table),
            "removed table still exists {removed_table}"
        );
    }
}

#[tokio::test]
async fn ensure_schema_creates_authorization_columns() {
    let (runtime, _db, _dir) = support::ensured_database().await;

    support::assert_table_columns(
        &runtime,
        "role_bindings",
        &[
            "id",
            "subject_type",
            "subject_id",
            "subject_organization_id",
            "provider_id",
            "resource_type",
            "resource_id",
            "role_id",
            "agreement_id",
            "state",
            "created_by_user_id",
            "created_at",
            "updated_at",
        ],
    )
    .await;

    support::assert_table_columns(
        &runtime,
        "actions",
        &[
            "id",
            "provider_id",
            "code",
            "name",
            "description",
            "risk_level",
            "visibility_granting",
            "grantable_to_applications",
            "consent_label",
            "consent_description",
            "created_at",
            "updated_at",
        ],
    )
    .await;

    support::assert_table_columns(
        &runtime,
        "applications",
        &[
            "id",
            "organization_id",
            "slug",
            "name",
            "description",
            "application_type",
            "app_type",
            "first_party",
            "consent_policy",
            "active",
            "rauthy_client_id",
            "secret_rotated_at",
            "created_by_user_id",
            "created_at",
            "updated_at",
        ],
    )
    .await;

    support::assert_table_columns(
        &runtime,
        "application_redirect_uris",
        &["application_id", "redirect_uri", "created_at"],
    )
    .await;

    support::assert_table_columns(
        &runtime,
        "application_actions",
        &["application_id", "action_id"],
    )
    .await;

    support::assert_table_columns(
        &runtime,
        "application_grants",
        &[
            "id",
            "user_id",
            "application_id",
            "organization_id",
            "status",
            "consented_at",
            "revoked_at",
            "revoked_by_user_id",
            "created_at",
            "updated_at",
        ],
    )
    .await;

    support::assert_table_columns(
        &runtime,
        "application_grant_actions",
        &["application_grant_id", "action_id"],
    )
    .await;
}

#[tokio::test]
async fn ensure_schema_creates_organization_identity_columns() {
    let (runtime, _db, _dir) = support::ensured_database().await;
    let organization_columns = support::table_columns(&runtime, "organizations").await;
    assert!(!organization_columns.contains(&"party_id".to_owned()));
    assert!(!organization_columns.contains(&"legal_entity_id".to_owned()));
    assert!(organization_columns.contains(&"business_no".to_owned()));
    assert!(organization_columns.contains(&"inactivated_at".to_owned()));
    assert!(!organization_columns.contains(&"active".to_owned()));

    support::assert_table_columns(
        &runtime,
        "organization_party_links",
        &[
            "organization_id",
            "party_id",
            "relation_type",
            "created_at",
            "updated_at",
        ],
    )
    .await;

    support::assert_table_columns(
        &runtime,
        "parties",
        &[
            "id",
            "business_no",
            "legal_name",
            "active",
            "created_at",
            "updated_at",
        ],
    )
    .await;
}

#[tokio::test]
async fn ensure_schema_creates_authorization_indexes() {
    let (runtime, _db, _dir) = support::ensured_database().await;
    support::assert_indexes(
        &runtime,
        &[
            "idx_agreements_grantor_grantee",
            "idx_actions_provider_code_unique",
            "idx_application_grants_active_unique",
            "idx_application_grants_user_application",
            "idx_organization_party_links_org_relation",
            "idx_parties_business_no",
            "idx_roles_global_code",
            "idx_service_providers_active",
            "idx_user_identities_provider_external",
        ],
    )
    .await;
    assert!(
        support::has_unique_index_on_columns(&runtime, "applications", &["slug"]).await,
        "applications.slug must have a unique index"
    );
    assert!(
        support::has_unique_index_on_columns(
            &runtime,
            "application_redirect_uris",
            &["application_id", "redirect_uri"],
        )
        .await,
        "application_redirect_uris(application_id, redirect_uri) must be unique"
    );
    assert!(
        support::has_unique_index_on_columns(&runtime, "actions", &["provider_id", "code"]).await,
        "actions(provider_id, code) must have a unique index"
    );
    assert!(
        !support::has_unique_index_on_columns(&runtime, "actions", &["code"]).await,
        "actions.code must not be globally unique"
    );
}

#[tokio::test]
async fn ensure_schema_drops_removed_grant_tables() {
    let (runtime, _dir) = test_runtime().await;
    runtime
        .batch(
            r#"
            CREATE TABLE agreement_documents (id TEXT PRIMARY KEY);
            CREATE TABLE assets (id TEXT PRIMARY KEY);
            CREATE TABLE asset_profiles (id TEXT PRIMARY KEY);
            CREATE TABLE registries (id TEXT PRIMARY KEY);
            CREATE TABLE registry_accounts (id TEXT PRIMARY KEY);
            CREATE TABLE registry_projects (id TEXT PRIMARY KEY);
            CREATE TABLE data_sources (id TEXT PRIMARY KEY);
            CREATE TABLE generation_data (id TEXT PRIMARY KEY);
            CREATE TABLE rate_data (id TEXT PRIMARY KEY);
            CREATE TABLE rate_data_old_real_value (id TEXT PRIMARY KEY);
            CREATE TABLE energy_certificates (id TEXT PRIMARY KEY);
            CREATE TABLE auth_credentials (id TEXT PRIMARY KEY);
            CREATE TABLE certificates (id TEXT PRIMARY KEY);
            CREATE TABLE cert_gateway_challenges (id TEXT PRIMARY KEY);
            CREATE TABLE crawl_task_tokens (id TEXT PRIMARY KEY);
            CREATE TABLE credential_bindings (id TEXT PRIMARY KEY);
            CREATE TABLE country_capabilities (id TEXT PRIMARY KEY);
            CREATE TABLE sync_runs (id TEXT PRIMARY KEY);
            CREATE TABLE sync_run_items (id TEXT PRIMARY KEY);
            CREATE TABLE effective_organization_asset_actions (id TEXT PRIMARY KEY);
            CREATE TABLE effective_user_asset_actions (id TEXT PRIMARY KEY);
            CREATE TABLE effective_user_asset_visibility (id TEXT PRIMARY KEY);
            CREATE TABLE effective_organization_plant_actions (id TEXT PRIMARY KEY);
            CREATE TABLE effective_user_plant_actions (id TEXT PRIMARY KEY);
            CREATE TABLE effective_user_plant_visibility (id TEXT PRIMARY KEY);
            CREATE TABLE plants (id TEXT PRIMARY KEY);
            CREATE TABLE generation_records (id TEXT PRIMARY KEY);
            CREATE TABLE rate_records (id TEXT PRIMARY KEY);
            CREATE TABLE rec_records (id TEXT PRIMARY KEY);
            CREATE TABLE documents (id TEXT PRIMARY KEY);
            "#
            .to_owned(),
        )
        .await
        .expect("seed removed tables");

    let db = support::test_database(runtime.clone());
    db.ensure_schema_only().await.expect("ensure schema only");

    let names = support::table_names(&runtime).await;
    for &removed_table in REMOVED_GRANT_TABLES {
        assert!(
            !names.contains(removed_table),
            "removed table still exists {removed_table}"
        );
    }

    runtime
        .batch(
            r#"
            INSERT INTO actions (
                id, provider_id, code, name, description, risk_level, visibility_granting,
                grantable_to_applications, consent_label, consent_description, created_at, updated_at
            ) VALUES
                ('plant.profile.read', 'grant', 'plant.profile.read', 'Read Plant Profile', NULL, 'low', 1, 1, NULL, NULL, '2026-04-12T00:00:00Z', '2026-04-12T00:00:00Z'),
                ('asset.profile.read', 'grant', 'asset.profile.read', 'Read Asset Profile', NULL, 'low', 1, 1, NULL, NULL, '2026-04-12T00:00:00Z', '2026-04-12T00:00:00Z'),
                ('organization.plant.create', 'grant', 'organization.plant.create', 'Create Plant', NULL, 'medium', 0, 0, NULL, NULL, '2026-04-12T00:00:00Z', '2026-04-12T00:00:00Z'),
                ('organization.credential.use', 'grant', 'organization.credential.use', 'Use Credential', NULL, 'critical', 0, 0, NULL, NULL, '2026-04-12T00:00:00Z', '2026-04-12T00:00:00Z'),
                ('document.profile.read', 'account', 'document.profile.read', 'Read Document Profile', NULL, 'low', 1, 1, NULL, NULL, '2026-04-12T00:00:00Z', '2026-04-12T00:00:00Z'),
                ('cert-data.credential.use', 'account', 'cert-data.credential.use', 'Use Cert Data Credential', NULL, 'critical', 0, 0, NULL, NULL, '2026-04-12T00:00:00Z', '2026-04-12T00:00:00Z'),
                ('organization.profile.read', 'grant', 'organization.profile.read', 'Read Organization Profile', NULL, 'low', 1, 1, NULL, NULL, '2026-04-12T00:00:00Z', '2026-04-12T00:00:00Z');
            INSERT INTO role_actions (role_id, action_id)
            VALUES
                ('role-old-plant', 'plant.profile.read'),
                ('role-old-asset', 'asset.profile.read'),
                ('role-old-document', 'document.profile.read'),
                ('role-old-cert-data', 'cert-data.credential.use'),
                ('role-grant-provider', 'organization.profile.read');
            INSERT INTO role_bindings (
                id, subject_type, subject_id, subject_organization_id, provider_id, resource_type, resource_id,
                role_id, agreement_id, state, created_by_user_id, created_at, updated_at
            ) VALUES
                (
                    'binding-old-plant', 'organization', 'org-1', 'org-1', 'grant', 'plant', 'plant-1',
                    'role-old-plant', NULL, 'active', 'bootstrap-user', '2026-04-12T00:00:00Z', '2026-04-12T00:00:00Z'
                ),
                (
                    'binding-old-account-document', 'organization', 'org-1', 'org-1', 'account', 'document', 'document-1',
                    'role-old-document', NULL, 'active', 'bootstrap-user', '2026-04-12T00:00:00Z', '2026-04-12T00:00:00Z'
                ),
                (
                    'binding-old-account-cert-data', 'organization', 'org-1', 'org-1', 'account', 'cert_data', 'credential-1',
                    'role-old-cert-data', NULL, 'active', 'bootstrap-user', '2026-04-12T00:00:00Z', '2026-04-12T00:00:00Z'
                );
            INSERT INTO application_actions (application_id, action_id)
            VALUES
                ('application-old', 'plant.profile.read'),
                ('application-old-document', 'document.profile.read'),
                ('application-grant-provider', 'organization.profile.read');
            INSERT INTO application_grant_actions (application_grant_id, action_id)
            VALUES
                ('grant-old', 'asset.profile.read'),
                ('grant-old-cert-data', 'cert-data.credential.use'),
                ('app-grant-provider', 'organization.profile.read');
            INSERT INTO service_providers (id, name, description, active, created_at, updated_at)
            VALUES ('compos', 'Compos', NULL, 1, '2026-04-12T00:00:00Z', '2026-04-12T00:00:00Z');
            INSERT INTO actions (
                id, provider_id, code, name, description, risk_level, visibility_granting,
                grantable_to_applications, consent_label, consent_description, created_at, updated_at
            ) VALUES (
                'compos.object.read', 'compos', 'compos.object.read', 'Read Compos Object',
                NULL, 'low', 1, 1, NULL, NULL, '2026-04-12T00:00:00Z', '2026-04-12T00:00:00Z'
            );
            INSERT INTO role_bindings (
                id, subject_type, subject_id, subject_organization_id, provider_id, resource_type, resource_id,
                role_id, agreement_id, state, created_by_user_id, created_at, updated_at
            ) VALUES (
                'binding-compos-object', 'user', 'bootstrap-user', 'org-1', 'compos', 'object_instance', 'obj-1',
                'role-grant-provider', NULL, 'active', 'bootstrap-user', '2026-04-12T00:00:00Z', '2026-04-12T00:00:00Z'
            );
            "#
            .to_owned(),
        )
        .await
        .expect("seed stale authorization rows");

    db.ensure_schema_only().await.expect("cleanup stale rows");

    let removed_actions: Vec<TableRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT code AS name
             FROM actions
             WHERE code LIKE 'plant.%'
                OR code LIKE 'asset.%'
                OR code LIKE 'document.%'
                OR code LIKE 'cert-data.%'
                OR code LIKE 'organization.plant.%'
                OR code = 'organization.credential.use'",
            vec![],
        )
        .await
        .expect("removed actions");
    assert!(removed_actions.is_empty());

    let stale_refs: Vec<TableRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT action_id AS name FROM role_actions WHERE action_id <> 'organization.profile.read'
             UNION ALL
             SELECT action_id AS name FROM application_actions WHERE action_id <> 'organization.profile.read'
             UNION ALL
             SELECT action_id AS name FROM application_grant_actions WHERE action_id <> 'organization.profile.read'
             UNION ALL
             SELECT resource_type AS name FROM role_bindings
             WHERE provider_id IN ('grant', 'account') AND resource_type <> 'organization'",
            vec![],
        )
        .await
        .expect("stale authorization references");
    assert!(stale_refs.is_empty());

    let kept_actions: Vec<TableRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT code AS name FROM actions WHERE code = 'organization.profile.read'",
            vec![],
        )
        .await
        .expect("kept actions");
    assert_eq!(kept_actions.len(), 1);

    let kept_external_bindings: Vec<TableRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT id AS name FROM role_bindings WHERE id = 'binding-compos-object'",
            vec![],
        )
        .await
        .expect("kept external bindings");
    assert_eq!(kept_external_bindings.len(), 1);
}

#[tokio::test]
async fn replace_provider_actions_rejects_provider_mismatch_before_deleting_existing_actions() {
    let (runtime, _dir) = test_runtime().await;
    let db = support::test_database(runtime);
    db.ensure_schema_only().await.expect("ensure schema only");
    db.upsert_service_provider(&test_provider("compos"))
        .await
        .expect("provider");
    db.upsert_service_provider(&test_provider("other"))
        .await
        .expect("other provider");
    db.upsert_action(&test_action("compos.object.read", "compos"))
        .await
        .expect("existing action");

    let err = db
        .replace_provider_actions("compos", &[test_action("other.object.read", "other")])
        .await
        .expect_err("provider mismatch should fail");
    assert!(
        err.to_string().contains("does not match path provider"),
        "unexpected error: {err:#}"
    );

    let actions = db
        .list_actions_for_provider_with_consistency("compos", Some(ReadConsistency::Strong))
        .await
        .expect("actions");
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].id, "compos.object.read");
}

#[tokio::test]
async fn replace_provider_actions_rejects_duplicate_codes_before_deleting_existing_actions() {
    let (runtime, _dir) = test_runtime().await;
    let db = support::test_database(runtime);
    db.ensure_schema_only().await.expect("ensure schema only");
    db.upsert_service_provider(&test_provider("compos"))
        .await
        .expect("provider");
    db.upsert_action(&test_action("compos.object.read", "compos"))
        .await
        .expect("existing action");

    let err = db
        .replace_provider_actions(
            "compos",
            &[
                test_action("compos.object.write", "compos"),
                test_action("compos.object.write", "compos"),
            ],
        )
        .await
        .expect_err("duplicate codes should fail");
    assert!(
        err.to_string().contains("duplicate action code"),
        "unexpected error: {err:#}"
    );

    let actions = db
        .list_actions_for_provider_with_consistency("compos", Some(ReadConsistency::Strong))
        .await
        .expect("actions");
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].id, "compos.object.read");
}

#[tokio::test]
async fn actions_allow_same_code_across_different_providers() {
    let (runtime, _dir) = test_runtime().await;
    let db = support::test_database(runtime);
    db.ensure_schema_only().await.expect("ensure schema only");
    db.upsert_service_provider(&test_provider("compos"))
        .await
        .expect("provider");
    db.upsert_service_provider(&test_provider("other"))
        .await
        .expect("other provider");

    db.upsert_action(&test_action_with_code(
        "compos.object.read",
        "compos",
        "object.read",
    ))
    .await
    .expect("compos action");
    db.upsert_action(&test_action_with_code(
        "other.object.read",
        "other",
        "object.read",
    ))
    .await
    .expect("other action with reused code");

    let compos_actions = db
        .list_actions_for_provider_with_consistency("compos", Some(ReadConsistency::Strong))
        .await
        .expect("compos actions");
    let other_actions = db
        .list_actions_for_provider_with_consistency("other", Some(ReadConsistency::Strong))
        .await
        .expect("other actions");
    assert_eq!(compos_actions.len(), 1);
    assert_eq!(compos_actions[0].code, "object.read");
    assert_eq!(other_actions.len(), 1);
    assert_eq!(other_actions[0].code, "object.read");
}

#[tokio::test]
async fn ensure_schema_migrates_legacy_account_organization_bindings_to_grant() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "bootstrap-user".to_owned(),
        "Bootstrap User".to_owned(),
    );
    db.ensure_schema().await.expect("ensure schema");
    db.upsert_organization_with_owner("legacy-org", "Legacy Org", None, "Legacy Org")
        .await
        .expect("legacy org");
    db.upsert_organization_with_owner(
        "legacy-account-only-org",
        "Legacy Account Only Org",
        None,
        "Legacy Account Only Org",
    )
    .await
    .expect("legacy account-only org");

    runtime
        .batch(
            r#"
            INSERT INTO actions (
                id, provider_id, code, name, description, risk_level, visibility_granting,
                grantable_to_applications, consent_label, consent_description, created_at, updated_at
            ) VALUES (
                'legacy-account-profile-read', 'account', 'organization.profile.read',
                'Read Organization Profile', NULL, 'low', 1, 1, NULL, NULL,
                '2026-04-12T00:00:00Z', '2026-04-12T00:00:00Z'
            );
            INSERT INTO role_bindings (
                id, subject_type, subject_id, subject_organization_id, provider_id, resource_type, resource_id,
                role_id, agreement_id, state, created_by_user_id, created_at, updated_at
            ) VALUES
                (
                    'binding-legacy-account-duplicate', 'user', 'bootstrap-user', 'bootstrap-user',
                    'account', 'organization', 'bootstrap-user', 'role-system-bootstrap-owner',
                    NULL, 'active', 'bootstrap-user', '2026-04-12T00:00:00Z', '2026-04-12T00:00:00Z'
                ),
                (
                    'binding-legacy-account-unique', 'user', 'bootstrap-user', 'legacy-org',
                    'account', 'organization', 'legacy-org', 'role-system-bootstrap-owner',
                    NULL, 'active', 'bootstrap-user', '2026-04-12T00:00:00Z', '2026-04-13T00:00:00Z'
                ),
                (
                    'binding-legacy-grant-disabled-duplicate', 'user', 'bootstrap-user', 'legacy-org',
                    'grant', 'organization', 'legacy-org', 'role-system-bootstrap-owner',
                    NULL, 'disabled', 'bootstrap-user', '2026-04-12T00:00:00Z', '2026-04-12T00:00:00Z'
                ),
                (
                    'binding-legacy-account-only', 'user', 'bootstrap-user', 'legacy-account-only-org',
                    'account', 'organization', 'legacy-account-only-org', 'role-system-bootstrap-owner',
                    NULL, 'active', 'bootstrap-user', '2026-04-12T00:00:00Z', '2026-04-12T00:00:00Z'
                );
            "#
            .to_owned(),
        )
        .await
        .expect("seed legacy account bindings");

    db.ensure_schema().await.expect("migrate legacy bindings");

    let account_bindings: Vec<TableRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT id AS name FROM role_bindings WHERE provider_id = 'account' AND resource_type = 'organization'",
            vec![],
        )
        .await
        .expect("account bindings");
    assert!(account_bindings.is_empty());

    let migrated_bindings: Vec<TableRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT id AS name FROM role_bindings WHERE id = 'binding-legacy-account-only' AND provider_id = 'grant'",
            vec![],
        )
        .await
        .expect("migrated bindings");
    assert_eq!(migrated_bindings.len(), 1);

    let merged_duplicate_bindings: Vec<TableRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT state AS name
             FROM role_bindings
             WHERE id = 'binding-legacy-grant-disabled-duplicate'
               AND provider_id = 'grant'
               AND updated_at = '2026-04-13T00:00:00Z'",
            vec![],
        )
        .await
        .expect("merged duplicate bindings");
    assert_eq!(
        merged_duplicate_bindings
            .first()
            .map(|row| row.name.as_str()),
        Some("active")
    );

    let duplicate_bindings: Vec<TableRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT id AS name
             FROM role_bindings
             WHERE id IN ('binding-legacy-account-duplicate', 'binding-legacy-account-unique')",
            vec![],
        )
        .await
        .expect("duplicate bindings");
    assert!(duplicate_bindings.is_empty());

    let allowed = db
        .has_user_organization_action_with_consistency(
            "bootstrap-user",
            "legacy-org",
            "legacy-org",
            "organization.profile.read",
            Some(ReadConsistency::Strong),
        )
        .await
        .expect("legacy org action");
    assert!(allowed);

    let account_only_allowed = db
        .has_user_organization_action_with_consistency(
            "bootstrap-user",
            "legacy-account-only-org",
            "legacy-account-only-org",
            "organization.profile.read",
            Some(ReadConsistency::Strong),
        )
        .await
        .expect("legacy account-only org action");
    assert!(account_only_allowed);
}

fn test_provider(id: &str) -> ServiceProvider {
    ServiceProvider {
        id: id.to_owned(),
        name: id.to_owned(),
        description: None,
        active: true,
    }
}

fn test_action(id: &str, provider_id: &str) -> Action {
    test_action_with_code(id, provider_id, id)
}

fn test_action_with_code(id: &str, provider_id: &str, code: &str) -> Action {
    Action {
        id: id.to_owned(),
        provider_id: provider_id.to_owned(),
        code: code.to_owned(),
        name: id.to_owned(),
        description: None,
        risk_level: ActionRiskLevel::Low,
        visibility_granting: true,
        grantable_to_applications: true,
        consent_label: None,
        consent_description: None,
    }
}

#[tokio::test]
async fn ensure_schema_adds_provider_columns_to_existing_authorization_tables() {
    let (runtime, _dir) = test_runtime().await;
    runtime
        .batch(
            r#"
            CREATE TABLE actions (
                id TEXT PRIMARY KEY,
                code TEXT NOT NULL UNIQUE,
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
            CREATE TABLE role_bindings (
                id TEXT PRIMARY KEY,
                subject_type TEXT NOT NULL,
                subject_id TEXT NOT NULL,
                subject_organization_id TEXT NOT NULL,
                resource_type TEXT NOT NULL,
                resource_id TEXT NOT NULL,
                role_id TEXT NOT NULL,
                agreement_id TEXT,
                state TEXT NOT NULL,
                created_by_user_id TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            "#
            .to_owned(),
        )
        .await
        .expect("seed legacy auth tables");

    let db = support::test_database(runtime.clone());
    db.ensure_schema_only().await.expect("ensure schema only");

    for (table, column) in [("actions", "provider_id"), ("role_bindings", "provider_id")] {
        let columns = support::table_columns(&runtime, table).await;
        assert!(
            columns.iter().any(|name| name == column),
            "missing {table}.{column}"
        );
    }
}

#[tokio::test]
async fn ensure_schema_adds_developer_console_columns_to_existing_applications() {
    let (runtime, _dir) = test_runtime().await;
    runtime
        .batch(
            r#"
            CREATE TABLE applications (
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
                created_by_user_id TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            "#
            .to_owned(),
        )
        .await
        .expect("seed legacy applications table");

    let db = support::test_database(runtime.clone());
    db.ensure_schema_only().await.expect("ensure schema only");

    let columns = support::table_columns(&runtime, "applications").await;
    assert!(columns.contains(&"rauthy_client_id".to_owned()));
    assert!(columns.contains(&"secret_rotated_at".to_owned()));
}

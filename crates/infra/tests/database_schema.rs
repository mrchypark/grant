use hiqlite::{Node, NodeConfig, Row, s3::EncKeys};
use infra::{config::ReadConsistency, database::Database, runtime::HiqliteRuntime};
use tempfile::TempDir;
use uuid::Uuid;

#[derive(Debug)]
struct TableRow {
    name: String,
}

impl From<&mut Row<'_>> for TableRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            name: row.get("name"),
        }
    }
}

#[derive(Debug)]
struct ColumnRow {
    name: String,
}

impl From<&mut Row<'_>> for ColumnRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            name: row.get("name"),
        }
    }
}

#[derive(Debug)]
struct IndexRow {
    name: String,
}

impl From<&mut Row<'_>> for IndexRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            name: row.get("name"),
        }
    }
}

#[derive(Debug)]
struct IndexListRow {
    name: String,
    is_unique: i64,
}

impl From<&mut Row<'_>> for IndexListRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            name: row.get("name"),
            is_unique: row.get("is_unique"),
        }
    }
}

#[derive(Debug)]
struct IndexColumnRow {
    name: String,
}

impl From<&mut Row<'_>> for IndexColumnRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            name: row.get("name"),
        }
    }
}

fn free_local_addr() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    drop(listener);
    addr.to_string()
}

async fn test_runtime() -> (HiqliteRuntime, TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let api_addr = free_local_addr();
    let raft_addr = free_local_addr();
    let mut cfg = NodeConfig::default();
    cfg.node_id = 1;
    cfg.nodes = vec![Node {
        id: 1,
        addr_raft: raft_addr.clone(),
        addr_api: api_addr.clone(),
    }];
    cfg.listen_addr_api = "127.0.0.1".into();
    cfg.listen_addr_raft = "127.0.0.1".into();
    cfg.data_dir = dir.path().to_string_lossy().into_owned().into();
    cfg.filename_db = "test.sqlite".into();
    cfg.secret_api = "0123456789abcdef0123456789abcdef".to_owned();
    cfg.secret_raft = "fedcba9876543210fedcba9876543210".to_owned();
    cfg.enc_keys = EncKeys::generate().expect("enc keys");
    cfg.health_check_delay_secs = 0;
    cfg.raft_config = NodeConfig::default_raft_config(128);

    let application = hiqlite::start_node(cfg).await.expect("start hiqlite");
    (
        HiqliteRuntime::embedded(application, ReadConsistency::Local),
        dir,
    )
}

#[tokio::test]
async fn ensure_schema_creates_authorization_tables() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        format!("bootstrap-{}", Uuid::new_v4().simple()),
        "Bootstrap User".to_owned(),
    );

    db.ensure_schema().await.expect("ensure schema");

    let rows: Vec<TableRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name ASC",
            vec![],
        )
        .await
        .expect("list tables");

    let names: std::collections::BTreeSet<String> = rows.into_iter().map(|row| row.name).collect();

    for table in [
        "user_identities",
        "parties",
        "organizations",
        "organization_party_links",
        "organization_memberships",
        "groups",
        "group_memberships",
        "agreements",
        "actions",
        "roles",
        "role_actions",
        "role_bindings",
        "applications",
        "application_actions",
        "application_grants",
        "application_grant_actions",
    ] {
        assert!(names.contains(table), "missing table {table}");
    }

    assert!(!names.contains("businesses"));
    assert!(!names.contains("business_memberships"));
    assert!(!names.contains("organization_edges"));
    assert!(!names.contains("organization_closure"));
    for removed_table in [
        "agreement_documents",
        "assets",
        "asset_profiles",
        "registries",
        "registry_accounts",
        "registry_projects",
        "data_sources",
        "generation_data",
        "rate_data",
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
    ] {
        assert!(
            !names.contains(removed_table),
            "removed table still exists {removed_table}"
        );
    }

    let binding_columns: Vec<ColumnRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT name FROM pragma_table_info('role_bindings') ORDER BY cid ASC",
            vec![],
        )
        .await
        .expect("role_bindings columns");
    let binding_columns: Vec<String> = binding_columns
        .into_iter()
        .map(|column| column.name)
        .collect();
    assert_eq!(
        binding_columns,
        vec![
            "id",
            "subject_type",
            "subject_id",
            "subject_organization_id",
            "resource_type",
            "resource_id",
            "role_id",
            "agreement_id",
            "state",
            "created_by_user_id",
            "created_at",
            "updated_at",
        ]
    );

    let action_columns: Vec<ColumnRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT name FROM pragma_table_info('actions') ORDER BY cid ASC",
            vec![],
        )
        .await
        .expect("actions columns");
    let action_columns: Vec<String> = action_columns
        .into_iter()
        .map(|column| column.name)
        .collect();
    assert_eq!(
        action_columns,
        vec![
            "id",
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
        ]
    );

    let application_columns: Vec<ColumnRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT name FROM pragma_table_info('applications') ORDER BY cid ASC",
            vec![],
        )
        .await
        .expect("applications columns");
    let application_columns: Vec<String> = application_columns
        .into_iter()
        .map(|column| column.name)
        .collect();
    assert_eq!(
        application_columns,
        vec![
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
            "created_by_user_id",
            "created_at",
            "updated_at",
        ]
    );

    let application_action_columns: Vec<ColumnRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT name FROM pragma_table_info('application_actions') ORDER BY cid ASC",
            vec![],
        )
        .await
        .expect("application_actions columns");
    let application_action_columns: Vec<String> = application_action_columns
        .into_iter()
        .map(|column| column.name)
        .collect();
    assert_eq!(
        application_action_columns,
        vec!["application_id", "action_id"]
    );

    let application_grant_columns: Vec<ColumnRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT name FROM pragma_table_info('application_grants') ORDER BY cid ASC",
            vec![],
        )
        .await
        .expect("application_grants columns");
    let application_grant_columns: Vec<String> = application_grant_columns
        .into_iter()
        .map(|column| column.name)
        .collect();
    assert_eq!(
        application_grant_columns,
        vec![
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
        ]
    );

    let application_grant_action_columns: Vec<ColumnRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT name FROM pragma_table_info('application_grant_actions') ORDER BY cid ASC",
            vec![],
        )
        .await
        .expect("application_grant_actions columns");
    let application_grant_action_columns: Vec<String> = application_grant_action_columns
        .into_iter()
        .map(|column| column.name)
        .collect();
    assert_eq!(
        application_grant_action_columns,
        vec!["application_grant_id", "action_id"]
    );

    let organization_columns: Vec<ColumnRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT name FROM pragma_table_info('organizations') ORDER BY cid ASC",
            vec![],
        )
        .await
        .expect("organizations columns");
    let organization_columns: Vec<String> = organization_columns
        .into_iter()
        .map(|column| column.name)
        .collect();
    assert!(!organization_columns.contains(&"party_id".to_owned()));
    assert!(!organization_columns.contains(&"legal_entity_id".to_owned()));
    assert!(organization_columns.contains(&"business_no".to_owned()));
    assert!(organization_columns.contains(&"inactivated_at".to_owned()));
    assert!(!organization_columns.contains(&"active".to_owned()));

    let organization_party_link_columns: Vec<ColumnRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT name FROM pragma_table_info('organization_party_links') ORDER BY cid ASC",
            vec![],
        )
        .await
        .expect("organization_party_links columns");
    let organization_party_link_columns: Vec<String> = organization_party_link_columns
        .into_iter()
        .map(|column| column.name)
        .collect();
    assert_eq!(
        organization_party_link_columns,
        vec![
            "organization_id",
            "party_id",
            "relation_type",
            "created_at",
            "updated_at",
        ]
    );

    let party_columns: Vec<ColumnRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT name FROM pragma_table_info('parties') ORDER BY cid ASC",
            vec![],
        )
        .await
        .expect("parties columns");
    let party_columns: Vec<String> = party_columns
        .into_iter()
        .map(|column| column.name)
        .collect();
    assert_eq!(
        party_columns,
        vec![
            "id",
            "business_no",
            "legal_name",
            "active",
            "created_at",
            "updated_at",
        ]
    );

    let organization_columns: Vec<ColumnRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT name FROM pragma_table_info('organizations') ORDER BY cid ASC",
            vec![],
        )
        .await
        .expect("organizations columns");
    let organization_columns: Vec<String> = organization_columns
        .into_iter()
        .map(|column| column.name)
        .collect();
    assert!(organization_columns.contains(&"inactivated_at".to_owned()));
    assert!(!organization_columns.contains(&"active".to_owned()));

    let indexes: Vec<IndexRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT name FROM sqlite_master WHERE type = 'index' ORDER BY name ASC",
            vec![],
        )
        .await
        .expect("list indexes");
    let index_names: std::collections::BTreeSet<String> =
        indexes.into_iter().map(|index| index.name).collect();
    for index in [
        "idx_agreements_grantor_grantee",
        "idx_application_grants_active_unique",
        "idx_application_grants_user_application",
        "idx_organization_party_links_org_relation",
        "idx_parties_business_no",
        "idx_roles_global_code",
        "idx_user_identities_provider_external",
    ] {
        assert!(index_names.contains(index), "missing index {index}");
    }

    let application_indexes: Vec<IndexListRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT name, \"unique\" as is_unique FROM pragma_index_list('applications')",
            vec![],
        )
        .await
        .expect("applications indexes");
    let mut has_unique_slug_index = false;
    for index in application_indexes {
        if index.is_unique != 1 {
            continue;
        }
        let index_sql = format!(
            "SELECT name FROM pragma_index_info('{}') ORDER BY seqno ASC",
            index.name.replace('\'', "''")
        );
        let columns: Vec<IndexColumnRow> = runtime
            .query_rows(ReadConsistency::Local, index_sql, vec![])
            .await
            .expect("application index columns");
        if columns.len() == 1 && columns[0].name == "slug" {
            has_unique_slug_index = true;
            break;
        }
    }
    assert!(
        has_unique_slug_index,
        "applications.slug must have a unique index"
    );
}

#[tokio::test]
async fn ensure_schema_drops_removed_certd_tables() {
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

    let db = Database::new(
        runtime.clone(),
        format!("bootstrap-{}", Uuid::new_v4().simple()),
        "Bootstrap User".to_owned(),
    );
    db.ensure_schema_only().await.expect("ensure schema only");

    let rows: Vec<TableRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name ASC",
            vec![],
        )
        .await
        .expect("list tables");
    let names: std::collections::BTreeSet<String> = rows.into_iter().map(|row| row.name).collect();
    for removed_table in [
        "agreement_documents",
        "assets",
        "asset_profiles",
        "registries",
        "registry_accounts",
        "registry_projects",
        "data_sources",
        "generation_data",
        "rate_data",
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
    ] {
        assert!(
            !names.contains(removed_table),
            "removed table still exists {removed_table}"
        );
    }

    runtime
        .batch(
            r#"
            INSERT INTO actions (
                id, code, name, description, risk_level, visibility_granting,
                grantable_to_applications, consent_label, consent_description, created_at, updated_at
            ) VALUES
                ('plant.profile.read', 'plant.profile.read', 'Read Plant Profile', NULL, 'low', 1, 1, NULL, NULL, '2026-04-12T00:00:00Z', '2026-04-12T00:00:00Z'),
                ('asset.profile.read', 'asset.profile.read', 'Read Asset Profile', NULL, 'low', 1, 1, NULL, NULL, '2026-04-12T00:00:00Z', '2026-04-12T00:00:00Z'),
                ('organization.plant.create', 'organization.plant.create', 'Create Plant', NULL, 'medium', 0, 0, NULL, NULL, '2026-04-12T00:00:00Z', '2026-04-12T00:00:00Z'),
                ('organization.credential.use', 'organization.credential.use', 'Use Credential', NULL, 'critical', 0, 0, NULL, NULL, '2026-04-12T00:00:00Z', '2026-04-12T00:00:00Z'),
                ('organization.profile.read', 'organization.profile.read', 'Read Organization Profile', NULL, 'low', 1, 1, NULL, NULL, '2026-04-12T00:00:00Z', '2026-04-12T00:00:00Z');
            INSERT INTO role_actions (role_id, action_id)
            VALUES
                ('role-old-plant', 'plant.profile.read'),
                ('role-old-asset', 'asset.profile.read'),
                ('role-account', 'organization.profile.read');
            INSERT INTO role_bindings (
                id, subject_type, subject_id, subject_organization_id, resource_type, resource_id,
                role_id, agreement_id, state, created_by_user_id, created_at, updated_at
            ) VALUES (
                'binding-old-plant', 'organization', 'org-1', 'org-1', 'plant', 'plant-1',
                'role-old-plant', NULL, 'active', 'bootstrap-user', '2026-04-12T00:00:00Z', '2026-04-12T00:00:00Z'
            );
            INSERT INTO application_actions (application_id, action_id)
            VALUES
                ('application-old', 'plant.profile.read'),
                ('application-account', 'organization.profile.read');
            INSERT INTO application_grant_actions (application_grant_id, action_id)
            VALUES
                ('grant-old', 'asset.profile.read'),
                ('grant-account', 'organization.profile.read');
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
             SELECT resource_type AS name FROM role_bindings WHERE resource_type <> 'organization'",
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
}

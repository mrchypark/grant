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

    let client = hiqlite::start_node(cfg).await.expect("start hiqlite");
    (
        HiqliteRuntime::embedded(client, ReadConsistency::Local),
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
        "cert_gateway_challenges",
        "crawl_task_tokens",
        "user_identities",
        "parties",
        "organizations",
        "organization_party_links",
        "organization_memberships",
        "groups",
        "group_memberships",
        "agreements",
        "agreement_documents",
        "actions",
        "roles",
        "role_actions",
        "role_bindings",
        "clients",
        "client_actions",
        "client_grants",
        "client_grant_actions",
        "effective_organization_plant_actions",
        "effective_user_plant_actions",
        "effective_user_plant_visibility",
    ] {
        assert!(names.contains(table), "missing table {table}");
    }

    assert!(!names.contains("businesses"));
    assert!(!names.contains("business_memberships"));
    assert!(!names.contains("organization_edges"));
    assert!(!names.contains("organization_closure"));

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
            "grantable_to_clients",
            "consent_label",
            "consent_description",
            "created_at",
            "updated_at",
        ]
    );

    let client_columns: Vec<ColumnRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT name FROM pragma_table_info('clients') ORDER BY cid ASC",
            vec![],
        )
        .await
        .expect("clients columns");
    let client_columns: Vec<String> = client_columns
        .into_iter()
        .map(|column| column.name)
        .collect();
    assert_eq!(
        client_columns,
        vec![
            "id",
            "organization_id",
            "slug",
            "name",
            "description",
            "client_type",
            "app_type",
            "first_party",
            "consent_policy",
            "active",
            "created_by_user_id",
            "created_at",
            "updated_at",
        ]
    );

    let client_action_columns: Vec<ColumnRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT name FROM pragma_table_info('client_actions') ORDER BY cid ASC",
            vec![],
        )
        .await
        .expect("client_actions columns");
    let client_action_columns: Vec<String> = client_action_columns
        .into_iter()
        .map(|column| column.name)
        .collect();
    assert_eq!(client_action_columns, vec!["client_id", "action_id"]);

    let client_grant_columns: Vec<ColumnRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT name FROM pragma_table_info('client_grants') ORDER BY cid ASC",
            vec![],
        )
        .await
        .expect("client_grants columns");
    let client_grant_columns: Vec<String> = client_grant_columns
        .into_iter()
        .map(|column| column.name)
        .collect();
    assert_eq!(
        client_grant_columns,
        vec![
            "id",
            "user_id",
            "client_id",
            "organization_id",
            "status",
            "consented_at",
            "revoked_at",
            "revoked_by_user_id",
            "created_at",
            "updated_at",
        ]
    );

    let client_grant_action_columns: Vec<ColumnRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT name FROM pragma_table_info('client_grant_actions') ORDER BY cid ASC",
            vec![],
        )
        .await
        .expect("client_grant_actions columns");
    let client_grant_action_columns: Vec<String> = client_grant_action_columns
        .into_iter()
        .map(|column| column.name)
        .collect();
    assert_eq!(
        client_grant_action_columns,
        vec!["client_grant_id", "action_id"]
    );

    let plant_columns: Vec<ColumnRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT name FROM pragma_table_info('plants') ORDER BY cid ASC",
            vec![],
        )
        .await
        .expect("plants columns");
    let plant_columns: Vec<String> = plant_columns
        .into_iter()
        .map(|column| column.name)
        .collect();
    assert!(plant_columns.contains(&"owner_organization_id".to_owned()));
    assert!(plant_columns.contains(&"created_by_organization_id".to_owned()));
    assert!(plant_columns.contains(&"facility_code".to_owned()));

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

    let agreement_document_columns: Vec<ColumnRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT name FROM pragma_table_info('agreement_documents') ORDER BY cid ASC",
            vec![],
        )
        .await
        .expect("agreement_documents columns");
    let agreement_document_columns: Vec<String> = agreement_document_columns
        .into_iter()
        .map(|column| column.name)
        .collect();
    assert_eq!(
        agreement_document_columns,
        vec![
            "id",
            "agreement_id",
            "document_type",
            "file_name",
            "object_key",
            "content_type",
            "document_no",
            "uploaded_at",
        ]
    );
    let certificate_columns: Vec<ColumnRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT name FROM pragma_table_info('certificates') ORDER BY cid ASC",
            vec![],
        )
        .await
        .expect("certificates columns");
    let certificate_columns: Vec<String> = certificate_columns
        .into_iter()
        .map(|column| column.name)
        .collect();
    assert_eq!(
        certificate_columns,
        vec![
            "id",
            "party_id",
            "common_name",
            "encrypted_blob",
            "wrapped_dek",
            "kms_key_id",
            "active",
            "last_used_at",
            "created_at",
        ]
    );

    let challenge_columns: Vec<ColumnRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT name FROM pragma_table_info('cert_gateway_challenges') ORDER BY cid ASC",
            vec![],
        )
        .await
        .expect("cert_gateway_challenges columns");
    let challenge_columns: Vec<String> = challenge_columns
        .into_iter()
        .map(|column| column.name)
        .collect();
    assert!(challenge_columns.contains(&"expires_at".to_owned()));
    assert!(challenge_columns.contains(&"consumed_at".to_owned()));

    let crawl_token_columns: Vec<ColumnRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT name FROM pragma_table_info('crawl_task_tokens') ORDER BY cid ASC",
            vec![],
        )
        .await
        .expect("crawl_task_tokens columns");
    let crawl_token_columns: Vec<String> = crawl_token_columns
        .into_iter()
        .map(|column| column.name)
        .collect();
    assert!(crawl_token_columns.contains(&"token_hash".to_owned()));
    assert!(crawl_token_columns.contains(&"used_at".to_owned()));

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
        "idx_cert_gateway_challenges_org_expires",
        "idx_certificates_party_active",
        "idx_crawl_task_tokens_org_expires",
        "idx_agreement_documents_agreement",
        "idx_client_grants_active_unique",
        "idx_client_grants_user_client",
        "idx_organization_party_links_org_relation",
        "idx_parties_business_no",
        "idx_plants_owner_facility",
        "idx_roles_global_code",
        "idx_sync_runs_plant_started",
        "idx_user_identities_provider_external",
    ] {
        assert!(index_names.contains(index), "missing index {index}");
    }

    let client_indexes: Vec<IndexListRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT name, \"unique\" as is_unique FROM pragma_index_list('clients')",
            vec![],
        )
        .await
        .expect("clients indexes");
    let mut has_unique_slug_index = false;
    for index in client_indexes {
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
            .expect("client index columns");
        if columns.len() == 1 && columns[0].name == "slug" {
            has_unique_slug_index = true;
            break;
        }
    }
    assert!(
        has_unique_slug_index,
        "clients.slug must have a unique index"
    );
}

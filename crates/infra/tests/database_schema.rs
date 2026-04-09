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
        "organizations",
        "organization_memberships",
        "groups",
        "group_memberships",
        "agreements",
        "actions",
        "roles",
        "role_rules",
        "role_actions",
        "role_bindings",
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
    assert!(certificate_columns.contains(&"wrapped_dek".to_owned()));
    assert!(certificate_columns.contains(&"kms_key_id".to_owned()));

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
        "idx_certificates_organization_active",
        "idx_crawl_task_tokens_org_expires",
        "idx_plants_owner_facility",
        "idx_sync_runs_plant_started",
        "idx_user_identities_provider_external",
    ] {
        assert!(index_names.contains(index), "missing index {index}");
    }
}

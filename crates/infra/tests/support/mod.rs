#![allow(dead_code)]

use domain::{
    Agreement, AgreementStatus, AgreementType, Application, ApplicationAppType,
    ApplicationConsentPolicy, ApplicationGrant, ApplicationGrantStatus, ApplicationType,
    RoleBinding, RoleBindingState, RoleBindingSubjectType,
};
use hiqlite::{Node, NodeConfig, Row, s3::EncKeys};
use infra::{config::ReadConsistency, database::Database, runtime::HiqliteRuntime};
use std::collections::BTreeSet;
use tempfile::TempDir;
use uuid::Uuid;

pub const BOOTSTRAP_USER_ID: &str = "bootstrap-user";
pub const FIXED_TEST_TIME: &str = "2026-04-12T00:00:00Z";

#[derive(Debug)]
pub struct NameRow {
    pub name: String,
}

impl From<&mut Row<'_>> for NameRow {
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

fn free_local_addr() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    drop(listener);
    addr.to_string()
}

pub async fn test_runtime() -> (HiqliteRuntime, TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let api_addr = free_local_addr();
    let raft_addr = free_local_addr();
    let cfg = NodeConfig {
        node_id: 1,
        nodes: vec![Node {
            id: 1,
            addr_raft: raft_addr,
            addr_api: api_addr,
        }],
        listen_addr_api: "127.0.0.1".into(),
        listen_addr_raft: "127.0.0.1".into(),
        data_dir: dir.path().to_string_lossy().into_owned().into(),
        filename_db: "test.sqlite".into(),
        secret_api: "0123456789abcdef0123456789abcdef".to_owned(),
        secret_raft: "fedcba9876543210fedcba9876543210".to_owned(),
        enc_keys: EncKeys::generate().expect("enc keys"),
        health_check_delay_secs: 0,
        raft_config: NodeConfig::default_raft_config(128),
        ..Default::default()
    };

    let application = hiqlite::start_node(cfg).await.expect("start hiqlite");
    (
        HiqliteRuntime::embedded(application, ReadConsistency::Local),
        dir,
    )
}

pub fn test_database(runtime: HiqliteRuntime) -> Database {
    Database::new(
        runtime,
        format!("bootstrap-{}", Uuid::new_v4().simple()),
        "Bootstrap User".to_owned(),
    )
}

pub async fn ensured_database() -> (HiqliteRuntime, Database, TempDir) {
    let (runtime, dir) = test_runtime().await;
    let db = test_database(runtime.clone());
    db.ensure_schema().await.expect("ensure schema");
    (runtime, db, dir)
}

pub async fn table_names(runtime: &HiqliteRuntime) -> BTreeSet<String> {
    let rows: Vec<NameRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name ASC",
            vec![],
        )
        .await
        .expect("list tables");
    rows.into_iter().map(|row| row.name).collect()
}

pub async fn table_columns(runtime: &HiqliteRuntime, table: &str) -> Vec<String> {
    let rows: Vec<NameRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            format!("SELECT name FROM pragma_table_info('{table}') ORDER BY cid ASC"),
            vec![],
        )
        .await
        .expect("table columns");
    rows.into_iter().map(|row| row.name).collect()
}

pub async fn index_names(runtime: &HiqliteRuntime) -> BTreeSet<String> {
    let rows: Vec<NameRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT name FROM sqlite_master WHERE type = 'index' ORDER BY name ASC",
            vec![],
        )
        .await
        .expect("list indexes");
    rows.into_iter().map(|row| row.name).collect()
}

pub async fn assert_table_columns(runtime: &HiqliteRuntime, table: &str, expected: &[&str]) {
    let actual = table_columns(runtime, table).await;
    assert_eq!(actual, expected, "{table} columns");
}

pub async fn assert_indexes(runtime: &HiqliteRuntime, expected: &[&str]) {
    let actual = index_names(runtime).await;
    for index in expected {
        assert!(actual.contains(*index), "missing index {index}");
    }
}

pub async fn has_unique_index_on_columns(
    runtime: &HiqliteRuntime,
    table: &str,
    expected_columns: &[&str],
) -> bool {
    let index_rows: Vec<IndexListRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            format!("SELECT name, \"unique\" as is_unique FROM pragma_index_list('{table}')"),
            vec![],
        )
        .await
        .expect("index list");
    for index in index_rows {
        if index.is_unique != 1 {
            continue;
        }
        let index_sql = format!(
            "SELECT name FROM pragma_index_info('{}') ORDER BY seqno ASC",
            index.name.replace('\'', "''")
        );
        let columns: Vec<NameRow> = runtime
            .query_rows(ReadConsistency::Local, index_sql, vec![])
            .await
            .expect("index columns");
        let columns: Vec<String> = columns.into_iter().map(|column| column.name).collect();
        if columns == expected_columns {
            return true;
        }
    }
    false
}

pub fn test_application(id: &str, organization_id: &str) -> Application {
    Application {
        id: id.to_owned(),
        organization_id: organization_id.to_owned(),
        slug: id.trim_start_matches("application-").to_owned(),
        name: id.to_owned(),
        description: None,
        application_type: ApplicationType::Confidential,
        app_type: ApplicationAppType::Web,
        first_party: false,
        consent_policy: ApplicationConsentPolicy::Explicit,
        active: true,
        rauthy_client_id: None,
        provisioning_status: domain::ApplicationProvisioningStatus::Ready,
        created_by_user_id: BOOTSTRAP_USER_ID.to_owned(),
    }
}

pub fn test_application_grant(
    id: &str,
    user_id: &str,
    application_id: &str,
    organization_id: &str,
) -> ApplicationGrant {
    ApplicationGrant {
        id: id.to_owned(),
        user_id: user_id.to_owned(),
        application_id: application_id.to_owned(),
        organization_id: organization_id.to_owned(),
        status: ApplicationGrantStatus::Active,
        consented_at: FIXED_TEST_TIME.to_owned(),
        revoked_at: None,
        revoked_by_user_id: None,
        created_at: FIXED_TEST_TIME.to_owned(),
        updated_at: FIXED_TEST_TIME.to_owned(),
    }
}

pub fn organization_role_binding(
    id: &str,
    subject_organization_id: &str,
    resource_organization_id: &str,
    role_id: &str,
    agreement_id: Option<&str>,
) -> RoleBinding {
    RoleBinding {
        id: id.to_owned(),
        subject_type: RoleBindingSubjectType::Organization,
        subject_id: subject_organization_id.to_owned(),
        subject_organization_id: subject_organization_id.to_owned(),
        provider_id: "grant".to_owned(),
        resource_type: "organization".to_owned(),
        resource_id: resource_organization_id.to_owned(),
        role_id: role_id.to_owned(),
        agreement_id: agreement_id.map(str::to_owned),
        state: RoleBindingState::Active,
        created_by_user_id: BOOTSTRAP_USER_ID.to_owned(),
        created_at: FIXED_TEST_TIME.to_owned(),
        updated_at: FIXED_TEST_TIME.to_owned(),
    }
}

pub fn test_agreement(
    id: &str,
    grantor_organization_id: &str,
    grantee_organization_id: &str,
) -> Agreement {
    Agreement {
        id: id.to_owned(),
        agreement_type: AgreementType::Delegation,
        grantor_organization_id: grantor_organization_id.to_owned(),
        grantee_organization_id: grantee_organization_id.to_owned(),
        status: AgreementStatus::Active,
        title: id.to_owned(),
        reference_no: None,
        starts_at: Some("2026-01-01T00:00:00Z".to_owned()),
        ends_at: None,
    }
}

use base64::{Engine, engine::general_purpose::STANDARD};
use hiqlite::{Node, NodeConfig, Row, s3::EncKeys};
use infra::{config::ReadConsistency, database::Database, runtime::HiqliteRuntime};
use tempfile::TempDir;
use tokio::task::JoinSet;

#[derive(Debug)]
struct OrganizationRow {
    id: String,
    business_no: Option<String>,
    name: String,
}

impl From<&mut Row<'_>> for OrganizationRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            id: row.get("id"),
            business_no: row.get("business_no"),
            name: row.get("name"),
        }
    }
}

#[derive(Debug)]
struct MembershipRow {
    organization_id: String,
    user_id: String,
    membership_role: String,
    status: String,
}

impl From<&mut Row<'_>> for MembershipRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            organization_id: row.get("organization_id"),
            user_id: row.get("user_id"),
            membership_role: row.get("membership_role"),
            status: row.get("status"),
        }
    }
}

#[derive(Debug)]
struct ActionRow {
    code: String,
    visibility_granting: i64,
}

impl From<&mut Row<'_>> for ActionRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            code: row.get("code"),
            visibility_granting: row.get("visibility_granting"),
        }
    }
}

#[derive(Debug)]
struct RoleRow {
    id: String,
    code: String,
}

impl From<&mut Row<'_>> for RoleRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            id: row.get("id"),
            code: row.get("code"),
        }
    }
}

#[derive(Debug)]
struct RoleActionRow {
    role_id: String,
    action_id: String,
}

impl From<&mut Row<'_>> for RoleActionRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            role_id: row.get("role_id"),
            action_id: row.get("action_id"),
        }
    }
}

#[derive(Debug)]
struct RoleBindingRow {
    subject_type: String,
    subject_id: String,
    subject_organization_id: String,
    resource_type: String,
    resource_id: String,
    role_id: String,
    state: String,
}

impl From<&mut Row<'_>> for RoleBindingRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            subject_type: row.get("subject_type"),
            subject_id: row.get("subject_id"),
            subject_organization_id: row.get("subject_organization_id"),
            resource_type: row.get("resource_type"),
            resource_id: row.get("resource_id"),
            role_id: row.get("role_id"),
            state: row.get("state"),
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
async fn ensure_schema_seeds_bootstrap_organization_and_actions() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "conalog-account".to_owned(),
        "Conalog Account".to_owned(),
    );

    db.ensure_schema().await.expect("ensure schema");

    let organizations: Vec<OrganizationRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT id, business_no, name FROM organizations ORDER BY id ASC",
            vec![],
        )
        .await
        .expect("organizations");
    assert_eq!(organizations.len(), 1);
    assert_eq!(organizations[0].id, "conalog-account");
    assert_eq!(
        organizations[0].business_no.as_deref(),
        Some("conalog-account")
    );
    assert_eq!(organizations[0].name, "Conalog Account");

    let memberships: Vec<MembershipRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT organization_id, user_id, membership_role, status FROM organization_memberships ORDER BY organization_id ASC",
            vec![],
        )
        .await
        .expect("memberships");
    assert_eq!(memberships.len(), 1);
    assert_eq!(memberships[0].organization_id, "conalog-account");
    assert_eq!(memberships[0].user_id, "conalog-account");
    assert_eq!(memberships[0].membership_role, "owner");
    assert_eq!(memberships[0].status, "active");

    let actions: Vec<ActionRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT code, visibility_granting FROM actions ORDER BY code ASC",
            vec![],
        )
        .await
        .expect("actions");
    assert!(!actions.is_empty(), "action catalog must be seeded");
    let profile_read = actions
        .iter()
        .find(|action| action.code == "plant.profile.read")
        .expect("plant.profile.read action");
    assert_eq!(profile_read.visibility_granting, 1);
    assert!(
        actions
            .iter()
            .any(|action| action.code == "organization.plant.create"),
        "organization.plant.create action must exist"
    );

    let roles: Vec<RoleRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT id, code FROM roles ORDER BY code ASC",
            vec![],
        )
        .await
        .expect("roles");
    assert!(
        roles
            .iter()
            .any(|role| role.id == "role-system-bootstrap-owner"
                && role.code == "system.bootstrap.owner"),
        "bootstrap system role must be seeded"
    );

    let role_actions: Vec<RoleActionRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT role_id, action_id FROM role_actions WHERE role_id = 'role-system-bootstrap-owner' ORDER BY action_id ASC",
            vec![],
        )
        .await
        .expect("role actions");
    let seeded_actions: Vec<String> = role_actions
        .into_iter()
        .map(|row| {
            assert_eq!(row.role_id, "role-system-bootstrap-owner");
            row.action_id
        })
        .collect();
    assert!(seeded_actions.contains(&"organization.plant.create".to_owned()));

    let bindings: Vec<RoleBindingRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT subject_type, subject_id, subject_organization_id, resource_type, resource_id, role_id, state
             FROM role_bindings
             WHERE id = 'binding-system-bootstrap-owner'",
            vec![],
        )
        .await
        .expect("role bindings");
    assert_eq!(bindings.len(), 1);
    let binding = &bindings[0];
    assert_eq!(binding.subject_type, "user");
    assert_eq!(binding.subject_id, "conalog-account");
    assert_eq!(binding.subject_organization_id, "conalog-account");
    assert_eq!(binding.resource_type, "organization");
    assert_eq!(binding.resource_id, "conalog-account");
    assert_eq!(binding.role_id, "role-system-bootstrap-owner");
    assert_eq!(binding.state, "active");
}

#[tokio::test]
async fn ensure_schema_preserves_bootstrap_organization_deactivation() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "conalog-account".to_owned(),
        "Conalog Account".to_owned(),
    );

    db.ensure_schema().await.expect("ensure schema");
    let deactivated = db
        .deactivate_organization("conalog-account")
        .await
        .expect("deactivate bootstrap organization");
    assert!(deactivated.inactivated_at.is_some());

    db.ensure_schema().await.expect("ensure schema again");
    let bootstrap = db
        .get_organization("conalog-account")
        .await
        .expect("bootstrap organization");
    assert!(
        bootstrap.inactivated_at.is_some(),
        "bootstrap organization should stay deactivated across ensure_schema"
    );
}

#[tokio::test]
async fn organization_upserts_preserve_existing_deactivation_state() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "conalog-account".to_owned(),
        "Conalog Account".to_owned(),
    );

    db.ensure_schema().await.expect("ensure schema");
    db.deactivate_organization("conalog-account")
        .await
        .expect("deactivate bootstrap organization");

    let owner_upserted = db
        .upsert_organization_with_owner("conalog-account", "Renamed Org", "ignored")
        .await
        .expect("upsert organization with owner");
    assert!(
        owner_upserted.inactivated_at.is_some(),
        "owner upsert should not reactivate an inactive organization"
    );

    let identity_upserted = db
        .upsert_organization_identity("conalog-account", Some("conalog-account"), "Renamed Again")
        .await
        .expect("upsert organization identity");
    assert!(
        identity_upserted.inactivated_at.is_some(),
        "identity upsert should not reactivate an inactive organization"
    );
}

#[tokio::test]
async fn certificate_gateway_rows_round_trip() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "conalog-account".to_owned(),
        "Conalog Account".to_owned(),
    );

    db.ensure_schema().await.expect("ensure schema");

    let cert = db
        .save_certificate(
            "conalog-account",
            Some("2768603291"),
            "Test Certificate",
            "encrypted-blob",
            b"wrapped-dek",
            "kms-key-v1",
        )
        .await
        .expect("save certificate");
    assert_eq!(cert.kms_key_id, "kms-key-v1");
    assert_eq!(cert.wrapped_dek, b"wrapped-dek");
    let listed = db
        .list_certificates("conalog-account")
        .await
        .expect("list certificates for active organization");
    assert_eq!(listed.len(), 1);

    let first_load = db
        .load_active_certificate("conalog-account")
        .await
        .expect("first active certificate load");
    db.touch_certificate_last_used_at_if_stale(&first_load)
        .await
        .expect("touch certificate last_used_at");
    let first_loaded = db
        .get_certificate_by_id(&first_load.id)
        .await
        .expect("first reloaded certificate");
    let first_last_used_at = first_loaded
        .last_used_at
        .clone()
        .expect("first load should stamp last_used_at");

    let second_load = db
        .load_active_certificate("conalog-account")
        .await
        .expect("second active certificate load");
    db.touch_certificate_last_used_at_if_stale(&second_load)
        .await
        .expect("skip redundant touch");
    let second_loaded = db
        .get_certificate_by_id(&second_load.id)
        .await
        .expect("second reloaded certificate");
    assert_eq!(
        second_loaded.last_used_at.as_deref(),
        Some(first_last_used_at.as_str()),
        "recent repeated loads should not rewrite last_used_at immediately"
    );

    db.deactivate_organization("conalog-account")
        .await
        .expect("deactivate organization");
    let listed = db
        .list_certificates("conalog-account")
        .await
        .expect("list certificates for inactive organization");
    assert!(
        listed.is_empty(),
        "inactive organization should not list stored certificates"
    );
    let err = db
        .load_active_certificate("conalog-account")
        .await
        .expect_err("inactive organization should not load active certificate");
    assert!(
        err.to_string().contains("certificate not found"),
        "unexpected error: {err:#}"
    );

    runtime
        .execute(
            "INSERT INTO certificates (id, organization_id, common_name, encrypted_blob, wrapped_dek, kms_key_id, active, last_used_at, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            vec![
                "cert-recent-created".into(),
                "conalog-account".into(),
                "Recent Created".into(),
                "blob-created".into(),
                STANDARD.encode("wrapped-created").into(),
                "kms-key-v1".into(),
                1_i64.into(),
                Option::<String>::None.into(),
                "2026-04-14T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert recent-created certificate");
    runtime
        .execute(
            "INSERT INTO certificates (id, organization_id, common_name, encrypted_blob, wrapped_dek, kms_key_id, active, last_used_at, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            vec![
                "cert-recently-used".into(),
                "conalog-account".into(),
                "Recently Used".into(),
                "blob-used".into(),
                STANDARD.encode("wrapped-used").into(),
                "kms-key-v1".into(),
                1_i64.into(),
                "2026-04-15T00:00:00Z".into(),
                "2026-04-13T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert recently-used certificate");

    db.reactivate_organization("conalog-account")
        .await
        .expect("reactivate organization for ordering check");
    let preferred = db
        .load_active_certificate("conalog-account")
        .await
        .expect("load preferred active certificate");
    assert_eq!(
        preferred.id, "cert-recently-used",
        "last_used_at should take precedence over created_at for active certificate selection"
    );
    let listed = db
        .list_certificates("conalog-account")
        .await
        .expect("list certificates ordered by usage");
    assert_eq!(
        listed.first().map(|certificate| certificate.id.as_str()),
        Some("cert-recently-used"),
        "certificate listing should prefer the most recently used active certificate first"
    );

    let challenge = db
        .create_cert_gateway_challenge(
            "conalog-account",
            "ephemeral",
            "nonce-1",
            "payload-hash-1",
            Some("FAC-001"),
            "2099-01-01T00:00:00Z",
            "conalog-account",
        )
        .await
        .expect("create challenge");
    assert_eq!(challenge.mode, "ephemeral");
    assert_eq!(challenge.consumed_at, None);

    let consumed = db
        .consume_cert_gateway_challenge(&challenge.id)
        .await
        .expect("consume challenge");
    assert!(consumed.consumed_at.is_some());

    let token = db
        .issue_crawl_task_token(
            "conalog-account",
            Some(&cert.id),
            "token-hash-1",
            "{\"facilityCode\":\"FAC-001\"}",
            "2099-01-01T00:01:00Z",
            "conalog-account",
        )
        .await
        .expect("issue crawl task token");
    assert_eq!(token.used_at, None);

    let consumed_token = db
        .consume_crawl_task_token("token-hash-1")
        .await
        .expect("consume crawl task token");
    assert!(consumed_token.used_at.is_some());
    assert_eq!(
        consumed_token.certificate_id.as_deref(),
        Some(cert.id.as_str())
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cert_gateway_challenge_consumption_allows_only_one_concurrent_winner() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "conalog-account".to_owned(),
        "Conalog Account".to_owned(),
    );
    db.ensure_schema().await.expect("ensure schema");

    let challenge = db
        .create_cert_gateway_challenge(
            "conalog-account",
            "ephemeral",
            "nonce-1",
            "payload-hash-1",
            Some("FAC-001"),
            "2099-01-01T00:00:00Z",
            "conalog-account",
        )
        .await
        .expect("create challenge");

    let mut tasks = JoinSet::new();
    for _ in 0..16 {
        let db = db.clone();
        let challenge_id = challenge.id.clone();
        tasks.spawn(async move { db.consume_cert_gateway_challenge(&challenge_id).await });
    }

    let mut success_count = 0usize;
    while let Some(result) = tasks.join_next().await {
        match result.expect("task join") {
            Ok(_) => success_count += 1,
            Err(err) => assert!(
                err.to_string().contains("already consumed"),
                "unexpected error: {err:#}"
            ),
        }
    }

    assert_eq!(
        success_count, 1,
        "challenge should be consumable exactly once"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn crawl_task_token_consumption_allows_only_one_concurrent_winner() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "conalog-account".to_owned(),
        "Conalog Account".to_owned(),
    );
    db.ensure_schema().await.expect("ensure schema");

    let cert = db
        .save_certificate(
            "conalog-account",
            Some("2768603291"),
            "Test Certificate",
            "encrypted-blob",
            b"wrapped-dek",
            "kms-key-v1",
        )
        .await
        .expect("save certificate");

    db.issue_crawl_task_token(
        "conalog-account",
        Some(&cert.id),
        "token-hash-1",
        "{\"facilityCode\":\"FAC-001\"}",
        "2099-01-01T00:01:00Z",
        "conalog-account",
    )
    .await
    .expect("issue crawl task token");

    let mut tasks = JoinSet::new();
    for _ in 0..16 {
        let db = db.clone();
        tasks.spawn(async move { db.consume_crawl_task_token("token-hash-1").await });
    }

    let mut success_count = 0usize;
    while let Some(result) = tasks.join_next().await {
        match result.expect("task join") {
            Ok(_) => success_count += 1,
            Err(err) => assert!(
                err.to_string().contains("already used"),
                "unexpected error: {err:#}"
            ),
        }
    }

    assert_eq!(
        success_count, 1,
        "crawl task token should be consumable exactly once"
    );
}

#[tokio::test]
async fn get_certificate_by_id_returns_typed_not_found() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime,
        "conalog-account".to_owned(),
        "Conalog Account".to_owned(),
    );
    db.ensure_schema().await.expect("ensure schema");

    let err = db
        .get_certificate_by_id("missing-certificate")
        .await
        .expect_err("missing certificate should fail");
    assert!(matches!(
        err.downcast_ref::<infra::database::DatabaseLookupError>(),
        Some(infra::database::DatabaseLookupError::CertificateNotFound)
    ));
}

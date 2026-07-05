#[path = "support/auth.rs"]
mod auth_support;
mod support;

use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use hiqlite::{Node, NodeConfig, s3::EncKeys};
use http_body_util::BodyExt;
use infra::{
    artifacts::ArtifactStore,
    config::{
        AppConfig, ArtifactBackendConfig, ArtifactConfig, HiqliteConfig, LocalNodeRole,
        ReadConsistency, RuntimeMode, SecurityConfig, ServerConfig,
    },
    database::Database,
    runtime::HiqliteRuntime,
    service::AppService,
};
use serde_json::Value;
use tempfile::TempDir;
use tower::ServiceExt;

#[derive(Debug, Clone)]
struct CountRow {
    count: i64,
}

impl From<&mut hiqlite::Row<'_>> for CountRow {
    fn from(row: &mut hiqlite::Row<'_>) -> Self {
        Self {
            count: row.get("count"),
        }
    }
}

#[derive(Debug, Clone)]
struct UserIdentityMetaRow {
    updated_at: String,
    last_seen_at: Option<String>,
}

impl From<&mut hiqlite::Row<'_>> for UserIdentityMetaRow {
    fn from(row: &mut hiqlite::Row<'_>) -> Self {
        Self {
            updated_at: row.get("updated_at"),
            last_seen_at: row.get("last_seen_at"),
        }
    }
}

fn free_local_addr() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    drop(listener);
    addr.to_string()
}

async fn test_runtime(default_read_consistency: ReadConsistency) -> (HiqliteRuntime, TempDir) {
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
        HiqliteRuntime::embedded(client, default_read_consistency),
        dir,
    )
}

async fn build_service() -> (Arc<AppService>, Database, HiqliteRuntime, TempDir, TempDir) {
    build_service_with_default_consistency(ReadConsistency::Local).await
}

async fn build_service_with_default_consistency(
    default_read_consistency: ReadConsistency,
) -> (Arc<AppService>, Database, HiqliteRuntime, TempDir, TempDir) {
    let trailbase_base_url = support::start_mock_trailbase().await;
    let (runtime, db_dir) = test_runtime(default_read_consistency.clone()).await;
    let artifacts_dir = tempfile::tempdir().expect("artifact tempdir");
    let cfg = AppConfig {
        server: ServerConfig {
            bind_addr: "127.0.0.1:0".to_owned(),
        },
        security: SecurityConfig {
            master_key: "dev-only-master-key".to_owned(),
            bootstrap_user_id: "bootstrap-user".to_owned(),
            bootstrap_user_name: "Bootstrap User".to_owned(),
        },
        auth: support::trailbase_auth_config(trailbase_base_url),
        kms: support::test_kms_config(),
        artifacts: ArtifactConfig {
            backend: ArtifactBackendConfig::FileSystem {
                root_dir: artifacts_dir.path().to_path_buf(),
            },
        },
        hiqlite: HiqliteConfig {
            runtime_mode: RuntimeMode::Embedded,
            local_node_role: LocalNodeRole::Voter,
            default_read_consistency,
            remote_nodes: Vec::new(),
            remote_tls: false,
            remote_tls_no_verify: false,
            remote_api_secret: None,
            remote_with_proxy: false,
        },
    };
    let db = Database::new(
        runtime.clone(),
        cfg.security.bootstrap_user_id.clone(),
        cfg.security.bootstrap_user_name.clone(),
    );
    let artifacts = ArtifactStore::new(&cfg.artifacts)
        .await
        .expect("artifact store");
    let service = Arc::new(AppService::new(cfg, db.clone(), artifacts));
    service.initialize().await.expect("initialize service");
    (service, db, runtime, db_dir, artifacts_dir)
}

async fn query_count(runtime: &HiqliteRuntime, sql: &str, params: Vec<hiqlite::Param>) -> i64 {
    runtime
        .query_rows::<CountRow, _>(ReadConsistency::Strong, sql.to_owned(), params)
        .await
        .expect("query count")
        .into_iter()
        .next()
        .expect("count row")
        .count
}

async fn query_identity_meta(
    runtime: &HiqliteRuntime,
    external_user_id: &str,
) -> UserIdentityMetaRow {
    runtime
        .query_rows::<UserIdentityMetaRow, _>(
            ReadConsistency::Strong,
            "SELECT updated_at, last_seen_at
             FROM user_identities
             WHERE provider = $1 AND external_user_id = $2"
                .to_owned(),
            vec!["trailbase".into(), external_user_id.into()],
        )
        .await
        .expect("query identity meta")
        .into_iter()
        .next()
        .expect("identity meta row")
}

#[tokio::test]
async fn current_user_returns_local_user_and_active_organizations() {
    let (service, db, runtime, _db_dir, _artifacts_dir) = build_service().await;
    db.upsert_organization_with_owner("alpha-org", "Alpha Org", None, "Alpha Org")
        .await
        .expect("upsert alpha org");
    db.upsert_organization_with_owner("beta-org", "Beta Org", None, "Beta Org")
        .await
        .expect("upsert beta org");
    let user = db
        .resolve_or_create_user_identity(
            "trailbase",
            "me-user",
            Some("me-user@trailbase.test"),
            "Me User",
            true,
        )
        .await
        .expect("create shadow user");
    for (membership_id, organization_id) in [
        ("membership-alpha", "alpha-org"),
        ("membership-beta", "beta-org"),
    ] {
        runtime.execute(
            "INSERT INTO organization_memberships (id, organization_id, user_id, membership_role, status, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            vec![
                membership_id.into(),
                organization_id.into(),
                user.id.clone().into(),
                "member".into(),
                "active".into(),
                "2026-04-03T00:00:00Z".into(),
                "2026-04-03T00:00:00Z".into(),
            ],
        ).await.expect("membership");
    }
    let app = api::build_router(service);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/me")
                .header("authorization", auth_support::bearer_header("me-user"))
                .header("x-organization-id", "alpha-org")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let json: Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(json["provider"], "trailbase");
    assert_eq!(json["externalUserId"], "me-user");
    assert_eq!(json["user"]["id"], Value::String(user.id.clone()));
    assert_eq!(json["activeOrganizationId"], "alpha-org");
    let organizations = json["organizations"].as_array().expect("organizations");
    assert!(organizations.iter().any(|org| org["id"] == "alpha-org"));
    assert!(organizations.iter().any(|org| org["id"] == "beta-org"));
}

#[tokio::test]
async fn trailbase_identity_relinks_same_email_after_restore() {
    let (service, db, runtime, _db_dir, _artifacts_dir) = build_service().await;
    db.upsert_organization_with_owner("restore-org", "Restore Org", None, "Restore Org")
        .await
        .expect("upsert restore org");

    let original_user = db
        .resolve_or_create_user_identity(
            "trailbase",
            "original-trailbase-user",
            Some("restore-user@trailbase.test"),
            "Restore User",
            true,
        )
        .await
        .expect("create original shadow user");

    runtime.execute(
        "INSERT INTO organization_memberships (id, organization_id, user_id, membership_role, status, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
        vec![
            "membership-restore".into(),
            "restore-org".into(),
            original_user.id.clone().into(),
            "member".into(),
            "active".into(),
            "2026-04-03T00:00:00Z".into(),
            "2026-04-03T00:00:00Z".into(),
        ],
    )
    .await
    .expect("restore membership");

    let relinked_user = db
        .resolve_or_create_user_identity(
            "trailbase",
            "restored-trailbase-user",
            Some("restore-user@trailbase.test"),
            "Restore User",
            true,
        )
        .await
        .expect("relink restored shadow user");

    assert_eq!(relinked_user.id, original_user.id);

    let app = api::build_router(service);
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/me")
                .header(
                    "authorization",
                    auth_support::bearer_header("restored-trailbase-user"),
                )
                .header("x-organization-id", "restore-org")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let json: Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(json["user"]["id"], Value::String(original_user.id));
    assert_eq!(json["activeOrganizationId"], "restore-org");
    let organizations = json["organizations"].as_array().expect("organizations");
    assert!(organizations.iter().any(|org| org["id"] == "restore-org"));
}

#[tokio::test]
async fn learner_router_rejects_mutation_and_admin_endpoints() {
    let (service, _db, _runtime, _db_dir, _artifacts_dir) = build_service().await;
    let app = api::build_learner_router(service);

    for (method, uri) in [
        ("POST", "/api/v1/organizations/bootstrap-user/applications"),
        ("POST", "/api/v1/admin/bootstrap/trailbase-owner"),
        ("GET", "/api/v1/admin/backups/s3"),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        let json: Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(json["error"], "endpoint requires voter node");
    }
}

#[tokio::test]
async fn learner_local_current_user_rejects_unlinked_identity_without_writes() {
    let (service, _db, runtime, _db_dir, _artifacts_dir) = build_service().await;
    runtime
        .execute(
            "INSERT INTO organizations (id, business_no, name, inactivated_at, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6)",
            vec![
                "local-org".into(),
                Option::<String>::None.into(),
                "Local Org".into(),
                Option::<String>::None.into(),
                "2026-04-06T00:00:00Z".into(),
                "2026-04-06T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert organization");
    runtime
        .execute(
            "INSERT INTO users (id, email, display_name, active, created_at)
             VALUES ($1, $2, $3, $4, $5)",
            vec![
                "local-user".into(),
                "local-user@trailbase.test".into(),
                "local-user".into(),
                1_i64.into(),
                "2026-04-06T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert user");
    runtime
        .execute(
            "INSERT INTO organization_memberships (id, organization_id, user_id, membership_role, status, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            vec![
                "membership-local-user".into(),
                "local-org".into(),
                "local-user".into(),
                "member".into(),
                "active".into(),
                "2026-04-06T00:00:00Z".into(),
                "2026-04-06T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert membership");

    let app = api::build_learner_router(service);
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/me")
                .header("authorization", auth_support::bearer_header("local-user"))
                .header("x-organization-id", "local-org")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        query_count(
            &runtime,
            "SELECT COUNT(*) as count
             FROM user_identities
             WHERE provider = $1 AND external_user_id = $2",
            vec!["trailbase".into(), "local-user".into()],
        )
        .await,
        0
    );
    assert_eq!(
        query_count(
            &runtime,
            "SELECT COUNT(*) as count
             FROM auth_identity_cache",
            Vec::new(),
        )
        .await,
        0
    );
}

#[tokio::test]
async fn learner_local_current_user_uses_existing_identity_without_mutation() {
    let (service, _db, runtime, _db_dir, _artifacts_dir) = build_service().await;
    runtime
        .execute(
            "INSERT INTO organizations (id, business_no, name, inactivated_at, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6)",
            vec![
                "linked-org".into(),
                Option::<String>::None.into(),
                "Linked Org".into(),
                Option::<String>::None.into(),
                "2026-04-06T00:00:00Z".into(),
                "2026-04-06T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert organization");
    runtime
        .execute(
            "INSERT INTO users (id, email, display_name, active, created_at)
             VALUES ($1, $2, $3, $4, $5)",
            vec![
                "linked-user".into(),
                "linked-user@trailbase.test".into(),
                "linked-user".into(),
                1_i64.into(),
                "2026-04-06T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert user");
    runtime
        .execute(
            "INSERT INTO user_identities (
                id, user_id, provider, external_user_id, email_snapshot, display_name_snapshot, last_seen_at, created_at, updated_at
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            vec![
                "identity-linked-user".into(),
                "linked-user".into(),
                "trailbase".into(),
                "linked-user".into(),
                "linked-user@trailbase.test".into(),
                "linked-user".into(),
                Option::<String>::Some("2026-04-05T00:00:00Z".to_owned()).into(),
                "2026-04-05T00:00:00Z".into(),
                "2026-04-05T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert user identity");
    runtime
        .execute(
            "INSERT INTO organization_memberships (id, organization_id, user_id, membership_role, status, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            vec![
                "membership-linked-user".into(),
                "linked-org".into(),
                "linked-user".into(),
                "member".into(),
                "active".into(),
                "2026-04-06T00:00:00Z".into(),
                "2026-04-06T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert membership");

    let before = query_identity_meta(&runtime, "linked-user").await;
    let app = api::build_learner_router(service);
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/me")
                .header("authorization", auth_support::bearer_header("linked-user"))
                .header("x-organization-id", "linked-org")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let after = query_identity_meta(&runtime, "linked-user").await;
    assert_eq!(after.updated_at, before.updated_at);
    assert_eq!(after.last_seen_at, before.last_seen_at);
    assert_eq!(
        query_count(
            &runtime,
            "SELECT COUNT(*) as count
             FROM auth_identity_cache",
            Vec::new(),
        )
        .await,
        0
    );
}

#[tokio::test]
async fn learner_strong_current_user_persists_identity_and_shared_cache() {
    let (service, _db, runtime, _db_dir, _artifacts_dir) = build_service().await;
    runtime
        .execute(
            "INSERT INTO organizations (id, business_no, name, inactivated_at, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6)",
            vec![
                "strong-org".into(),
                Option::<String>::None.into(),
                "Strong Org".into(),
                Option::<String>::None.into(),
                "2026-04-06T00:00:00Z".into(),
                "2026-04-06T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert organization");
    runtime
        .execute(
            "INSERT INTO users (id, email, display_name, active, created_at)
             VALUES ($1, $2, $3, $4, $5)",
            vec![
                "strong-user".into(),
                "strong-user@trailbase.test".into(),
                "strong-user".into(),
                1_i64.into(),
                "2026-04-06T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert user");
    runtime
        .execute(
            "INSERT INTO organization_memberships (id, organization_id, user_id, membership_role, status, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            vec![
                "membership-strong-user".into(),
                "strong-org".into(),
                "strong-user".into(),
                "member".into(),
                "active".into(),
                "2026-04-06T00:00:00Z".into(),
                "2026-04-06T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert membership");

    let app = api::build_learner_router(service);
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/me?consistency=strong")
                .header("authorization", auth_support::bearer_header("strong-user"))
                .header("x-organization-id", "strong-org")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        query_count(
            &runtime,
            "SELECT COUNT(*) as count
             FROM user_identities
             WHERE provider = $1 AND external_user_id = $2",
            vec!["trailbase".into(), "strong-user".into()],
        )
        .await,
        1
    );
    assert_eq!(
        query_count(
            &runtime,
            "SELECT COUNT(*) as count
             FROM auth_identity_cache",
            Vec::new(),
        )
        .await,
        1
    );
}

#[tokio::test]
async fn voter_default_strong_current_user_syncs_identity_without_query_override() {
    let (service, _db, runtime, _db_dir, _artifacts_dir) =
        build_service_with_default_consistency(ReadConsistency::Strong).await;
    runtime
        .execute(
            "INSERT INTO organizations (id, business_no, name, inactivated_at, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6)",
            vec![
                "strong-org".into(),
                Option::<String>::None.into(),
                "Strong Org".into(),
                Option::<String>::None.into(),
                "2026-04-06T00:00:00Z".into(),
                "2026-04-06T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert organization");
    runtime
        .execute(
            "INSERT INTO users (id, email, display_name, active, created_at)
             VALUES ($1, $2, $3, $4, $5)",
            vec![
                "strong-user".into(),
                "strong-user@trailbase.test".into(),
                "strong-user".into(),
                1_i64.into(),
                "2026-04-06T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert user");
    runtime
        .execute(
            "INSERT INTO organization_memberships (id, organization_id, user_id, membership_role, status, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            vec![
                "membership-strong-user".into(),
                "strong-org".into(),
                "strong-user".into(),
                "member".into(),
                "active".into(),
                "2026-04-06T00:00:00Z".into(),
                "2026-04-06T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert membership");

    let app = api::build_router(service);
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/me")
                .header("authorization", auth_support::bearer_header("strong-user"))
                .header("x-organization-id", "strong-org")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        query_count(
            &runtime,
            "SELECT COUNT(*) as count
             FROM user_identities
             WHERE provider = $1 AND external_user_id = $2",
            vec!["trailbase".into(), "strong-user".into()],
        )
        .await,
        1
    );
    assert_eq!(
        query_count(
            &runtime,
            "SELECT COUNT(*) as count
             FROM auth_identity_cache",
            Vec::new(),
        )
        .await,
        1
    );
}

#[tokio::test]
async fn voter_default_prefer_strong_current_user_syncs_identity_without_query_override() {
    let (service, _db, runtime, _db_dir, _artifacts_dir) =
        build_service_with_default_consistency(ReadConsistency::PreferStrong).await;
    runtime
        .execute(
            "INSERT INTO organizations (id, business_no, name, inactivated_at, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6)",
            vec![
                "prefer-strong-org".into(),
                Option::<String>::None.into(),
                "Prefer Strong Org".into(),
                Option::<String>::None.into(),
                "2026-04-06T00:00:00Z".into(),
                "2026-04-06T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert organization");
    runtime
        .execute(
            "INSERT INTO users (id, email, display_name, active, created_at)
             VALUES ($1, $2, $3, $4, $5)",
            vec![
                "prefer-strong-user".into(),
                "prefer-strong-user@trailbase.test".into(),
                "prefer-strong-user".into(),
                1_i64.into(),
                "2026-04-06T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert user");
    runtime
        .execute(
            "INSERT INTO organization_memberships (id, organization_id, user_id, membership_role, status, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            vec![
                "membership-prefer-strong-user".into(),
                "prefer-strong-org".into(),
                "prefer-strong-user".into(),
                "member".into(),
                "active".into(),
                "2026-04-06T00:00:00Z".into(),
                "2026-04-06T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert membership");

    let app = api::build_router(service);
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/me")
                .header(
                    "authorization",
                    auth_support::bearer_header("prefer-strong-user"),
                )
                .header("x-organization-id", "prefer-strong-org")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        query_count(
            &runtime,
            "SELECT COUNT(*) as count
             FROM user_identities
             WHERE provider = $1 AND external_user_id = $2",
            vec!["trailbase".into(), "prefer-strong-user".into()],
        )
        .await,
        1
    );
    assert_eq!(
        query_count(
            &runtime,
            "SELECT COUNT(*) as count
             FROM auth_identity_cache",
            Vec::new(),
        )
        .await,
        1
    );
}

#[tokio::test]
async fn voter_default_strong_authenticated_read_syncs_identity_without_query_override() {
    let (service, _db, runtime, _db_dir, _artifacts_dir) =
        build_service_with_default_consistency(ReadConsistency::Strong).await;
    runtime
        .execute(
            "INSERT INTO organizations (id, business_no, name, inactivated_at, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6)",
            vec![
                "strong-org".into(),
                Option::<String>::None.into(),
                "Strong Org".into(),
                Option::<String>::None.into(),
                "2026-04-06T00:00:00Z".into(),
                "2026-04-06T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert organization");
    runtime
        .execute(
            "INSERT INTO users (id, email, display_name, active, created_at)
             VALUES ($1, $2, $3, $4, $5)",
            vec![
                "strong-user".into(),
                "strong-user@trailbase.test".into(),
                "strong-user".into(),
                1_i64.into(),
                "2026-04-06T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert user");
    runtime
        .execute(
            "INSERT INTO organization_memberships (id, organization_id, user_id, membership_role, status, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            vec![
                "membership-strong-user".into(),
                "strong-org".into(),
                "strong-user".into(),
                "member".into(),
                "active".into(),
                "2026-04-06T00:00:00Z".into(),
                "2026-04-06T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert membership");

    let app = api::build_router(service);
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/me")
                .header("authorization", auth_support::bearer_header("strong-user"))
                .header("x-organization-id", "strong-org")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        query_count(
            &runtime,
            "SELECT COUNT(*) as count
             FROM user_identities
             WHERE provider = $1 AND external_user_id = $2",
            vec!["trailbase".into(), "strong-user".into()],
        )
        .await,
        1
    );
}

#[tokio::test]
async fn voter_default_prefer_strong_authenticated_read_syncs_identity_without_query_override() {
    let (service, _db, runtime, _db_dir, _artifacts_dir) =
        build_service_with_default_consistency(ReadConsistency::PreferStrong).await;
    runtime
        .execute(
            "INSERT INTO organizations (id, business_no, name, inactivated_at, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6)",
            vec![
                "prefer-strong-org".into(),
                Option::<String>::None.into(),
                "Prefer Strong Org".into(),
                Option::<String>::None.into(),
                "2026-04-06T00:00:00Z".into(),
                "2026-04-06T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert organization");
    runtime
        .execute(
            "INSERT INTO users (id, email, display_name, active, created_at)
             VALUES ($1, $2, $3, $4, $5)",
            vec![
                "prefer-strong-user".into(),
                "prefer-strong-user@trailbase.test".into(),
                "prefer-strong-user".into(),
                1_i64.into(),
                "2026-04-06T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert user");
    runtime
        .execute(
            "INSERT INTO organization_memberships (id, organization_id, user_id, membership_role, status, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            vec![
                "membership-prefer-strong-user".into(),
                "prefer-strong-org".into(),
                "prefer-strong-user".into(),
                "member".into(),
                "active".into(),
                "2026-04-06T00:00:00Z".into(),
                "2026-04-06T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert membership");

    let app = api::build_router(service);
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/me")
                .header(
                    "authorization",
                    auth_support::bearer_header("prefer-strong-user"),
                )
                .header("x-organization-id", "prefer-strong-org")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        query_count(
            &runtime,
            "SELECT COUNT(*) as count
             FROM user_identities
             WHERE provider = $1 AND external_user_id = $2",
            vec!["trailbase".into(), "prefer-strong-user".into()],
        )
        .await,
        1
    );
}

#[tokio::test]
async fn read_routes_reject_invalid_consistency_query() {
    let (service, _db, _runtime, _db_dir, _artifacts_dir) = build_service().await;
    let app = api::build_router(service);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/me?consistency=bogus")
                .header(
                    "authorization",
                    auth_support::bearer_header("invalid-consistency-user"),
                )
                .header("x-organization-id", "bootstrap-user")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let json: Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(json["error"], "invalid consistency: bogus");
}

#[tokio::test]
async fn read_routes_allow_explicit_strong_consistency() {
    let (service, _db, _runtime, _db_dir, _artifacts_dir) = build_service().await;
    let app = api::build_router(service);

    let bootstrap = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/admin/bootstrap/trailbase-owner")
                .header(
                    "authorization",
                    auth_support::bearer_header("strong-consistency-user"),
                )
                .header("x-certd-master-key", "dev-only-master-key")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(bootstrap.status(), StatusCode::CREATED);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/me?consistency=strong")
                .header(
                    "authorization",
                    auth_support::bearer_header("strong-consistency-user"),
                )
                .header("x-organization-id", "bootstrap-user")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
}

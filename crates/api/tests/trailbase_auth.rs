mod support;

use std::sync::Arc;

use axum::{body::Body, http::{Request, StatusCode}};
use http_body_util::BodyExt;
use hiqlite::{Node, NodeConfig, s3::EncKeys};
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
use tempfile::TempDir;
use serde_json::Value;
use tower::ServiceExt;

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

async fn build_service() -> (Arc<AppService>, Database, HiqliteRuntime, TempDir, TempDir) {
    let trailbase_base_url = support::start_mock_trailbase().await;
    let (runtime, db_dir) = test_runtime().await;
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
        artifacts: ArtifactConfig {
            backend: ArtifactBackendConfig::FileSystem {
                root_dir: artifacts_dir.path().to_path_buf(),
            },
        },
        hiqlite: HiqliteConfig {
            runtime_mode: RuntimeMode::Embedded,
            local_node_role: LocalNodeRole::Voter,
            default_read_consistency: ReadConsistency::Local,
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

#[tokio::test]
async fn trailbase_bearer_token_resolves_user_and_allows_list() {
    let (service, db, runtime, _db_dir, _artifacts_dir) = build_service().await;
    db.upsert_organization_with_owner("owner-org", "Owner Org", "Owner Org")
        .await
        .expect("upsert owner org");
    db.upsert_organization_with_owner("worker-org", "Worker Org", "Worker Org")
        .await
        .expect("upsert worker org");
    db.upsert_agreement(&domain::Agreement {
        id: "agreement-1".into(),
        agreement_type: domain::AgreementType::Delegation,
        grantor_organization_id: "owner-org".into(),
        grantee_organization_id: "worker-org".into(),
        status: domain::AgreementStatus::Active,
        title: "Worker Delegation".into(),
        reference_no: Some("AG-1".into()),
        starts_at: Some("2026-01-01T00:00:00Z".into()),
        ends_at: Some("2026-12-31T23:59:59Z".into()),
    })
    .await
    .expect("agreement");
    runtime.execute(
        "INSERT INTO users (id, email, display_name, active, created_at)
         VALUES ($1, $2, $3, $4, $5)",
        vec![
            "local-user".into(),
            "trailbase@example.com".into(),
            "TrailBase User".into(),
            1_i64.into(),
            "2026-04-03T00:00:00Z".into(),
        ],
    ).await.expect("insert user");
    runtime.execute(
        "INSERT INTO organization_memberships (id, organization_id, user_id, membership_role, status, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
        vec![
            "membership-1".into(),
            "worker-org".into(),
            "local-user".into(),
            "member".into(),
            "active".into(),
            "2026-04-03T00:00:00Z".into(),
            "2026-04-03T00:00:00Z".into(),
        ],
    ).await.expect("membership");
    runtime.execute(
        "INSERT INTO plants (id, facility_code, owner_organization_id, created_by_organization_id, name, contract_no, address, installed_kw, rec_start_date, confirm_issued, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
        vec![
            "plant-1".into(),
            "FAC-1".into(),
            "owner-org".into(),
            Option::<String>::Some("worker-org".to_owned()).into(),
            "Plant One".into(),
            "CN-1".into(),
            "Seoul".into(),
            50.0_f64.into(),
            "2026-01-01".into(),
            "Y".into(),
            "2026-04-03T00:00:00Z".into(),
            "2026-04-03T00:00:00Z".into(),
        ],
    ).await.expect("plant");
    db.upsert_role(&domain::Role {
        id: "role-profile-reader".into(),
        organization_id: None,
        code: "profile.reader".into(),
        name: "Profile Reader".into(),
        description: None,
    }).await.expect("role");
    db.upsert_role_action("role-profile-reader", "plant.profile.read")
        .await
        .expect("role action");
    db.save_role_binding(&domain::RoleBinding {
        id: "binding-org-profile".into(),
        subject_type: domain::RoleBindingSubjectType::Organization,
        subject_id: "worker-org".into(),
        subject_organization_id: "worker-org".into(),
        resource_type: domain::RoleBindingResourceType::Plant,
        resource_id: "plant-1".into(),
        role_id: "role-profile-reader".into(),
        agreement_id: Some("agreement-1".into()),
        state: domain::RoleBindingState::Active,
        created_by_user_id: "bootstrap-user".into(),
        created_at: "2026-04-03T00:00:00Z".into(),
        updated_at: "2026-04-03T00:00:00Z".into(),
    }).await.expect("org binding");
    db.save_role_binding(&domain::RoleBinding {
        id: "binding-user-profile".into(),
        subject_type: domain::RoleBindingSubjectType::User,
        subject_id: "local-user".into(),
        subject_organization_id: "worker-org".into(),
        resource_type: domain::RoleBindingResourceType::Plant,
        resource_id: "plant-1".into(),
        role_id: "role-profile-reader".into(),
        agreement_id: None,
        state: domain::RoleBindingState::Active,
        created_by_user_id: "bootstrap-user".into(),
        created_at: "2026-04-03T00:00:00Z".into(),
        updated_at: "2026-04-03T00:00:00Z".into(),
    }).await.expect("user binding");

    let app = api::build_router(service);
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/plants")
                .header("authorization", support::bearer_header("local-user"))
                .header("x-organization-id", "worker-org")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn bootstrap_trailbase_owner_binds_first_user_to_bootstrap_org() {
    let (service, _db, _runtime, _db_dir, _artifacts_dir) = build_service().await;
    let app = api::build_router(service);

    let before = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/plants")
                .header("authorization", support::bearer_header("first-user"))
                .header("x-organization-id", "bootstrap-user")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(before.status(), StatusCode::FORBIDDEN);

    let bootstrap = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/admin/bootstrap/trailbase-owner")
                .header("authorization", support::bearer_header("first-user"))
                .header("x-certd-master-key", "dev-only-master-key")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    let bootstrap_status = bootstrap.status();
    let bootstrap_body = bootstrap
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    assert_eq!(
        bootstrap_status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&bootstrap_body)
    );

    let after = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/plants")
                .header("authorization", support::bearer_header("first-user"))
                .header("x-organization-id", "bootstrap-user")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(after.status(), StatusCode::OK);
}

#[tokio::test]
async fn current_user_returns_local_user_and_active_organizations() {
    let (service, db, runtime, _db_dir, _artifacts_dir) = build_service().await;
    db.upsert_organization_with_owner("alpha-org", "Alpha Org", "Alpha Org")
        .await
        .expect("upsert alpha org");
    db.upsert_organization_with_owner("beta-org", "Beta Org", "Beta Org")
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
    for (membership_id, organization_id) in [("membership-alpha", "alpha-org"), ("membership-beta", "beta-org")] {
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
                .header("authorization", support::bearer_header("me-user"))
                .header("x-organization-id", "alpha-org")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.expect("body").to_bytes();
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
    db.upsert_organization_with_owner("restore-org", "Restore Org", "Restore Org")
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
                .header("authorization", support::bearer_header("restored-trailbase-user"))
                .header("x-organization-id", "restore-org")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.expect("body").to_bytes();
    let json: Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(json["user"]["id"], Value::String(original_user.id));
    assert_eq!(json["activeOrganizationId"], "restore-org");
    let organizations = json["organizations"].as_array().expect("organizations");
    assert!(organizations.iter().any(|org| org["id"] == "restore-org"));
}

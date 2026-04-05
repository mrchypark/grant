mod support;

use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use domain::{Agreement, AgreementStatus, AgreementType, Role, RoleBinding, RoleBindingResourceType, RoleBindingState, RoleBindingSubjectType};
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
use serde_json::json;
use tempfile::TempDir;
use tower::ServiceExt;
use uuid::Uuid;

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

async fn test_service() -> (Arc<AppService>, Database, TempDir) {
    let trailbase_base_url = support::start_mock_trailbase().await;
    let (runtime, dir) = test_runtime().await;
    let artifacts_root = dir.path().join("artifacts");
    let app_cfg = AppConfig {
        server: ServerConfig {
            bind_addr: free_local_addr(),
        },
        security: SecurityConfig {
            master_key: "dev-only-master-key".to_owned(),
            bootstrap_user_id: "bootstrap-user".to_owned(),
            bootstrap_user_name: "Bootstrap User".to_owned(),
        },
        auth: support::trailbase_auth_config(trailbase_base_url),
        artifacts: ArtifactConfig {
            backend: ArtifactBackendConfig::FileSystem {
                root_dir: artifacts_root,
            },
        },
        hiqlite: HiqliteConfig {
            runtime_mode: RuntimeMode::Embedded,
            local_node_role: LocalNodeRole::Voter,
            default_read_consistency: ReadConsistency::Local,
            remote_nodes: vec![],
            remote_tls: false,
            remote_tls_no_verify: false,
            remote_api_secret: None,
            remote_with_proxy: false,
        },
    };
    let db = Database::new(
        runtime,
        app_cfg.security.bootstrap_user_id.clone(),
        app_cfg.security.bootstrap_user_name.clone(),
    );
    let db_for_tests = db.clone();
    let artifacts = ArtifactStore::new(&app_cfg.artifacts)
        .await
        .expect("artifact store");
    let service = Arc::new(AppService::new(app_cfg, db, artifacts));
    service.initialize().await.expect("initialize");
    (service, db_for_tests, dir)
}

fn sync_request(organization_id: &str, token: &str, active_organization_id: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(format!("/api/v1/organizations/{organization_id}/sync"))
        .header("content-type", "application/json")
        .header("authorization", support::bearer_header(token))
        .header("x-organization-id", active_organization_id)
        .body(Body::from(
            json!({
                "facilityCode": "FAC-SYNC-001",
                "mode": "full"
            })
            .to_string(),
        ))
        .expect("request")
}

async fn seed_sync_fixture(db: &Database) {
    db.upsert_organization_with_owner("owner-org", "Owner Org", "Owner Org")
        .await
        .expect("owner org");
    db.upsert_organization_with_owner("operator-org", "Operator Org", "Operator Org")
        .await
        .expect("operator org");
    db.upsert_organization_identity("owner-org", Some("1234567890"), "Owner Org")
        .await
        .expect("owner business no");
    db.upsert_organization_identity("operator-org", Some("2768603291"), "Operator Org")
        .await
        .expect("operator business no");

    db.upsert_role(&Role {
        id: "role-plant-sync-execute".to_owned(),
        organization_id: None,
        code: "plant.sync.execute".to_owned(),
        name: "Plant Sync Execute".to_owned(),
        description: None,
    })
    .await
    .expect("upsert sync role");
    db.upsert_role_action("role-plant-sync-execute", "plant.sync.execute")
        .await
        .expect("upsert sync action");

    db.upsert_agreement(&Agreement {
        id: "agreement-sync".to_owned(),
        agreement_type: AgreementType::Delegation,
        grantor_organization_id: "owner-org".to_owned(),
        grantee_organization_id: "operator-org".to_owned(),
        status: AgreementStatus::Active,
        title: "Sync Delegation".to_owned(),
        reference_no: None,
        starts_at: None,
        ends_at: None,
    })
    .await
    .expect("upsert agreement");

    db.upsert_plant(&domain::Plant {
        id: "FAC-SYNC-001".to_owned(),
        facility_code: "FAC-SYNC-001".to_owned(),
        name: "Sync Plant".to_owned(),
        owner_organization_id: "owner-org".to_owned(),
        created_by_organization_id: Some("operator-org".to_owned()),
        contract_no: "CN-SYNC-001".to_owned(),
        address: "Seoul".to_owned(),
        installed_kw: 100.0,
        rec_start_date: "2026-01-01".to_owned(),
        confirm_issued: "Y".to_owned(),
    }).await.expect("upsert plant");
}

#[tokio::test]
async fn sync_requires_plant_sync_execute() {
    let (service, db, _dir) = test_service().await;
    seed_sync_fixture(&db).await;
    let app = api::build_router(service);

    let response = app
        .oneshot(sync_request("operator-org", "bootstrap-user", "operator-org"))
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn sync_authorization_allows_plant_sync_execute_binding() {
    let (service, db, _dir) = test_service().await;
    seed_sync_fixture(&db).await;

    db.save_role_binding(&RoleBinding {
        id: Uuid::new_v4().simple().to_string(),
        subject_type: RoleBindingSubjectType::Organization,
        subject_id: "operator-org".to_owned(),
        subject_organization_id: "operator-org".to_owned(),
        resource_type: RoleBindingResourceType::Plant,
        resource_id: "FAC-SYNC-001".to_owned(),
        role_id: "role-plant-sync-execute".to_owned(),
        agreement_id: Some("agreement-sync".to_owned()),
        state: RoleBindingState::Active,
        created_by_user_id: "bootstrap-user".to_owned(),
        created_at: "2026-04-03T00:00:00Z".to_owned(),
        updated_at: "2026-04-03T00:00:00Z".to_owned(),
    })
    .await
    .expect("save organization binding");

    db.save_role_binding(&RoleBinding {
        id: Uuid::new_v4().simple().to_string(),
        subject_type: RoleBindingSubjectType::User,
        subject_id: "bootstrap-user".to_owned(),
        subject_organization_id: "operator-org".to_owned(),
        resource_type: RoleBindingResourceType::Plant,
        resource_id: "FAC-SYNC-001".to_owned(),
        role_id: "role-plant-sync-execute".to_owned(),
        agreement_id: None,
        state: RoleBindingState::Active,
        created_by_user_id: "bootstrap-user".to_owned(),
        created_at: "2026-04-03T00:00:00Z".to_owned(),
        updated_at: "2026-04-03T00:00:00Z".to_owned(),
    })
    .await
    .expect("save user binding");

    service
        .authorize_sync(
            "bootstrap-user",
            "operator-org",
            Some("FAC-SYNC-001"),
        )
        .await
        .expect("sync authorization should pass");
}

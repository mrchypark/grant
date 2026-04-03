use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use domain::{Agreement, AgreementStatus, AgreementType, Role, RoleBinding, RoleBindingResourceType, RoleBindingState, RoleBindingSubjectType};
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
use serde_json::{Value, json};
use std::sync::Arc;
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

fn app_request(
    path: &str,
    user_id: &str,
    organization_id: &str,
    body: Value,
) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .header("x-user-id", user_id)
        .header("x-organization-id", organization_id)
        .body(Body::from(body.to_string()))
        .expect("request")
}

async fn seed_creator_context(db: &Database) {
    db.upsert_organization_with_owner("creator-org", "Creator Org", "Creator Org")
        .await
        .expect("creator org");
    db.upsert_organization_with_owner("owner-org", "Owner Org", "Owner Org")
        .await
        .expect("owner org");

    db.upsert_role(&Role {
        id: "role-org-plant-create".to_owned(),
        organization_id: None,
        code: "organization.plant.create".to_owned(),
        name: "Org Plant Create".to_owned(),
        description: None,
    })
    .await
    .expect("upsert org create role");
    db.upsert_role_action("role-org-plant-create", "organization.plant.create")
        .await
        .expect("upsert create action");

    db.upsert_role(&Role {
        id: "role-org-plant-create-for-owner".to_owned(),
        organization_id: None,
        code: "organization.plant.create_for_owner".to_owned(),
        name: "Org Plant Create For Owner".to_owned(),
        description: None,
    })
    .await
    .expect("upsert org create-for-owner role");
    db.upsert_role_action(
        "role-org-plant-create-for-owner",
        "organization.plant.create_for_owner",
    )
    .await
    .expect("upsert create-for-owner action");

    db.save_role_binding(&RoleBinding {
        id: Uuid::new_v4().simple().to_string(),
        subject_type: RoleBindingSubjectType::Organization,
        subject_id: "creator-org".to_owned(),
        subject_organization_id: "creator-org".to_owned(),
        resource_type: RoleBindingResourceType::Organization,
        resource_id: "creator-org".to_owned(),
        role_id: "role-org-plant-create".to_owned(),
        agreement_id: None,
        state: RoleBindingState::Active,
        created_by_user_id: "bootstrap-user".to_owned(),
        created_at: "2026-04-03T00:00:00Z".to_owned(),
        updated_at: "2026-04-03T00:00:00Z".to_owned(),
    })
    .await
    .expect("save org create binding");
}

async fn bind_user_action(db: &Database, role_id: &str) {
    db.save_role_binding(&RoleBinding {
        id: Uuid::new_v4().simple().to_string(),
        subject_type: RoleBindingSubjectType::User,
        subject_id: "bootstrap-user".to_owned(),
        subject_organization_id: "creator-org".to_owned(),
        resource_type: RoleBindingResourceType::Organization,
        resource_id: "creator-org".to_owned(),
        role_id: role_id.to_owned(),
        agreement_id: None,
        state: RoleBindingState::Active,
        created_by_user_id: "bootstrap-user".to_owned(),
        created_at: "2026-04-03T00:00:00Z".to_owned(),
        updated_at: "2026-04-03T00:00:00Z".to_owned(),
    })
    .await
    .expect("save user org binding");
}

#[tokio::test]
async fn create_plant_requires_organization_scoped_create() {
    let (service, db, _dir) = test_service().await;
    seed_creator_context(&db).await;
    let app = api::build_router(service);

    let response = app
        .oneshot(app_request(
            "/api/v1/organizations/creator-org/plants",
            "bootstrap-user",
            "creator-org",
            json!({
                "facilityCode": "FAC-001",
                "name": "Plant One",
                "contractNo": "CN-001",
                "address": "Seoul",
                "installedKw": 100.0,
                "recStartDate": "2026-01-01",
                "confirmIssued": "Y"
            }),
        ))
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn create_for_owner_requires_extra_permission() {
    let (service, db, _dir) = test_service().await;
    seed_creator_context(&db).await;
    bind_user_action(&db, "role-org-plant-create").await;
    let app = api::build_router(service);

    let response = app
        .oneshot(app_request(
            "/api/v1/organizations/creator-org/plants",
            "bootstrap-user",
            "creator-org",
            json!({
                "facilityCode": "FAC-002",
                "name": "Plant Two",
                "contractNo": "CN-002",
                "address": "Busan",
                "installedKw": 80.0,
                "recStartDate": "2026-01-01",
                "confirmIssued": "Y",
                "ownerOrganizationId": "owner-org"
            }),
        ))
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn create_plant_persists_owner_and_creator_and_bootstraps_visibility() {
    let (service, db, _dir) = test_service().await;
    seed_creator_context(&db).await;
    bind_user_action(&db, "role-org-plant-create").await;
    bind_user_action(&db, "role-org-plant-create-for-owner").await;
    db.upsert_agreement(&Agreement {
        id: "agreement-create-owner".to_owned(),
        agreement_type: AgreementType::Delegation,
        grantor_organization_id: "owner-org".to_owned(),
        grantee_organization_id: "creator-org".to_owned(),
        status: AgreementStatus::Active,
        title: "Create Owner Agreement".to_owned(),
        reference_no: None,
        starts_at: None,
        ends_at: None,
    })
    .await
    .expect("agreement");

    let app = api::build_router(service.clone());
    let response = app
        .oneshot(app_request(
            "/api/v1/organizations/creator-org/plants",
            "bootstrap-user",
            "creator-org",
            json!({
                "facilityCode": "FAC-003",
                "name": "Plant Three",
                "contractNo": "CN-003",
                "address": "Incheon",
                "installedKw": 65.0,
                "recStartDate": "2026-01-01",
                "confirmIssued": "Y",
                "ownerOrganizationId": "owner-org"
            }),
        ))
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::CREATED);
    let body = response.into_body().collect().await.expect("body").to_bytes();
    let payload: Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(payload["facilityCode"], "FAC-003");
    assert_eq!(payload["ownerOrganizationId"], "owner-org");
    assert_eq!(payload["createdByOrganizationId"], "creator-org");

    let plant = db
        .get_plant_by_facility_code("FAC-003")
        .await
        .expect("created plant");
    assert_eq!(plant.owner_organization_id, "owner-org");
    assert_eq!(
        plant.created_by_organization_id.as_deref(),
        Some("creator-org")
    );

    let visible = service
        .list_visible_plants("bootstrap-user", "creator-org")
        .await
        .expect("creator visibility");
    assert!(
        visible.iter().any(|plant| plant.facility_code == "FAC-003"),
        "creator org should retain profile visibility after create"
    );

    let can_sync = db
        .has_user_plant_action("bootstrap-user", "creator-org", "FAC-003", "plant.sync.execute")
        .await
        .expect("sync action lookup");
    assert!(
        can_sync,
        "creator should receive plant.sync.execute bootstrap permission after create"
    );
}

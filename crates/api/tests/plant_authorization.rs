mod support;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use domain::{Role, RoleBinding, RoleBindingResourceType, RoleBindingState, RoleBindingSubjectType};
use http_body_util::BodyExt;
use infra::{
    artifacts::ArtifactStore,
    config::{
        AppConfig, ArtifactBackendConfig, ArtifactConfig, HiqliteConfig, LocalNodeRole,
        ReadConsistency, RuntimeMode, SecurityConfig, ServerConfig,
    },
    database::Database,
    service::AppService,
    runtime::HiqliteRuntime,
};
use hiqlite::{Node, NodeConfig, s3::EncKeys};
use serde_json::Value;
use std::sync::Arc;
use tempfile::TempDir;
use serial_test::serial;
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

async fn seed_user_visible_plant(service: &Arc<AppService>, db: &Database) {
    db.upsert_organization_with_owner("owner-org", "Owner Org", "Owner Org")
        .await
        .expect("owner org");
    db.upsert_organization_with_owner("viewer-org", "Viewer Org", "Viewer Org")
        .await
        .expect("viewer org");

    db.upsert_role(&Role {
        id: "role-plant-profile".to_owned(),
        organization_id: None,
        code: "plant.profile.reader".to_owned(),
        name: "Plant Profile Reader".to_owned(),
        description: None,
    })
    .await
    .expect("profile role");
    db.upsert_role_action("role-plant-profile", "plant.profile.read")
        .await
        .expect("profile role action");
    db.upsert_role(&Role {
        id: "role-plant-document".to_owned(),
        organization_id: None,
        code: "plant.document.reader".to_owned(),
        name: "Plant Document Reader".to_owned(),
        description: None,
    })
    .await
    .expect("document role");
    db.upsert_role_action("role-plant-document", "plant.document.read")
        .await
        .expect("document role action");

    db.upsert_plant(&domain::Plant {
        id: "plant-visible".to_owned(),
        facility_code: "FAC-VISIBLE".to_owned(),
        owner_organization_id: "owner-org".to_owned(),
        created_by_organization_id: Some("viewer-org".to_owned()),
        name: "Visible Plant".to_owned(),
        contract_no: "CN-001".to_owned(),
        address: "Seoul".to_owned(),
        installed_kw: 100.0_f64,
        rec_start_date: "2026-01-01".to_owned(),
        confirm_issued: "Y".to_owned(),
    })
    .await
    .expect("visible plant");

    db.upsert_plant(&domain::Plant {
        id: "plant-hidden".to_owned(),
        facility_code: "FAC-HIDDEN".to_owned(),
        owner_organization_id: "owner-org".to_owned(),
        created_by_organization_id: Some("viewer-org".to_owned()),
        name: "Hidden Plant".to_owned(),
        contract_no: "CN-002".to_owned(),
        address: "Busan".to_owned(),
        installed_kw: 80.0_f64,
        rec_start_date: "2026-01-01".to_owned(),
        confirm_issued: "Y".to_owned(),
    })
    .await
    .expect("hidden plant");

    db.upsert_agreement(&domain::Agreement {
        id: "agreement-1".to_owned(),
        agreement_type: domain::AgreementType::Delegation,
        grantor_organization_id: "owner-org".to_owned(),
        grantee_organization_id: "viewer-org".to_owned(),
        status: domain::AgreementStatus::Active,
        title: "Delegation Agreement".to_owned(),
        reference_no: None,
        starts_at: None,
        ends_at: None,
    })
    .await
    .expect("agreement");

    let profile_role = RoleBinding {
        id: Uuid::new_v4().simple().to_string(),
        subject_type: RoleBindingSubjectType::Organization,
        subject_id: "viewer-org".to_owned(),
        subject_organization_id: "viewer-org".to_owned(),
        resource_type: RoleBindingResourceType::Plant,
        resource_id: "plant-visible".to_owned(),
        role_id: "role-plant-profile".to_owned(),
        agreement_id: Some("agreement-1".to_owned()),
        state: RoleBindingState::Active,
        created_by_user_id: "bootstrap-user".to_owned(),
        created_at: "2026-04-03T00:00:00Z".to_owned(),
        updated_at: "2026-04-03T00:00:00Z".to_owned(),
    };
    let profile_user_role = RoleBinding {
        id: Uuid::new_v4().simple().to_string(),
        subject_type: RoleBindingSubjectType::User,
        subject_id: "bootstrap-user".to_owned(),
        subject_organization_id: "viewer-org".to_owned(),
        resource_type: RoleBindingResourceType::Plant,
        resource_id: "plant-visible".to_owned(),
        role_id: "role-plant-profile".to_owned(),
        agreement_id: None,
        state: RoleBindingState::Active,
        created_by_user_id: "bootstrap-user".to_owned(),
        created_at: "2026-04-03T00:00:00Z".to_owned(),
        updated_at: "2026-04-03T00:00:00Z".to_owned(),
    };
    let document_org_role = RoleBinding {
        id: Uuid::new_v4().simple().to_string(),
        subject_type: RoleBindingSubjectType::Organization,
        subject_id: "viewer-org".to_owned(),
        subject_organization_id: "viewer-org".to_owned(),
        resource_type: RoleBindingResourceType::Plant,
        resource_id: "plant-hidden".to_owned(),
        role_id: "role-plant-document".to_owned(),
        agreement_id: Some("agreement-1".to_owned()),
        state: RoleBindingState::Active,
        created_by_user_id: "bootstrap-user".to_owned(),
        created_at: "2026-04-03T00:00:00Z".to_owned(),
        updated_at: "2026-04-03T00:00:00Z".to_owned(),
    };
    let document_role = RoleBinding {
        id: Uuid::new_v4().simple().to_string(),
        subject_type: RoleBindingSubjectType::User,
        subject_id: "bootstrap-user".to_owned(),
        subject_organization_id: "viewer-org".to_owned(),
        resource_type: RoleBindingResourceType::Plant,
        resource_id: "plant-hidden".to_owned(),
        role_id: "role-plant-document".to_owned(),
        agreement_id: None,
        state: RoleBindingState::Active,
        created_by_user_id: "bootstrap-user".to_owned(),
        created_at: "2026-04-03T00:00:00Z".to_owned(),
        updated_at: "2026-04-03T00:00:00Z".to_owned(),
    };
    service
        .save_role_binding(&profile_role)
        .await
        .expect("save profile org role binding");
    service
        .save_role_binding(&profile_user_role)
        .await
        .expect("save profile user role binding");
    service
        .save_role_binding(&document_org_role)
        .await
        .expect("save document org role binding");
    service
        .save_role_binding(&document_role)
        .await
        .expect("save document role binding");
}

fn app_request(method: &str, path: &str, token: &str, organization_id: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", support::bearer_header(token))
        .header("x-organization-id", organization_id)
        .body(Body::empty())
        .expect("request")
}

async fn response_json(response: axum::response::Response) -> Value {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("json body")
}

#[tokio::test]
#[serial]
async fn plant_list_is_driven_by_effective_visibility() {
    let (service, db, _dir) = test_service().await;
    seed_user_visible_plant(&service, &db).await;

    let router = api::build_router(service.clone());
    let response = router
        .oneshot(app_request("GET", "/api/v1/plants", "bootstrap-user", "viewer-org"))
        .await
        .expect("router response");
    assert_eq!(response.status(), StatusCode::OK);

    let body = response_json(response).await;
    let plants = body.as_array().expect("plant list array");
    let facility_codes: Vec<_> = plants
        .iter()
        .map(|plant| plant["facilityCode"].as_str().expect("facility code"))
        .collect();
    assert_eq!(facility_codes, vec!["FAC-VISIBLE"]);
}

#[tokio::test]
#[serial]
async fn plant_detail_is_forbidden_without_profile_read() {
    let (service, db, _dir) = test_service().await;
    seed_user_visible_plant(&service, &db).await;

    let router = api::build_router(service.clone());
    let response = router
        .oneshot(app_request(
            "GET",
            "/api/v1/plants/FAC-HIDDEN",
            "bootstrap-user",
            "viewer-org",
        ))
        .await
        .expect("router response");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
#[serial]
async fn plant_detail_is_allowed_with_profile_read() {
    let (service, db, _dir) = test_service().await;
    seed_user_visible_plant(&service, &db).await;

    let router = api::build_router(service.clone());
    let response = router
        .oneshot(app_request(
            "GET",
            "/api/v1/plants/FAC-VISIBLE",
            "bootstrap-user",
            "viewer-org",
        ))
        .await
        .expect("router response");
    assert_eq!(response.status(), StatusCode::OK);

    let body = response_json(response).await;
    assert_eq!(body["plant"]["facilityCode"], "FAC-VISIBLE");
}

#[tokio::test]
#[serial]
async fn document_read_alone_does_not_make_a_plant_visible() {
    let (service, db, _dir) = test_service().await;
    seed_user_visible_plant(&service, &db).await;

    let router = api::build_router(service.clone());
    let response = router
        .oneshot(app_request("GET", "/api/v1/plants", "bootstrap-user", "viewer-org"))
        .await
        .expect("router response");
    assert_eq!(response.status(), StatusCode::OK);

    let body = response_json(response).await;
    let plants = body.as_array().expect("plant list array");
    assert_eq!(plants.len(), 1);
    assert_eq!(plants[0]["facilityCode"], "FAC-VISIBLE");
}

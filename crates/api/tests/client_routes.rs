mod support;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
};
use domain::{
    Role, RoleBinding, RoleBindingResourceType, RoleBindingState, RoleBindingSubjectType,
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
        kms: support::test_kms_config(),
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
    method: Method,
    path: &str,
    token: &str,
    organization_id: &str,
    body: Option<Value>,
) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", support::bearer_header(token))
        .header("x-organization-id", organization_id);
    if body.is_some() {
        builder = builder.header("content-type", "application/json");
    }
    builder
        .body(match body {
            Some(body) => Body::from(body.to_string()),
            None => Body::empty(),
        })
        .expect("request")
}

async fn seed_client_manager(db: &Database, organization_id: &str) {
    db.upsert_organization_with_owner(organization_id, "Creator Org", None, "Creator Org")
        .await
        .expect("creator org");
    db.upsert_role(&Role {
        id: "role-org-client-manage".to_owned(),
        organization_id: None,
        code: "organization.client.manage".to_owned(),
        name: "Org Client Manage".to_owned(),
        description: None,
    })
    .await
    .expect("upsert client manage role");
    db.upsert_role_action("role-org-client-manage", "organization.client.manage")
        .await
        .expect("upsert client manage action");
    db.save_role_binding(&RoleBinding {
        id: Uuid::new_v4().simple().to_string(),
        subject_type: RoleBindingSubjectType::User,
        subject_id: "bootstrap-user".to_owned(),
        subject_organization_id: organization_id.to_owned(),
        resource_type: RoleBindingResourceType::Organization,
        resource_id: organization_id.to_owned(),
        role_id: "role-org-client-manage".to_owned(),
        agreement_id: None,
        state: RoleBindingState::Active,
        created_by_user_id: "bootstrap-user".to_owned(),
        created_at: "2026-04-12T00:00:00Z".to_owned(),
        updated_at: "2026-04-12T00:00:00Z".to_owned(),
    })
    .await
    .expect("save client manage binding");
}

#[tokio::test]
async fn client_routes_create_list_get_and_replace_actions() {
    let (service, db, _dir) = test_service().await;
    seed_client_manager(&db, "creator-org").await;
    let app = api::build_router(service);

    let create = app
        .clone()
        .oneshot(app_request(
            Method::POST,
            "/api/v1/organizations/creator-org/clients",
            "bootstrap-user",
            "creator-org",
            Some(json!({
                "id": "client-awesome-dashboard",
                "slug": "awesome-dashboard",
                "name": "Awesome Dashboard",
                "description": "External analytics dashboard",
                "clientType": "confidential",
                "appType": "web",
                "firstParty": false,
                "consentPolicy": "explicit",
                "active": true
            })),
        ))
        .await
        .expect("create response");
    assert_eq!(create.status(), StatusCode::CREATED);

    let body = create.into_body().collect().await.expect("body").to_bytes();
    let payload: Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(payload["id"], "client-awesome-dashboard");
    assert_eq!(payload["organizationId"], "creator-org");
    assert_eq!(payload["createdByUserId"], "bootstrap-user");

    let list = app
        .clone()
        .oneshot(app_request(
            Method::GET,
            "/api/v1/organizations/creator-org/clients",
            "bootstrap-user",
            "creator-org",
            None,
        ))
        .await
        .expect("list response");
    assert_eq!(list.status(), StatusCode::OK);
    let list_body = list.into_body().collect().await.expect("body").to_bytes();
    let list_payload: Value = serde_json::from_slice(&list_body).expect("json");
    assert_eq!(list_payload.as_array().map(Vec::len), Some(1));

    let get = app
        .clone()
        .oneshot(app_request(
            Method::GET,
            "/api/v1/organizations/creator-org/clients/client-awesome-dashboard",
            "bootstrap-user",
            "creator-org",
            None,
        ))
        .await
        .expect("get response");
    assert_eq!(get.status(), StatusCode::OK);

    let update = app
        .clone()
        .oneshot(app_request(
            Method::PUT,
            "/api/v1/organizations/creator-org/clients/client-awesome-dashboard",
            "bootstrap-user",
            "creator-org",
            Some(json!({
                "slug": "awesome-dashboard",
                "name": "Awesome Dashboard v2",
                "description": "Updated external analytics dashboard",
                "clientType": "confidential",
                "appType": "web",
                "firstParty": false,
                "consentPolicy": "explicit",
                "active": true
            })),
        ))
        .await
        .expect("update response");
    assert_eq!(update.status(), StatusCode::OK);
    let update_body = update.into_body().collect().await.expect("body").to_bytes();
    let update_payload: Value = serde_json::from_slice(&update_body).expect("json");
    assert_eq!(update_payload["name"], "Awesome Dashboard v2");

    let replace_actions = app
        .clone()
        .oneshot(app_request(
            Method::PUT,
            "/api/v1/organizations/creator-org/clients/client-awesome-dashboard/actions",
            "bootstrap-user",
            "creator-org",
            Some(json!({
                "actionIds": ["plant.profile.read", "plant.report.read"]
            })),
        ))
        .await
        .expect("replace actions response");
    assert_eq!(replace_actions.status(), StatusCode::OK);
    let actions_body = replace_actions
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let actions_payload: Value = serde_json::from_slice(&actions_body).expect("json");
    let action_codes: Vec<&str> = actions_payload
        .as_array()
        .expect("array")
        .iter()
        .filter_map(|item| item.get("code").and_then(Value::as_str))
        .collect();
    assert_eq!(
        action_codes,
        vec!["plant.profile.read", "plant.report.read"]
    );

    let get_actions = app
        .oneshot(app_request(
            Method::GET,
            "/api/v1/organizations/creator-org/clients/client-awesome-dashboard/actions",
            "bootstrap-user",
            "creator-org",
            None,
        ))
        .await
        .expect("get actions response");
    assert_eq!(get_actions.status(), StatusCode::OK);
}

#[tokio::test]
async fn client_routes_require_organization_scoped_manage_permission() {
    let (service, db, _dir) = test_service().await;
    db.upsert_organization_with_owner("creator-org", "Creator Org", None, "Creator Org")
        .await
        .expect("creator org");
    let app = api::build_router(service);

    let response = app
        .oneshot(app_request(
            Method::POST,
            "/api/v1/organizations/creator-org/clients",
            "bootstrap-user",
            "creator-org",
            Some(json!({
                "id": "client-denied",
                "slug": "denied",
                "name": "Denied",
                "clientType": "public",
                "appType": "web",
                "firstParty": false,
                "consentPolicy": "explicit",
                "active": true
            })),
        ))
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn client_create_cannot_rehome_existing_client_to_another_organization() {
    let (service, db, _dir) = test_service().await;
    seed_client_manager(&db, "org-a").await;
    seed_client_manager(&db, "org-b").await;
    let app = api::build_router(service);

    let first = app
        .clone()
        .oneshot(app_request(
            Method::POST,
            "/api/v1/organizations/org-a/clients",
            "bootstrap-user",
            "org-a",
            Some(json!({
                "id": "client-shared",
                "slug": "shared-client",
                "name": "Shared Client",
                "clientType": "confidential",
                "appType": "web",
                "firstParty": false,
                "consentPolicy": "explicit",
                "active": true
            })),
        ))
        .await
        .expect("first response");
    assert_eq!(first.status(), StatusCode::CREATED);

    let second = app
        .oneshot(app_request(
            Method::POST,
            "/api/v1/organizations/org-b/clients",
            "bootstrap-user",
            "org-b",
            Some(json!({
                "id": "client-shared",
                "slug": "shared-client-org-b",
                "name": "Hijack Attempt",
                "clientType": "confidential",
                "appType": "web",
                "firstParty": false,
                "consentPolicy": "explicit",
                "active": true
            })),
        ))
        .await
        .expect("second response");
    assert_eq!(second.status(), StatusCode::CONFLICT);

    let stored = db
        .get_client_with_consistency("client-shared", None)
        .await
        .expect("stored client");
    assert_eq!(stored.organization_id, "org-a");
    assert_eq!(stored.slug, "shared-client");
}

#[tokio::test]
async fn client_create_reports_duplicate_slug_as_conflict() {
    let (service, db, _dir) = test_service().await;
    seed_client_manager(&db, "org-a").await;
    seed_client_manager(&db, "org-b").await;
    let app = api::build_router(service);

    let first = app
        .clone()
        .oneshot(app_request(
            Method::POST,
            "/api/v1/organizations/org-a/clients",
            "bootstrap-user",
            "org-a",
            Some(json!({
                "id": "client-a",
                "slug": "shared-slug",
                "name": "Client A",
                "clientType": "public",
                "appType": "web",
                "firstParty": false,
                "consentPolicy": "explicit",
                "active": true
            })),
        ))
        .await
        .expect("first response");
    assert_eq!(first.status(), StatusCode::CREATED);

    let second = app
        .oneshot(app_request(
            Method::POST,
            "/api/v1/organizations/org-b/clients",
            "bootstrap-user",
            "org-b",
            Some(json!({
                "id": "client-b",
                "slug": "shared-slug",
                "name": "Client B",
                "clientType": "public",
                "appType": "web",
                "firstParty": false,
                "consentPolicy": "explicit",
                "active": true
            })),
        ))
        .await
        .expect("second response");
    assert_eq!(second.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn client_routes_hide_clients_owned_by_other_organizations() {
    let (service, db, _dir) = test_service().await;
    seed_client_manager(&db, "org-a").await;
    seed_client_manager(&db, "org-b").await;
    let app = api::build_router(service);

    let create = app
        .clone()
        .oneshot(app_request(
            Method::POST,
            "/api/v1/organizations/org-a/clients",
            "bootstrap-user",
            "org-a",
            Some(json!({
                "id": "client-owned-by-a",
                "slug": "owned-by-a",
                "name": "Owned by A",
                "clientType": "confidential",
                "appType": "web",
                "firstParty": false,
                "consentPolicy": "explicit",
                "active": true
            })),
        ))
        .await
        .expect("create response");
    assert_eq!(create.status(), StatusCode::CREATED);

    let get = app
        .clone()
        .oneshot(app_request(
            Method::GET,
            "/api/v1/organizations/org-b/clients/client-owned-by-a",
            "bootstrap-user",
            "org-b",
            None,
        ))
        .await
        .expect("get response");
    assert_eq!(get.status(), StatusCode::NOT_FOUND);

    let replace_actions = app
        .oneshot(app_request(
            Method::PUT,
            "/api/v1/organizations/org-b/clients/client-owned-by-a/actions",
            "bootstrap-user",
            "org-b",
            Some(json!({
                "actionIds": ["plant.profile.read"]
            })),
        ))
        .await
        .expect("replace response");
    assert_eq!(replace_actions.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn client_routes_forbid_path_organization_mismatch() {
    let (service, db, _dir) = test_service().await;
    seed_client_manager(&db, "org-a").await;
    seed_client_manager(&db, "org-b").await;
    let app = api::build_router(service);

    let response = app
        .oneshot(app_request(
            Method::GET,
            "/api/v1/organizations/org-a/clients",
            "bootstrap-user",
            "org-b",
            None,
        ))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn client_routes_reject_invalid_action_ids_as_bad_request() {
    let (service, db, _dir) = test_service().await;
    seed_client_manager(&db, "creator-org").await;
    let app = api::build_router(service);

    let create = app
        .clone()
        .oneshot(app_request(
            Method::POST,
            "/api/v1/organizations/creator-org/clients",
            "bootstrap-user",
            "creator-org",
            Some(json!({
                "id": "client-invalid-actions",
                "slug": "invalid-actions",
                "name": "Invalid Actions",
                "clientType": "public",
                "appType": "web",
                "firstParty": false,
                "consentPolicy": "explicit",
                "active": true
            })),
        ))
        .await
        .expect("create response");
    assert_eq!(create.status(), StatusCode::CREATED);

    let response = app
        .oneshot(app_request(
            Method::PUT,
            "/api/v1/organizations/creator-org/clients/client-invalid-actions/actions",
            "bootstrap-user",
            "creator-org",
            Some(json!({
                "actionIds": ["organization.member.manage", ""]
            })),
        ))
        .await
        .expect("replace response");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

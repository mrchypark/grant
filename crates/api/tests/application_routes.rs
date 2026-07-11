mod support;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
};
use domain::{
    Application, ApplicationAppType, ApplicationConsentPolicy, ApplicationType, Role, RoleBinding,
    RoleBindingState, RoleBindingSubjectType,
};
use hiqlite::{Node, NodeConfig, s3::EncKeys};
use http_body_util::BodyExt;
use infra::{
    config::{
        AppConfig, HiqliteConfig, LocalNodeRole, ReadConsistency, RuntimeMode, SecurityConfig,
        ServerConfig,
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
    let cfg = NodeConfig {
        node_id: 1,
        nodes: vec![Node {
            id: 1,
            addr_raft: raft_addr.clone(),
            addr_api: api_addr.clone(),
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

async fn test_service() -> (Arc<AppService>, Database, TempDir) {
    let rauthy_base_url = support::start_mock_rauthy().await;
    let (runtime, dir) = test_runtime().await;
    let app_cfg = AppConfig {
        server: ServerConfig {
            bind_addr: free_local_addr(),
        },
        security: SecurityConfig {
            master_key: "dev-only-master-key".to_owned(),
            bootstrap_user_id: "bootstrap-user".to_owned(),
            bootstrap_user_name: "Bootstrap User".to_owned(),
        },
        auth: support::rauthy_auth_config(rauthy_base_url),
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
    let service = Arc::new(AppService::new(app_cfg, db));
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
        .header("authorization", support::auth::bearer_header(token))
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

async fn seed_application_manager(db: &Database, organization_id: &str) {
    db.upsert_organization_with_owner(organization_id, "Creator Org", None, "Creator Org")
        .await
        .expect("creator org");
    db.upsert_role(&Role {
        id: "role-org-application-manage".to_owned(),
        organization_id: None,
        code: "organization.application.manage".to_owned(),
        name: "Org Application Manage".to_owned(),
        description: None,
    })
    .await
    .expect("upsert application manage role");
    db.upsert_role_action(
        "role-org-application-manage",
        "organization.application.manage",
    )
    .await
    .expect("upsert application manage action");
    db.save_role_binding(&RoleBinding {
        id: Uuid::new_v4().simple().to_string(),
        subject_type: RoleBindingSubjectType::User,
        subject_id: "bootstrap-user".to_owned(),
        subject_organization_id: organization_id.to_owned(),
        provider_id: "grant".to_owned(),
        resource_type: "organization".to_owned(),
        resource_id: organization_id.to_owned(),
        role_id: "role-org-application-manage".to_owned(),
        agreement_id: None,
        state: RoleBindingState::Active,
        created_by_user_id: "bootstrap-user".to_owned(),
        created_at: "2026-04-12T00:00:00Z".to_owned(),
        updated_at: "2026-04-12T00:00:00Z".to_owned(),
    })
    .await
    .expect("save application manage binding");
}

async fn create_application_via_api(
    app: axum::Router,
    organization_id: &str,
    application_id: &str,
) {
    create_application_with_type_via_api(app, organization_id, application_id, "confidential")
        .await;
}

async fn create_application_with_type_via_api(
    app: axum::Router,
    organization_id: &str,
    application_id: &str,
    application_type: &str,
) {
    let response = app
        .oneshot(app_request(
            Method::POST,
            &format!("/api/v1/orgs/{organization_id}/apps"),
            "bootstrap-user",
            organization_id,
            Some(json!({
                "id": application_id,
                "slug": application_id.trim_start_matches("application-"),
                "name": application_id,
                "applicationType": application_type,
                "appType": "web",
                "firstParty": false,
                "consentPolicy": "explicit",
                "active": true
            })),
        ))
        .await
        .expect("create application response");
    assert_eq!(response.status(), StatusCode::CREATED);
}

async fn seed_application_without_manage(
    db: &Database,
    organization_id: &str,
    application_id: &str,
) {
    db.upsert_organization_with_owner(organization_id, organization_id, None, organization_id)
        .await
        .expect("organization");
    db.insert_application(&Application {
        id: application_id.to_owned(),
        organization_id: organization_id.to_owned(),
        slug: application_id.trim_start_matches("application-").to_owned(),
        name: application_id.to_owned(),
        description: None,
        application_type: ApplicationType::Confidential,
        app_type: ApplicationAppType::Web,
        first_party: false,
        consent_policy: ApplicationConsentPolicy::Explicit,
        active: true,
        rauthy_client_id: None,
        secret_rotated_at: None,
        created_by_user_id: "bootstrap-user".to_owned(),
    })
    .await
    .expect("application");
}

#[tokio::test]
async fn public_application_sync_excludes_client_credentials_flow() {
    let (service, db, _dir) = test_service().await;
    seed_application_manager(&db, "creator-org").await;
    let app = api::build_router(service);
    let application_id = "application-public-client";
    create_application_with_type_via_api(app.clone(), "creator-org", application_id, "public")
        .await;

    for redirect_uri in [
        "https://public.example/callback",
        "https://public.example/second-callback",
    ] {
        let response = app
            .clone()
            .oneshot(app_request(
                Method::POST,
                &format!("/api/v1/orgs/creator-org/apps/{application_id}/redirect-uris"),
                "bootstrap-user",
                "creator-org",
                Some(json!({ "redirectUri": redirect_uri })),
            ))
            .await
            .expect("create public redirect uri response");
        assert_eq!(response.status(), StatusCode::CREATED);
    }
}

#[tokio::test]
async fn application_redirect_uri_routes_create_list_and_delete() {
    let (service, db, _dir) = test_service().await;
    seed_application_manager(&db, "creator-org").await;
    let app = api::build_router(service);
    create_application_via_api(app.clone(), "creator-org", "application-developer-console").await;

    let create = app
        .clone()
        .oneshot(app_request(
            Method::POST,
            "/api/v1/orgs/creator-org/apps/application-developer-console/redirect-uris",
            "bootstrap-user",
            "creator-org",
            Some(json!({ "redirectUri": "https://client.example/callback" })),
        ))
        .await
        .expect("create redirect uri response");
    assert_eq!(create.status(), StatusCode::CREATED);
    let create_body = create.into_body().collect().await.expect("body").to_bytes();
    let created: Value = serde_json::from_slice(&create_body).expect("json");
    assert_eq!(created["applicationId"], "application-developer-console");
    assert_eq!(created["redirectUri"], "https://client.example/callback");

    let duplicate = app
        .clone()
        .oneshot(app_request(
            Method::POST,
            "/api/v1/orgs/creator-org/apps/application-developer-console/redirect-uris",
            "bootstrap-user",
            "creator-org",
            Some(json!({ "redirectUri": "https://client.example/callback" })),
        ))
        .await
        .expect("duplicate redirect uri response");
    assert_eq!(duplicate.status(), StatusCode::CONFLICT);

    let list = app
        .clone()
        .oneshot(app_request(
            Method::GET,
            "/api/v1/orgs/creator-org/apps/application-developer-console/redirect-uris",
            "bootstrap-user",
            "creator-org",
            None,
        ))
        .await
        .expect("list redirect uri response");
    assert_eq!(list.status(), StatusCode::OK);
    let list_body = list.into_body().collect().await.expect("body").to_bytes();
    let listed: Value = serde_json::from_slice(&list_body).expect("json");
    assert_eq!(listed.as_array().map(Vec::len), Some(1));

    let create_second = app
        .clone()
        .oneshot(app_request(
            Method::POST,
            "/api/v1/orgs/creator-org/apps/application-developer-console/redirect-uris",
            "bootstrap-user",
            "creator-org",
            Some(json!({ "redirectUri": "https://client.example/second-callback" })),
        ))
        .await
        .expect("create second redirect uri response");
    assert_eq!(create_second.status(), StatusCode::CREATED);

    let delete = app
        .clone()
        .oneshot(app_request(
            Method::DELETE,
            "/api/v1/orgs/creator-org/apps/application-developer-console/redirect-uris",
            "bootstrap-user",
            "creator-org",
            Some(json!({ "redirectUri": "https://client.example/callback" })),
        ))
        .await
        .expect("delete redirect uri response");
    assert_eq!(delete.status(), StatusCode::NO_CONTENT);

    let list_after_delete = app
        .oneshot(app_request(
            Method::GET,
            "/api/v1/orgs/creator-org/apps/application-developer-console/redirect-uris",
            "bootstrap-user",
            "creator-org",
            None,
        ))
        .await
        .expect("list after delete response");
    assert_eq!(list_after_delete.status(), StatusCode::OK);
    let list_after_delete_body = list_after_delete
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let listed_after_delete: Value = serde_json::from_slice(&list_after_delete_body).expect("json");
    assert_eq!(listed_after_delete.as_array().map(Vec::len), Some(1));
    assert_eq!(
        listed_after_delete[0]["redirectUri"],
        "https://client.example/second-callback"
    );
}

#[tokio::test]
async fn application_redirect_uri_routes_reject_invalid_redirect_uris() {
    let (service, db, _dir) = test_service().await;
    seed_application_manager(&db, "creator-org").await;
    let app = api::build_router(service);
    create_application_via_api(app.clone(), "creator-org", "application-invalid-redirect").await;

    for redirect_uri in [
        "ftp://client.example/callback",
        "https:///callback",
        "https://client.example:bad/callback",
        "https://[::1/callback",
        "https://client.example/callback#fragment",
        "http://",
    ] {
        let response = app
            .clone()
            .oneshot(app_request(
                Method::POST,
                "/api/v1/orgs/creator-org/apps/application-invalid-redirect/redirect-uris",
                "bootstrap-user",
                "creator-org",
                Some(json!({ "redirectUri": redirect_uri })),
            ))
            .await
            .expect("invalid redirect uri response");
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "redirect URI should be rejected: {redirect_uri}"
        );
    }
}

#[tokio::test]
async fn application_redirect_uri_routes_canonicalize_before_duplicate_check() {
    let (service, db, _dir) = test_service().await;
    seed_application_manager(&db, "creator-org").await;
    let app = api::build_router(service);
    create_application_via_api(app.clone(), "creator-org", "application-canonical-redirect").await;

    let create = app
        .clone()
        .oneshot(app_request(
            Method::POST,
            "/api/v1/orgs/creator-org/apps/application-canonical-redirect/redirect-uris",
            "bootstrap-user",
            "creator-org",
            Some(json!({ "redirectUri": "https://CLIENT.EXAMPLE" })),
        ))
        .await
        .expect("create canonical redirect uri response");
    assert_eq!(create.status(), StatusCode::CREATED);
    let create_body = create.into_body().collect().await.expect("body").to_bytes();
    let created: Value = serde_json::from_slice(&create_body).expect("json");
    assert_eq!(created["redirectUri"], "https://client.example/");

    let duplicate = app
        .oneshot(app_request(
            Method::POST,
            "/api/v1/orgs/creator-org/apps/application-canonical-redirect/redirect-uris",
            "bootstrap-user",
            "creator-org",
            Some(json!({ "redirectUri": "https://client.example/" })),
        ))
        .await
        .expect("duplicate canonical redirect uri response");
    assert_eq!(duplicate.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn application_update_sync_failure_rolls_back_local_state() {
    let (service, db, _dir) = test_service().await;
    seed_application_manager(&db, "creator-org").await;
    let app = api::build_router(service);
    let application_id = "application-sync-rollback";
    create_application_via_api(app.clone(), "creator-org", application_id).await;

    let redirect = app
        .clone()
        .oneshot(app_request(
            Method::POST,
            &format!("/api/v1/orgs/creator-org/apps/{application_id}/redirect-uris"),
            "bootstrap-user",
            "creator-org",
            Some(json!({ "redirectUri": "https://client.example/callback" })),
        ))
        .await
        .expect("create redirect uri response");
    assert_eq!(redirect.status(), StatusCode::CREATED);

    let update = app
        .oneshot(app_request(
            Method::PUT,
            &format!("/api/v1/orgs/creator-org/apps/{application_id}"),
            "bootstrap-user",
            "creator-org",
            Some(json!({
                "slug": "sync-rollback",
                "name": "reject-sync",
                "applicationType": "confidential",
                "appType": "web",
                "firstParty": false,
                "consentPolicy": "explicit",
                "active": false
            })),
        ))
        .await
        .expect("update response");
    assert_eq!(update.status(), StatusCode::SERVICE_UNAVAILABLE);

    let stored = db
        .get_application_with_consistency(application_id, Some(ReadConsistency::Strong))
        .await
        .expect("stored application");
    assert_eq!(stored.name, application_id);
    assert!(stored.active);
}

#[tokio::test]
async fn application_client_secret_rotate_returns_secret_and_updates_application() {
    let (service, db, _dir) = test_service().await;
    seed_application_manager(&db, "creator-org").await;
    let app = api::build_router(service);
    create_application_via_api(app.clone(), "creator-org", "application-secret-rotation").await;

    let redirect = app
        .clone()
        .oneshot(app_request(
            Method::POST,
            "/api/v1/orgs/creator-org/apps/application-secret-rotation/redirect-uris",
            "bootstrap-user",
            "creator-org",
            Some(json!({ "redirectUri": "https://client.example/callback" })),
        ))
        .await
        .expect("create redirect uri response");
    assert_eq!(redirect.status(), StatusCode::CREATED);

    let rotate = app
        .clone()
        .oneshot(app_request(
            Method::POST,
            "/api/v1/orgs/creator-org/apps/application-secret-rotation/client-secret/rotate",
            "bootstrap-user",
            "creator-org",
            None,
        ))
        .await
        .expect("rotate response");
    let rotate_status = rotate.status();
    let rotate_body = rotate.into_body().collect().await.expect("body").to_bytes();
    let rotated: Value = serde_json::from_slice(&rotate_body).expect("json");
    assert_eq!(
        rotate_status,
        StatusCode::OK,
        "rotate response body: {rotated}"
    );
    assert!(
        rotated["clientSecret"]
            .as_str()
            .is_some_and(|value| value == "rotated-application-secret-rotation")
    );
    let rotated_at = rotated["secretRotatedAt"]
        .as_str()
        .expect("secretRotatedAt")
        .to_owned();

    let get = app
        .oneshot(app_request(
            Method::GET,
            "/api/v1/orgs/creator-org/apps/application-secret-rotation",
            "bootstrap-user",
            "creator-org",
            None,
        ))
        .await
        .expect("get response");
    assert_eq!(get.status(), StatusCode::OK);
    let get_body = get.into_body().collect().await.expect("body").to_bytes();
    let application: Value = serde_json::from_slice(&get_body).expect("json");
    assert_eq!(application["secretRotatedAt"], rotated_at);
}

#[tokio::test]
async fn application_client_secret_rotate_requires_provisioned_rauthy_client() {
    let (service, db, _dir) = test_service().await;
    seed_application_manager(&db, "creator-org").await;
    let app = api::build_router(service);
    create_application_via_api(
        app.clone(),
        "creator-org",
        "application-secret-not-provisioned",
    )
    .await;

    let rotate = app
        .oneshot(app_request(
            Method::POST,
            "/api/v1/orgs/creator-org/apps/application-secret-not-provisioned/client-secret/rotate",
            "bootstrap-user",
            "creator-org",
            None,
        ))
        .await
        .expect("rotate response");
    assert_eq!(rotate.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn application_developer_console_routes_require_manage_permission() {
    let (service, db, _dir) = test_service().await;
    seed_application_without_manage(&db, "creator-org", "application-forbidden-console").await;
    let app = api::build_router(service);

    let redirect = app
        .clone()
        .oneshot(app_request(
            Method::POST,
            "/api/v1/orgs/creator-org/apps/application-forbidden-console/redirect-uris",
            "bootstrap-user",
            "creator-org",
            Some(json!({ "redirectUri": "https://client.example/callback" })),
        ))
        .await
        .expect("redirect response");
    assert_eq!(redirect.status(), StatusCode::FORBIDDEN);

    let rotate = app
        .oneshot(app_request(
            Method::POST,
            "/api/v1/orgs/creator-org/apps/application-forbidden-console/client-secret/rotate",
            "bootstrap-user",
            "creator-org",
            None,
        ))
        .await
        .expect("rotate response");
    assert_eq!(rotate.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn application_routes_create_list_get_and_replace_actions() {
    let (service, db, _dir) = test_service().await;
    seed_application_manager(&db, "creator-org").await;
    let app = api::build_router(service);

    let create = app
        .clone()
        .oneshot(app_request(
            Method::POST,
            "/api/v1/orgs/creator-org/apps",
            "bootstrap-user",
            "creator-org",
            Some(json!({
                "id": "application-awesome-dashboard",
                "slug": "awesome-dashboard",
                "name": "Awesome Dashboard",
                "description": "External analytics dashboard",
                "applicationType": "confidential",
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
    assert_eq!(payload["id"], "application-awesome-dashboard");
    assert_eq!(payload["organizationId"], "creator-org");
    assert_eq!(payload["rauthyClientId"], "application-awesome-dashboard");
    assert_eq!(payload["createdByUserId"], "bootstrap-user");

    let list = app
        .clone()
        .oneshot(app_request(
            Method::GET,
            "/api/v1/orgs/creator-org/apps",
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
            "/api/v1/orgs/creator-org/apps/application-awesome-dashboard",
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
            "/api/v1/orgs/creator-org/apps/application-awesome-dashboard",
            "bootstrap-user",
            "creator-org",
            Some(json!({
                "slug": "awesome-dashboard",
                "name": "Awesome Dashboard v2",
                "description": "Updated external analytics dashboard",
                "applicationType": "confidential",
                "appType": "web",
                "firstParty": false,
                "consentPolicy": "explicit",
                "active": true
            })),
        ))
        .await
        .expect("update response");
    let update_status = update.status();
    let update_body = update.into_body().collect().await.expect("body").to_bytes();
    let update_payload: Value = serde_json::from_slice(&update_body).expect("json");
    assert_eq!(
        update_status,
        StatusCode::OK,
        "update response body: {update_payload}"
    );
    assert_eq!(update_payload["name"], "Awesome Dashboard v2");

    let replace_actions = app
        .clone()
        .oneshot(app_request(
            Method::PUT,
            "/api/v1/orgs/creator-org/apps/application-awesome-dashboard/actions",
            "bootstrap-user",
            "creator-org",
            Some(json!({
                "actionIds": ["organization.profile.read"]
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
    assert_eq!(action_codes, vec!["organization.profile.read"]);

    let get_actions = app
        .oneshot(app_request(
            Method::GET,
            "/api/v1/orgs/creator-org/apps/application-awesome-dashboard/actions",
            "bootstrap-user",
            "creator-org",
            None,
        ))
        .await
        .expect("get actions response");
    assert_eq!(get_actions.status(), StatusCode::OK);
}

#[tokio::test]
async fn application_grant_routes_create_list_replace_actions_and_revoke() {
    let (service, db, _dir) = test_service().await;
    seed_application_manager(&db, "creator-org").await;
    let app = api::build_router(service);

    let create_app = app
        .clone()
        .oneshot(app_request(
            Method::POST,
            "/api/v1/orgs/creator-org/apps",
            "bootstrap-user",
            "creator-org",
            Some(json!({
                "id": "application-grant-target",
                "slug": "grant-target",
                "name": "Grant Target",
                "description": null,
                "applicationType": "confidential",
                "appType": "web",
                "firstParty": false,
                "consentPolicy": "explicit",
                "active": true
            })),
        ))
        .await
        .expect("create app response");
    assert_eq!(create_app.status(), StatusCode::CREATED);

    let replace_app_actions = app
        .clone()
        .oneshot(app_request(
            Method::PUT,
            "/api/v1/orgs/creator-org/apps/application-grant-target/actions",
            "bootstrap-user",
            "creator-org",
            Some(json!({
                "actionIds": ["organization.profile.read"]
            })),
        ))
        .await
        .expect("replace app actions response");
    assert_eq!(replace_app_actions.status(), StatusCode::OK);

    let invalid_grant = app
        .clone()
        .oneshot(app_request(
            Method::POST,
            "/api/v1/orgs/creator-org/app-grants",
            "bootstrap-user",
            "creator-org",
            Some(json!({
                "id": "grant-invalid-action",
                "userId": "bootstrap-user",
                "applicationId": "application-grant-target",
                "actionIds": ["organization.permission.manage"]
            })),
        ))
        .await
        .expect("invalid grant response");
    assert_eq!(invalid_grant.status(), StatusCode::BAD_REQUEST);
    let list_after_invalid = app
        .clone()
        .oneshot(app_request(
            Method::GET,
            "/api/v1/orgs/creator-org/app-grants",
            "bootstrap-user",
            "creator-org",
            None,
        ))
        .await
        .expect("list after invalid response");
    assert_eq!(list_after_invalid.status(), StatusCode::OK);
    let list_after_invalid_body = list_after_invalid
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let listed_after_invalid: Value =
        serde_json::from_slice(&list_after_invalid_body).expect("json");
    assert_eq!(listed_after_invalid.as_array().map(Vec::len), Some(0));

    let create_grant = app
        .clone()
        .oneshot(app_request(
            Method::POST,
            "/api/v1/orgs/creator-org/app-grants",
            "bootstrap-user",
            "creator-org",
            Some(json!({
                "id": "grant-bootstrap-application",
                "userId": "bootstrap-user",
                "applicationId": "application-grant-target",
                "actionIds": ["organization.profile.read"]
            })),
        ))
        .await
        .expect("create grant response");
    let create_grant_status = create_grant.status();
    let create_grant_body = create_grant
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let create_grant_payload: Value = serde_json::from_slice(&create_grant_body).expect("json");
    assert_eq!(
        create_grant_status,
        StatusCode::CREATED,
        "create grant response body: {create_grant_payload}"
    );

    let list = app
        .clone()
        .oneshot(app_request(
            Method::GET,
            "/api/v1/orgs/creator-org/app-grants",
            "bootstrap-user",
            "creator-org",
            None,
        ))
        .await
        .expect("list grants response");
    assert_eq!(list.status(), StatusCode::OK);
    let list_body = list.into_body().collect().await.expect("body").to_bytes();
    let listed: Value = serde_json::from_slice(&list_body).expect("json");
    assert_eq!(listed.as_array().map(Vec::len), Some(1));

    let replace_grant_actions = app
        .clone()
        .oneshot(app_request(
            Method::PUT,
            "/api/v1/orgs/creator-org/app-grants/grant-bootstrap-application/actions",
            "bootstrap-user",
            "creator-org",
            Some(json!({
                "actionIds": ["organization.profile.read"]
            })),
        ))
        .await
        .expect("replace grant actions response");
    assert_eq!(replace_grant_actions.status(), StatusCode::OK);

    let revoke = app
        .oneshot(app_request(
            Method::POST,
            "/api/v1/orgs/creator-org/app-grants/grant-bootstrap-application/revoke",
            "bootstrap-user",
            "creator-org",
            None,
        ))
        .await
        .expect("revoke response");
    assert_eq!(revoke.status(), StatusCode::OK);
    let revoke_body = revoke.into_body().collect().await.expect("body").to_bytes();
    let revoked: Value = serde_json::from_slice(&revoke_body).expect("json");
    assert_eq!(revoked["status"], "revoked");
}

#[tokio::test]
async fn application_routes_require_organization_scoped_manage_permission() {
    let (service, db, _dir) = test_service().await;
    db.upsert_organization_with_owner("creator-org", "Creator Org", None, "Creator Org")
        .await
        .expect("creator org");
    let app = api::build_router(service);

    let response = app
        .oneshot(app_request(
            Method::POST,
            "/api/v1/orgs/creator-org/apps",
            "bootstrap-user",
            "creator-org",
            Some(json!({
                "id": "application-denied",
                "slug": "denied",
                "name": "Denied",
                "applicationType": "public",
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
async fn application_create_cannot_rehome_existing_application_to_another_organization() {
    let (service, db, _dir) = test_service().await;
    seed_application_manager(&db, "org-a").await;
    seed_application_manager(&db, "org-b").await;
    let app = api::build_router(service);

    let first = app
        .clone()
        .oneshot(app_request(
            Method::POST,
            "/api/v1/orgs/org-a/apps",
            "bootstrap-user",
            "org-a",
            Some(json!({
                "id": "application-shared",
                "slug": "shared-application",
                "name": "Shared Application",
                "applicationType": "confidential",
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
            "/api/v1/orgs/org-b/apps",
            "bootstrap-user",
            "org-b",
            Some(json!({
                "id": "application-shared",
                "slug": "shared-application-org-b",
                "name": "Hijack Attempt",
                "applicationType": "confidential",
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
        .get_application_with_consistency("application-shared", None)
        .await
        .expect("stored application");
    assert_eq!(stored.organization_id, "org-a");
    assert_eq!(stored.slug, "shared-application");
}

#[tokio::test]
async fn application_create_reports_duplicate_slug_as_conflict() {
    let (service, db, _dir) = test_service().await;
    seed_application_manager(&db, "org-a").await;
    seed_application_manager(&db, "org-b").await;
    let app = api::build_router(service);

    let first = app
        .clone()
        .oneshot(app_request(
            Method::POST,
            "/api/v1/orgs/org-a/apps",
            "bootstrap-user",
            "org-a",
            Some(json!({
                "id": "application-a",
                "slug": "shared-slug",
                "name": "Application A",
                "applicationType": "public",
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
            "/api/v1/orgs/org-b/apps",
            "bootstrap-user",
            "org-b",
            Some(json!({
                "id": "application-b",
                "slug": "shared-slug",
                "name": "Application B",
                "applicationType": "public",
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
async fn application_update_reports_duplicate_slug_as_conflict() {
    let (service, db, _dir) = test_service().await;
    seed_application_manager(&db, "org-a").await;
    let app = api::build_router(service);

    for (id, slug, name) in [
        ("application-a", "slug-a", "Application A"),
        ("application-b", "slug-b", "Application B"),
    ] {
        let response = app
            .clone()
            .oneshot(app_request(
                Method::POST,
                "/api/v1/orgs/org-a/apps",
                "bootstrap-user",
                "org-a",
                Some(json!({
                    "id": id,
                    "slug": slug,
                    "name": name,
                    "applicationType": "public",
                    "appType": "web",
                    "firstParty": false,
                    "consentPolicy": "explicit",
                    "active": true
                })),
            ))
            .await
            .expect("create response");
        assert_eq!(response.status(), StatusCode::CREATED);
    }

    let update = app
        .oneshot(app_request(
            Method::PUT,
            "/api/v1/orgs/org-a/apps/application-b",
            "bootstrap-user",
            "org-a",
            Some(json!({
                "slug": "slug-a",
                "name": "Application B Updated",
                "applicationType": "public",
                "appType": "web",
                "firstParty": false,
                "consentPolicy": "explicit",
                "active": true
            })),
        ))
        .await
        .expect("update response");
    assert_eq!(update.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn replace_application_actions_rejects_requests_over_sqlite_parameter_limit() {
    let (service, db, _dir) = test_service().await;
    seed_application_manager(&db, "creator-org").await;
    let app = api::build_router(service);

    let create = app
        .clone()
        .oneshot(app_request(
            Method::POST,
            "/api/v1/orgs/creator-org/apps",
            "bootstrap-user",
            "creator-org",
            Some(json!({
                "id": "application-action-limit",
                "slug": "action-limit",
                "name": "Action Limit",
                "applicationType": "confidential",
                "appType": "web",
                "firstParty": false,
                "consentPolicy": "explicit",
                "active": true
            })),
        ))
        .await
        .expect("create response");
    assert_eq!(create.status(), StatusCode::CREATED);

    let replace = app
        .oneshot(app_request(
            Method::PUT,
            "/api/v1/orgs/creator-org/apps/application-action-limit/actions",
            "bootstrap-user",
            "creator-org",
            Some(json!({
                "actionIds": (0..1000).map(|index| format!("action-{index}")).collect::<Vec<_>>()
            })),
        ))
        .await
        .expect("replace response");
    assert_eq!(replace.status(), StatusCode::BAD_REQUEST);
    let body = replace
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let payload: Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(payload["error"], "actionIds must contain at most 999 items");
}

#[tokio::test]
async fn application_routes_hide_applications_owned_by_other_organizations() {
    let (service, db, _dir) = test_service().await;
    seed_application_manager(&db, "org-a").await;
    seed_application_manager(&db, "org-b").await;
    let app = api::build_router(service);

    let create = app
        .clone()
        .oneshot(app_request(
            Method::POST,
            "/api/v1/orgs/org-a/apps",
            "bootstrap-user",
            "org-a",
            Some(json!({
                "id": "application-owned-by-a",
                "slug": "owned-by-a",
                "name": "Owned by A",
                "applicationType": "confidential",
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
            "/api/v1/orgs/org-b/apps/application-owned-by-a",
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
            "/api/v1/orgs/org-b/apps/application-owned-by-a/actions",
            "bootstrap-user",
            "org-b",
            Some(json!({
                "actionIds": ["organization.profile.read"]
            })),
        ))
        .await
        .expect("replace response");
    assert_eq!(replace_actions.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn application_routes_forbid_path_organization_mismatch() {
    let (service, db, _dir) = test_service().await;
    seed_application_manager(&db, "org-a").await;
    seed_application_manager(&db, "org-b").await;
    let app = api::build_router(service);

    let response = app
        .oneshot(app_request(
            Method::GET,
            "/api/v1/orgs/org-a/apps",
            "bootstrap-user",
            "org-b",
            None,
        ))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn application_routes_reject_invalid_action_ids_as_bad_request() {
    let (service, db, _dir) = test_service().await;
    seed_application_manager(&db, "creator-org").await;
    let app = api::build_router(service);

    let create = app
        .clone()
        .oneshot(app_request(
            Method::POST,
            "/api/v1/orgs/creator-org/apps",
            "bootstrap-user",
            "creator-org",
            Some(json!({
                "id": "application-invalid-actions",
                "slug": "invalid-actions",
                "name": "Invalid Actions",
                "applicationType": "public",
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
            "/api/v1/orgs/creator-org/apps/application-invalid-actions/actions",
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

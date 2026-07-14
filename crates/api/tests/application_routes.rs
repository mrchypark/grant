mod support;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
};
use domain::{
    Action, ActionRiskLevel, Application, ApplicationAppType, ApplicationConsentPolicy,
    ApplicationType, Role, RoleBinding, RoleBindingState, RoleBindingSubjectType, ServiceProvider,
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

static HIQLITE_START_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn free_local_addrs() -> (String, String) {
    let api_listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral API port");
    let raft_listener =
        std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral Raft port");
    let api_addr = api_listener.local_addr().expect("API local addr");
    let raft_addr = raft_listener.local_addr().expect("Raft local addr");
    (api_addr.to_string(), raft_addr.to_string())
}

async fn test_runtime() -> (HiqliteRuntime, TempDir) {
    let _start_guard = HIQLITE_START_LOCK.lock().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let (api_addr, raft_addr) = free_local_addrs();
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

    let application =
        tokio::time::timeout(std::time::Duration::from_secs(30), hiqlite::start_node(cfg))
            .await
            .expect("start hiqlite within 30 seconds")
            .expect("start hiqlite");
    (
        HiqliteRuntime::embedded(application, ReadConsistency::Local),
        dir,
    )
}

async fn test_service_with_rauthy_url() -> (Arc<AppService>, Database, TempDir, String) {
    let rauthy_base_url = support::start_mock_rauthy().await;
    let (runtime, dir) = test_runtime().await;
    let app_cfg = AppConfig {
        server: ServerConfig {
            bind_addr: support::free_local_addr(),
        },
        security: SecurityConfig {
            master_key: "dev-only-master-key".to_owned(),
            app_auth_key: "dev-only-app-auth-key".to_owned(),
            bootstrap_user_id: "bootstrap-user".to_owned(),
            bootstrap_user_name: "Bootstrap User".to_owned(),
        },
        auth: support::rauthy_auth_config(rauthy_base_url.clone()),
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
    (service, db_for_tests, dir, rauthy_base_url)
}

async fn test_service() -> (Arc<AppService>, Database, TempDir) {
    let (service, db, dir, _) = test_service_with_rauthy_url().await;
    (service, db, dir)
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

#[tokio::test]
async fn application_create_reserves_generated_client_until_master_verifies_external_provisioning()
{
    let (service, db, _dir, rauthy_base_url) = test_service_with_rauthy_url().await;
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
                "id": "application-reservation",
                "slug": "reservation",
                "name": "Reserved Client",
                "description": null,
                "applicationType": "confidential",
                "appType": "web",
                "active": true
            })),
        ))
        .await
        .expect("create reservation response");
    assert_eq!(create.status(), StatusCode::CREATED);
    let body = create.into_body().collect().await.expect("body").to_bytes();
    let reservation: Value = serde_json::from_slice(&body).expect("reservation JSON");
    assert_eq!(reservation["provisioningStatus"], "unprovisioned");
    let client_id = reservation["rauthyClientId"]
        .as_str()
        .expect("generated client id");
    assert!(client_id.starts_with("grant-"));
    assert_ne!(client_id, "application-reservation");

    let external_provision = reqwest::Client::new()
        .post(format!("{rauthy_base_url}/auth/v1/clients"))
        .header("authorization", "API-Key grant-rauthy-api-key")
        .json(&json!({
            "id": client_id,
            "name": "Reserved Client",
            "confidential": true,
            "redirect_uris": []
        }))
        .send()
        .await
        .expect("external IaC provisioning");
    assert!(external_provision.status().is_success());

    let verify = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/admin/orgs/creator-org/apps/application-reservation/provisioning/verify")
                .header("x-grant-master-key", "dev-only-master-key")
                .body(Body::empty())
                .expect("verify request"),
        )
        .await
        .expect("verify response");
    assert_eq!(verify.status(), StatusCode::OK);
    let body = verify.into_body().collect().await.expect("body").to_bytes();
    let verified: Value = serde_json::from_slice(&body).expect("verified JSON");
    assert_eq!(verified["provisioningStatus"], "ready");
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
    assert!(
        payload["rauthyClientId"]
            .as_str()
            .is_some_and(|client_id| client_id.starts_with("grant-"))
    );
    assert_eq!(payload["provisioningStatus"], "unprovisioned");
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
    assert_eq!(update.status(), StatusCode::METHOD_NOT_ALLOWED);

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
async fn application_self_consent_replaces_and_revokes_while_admin_mutations_are_absent() {
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

    let invalid_consent = app
        .clone()
        .oneshot(app_request(
            Method::PUT,
            "/api/v1/orgs/creator-org/apps/application-grant-target/consent",
            "bootstrap-user",
            "creator-org",
            Some(json!({
                "actionIds": []
            })),
        ))
        .await
        .expect("invalid consent response");
    assert_eq!(invalid_consent.status(), StatusCode::BAD_REQUEST);

    let consent_request = || {
        app_request(
            Method::PUT,
            "/api/v1/orgs/creator-org/apps/application-grant-target/consent",
            "bootstrap-user",
            "creator-org",
            Some(json!({
                "actionIds": ["organization.profile.read"]
            })),
        )
    };
    let (first_consent, second_consent) = tokio::join!(
        app.clone().oneshot(consent_request()),
        app.clone().oneshot(consent_request())
    );
    let first_consent = first_consent.expect("first concurrent self consent response");
    let second_consent = second_consent.expect("second concurrent self consent response");
    assert_eq!(first_consent.status(), StatusCode::OK);
    assert_eq!(second_consent.status(), StatusCode::OK);
    let first_consent_body = first_consent
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let second_consent_body = second_consent
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let consent: Value = serde_json::from_slice(&first_consent_body).expect("json");
    let concurrent_consent: Value = serde_json::from_slice(&second_consent_body).expect("json");
    assert_eq!(consent["grant"]["userId"], "bootstrap-user");
    assert_eq!(consent["actions"].as_array().map(Vec::len), Some(1));
    assert_eq!(consent["grant"]["id"], concurrent_consent["grant"]["id"]);

    let get_consent = app
        .clone()
        .oneshot(app_request(
            Method::GET,
            "/api/v1/orgs/creator-org/apps/application-grant-target/consent",
            "bootstrap-user",
            "creator-org",
            None,
        ))
        .await
        .expect("get self consent response");
    assert_eq!(get_consent.status(), StatusCode::OK);

    let old_admin_mutation = app
        .clone()
        .oneshot(app_request(
            Method::POST,
            "/api/v1/orgs/creator-org/app-grants",
            "bootstrap-user",
            "creator-org",
            Some(json!({
                "id": "admin-grant",
                "userId": "someone-else",
                "applicationId": "application-grant-target",
                "actionIds": ["organization.profile.read"]
            })),
        ))
        .await
        .expect("old admin mutation response");
    assert_eq!(old_admin_mutation.status(), StatusCode::METHOD_NOT_ALLOWED);

    let revoke = app
        .clone()
        .oneshot(app_request(
            Method::DELETE,
            "/api/v1/orgs/creator-org/apps/application-grant-target/consent",
            "bootstrap-user",
            "creator-org",
            None,
        ))
        .await
        .expect("revoke self consent response");
    assert_eq!(revoke.status(), StatusCode::NO_CONTENT);

    let revoke_again = app
        .oneshot(app_request(
            Method::DELETE,
            "/api/v1/orgs/creator-org/apps/application-grant-target/consent",
            "bootstrap-user",
            "creator-org",
            None,
        ))
        .await
        .expect("idempotent revoke response");
    assert_eq!(revoke_again.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn application_self_consent_accepts_the_maximum_unique_action_set() {
    let (service, db, _dir) = test_service().await;
    seed_application_manager(&db, "creator-org").await;
    db.upsert_service_provider(&ServiceProvider {
        id: "bulk-consent".to_owned(),
        name: "Bulk Consent".to_owned(),
        description: None,
        active: true,
    })
    .await
    .expect("bulk provider");
    let action_ids = (0..999)
        .map(|index| format!("bulk-consent.action-{index:03}"))
        .collect::<Vec<_>>();
    let actions = action_ids
        .iter()
        .map(|action_id| Action {
            id: action_id.clone(),
            provider_id: "bulk-consent".to_owned(),
            code: action_id.clone(),
            name: action_id.clone(),
            description: None,
            risk_level: ActionRiskLevel::Low,
            visibility_granting: false,
            grantable_to_applications: true,
            consent_label: None,
            consent_description: None,
        })
        .collect::<Vec<_>>();
    db.replace_provider_actions("bulk-consent", &actions)
        .await
        .expect("bulk actions");
    db.insert_application(&Application {
        id: "application-bulk-consent".to_owned(),
        organization_id: "creator-org".to_owned(),
        slug: "bulk-consent".to_owned(),
        name: "Bulk Consent".to_owned(),
        description: None,
        application_type: ApplicationType::Confidential,
        app_type: ApplicationAppType::Web,
        first_party: false,
        consent_policy: ApplicationConsentPolicy::Explicit,
        active: true,
        rauthy_client_id: Some("application-bulk-consent".to_owned()),
        provisioning_status: domain::ApplicationProvisioningStatus::Ready,
        created_by_user_id: "bootstrap-user".to_owned(),
    })
    .await
    .expect("bulk consent application");
    db.replace_application_actions("application-bulk-consent", &action_ids)
        .await
        .expect("bulk application actions");

    let app = api::build_router(service);
    let response = app
        .oneshot(app_request(
            Method::PUT,
            "/api/v1/orgs/creator-org/apps/application-bulk-consent/consent",
            "bootstrap-user",
            "creator-org",
            Some(json!({ "actionIds": action_ids })),
        ))
        .await
        .expect("bulk consent response");
    let status = response.status();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let payload: Value = serde_json::from_slice(&body).expect("JSON");
    assert_eq!(status, StatusCode::OK, "response: {payload}");
    assert_eq!(payload["actions"].as_array().map(Vec::len), Some(999));

    db.set_application_trust("creator-org", "application-bulk-consent", true)
        .await
        .expect("promote trusted application");
    assert!(
        db.find_active_application_consent_with_consistency(
            "bootstrap-user",
            "application-bulk-consent",
            "creator-org",
            Some(ReadConsistency::Strong),
        )
        .await
        .expect("active consent after trust promotion")
        .is_none(),
        "trust promotion must revoke explicit consent in the same transaction"
    );
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

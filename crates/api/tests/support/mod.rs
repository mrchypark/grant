#![allow(dead_code)]

pub mod auth;

use axum::{
    Form, Json, Router,
    body::Body,
    extract::State,
    http::{HeaderMap, Method, Request, StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::{get, post, put},
};
use domain::{RoleBinding, RoleBindingState, RoleBindingSubjectType, User};
use hiqlite::{Node, NodeConfig, s3::EncKeys};
use http_body_util::BodyExt;
use infra::{
    config::{
        AppConfig, AuthConfig, AuthProviderMode, HiqliteConfig, LocalNodeRole, RauthyAuthConfig,
        ReadConsistency, RuntimeMode, SecurityConfig, ServerConfig,
    },
    database::Database,
    runtime::HiqliteRuntime,
    service::AppService,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tempfile::TempDir;
use tokio::sync::Notify;
use uuid::Uuid;

pub const MASTER_KEY: &str = "dev-only-master-key";
pub const APP_AUTH_KEY: &str = "dev-only-app-auth-key";
pub const BOOTSTRAP_USER_ID: &str = "bootstrap-user";
pub const BOOTSTRAP_USER_NAME: &str = "Bootstrap User";
pub const FIXED_TEST_TIME: &str = "2026-04-03T00:00:00Z";

#[derive(Clone)]
struct MockRauthyState {
    clients: Arc<Mutex<HashMap<String, MockClient>>>,
    reconcile_blocked: Arc<AtomicBool>,
    reconcile_started: Arc<Notify>,
    reconcile_release: Arc<Notify>,
    update_counts: Arc<Mutex<HashMap<String, usize>>>,
}

#[derive(Clone)]
struct MockClient {
    id: String,
    name: Option<String>,
    enabled: bool,
    confidential: bool,
    redirect_uris: Vec<String>,
    flows_enabled: Vec<String>,
    challenges: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct IntrospectionForm {
    token: String,
}

#[derive(Debug, Deserialize)]
struct MockClientRequest {
    id: String,
    name: Option<String>,
    confidential: bool,
    redirect_uris: Vec<String>,
    challenges: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct MockClientUpdateRequest {
    name: Option<String>,
    enabled: bool,
    confidential: bool,
    redirect_uris: Vec<String>,
    post_logout_redirect_uris: Option<Vec<String>>,
    allowed_origins: Option<Vec<String>>,
    flows_enabled: Vec<String>,
    auth_code_lifetime: i32,
    access_token_lifetime: i32,
    challenges: Option<Vec<String>>,
    client_uri: Option<String>,
    claims: Option<Value>,
}

#[derive(Debug, Serialize)]
struct MockClientResponse {
    id: String,
    name: Option<String>,
    enabled: bool,
    confidential: bool,
    redirect_uris: Vec<String>,
}

#[derive(Debug, Serialize)]
struct MockClientSecretResponse {
    id: String,
    confidential: bool,
    secret: String,
}

pub fn rauthy_auth_config(base_url: String) -> AuthConfig {
    AuthConfig {
        provider_mode: AuthProviderMode::Rauthy,
        rauthy: RauthyAuthConfig {
            base_url,
            introspect_path: "/auth/v1/oidc/introspect".to_owned(),
            userinfo_path: "/auth/v1/oidc/userinfo".to_owned(),
            client_id: "grant".to_owned(),
            client_secret: "test-rauthy-secret".to_owned(),
            client_read_api_key: "grant-rauthy-api-key".to_owned(),
            cache_ttl_seconds: 30,
        },
        patch_data: None,
    }
}

pub fn free_local_addr() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    drop(listener);
    addr.to_string()
}

pub async fn test_runtime(default_read_consistency: ReadConsistency) -> (HiqliteRuntime, TempDir) {
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

    let client = hiqlite::start_node(cfg).await.expect("start hiqlite");
    (
        HiqliteRuntime::embedded(client, default_read_consistency),
        dir,
    )
}

pub async fn test_service() -> (Arc<AppService>, Database, HiqliteRuntime, TempDir) {
    test_service_with_default_consistency(ReadConsistency::Local).await
}

pub async fn test_service_with_default_consistency(
    default_read_consistency: ReadConsistency,
) -> (Arc<AppService>, Database, HiqliteRuntime, TempDir) {
    test_service_with_security(default_read_consistency, None).await
}

pub async fn test_service_with_security(
    default_read_consistency: ReadConsistency,
    http_capability_pep_application_id: Option<String>,
) -> (Arc<AppService>, Database, HiqliteRuntime, TempDir) {
    let rauthy_base_url = start_mock_rauthy().await;
    let (runtime, dir) = test_runtime(default_read_consistency.clone()).await;
    let app_cfg = AppConfig {
        server: ServerConfig {
            bind_addr: free_local_addr(),
        },
        security: SecurityConfig {
            master_key: MASTER_KEY.to_owned(),
            app_auth_key: APP_AUTH_KEY.to_owned(),
            http_capability_pep_application_id,
            bootstrap_user_id: BOOTSTRAP_USER_ID.to_owned(),
            bootstrap_user_name: BOOTSTRAP_USER_NAME.to_owned(),
        },
        auth: rauthy_auth_config(rauthy_base_url),
        hiqlite: HiqliteConfig {
            runtime_mode: RuntimeMode::Embedded,
            local_node_role: LocalNodeRole::Voter,
            default_read_consistency,
            remote_nodes: vec![],
            remote_tls: false,
            remote_tls_no_verify: false,
            remote_api_secret: None,
            remote_with_proxy: false,
        },
    };
    let db = Database::new(
        runtime.clone(),
        app_cfg.security.bootstrap_user_id.clone(),
        app_cfg.security.bootstrap_user_name.clone(),
    );
    let service = Arc::new(AppService::new(app_cfg, db.clone()));
    service.initialize().await.expect("initialize service");
    (service, db, runtime, dir)
}

pub struct TestApi {
    pub app: Router,
    pub db: Database,
    pub runtime: HiqliteRuntime,
    pub _dir: TempDir,
}

impl TestApi {
    pub async fn new() -> Self {
        Self::with_default_consistency(ReadConsistency::Local).await
    }

    pub async fn with_default_consistency(default_read_consistency: ReadConsistency) -> Self {
        let (service, db, runtime, dir) =
            test_service_with_default_consistency(default_read_consistency).await;
        Self {
            app: api::build_router(service),
            db,
            runtime,
            _dir: dir,
        }
    }

    pub async fn with_http_capability_pep(application_id: &str) -> Self {
        let (service, db, runtime, dir) =
            test_service_with_security(ReadConsistency::Local, Some(application_id.to_owned()))
                .await;
        Self {
            app: api::build_router(service),
            db,
            runtime,
            _dir: dir,
        }
    }

    pub async fn learner() -> Self {
        let (service, db, runtime, dir) = test_service().await;
        Self {
            app: api::build_learner_router(service),
            db,
            runtime,
            _dir: dir,
        }
    }
}

pub fn empty_request(
    method: Method,
    path: &str,
    token: &str,
    organization_id: &str,
) -> Request<Body> {
    api_request(method, path, token, organization_id, None)
}

pub fn public_empty_request(method: Method, path: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .body(Body::empty())
        .expect("request")
}

pub fn auth_empty_request(method: Method, path: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", auth::bearer_header(token))
        .body(Body::empty())
        .expect("request")
}

pub fn public_json_request(method: Method, path: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request")
}

pub fn empty_post_request(path: &str, token: &str, organization_id: &str) -> Request<Body> {
    empty_request(Method::POST, path, token, organization_id)
}

pub fn json_request(
    method: Method,
    path: &str,
    token: &str,
    organization_id: &str,
    body: Value,
) -> Request<Body> {
    api_request(method, path, token, organization_id, Some(body))
}

pub fn api_request(
    method: Method,
    path: &str,
    token: &str,
    organization_id: &str,
    body: Option<Value>,
) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", auth::bearer_header(token))
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

pub fn master_json_request(method: Method, path: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .header("x-grant-master-key", MASTER_KEY)
        .body(Body::from(body.to_string()))
        .expect("request")
}

pub fn master_empty_request(method: Method, path: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", auth::bearer_header(token))
        .header("x-grant-master-key", MASTER_KEY)
        .body(Body::empty())
        .expect("request")
}

pub async fn response_json(response: Response) -> Value {
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    serde_json::from_slice(&body).expect("json")
}

pub async fn response_text(response: Response) -> String {
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    String::from_utf8_lossy(&body).into_owned()
}

pub async fn expect_json_status(response: Response, expected: StatusCode) -> Value {
    let status = response.status();
    let payload = response_json(response).await;
    assert_eq!(status, expected, "response body: {payload}");
    payload
}

pub async fn seed_identity(
    db: &Database,
    external_user_id: &str,
    email: Option<&str>,
    display_name: &str,
) -> User {
    db.resolve_or_create_user_identity("rauthy", external_user_id, email, display_name)
        .await
        .expect("seed identity")
}

pub async fn seed_org_user_membership(
    runtime: &HiqliteRuntime,
    membership_id: &str,
    organization_id: &str,
    user_id: &str,
) {
    runtime
        .execute(
            "INSERT INTO organization_memberships (id, organization_id, user_id, membership_role, status, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            vec![
                membership_id.into(),
                organization_id.into(),
                user_id.into(),
                "member".into(),
                "active".into(),
                FIXED_TEST_TIME.into(),
                FIXED_TEST_TIME.into(),
            ],
        )
        .await
        .expect("membership");
}

pub async fn seed_org_with_user(
    db: &Database,
    runtime: &HiqliteRuntime,
    organization_id: &str,
    external_user_id: &str,
    display_name: &str,
) -> User {
    db.upsert_organization_with_owner(organization_id, display_name, None, display_name)
        .await
        .expect("seed organization");
    let user = seed_identity(
        db,
        external_user_id,
        Some(&format!("{external_user_id}@rauthy.test")),
        display_name,
    )
    .await;
    seed_org_user_membership(
        runtime,
        &format!("membership-{organization_id}-{external_user_id}"),
        organization_id,
        &user.id,
    )
    .await;
    user
}

pub fn user_org_role_binding(
    role_id: &str,
    organization_id: &str,
    subject_id: &str,
) -> RoleBinding {
    RoleBinding {
        id: Uuid::new_v4().simple().to_string(),
        subject_type: RoleBindingSubjectType::User,
        subject_id: subject_id.to_owned(),
        subject_organization_id: organization_id.to_owned(),
        provider_id: "grant".to_owned(),
        resource_type: "organization".to_owned(),
        resource_id: organization_id.to_owned(),
        role_id: role_id.to_owned(),
        agreement_id: None,
        state: RoleBindingState::Active,
        created_by_user_id: BOOTSTRAP_USER_ID.to_owned(),
        created_at: FIXED_TEST_TIME.to_owned(),
        updated_at: FIXED_TEST_TIME.to_owned(),
    }
}

pub async fn start_mock_rauthy() -> String {
    async fn login_ui() -> Html<&'static str> {
        Html(
            r#"<!doctype html>
<html><body>
<h1>Rauthy Login</h1>
<form method="post" action="/auth/v1/oidc/authorize">
  <input name="email" />
  <input name="password" type="password" />
</form>
<link rel="stylesheet" href="/auth/v1/assets/app.css">
</body></html>"#,
        )
    }

    async fn asset() -> impl IntoResponse {
        (
            [(header::CONTENT_TYPE, "text/css")],
            "body { font-family: sans-serif; }",
        )
    }

    async fn redirect_response() -> impl IntoResponse {
        (
            StatusCode::FOUND,
            [
                (header::LOCATION, "/auth/v1/redirect-target"),
                (header::SET_COOKIE, "rauthy_session=test; Path=/; HttpOnly"),
            ],
        )
    }

    async fn redirect_target() -> &'static str {
        "redirect was followed upstream"
    }

    async fn echo_path(uri: axum::http::Uri) -> String {
        uri.path().to_owned()
    }

    fn require_client_read_api_key(headers: &HeaderMap) {
        let authorization = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        assert_eq!(authorization, "API-Key grant-rauthy-api-key");
    }

    async fn create_client(
        State(state): State<MockRauthyState>,
        headers: HeaderMap,
        Json(body): Json<MockClientRequest>,
    ) -> impl IntoResponse {
        require_client_read_api_key(&headers);
        let flows_enabled = if body.confidential {
            vec!["authorization_code", "client_credentials", "refresh_token"]
        } else {
            vec!["authorization_code", "refresh_token"]
        }
        .into_iter()
        .map(str::to_owned)
        .collect();
        let client = MockClient {
            id: body.id.clone(),
            name: body.name,
            enabled: true,
            confidential: body.confidential,
            redirect_uris: body.redirect_uris,
            flows_enabled,
            challenges: body.challenges.unwrap_or_else(|| vec!["S256".to_owned()]),
        };
        let mut clients = state.clients.lock().expect("clients");
        if clients.contains_key(&body.id) {
            return (
                StatusCode::CONFLICT,
                Json(json!({ "error": "client already exists" })),
            )
                .into_response();
        }
        clients.insert(body.id.clone(), client.clone());
        Json(MockClientResponse {
            id: client.id,
            name: client.name,
            enabled: true,
            confidential: client.confidential,
            redirect_uris: client.redirect_uris,
        })
        .into_response()
    }

    async fn update_client(
        State(state): State<MockRauthyState>,
        headers: HeaderMap,
        axum::extract::Path(client_id): axum::extract::Path<String>,
        Json(raw_body): Json<Value>,
    ) -> impl IntoResponse {
        require_client_read_api_key(&headers);
        assert!(
            raw_body.get("id").is_none(),
            "PUT must not include client id"
        );
        if client_id == "application-unknown-rauthy-field" {
            return (
                StatusCode::IM_A_TEAPOT,
                Json(json!({ "error": "unexpected put" })),
            )
                .into_response();
        }
        let body: MockClientUpdateRequest =
            serde_json::from_value(raw_body).expect("valid client update");
        let expected_flows = if body.confidential {
            vec!["authorization_code", "client_credentials", "refresh_token"]
        } else {
            vec!["authorization_code", "refresh_token"]
        };
        assert_eq!(body.flows_enabled, expected_flows);
        assert_eq!(
            body.post_logout_redirect_uris,
            Some(vec!["https://admin.example/logout".to_owned()])
        );
        assert_eq!(
            body.allowed_origins,
            Some(vec!["https://admin.example".to_owned()])
        );
        assert_eq!(body.auth_code_lifetime, 90);
        assert_eq!(body.access_token_lifetime, 7_200);
        assert_eq!(body.challenges, Some(vec!["S256".to_owned()]));
        assert_eq!(
            body.client_uri.as_deref(),
            Some("https://admin.example/client")
        );
        assert_eq!(body.claims, Some(json!({ "admin": "customized" })));
        if matches!(
            client_id.as_str(),
            "application-reconcile-stale"
                | "application-reconcile-read-failure"
                | "application-reconcile-second-failure"
        ) && !state.reconcile_blocked.swap(true, Ordering::SeqCst)
        {
            state.reconcile_started.notify_one();
            state.reconcile_release.notified().await;
        }
        let update_count = {
            let mut counts = state.update_counts.lock().expect("update counts");
            let count = counts.entry(client_id.clone()).or_default();
            *count += 1;
            *count
        };
        if client_id == "application-reconcile-second-failure" && update_count == 2 {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "error": "second reconciliation failed" })),
            )
                .into_response();
        }
        if body.name.as_deref() == Some("reject-sync") {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({
                    "error": "BadRequest",
                    "message": "sync rejected",
                    "client_secret": "must-not-leak"
                })),
            )
                .into_response();
        }
        let mut clients = state.clients.lock().expect("clients");
        let Some(client) = clients.get_mut(&client_id) else {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "client not found" })),
            )
                .into_response();
        };
        client.name = body.name;
        client.enabled = body.enabled;
        client.confidential = body.confidential;
        client.redirect_uris = body.redirect_uris;
        client.flows_enabled = body.flows_enabled;
        client.challenges = body.challenges.unwrap_or_default();
        Json(MockClientResponse {
            id: client.id.clone(),
            name: client.name.clone(),
            enabled: true,
            confidential: client.confidential,
            redirect_uris: client.redirect_uris.clone(),
        })
        .into_response()
    }

    async fn get_client(
        State(state): State<MockRauthyState>,
        headers: HeaderMap,
        axum::extract::Path(client_id): axum::extract::Path<String>,
    ) -> impl IntoResponse {
        require_client_read_api_key(&headers);
        let mut clients = state.clients.lock().expect("clients");
        if client_id == "application-create-race" && !clients.contains_key(&client_id) {
            clients.insert(
                client_id.clone(),
                MockClient {
                    id: client_id.clone(),
                    name: Some("Concurrent Client".to_owned()),
                    enabled: true,
                    confidential: true,
                    redirect_uris: vec!["https://concurrent.example/callback".to_owned()],
                    flows_enabled: vec!["authorization_code".to_owned()],
                    challenges: vec!["S256".to_owned()],
                },
            );
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "client not found" })),
            )
                .into_response();
        }
        let Some(client) = clients.get(&client_id) else {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "client not found" })),
            )
                .into_response();
        };
        let mut body = json!({
            "id": client.id,
            "name": client.name,
            "enabled": client.enabled,
            "confidential": client.confidential,
            "redirect_uris": client.redirect_uris,
            "post_logout_redirect_uris": ["https://admin.example/logout"],
            "allowed_origins": ["https://admin.example"],
            "flows_enabled": client.flows_enabled,
            "access_token_alg": "EdDSA",
            "id_token_alg": "EdDSA",
            "auth_code_lifetime": 90,
            "access_token_lifetime": 7200,
            "scopes": ["openid", "profile", "email"],
            "default_scopes": ["openid", "profile", "email"],
            "challenges": client.challenges,
            "force_mfa": false,
            "client_uri": "https://admin.example/client",
            "contacts": null,
            "backchannel_logout_uri": null,
            "restrict_group_prefix": null,
            "claims": { "admin": "customized" },
            "claims_at_root": false,
            "allowed_resources": null,
            "default_aud": null,
            "scim": null
        });
        if client_id == "application-unknown-rauthy-field" {
            body["future_setting"] = json!(true);
        }
        Json(body).into_response()
    }

    async fn wait_for_reconcile(State(state): State<MockRauthyState>) -> StatusCode {
        state.reconcile_started.notified().await;
        StatusCode::NO_CONTENT
    }

    async fn release_reconcile(State(state): State<MockRauthyState>) -> StatusCode {
        state.reconcile_release.notify_one();
        StatusCode::NO_CONTENT
    }

    async fn rotate_client_secret(
        State(state): State<MockRauthyState>,
        headers: HeaderMap,
        axum::extract::Path(client_id): axum::extract::Path<String>,
    ) -> impl IntoResponse {
        require_client_read_api_key(&headers);
        let clients = state.clients.lock().expect("clients");
        let Some(client) = clients.get(&client_id) else {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "client not found" })),
            )
                .into_response();
        };
        Json(MockClientSecretResponse {
            id: client.id.clone(),
            confidential: client.confidential,
            secret: format!("rotated-{}", client.id),
        })
        .into_response()
    }

    async fn introspect(
        State(_state): State<MockRauthyState>,
        headers: HeaderMap,
        Form(form): Form<IntrospectionForm>,
    ) -> Json<serde_json::Value> {
        let authorization = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        assert_eq!(authorization, "Basic Z3JhbnQ6dGVzdC1yYXV0aHktc2VjcmV0");
        let (client_id, sub) = form
            .token
            .strip_prefix("app-token/")
            .and_then(|value| value.split_once('/'))
            .unwrap_or(("grant", form.token.as_str()));
        let expires_at = if form.token == "expired-token" {
            1_i64
        } else {
            4_102_444_800_i64
        };
        Json(json!({
            "active": true,
            "sub": sub,
            "scope": "openid email profile",
            "client_id": client_id,
            "exp": expires_at
        }))
    }

    async fn userinfo(headers: HeaderMap) -> Json<serde_json::Value> {
        let token = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .or_else(|| {
                headers
                    .get(header::AUTHORIZATION)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.strip_prefix("bearer "))
            })
            .unwrap_or("anonymous");
        let token_sub = token
            .strip_prefix("app-token/")
            .and_then(|value| value.split_once('/'))
            .map(|(_, sub)| sub)
            .unwrap_or(token);
        let sub = if token == "userinfo-sub-mismatch" {
            "different-subject"
        } else {
            token_sub
        };
        Json(json!({
            "id": token,
            "sub": sub,
            "name": token,
            "email": format!("{token}@rauthy.test"),
            "email_verified": true,
            "roles": [],
            "mfa_enabled": false
        }))
    }

    let state = MockRauthyState {
        clients: Arc::new(Mutex::new(HashMap::new())),
        reconcile_blocked: Arc::new(AtomicBool::new(false)),
        reconcile_started: Arc::new(Notify::new()),
        reconcile_release: Arc::new(Notify::new()),
        update_counts: Arc::new(Mutex::new(HashMap::new())),
    };
    let app = Router::new()
        .route("/auth/v1", get(login_ui))
        .route("/auth/v1/redirect", get(redirect_response))
        .route("/auth/v1/redirect-target", get(redirect_target))
        .route("/auth/v1/echo/{*path}", get(echo_path))
        .route("/auth/v1/assets/{*path}", get(asset))
        .route("/_test/reconcile-started", get(wait_for_reconcile))
        .route("/_test/reconcile-release", post(release_reconcile))
        .route("/auth/v1/clients", post(create_client))
        .route(
            "/auth/v1/clients/{clientId}",
            get(get_client).put(update_client),
        )
        .route(
            "/auth/v1/clients/{clientId}/secret",
            put(rotate_client_secret),
        )
        .route("/auth/v1/oidc/introspect", post(introspect))
        .route("/auth/v1/oidc/userinfo", get(userinfo))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind rauthy");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve rauthy");
    });
    format!("http://{}", addr)
}

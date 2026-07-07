#![allow(dead_code)]

pub mod auth;

use axum::{
    Form, Json, Router,
    body::Body,
    extract::State,
    http::{HeaderMap, Method, Request, StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use domain::{RoleBinding, RoleBindingState, RoleBindingSubjectType, User};
use hiqlite::{Node, NodeConfig, s3::EncKeys};
use http_body_util::BodyExt;
use infra::{
    config::{
        AppConfig, AuthConfig, HiqliteConfig, LocalNodeRole, RauthyAuthConfig, ReadConsistency,
        RuntimeMode, SecurityConfig, ServerConfig,
    },
    database::Database,
    runtime::HiqliteRuntime,
    service::AppService,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use tempfile::TempDir;
use uuid::Uuid;

pub const MASTER_KEY: &str = "dev-only-master-key";
pub const BOOTSTRAP_USER_ID: &str = "bootstrap-user";
pub const BOOTSTRAP_USER_NAME: &str = "Bootstrap User";
pub const FIXED_TEST_TIME: &str = "2026-04-03T00:00:00Z";

#[derive(Clone)]
struct MockRauthyState;

#[derive(Debug, Deserialize)]
struct IntrospectionForm {
    token: String,
}

pub fn rauthy_auth_config(base_url: String) -> AuthConfig {
    AuthConfig {
        rauthy: RauthyAuthConfig {
            base_url,
            introspect_path: "/auth/v1/oidc/introspect".to_owned(),
            userinfo_path: "/auth/v1/oidc/userinfo".to_owned(),
            client_id: "grant".to_owned(),
            client_secret: "test-rauthy-secret".to_owned(),
            cache_ttl_seconds: 30,
        },
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
    let rauthy_base_url = start_mock_rauthy().await;
    let (runtime, dir) = test_runtime(default_read_consistency.clone()).await;
    let app_cfg = AppConfig {
        server: ServerConfig {
            bind_addr: free_local_addr(),
        },
        security: SecurityConfig {
            master_key: MASTER_KEY.to_owned(),
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
    active: bool,
) -> User {
    db.resolve_or_create_user_identity("rauthy", external_user_id, email, display_name, active)
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
        true,
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
        Json(json!({
            "active": true,
            "sub": form.token,
            "scope": "openid email profile",
            "client_id": "grant",
            "exp": 4_102_444_800i64
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
        Json(json!({
            "id": token,
            "sub": token,
            "name": token,
            "email": format!("{token}@rauthy.test"),
            "email_verified": true,
            "roles": [],
            "mfa_enabled": false
        }))
    }

    let app = Router::new()
        .route("/auth/v1", get(login_ui))
        .route("/auth/v1/assets/{*path}", get(asset))
        .route("/auth/v1/oidc/introspect", post(introspect))
        .route("/auth/v1/oidc/userinfo", get(userinfo))
        .with_state(MockRauthyState);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind rauthy");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve rauthy");
    });
    format!("http://{}", addr)
}

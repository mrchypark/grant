use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, header},
    response::{Html, IntoResponse},
    routing::{get, post},
};
use infra::config::{AuthConfig, KmsConfig, TrailbaseAuthConfig};
use serde_json::json;

#[derive(Clone)]
struct MockTrailbaseState;

pub fn trailbase_auth_config(base_url: String) -> AuthConfig {
    AuthConfig {
        trailbase: TrailbaseAuthConfig {
            base_url,
            introspect_path: "/api/certd/introspect".to_owned(),
            internal_secret: "test-trailbase-secret".to_owned(),
            cache_ttl_seconds: 30,
        },
    }
}

pub fn test_kms_config() -> KmsConfig {
    KmsConfig {
        active_key_id: "local-dev-v1".to_owned(),
        master_keys: vec![("local-dev-v1".to_owned(), b"dev-only-master-key".to_vec())],
    }
}

pub fn bearer_header(token: &str) -> String {
    format!("Bearer {token}")
}

pub async fn start_mock_trailbase() -> String {
    async fn login_ui() -> Html<&'static str> {
        Html(
            r#"<!doctype html>
<html><body>
<h1>TrailBase Login</h1>
<form method="post" action="/api/auth/v1/login">
  <input name="email" />
  <input name="password" type="password" />
</form>
<link rel="stylesheet" href="/_/auth/app.css">
</body></html>"#,
        )
    }

    async fn asset() -> impl IntoResponse {
        (
            [(header::CONTENT_TYPE, "text/css")],
            "body { font-family: sans-serif; }",
        )
    }

    async fn login_api() -> Json<serde_json::Value> {
        Json(json!({
            "authToken": "mock-auth-token",
            "refreshToken": "mock-refresh-token"
        }))
    }

    async fn introspect(
        State(_state): State<MockTrailbaseState>,
        headers: HeaderMap,
    ) -> Json<serde_json::Value> {
        let secret = headers
            .get("x-certd-internal-secret")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        assert_eq!(secret, "test-trailbase-secret");
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
            "valid": true,
            "trailbaseUserId": token,
            "email": format!("{token}@trailbase.test"),
            "displayName": token,
            "verified": true,
            "active": true,
            "expiresAt": "2099-01-01T00:00:00Z"
        }))
    }

    let app = Router::new()
        .route("/_/auth/login", get(login_ui))
        .route("/_/auth/{*path}", get(asset))
        .route("/api/auth/v1/login", post(login_api))
        .route("/api/certd/introspect", post(introspect))
        .with_state(MockTrailbaseState);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind trailbase");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve trailbase");
    });
    format!("http://{}", addr)
}

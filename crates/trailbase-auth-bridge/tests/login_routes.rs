use std::{
    env, fs,
    path::PathBuf,
    sync::{Mutex, OnceLock},
};

use axum::http::{Method, StatusCode};
use trailbase_auth_bridge::{
    BridgeConfig, LoginRouteConfig, build_test_app_with_login_routes,
    build_test_app_with_public_dir,
};

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct EnvSnapshot {
    keys: Vec<(&'static str, Option<String>)>,
}

impl EnvSnapshot {
    fn capture(keys: &[&'static str]) -> Self {
        Self {
            keys: keys.iter().map(|key| (*key, env::var(key).ok())).collect(),
        }
    }
}

impl Drop for EnvSnapshot {
    fn drop(&mut self) {
        for (key, value) in self.keys.iter().rev() {
            match value {
                Some(value) => unsafe { env::set_var(key, value) },
                None => unsafe { env::remove_var(key) },
            }
        }
    }
}

fn with_env<T>(updates: &[(&'static str, Option<&str>)], f: impl FnOnce() -> T) -> T {
    let _guard = env_lock();
    let snapshot = EnvSnapshot::capture(&[
        "TRAILBASE_CERTD_INTERNAL_SECRET",
        "TRAILBASE_PUBLIC_DIR",
        "TRAILBASE_CUSTOM_LOGIN_ENABLED",
        "TRAILBASE_LOGIN_FALLBACK_ENABLED",
        "TRAILBASE_KAKAO_ENABLED",
        "TRAILBASE_KAKAO_OIDC_READY",
        "TRAILBASE_KAKAO_LINKING_READY",
        "TRAILBASE_GOOGLE_OAUTH_ENABLED",
        "TRAILBASE_GOOGLE_OAUTH_READY",
        "TRAILBASE_APPLE_OAUTH_ENABLED",
        "TRAILBASE_APPLE_OAUTH_READY",
        "TRAILBASE_DEFAULT_POST_LOGIN_PATH",
    ]);

    for (key, value) in updates {
        match value {
            Some(value) => unsafe { env::set_var(key, value) },
            None => unsafe { env::remove_var(key) },
        }
    }

    let result = f();
    drop(snapshot);
    result
}

async fn assert_redirect_location(
    app: &trailbase_auth_bridge::TestApp,
    method: Method,
    uri: &str,
    expected_location: &str,
) {
    let (status, headers, _) = app.request_response(method, uri).await.expect("request");

    assert!(
        status.is_redirection(),
        "expected redirect status, got {status}"
    );
    assert_eq!(
        headers
            .get(axum::http::header::LOCATION)
            .and_then(|value| value.to_str().ok()),
        Some(expected_location),
    );
}

fn provider_anchor<'a>(body: &'a str, provider: &str) -> &'a str {
    let marker = format!("data-provider=\"{provider}\"");
    let start = body.find(&marker).expect("provider anchor marker");
    let prefix_start = body[..start].rfind("<a").expect("anchor start");
    let suffix_end = body[prefix_start..]
        .find('>')
        .map(|idx| prefix_start + idx + 1)
        .expect("anchor tag end");
    &body[prefix_start..suffix_end]
}

fn active_custom_login_routes() -> LoginRouteConfig {
    LoginRouteConfig {
        custom_login_enabled: true,
        login_fallback_enabled: false,
        kakao_enabled: false,
        kakao_oidc_ready: false,
        kakao_linking_ready: false,
        google_oauth_enabled: false,
        google_oauth_ready: false,
        apple_oauth_enabled: false,
        apple_oauth_ready: false,
        default_post_login_path: "/post-login".to_owned(),
    }
}

#[test]
fn from_env_defaults_bridge_login_config_fields() {
    let config = with_env(
        &[
            ("TRAILBASE_CERTD_INTERNAL_SECRET", Some("bridge-secret")),
            ("TRAILBASE_PUBLIC_DIR", None),
            ("TRAILBASE_CUSTOM_LOGIN_ENABLED", None),
            ("TRAILBASE_LOGIN_FALLBACK_ENABLED", None),
            ("TRAILBASE_KAKAO_ENABLED", None),
            ("TRAILBASE_KAKAO_OIDC_READY", None),
            ("TRAILBASE_KAKAO_LINKING_READY", None),
            ("TRAILBASE_GOOGLE_OAUTH_ENABLED", None),
            ("TRAILBASE_GOOGLE_OAUTH_READY", None),
            ("TRAILBASE_APPLE_OAUTH_ENABLED", None),
            ("TRAILBASE_APPLE_OAUTH_READY", None),
            ("TRAILBASE_DEFAULT_POST_LOGIN_PATH", None),
        ],
        || BridgeConfig::from_env().expect("bridge config"),
    );

    assert_eq!(config.public_dir, None);
    assert!(!config.custom_login_enabled);
    assert!(config.login_fallback_enabled);
    assert!(!config.kakao_enabled);
    assert!(!config.kakao_oidc_ready);
    assert!(!config.kakao_linking_ready);
    assert!(!config.google_oauth_enabled);
    assert!(!config.google_oauth_ready);
    assert!(!config.apple_oauth_enabled);
    assert!(!config.apple_oauth_ready);
    assert_eq!(config.default_post_login_path, "/app");
}

#[test]
fn from_env_reads_bridge_login_config_env_overrides() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let public_dir = PathBuf::from(temp_dir.path());

    let config = with_env(
        &[
            ("TRAILBASE_CERTD_INTERNAL_SECRET", Some("bridge-secret")),
            (
                "TRAILBASE_PUBLIC_DIR",
                Some(public_dir.to_str().expect("utf8")),
            ),
            ("TRAILBASE_CUSTOM_LOGIN_ENABLED", Some("true")),
            ("TRAILBASE_LOGIN_FALLBACK_ENABLED", Some("false")),
            ("TRAILBASE_KAKAO_ENABLED", Some("true")),
            ("TRAILBASE_KAKAO_OIDC_READY", Some("true")),
            ("TRAILBASE_KAKAO_LINKING_READY", Some("true")),
            ("TRAILBASE_GOOGLE_OAUTH_ENABLED", Some("true")),
            ("TRAILBASE_GOOGLE_OAUTH_READY", Some("true")),
            ("TRAILBASE_APPLE_OAUTH_ENABLED", Some("true")),
            ("TRAILBASE_APPLE_OAUTH_READY", Some("true")),
            ("TRAILBASE_DEFAULT_POST_LOGIN_PATH", Some("/auth/complete")),
        ],
        || BridgeConfig::from_env().expect("bridge config"),
    );

    assert_eq!(config.public_dir, Some(public_dir));
    assert!(config.custom_login_enabled);
    assert!(!config.login_fallback_enabled);
    assert!(config.kakao_enabled);
    assert!(config.kakao_oidc_ready);
    assert!(config.kakao_linking_ready);
    assert!(config.google_oauth_enabled);
    assert!(config.google_oauth_ready);
    assert!(config.apple_oauth_enabled);
    assert!(config.apple_oauth_ready);
    assert_eq!(config.default_post_login_path, "/auth/complete");
}

#[test]
fn from_env_rejects_malformed_login_flag_values() {
    for key in [
        "TRAILBASE_CUSTOM_LOGIN_ENABLED",
        "TRAILBASE_LOGIN_FALLBACK_ENABLED",
        "TRAILBASE_KAKAO_ENABLED",
        "TRAILBASE_KAKAO_OIDC_READY",
        "TRAILBASE_KAKAO_LINKING_READY",
        "TRAILBASE_GOOGLE_OAUTH_ENABLED",
        "TRAILBASE_GOOGLE_OAUTH_READY",
        "TRAILBASE_APPLE_OAUTH_ENABLED",
        "TRAILBASE_APPLE_OAUTH_READY",
    ] {
        let err = with_env(
            &[
                ("TRAILBASE_CERTD_INTERNAL_SECRET", Some("bridge-secret")),
                (key, Some("definitely-not-a-bool")),
            ],
            || BridgeConfig::from_env().expect_err("malformed flag should fail"),
        );

        assert!(
            err.to_string().contains(key),
            "expected error to mention {key}, got {err}"
        );
    }
}

#[test]
fn from_env_rejects_external_default_post_login_path() {
    let err = with_env(
        &[
            ("TRAILBASE_CERTD_INTERNAL_SECRET", Some("bridge-secret")),
            (
                "TRAILBASE_DEFAULT_POST_LOGIN_PATH",
                Some("https://evil.example/steal"),
            ),
        ],
        || BridgeConfig::from_env().expect_err("external post-login path should fail"),
    );

    assert!(
        err.to_string()
            .contains("TRAILBASE_DEFAULT_POST_LOGIN_PATH"),
        "expected error to mention the env key, got {err}",
    );
}

#[test]
fn from_env_rejects_auth_api_default_post_login_path() {
    let err = with_env(
        &[
            ("TRAILBASE_CERTD_INTERNAL_SECRET", Some("bridge-secret")),
            (
                "TRAILBASE_DEFAULT_POST_LOGIN_PATH",
                Some("/api/auth/v1/login"),
            ),
        ],
        || BridgeConfig::from_env().expect_err("auth api post-login path should fail"),
    );

    assert!(
        err.to_string()
            .contains("TRAILBASE_DEFAULT_POST_LOGIN_PATH"),
        "expected error to mention the env key, got {err}",
    );
}

#[test]
fn from_env_rejects_dot_segment_default_post_login_path() {
    let err = with_env(
        &[
            ("TRAILBASE_CERTD_INTERNAL_SECRET", Some("bridge-secret")),
            (
                "TRAILBASE_DEFAULT_POST_LOGIN_PATH",
                Some("/workspaces/../login"),
            ),
        ],
        || BridgeConfig::from_env().expect_err("dot-segment post-login path should fail"),
    );

    assert!(
        err.to_string()
            .contains("TRAILBASE_DEFAULT_POST_LOGIN_PATH"),
        "expected error to mention the env key, got {err}",
    );
}

#[tokio::test]
async fn serves_login_assets_from_public_dir() {
    let temp_dir = tempfile::tempdir().expect("public dir");
    let asset_path = temp_dir.path().join("login-assets");
    fs::create_dir_all(&asset_path).expect("asset dir");
    fs::write(asset_path.join("hello.txt"), "bridge static asset").expect("asset file");

    let app = build_test_app_with_public_dir(Some(temp_dir.path().to_path_buf()), "bridge-secret")
        .await
        .expect("test app");

    let (status, body) = app
        .request_bytes(Method::GET, "/login-assets/hello.txt")
        .await
        .expect("asset request");

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, b"bridge static asset");
    let (login_api_status, _) = app
        .request_bytes(Method::POST, "/api/auth/v1/login")
        .await
        .expect("login api request");
    assert_ne!(login_api_status, StatusCode::NOT_FOUND);

    let (ui_status, _) = app
        .request_bytes(Method::GET, "/_/auth/login")
        .await
        .expect("login ui request");
    assert_ne!(ui_status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn unauthenticated_root_redirects_to_login_when_custom_login_enabled_and_fallback_disabled() {
    let app = build_test_app_with_login_routes(None, "bridge-secret", active_custom_login_routes())
        .await
        .expect("test app");

    assert_redirect_location(&app, Method::GET, "/", "/login").await;
}

#[tokio::test]
async fn root_redirect_preserves_only_valid_return_to_targets() {
    let app = build_test_app_with_login_routes(None, "bridge-secret", active_custom_login_routes())
        .await
        .expect("test app");

    assert_redirect_location(
        &app,
        Method::GET,
        "/?return_to=%2Fteams%3Ftab%3Dops",
        "/login?return_to=%2Fteams%3Ftab%3Dops",
    )
    .await;
    assert_redirect_location(
        &app,
        Method::GET,
        "/?return_to=https%3A%2F%2Fevil.example&next=%2Fsafe",
        "/login?return_to=%2Fsafe",
    )
    .await;
    assert_redirect_location(
        &app,
        Method::GET,
        "/?return_to=%2F%2Fevil.example",
        "/login",
    )
    .await;
    assert_redirect_location(
        &app,
        Method::GET,
        "/?return_to=%2Fworkspaces%2F..%2Flogin&next=%2Fsafe",
        "/login?return_to=%2Fsafe",
    )
    .await;
}

#[tokio::test]
async fn root_redirect_ignores_legacy_view_query_parameter() {
    let app = build_test_app_with_login_routes(None, "bridge-secret", active_custom_login_routes())
        .await
        .expect("test app");

    for (uri, expected) in [
        ("/?return_to=%2Fteams&view=auth0", "/login?return_to=%2Fteams"),
        ("/?return_to=%2Fteams&view=AUTH0", "/login?return_to=%2Fteams"),
        ("/?return_to=%2Fteams&view=editorial&view=auth0", "/login?return_to=%2Fteams"),
    ] {
        assert_redirect_location(&app, Method::GET, uri, expected).await;
    }
}

#[tokio::test]
async fn custom_login_entrypoint_is_reachable_when_fallback_disabled() {
    let app = build_test_app_with_login_routes(None, "bridge-secret", active_custom_login_routes())
        .await
        .expect("test app");

    let (status, headers, body) = app
        .request_response(Method::GET, "/login")
        .await
        .expect("request");

    assert_eq!(status, StatusCode::OK);
    assert!(
        headers
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            == Some("text/html; charset=utf-8"),
        "expected html response",
    );
    assert_eq!(
        headers
            .get(axum::http::header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok()),
        Some("private, no-store"),
    );
    assert_eq!(
        headers
            .get(axum::http::header::VARY)
            .and_then(|value| value.to_str().ok()),
        Some("authorization, cookie"),
    );

    let body = String::from_utf8(body).expect("utf8");
    for expected in [
        "Sign in",
        "Sign in to continue",
        "Fast enough for daily work",
        "label for=\"email\"",
        "type=\"email\"",
        "label for=\"password\"",
        "type=\"password\"",
        "Continue with Kakao",
        "Logo",
        "/login-assets/login.v1.css",
    ] {
        assert!(
            body.contains(expected),
            "expected login shell to contain {expected:?}, got body: {body}",
        );
    }
    assert!(
        !body.contains("Custom login entrypoint"),
        "placeholder content should not be served from /login",
    );
    assert!(
        !body.contains("method=\"get\""),
        "login shell must not expose a GET credential submission path",
    );
    assert!(
        {
            let google_anchor = provider_anchor(&body, "google");
            google_anchor.contains("data-provider-rollout-enabled=\"false\"")
                && google_anchor.contains("data-provider-ready=\"false\"")
                && google_anchor.contains("href=\"/api/auth/v1/oauth/google/login\"")
                && google_anchor.contains(" hidden")
        },
        "google oauth should stay hidden unless rollout and readiness are both enabled, got body: {body}",
    );
    assert!(
        {
            let apple_anchor = provider_anchor(&body, "apple");
            apple_anchor.contains("data-provider-rollout-enabled=\"false\"")
                && apple_anchor.contains("data-provider-ready=\"false\"")
                && apple_anchor.contains("href=\"/api/auth/v1/oauth/apple/login\"")
                && apple_anchor.contains(" hidden")
        },
        "apple oauth should stay hidden unless rollout and readiness are both enabled, got body: {body}",
    );
}

#[tokio::test]
async fn legacy_view_query_still_serves_the_single_login_shell() {
    let app = build_test_app_with_login_routes(None, "bridge-secret", active_custom_login_routes())
        .await
        .expect("test app");

    let (status, _, body) = app
        .request_response(Method::GET, "/login?return_to=%2Fteams&view=auth0")
        .await
        .expect("request");

    assert_eq!(status, StatusCode::OK);

    let body = String::from_utf8(body).expect("utf8");
    for expected in [
        "Sign in to continue",
        "meta name=\"post-login-path\" content=\"/teams\"",
        "/login-assets/login.v1.css",
    ] {
        assert!(
            body.contains(expected),
            "expected single login shell to contain {expected:?}, got body: {body}",
        );
    }
}

#[tokio::test]
async fn provider_visibility_shows_google_and_apple_only_when_enabled_and_ready() {
    let app = build_test_app_with_login_routes(
        None,
        "bridge-secret",
        LoginRouteConfig {
            custom_login_enabled: true,
            login_fallback_enabled: false,
            kakao_enabled: false,
            kakao_oidc_ready: false,
            kakao_linking_ready: false,
            google_oauth_enabled: true,
            google_oauth_ready: true,
            apple_oauth_enabled: true,
            apple_oauth_ready: true,
            default_post_login_path: "/post-login".to_owned(),
        },
    )
    .await
    .expect("test app");

    let (status, _, body) = app
        .request_response(Method::GET, "/login")
        .await
        .expect("request");

    assert_eq!(status, StatusCode::OK);

    let body = String::from_utf8(body).expect("utf8");
    assert!(
        {
            let google_anchor = provider_anchor(&body, "google");
            google_anchor.contains("data-provider-rollout-enabled=\"true\"")
                && google_anchor.contains("data-provider-ready=\"true\"")
                && google_anchor.contains("href=\"/api/auth/v1/oauth/google/login\"")
                && !google_anchor.contains(" hidden")
                && body.contains("Continue with Google")
        },
        "google oauth should be visible when rollout and readiness are both enabled, got body: {body}",
    );
    assert!(
        {
            let apple_anchor = provider_anchor(&body, "apple");
            apple_anchor.contains("data-provider-rollout-enabled=\"true\"")
                && apple_anchor.contains("data-provider-ready=\"true\"")
                && apple_anchor.contains("href=\"/api/auth/v1/oauth/apple/login\"")
                && !apple_anchor.contains(" hidden")
                && body.contains("Continue with Apple")
        },
        "apple oauth should be visible when rollout and readiness are both enabled, got body: {body}",
    );
}

#[tokio::test]
async fn provider_visibility_keeps_ui_only_enablement_hidden_when_ready_is_false() {
    let app = build_test_app_with_login_routes(
        None,
        "bridge-secret",
        LoginRouteConfig {
            custom_login_enabled: true,
            login_fallback_enabled: false,
            kakao_enabled: false,
            kakao_oidc_ready: false,
            kakao_linking_ready: false,
            google_oauth_enabled: true,
            google_oauth_ready: false,
            apple_oauth_enabled: true,
            apple_oauth_ready: false,
            default_post_login_path: "/post-login".to_owned(),
        },
    )
    .await
    .expect("test app");

    let (status, _, body) = app
        .request_response(Method::GET, "/login")
        .await
        .expect("request");

    assert_eq!(status, StatusCode::OK);

    let body = String::from_utf8(body).expect("utf8");
    assert!(
        {
            let google_anchor = provider_anchor(&body, "google");
            google_anchor.contains("data-provider-rollout-enabled=\"true\"")
                && google_anchor.contains("data-provider-ready=\"false\"")
                && google_anchor.contains(" hidden")
        },
        "google oauth should remain hidden when readiness is false, got body: {body}",
    );
    assert!(
        {
            let apple_anchor = provider_anchor(&body, "apple");
            apple_anchor.contains("data-provider-rollout-enabled=\"true\"")
                && apple_anchor.contains("data-provider-ready=\"false\"")
                && apple_anchor.contains(" hidden")
        },
        "apple oauth should remain hidden when readiness is false, got body: {body}",
    );
}

#[tokio::test]
async fn login_stylesheet_route_serves_versioned_css() {
    let app = build_test_app_with_login_routes(None, "bridge-secret", active_custom_login_routes())
        .await
        .expect("test app");

    let (status, headers, body) = app
        .request_response(Method::GET, "/login-assets/login.v1.css")
        .await
        .expect("stylesheet request");

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/css; charset=utf-8"),
    );
    assert_eq!(
        headers
            .get(axum::http::header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok()),
        Some("public, max-age=31536000, immutable"),
    );
    assert!(
        String::from_utf8(body)
            .expect("utf8")
            .contains(".login-shell"),
        "expected stylesheet body",
    );
}

#[tokio::test]
async fn login_entry_routes_to_trailbase_login_when_fallback_enabled() {
    let app = build_test_app_with_login_routes(
        None,
        "bridge-secret",
        LoginRouteConfig {
            custom_login_enabled: true,
            login_fallback_enabled: true,
            kakao_enabled: false,
            kakao_oidc_ready: false,
            kakao_linking_ready: false,
            google_oauth_enabled: false,
            google_oauth_ready: false,
            apple_oauth_enabled: false,
            apple_oauth_ready: false,
            default_post_login_path: "/post-login".to_owned(),
        },
    )
    .await
    .expect("test app");

    assert_redirect_location(&app, Method::GET, "/", "/_/auth/login").await;
    assert_redirect_location(&app, Method::GET, "/login", "/_/auth/login").await;
    assert_redirect_location(
        &app,
        Method::GET,
        "/login?return_to=%2Fdashboard",
        "/_/auth/login?return_to=%2Fdashboard",
    )
    .await;
    assert_redirect_location(
        &app,
        Method::GET,
        "/login?return_to=https%3A%2F%2Fevil.example&next=%2Freports",
        "/_/auth/login?return_to=%2Freports",
    )
    .await;
    assert_redirect_location(
        &app,
        Method::GET,
        "/?return_to=%2Fdashboard&view=auth0",
        "/_/auth/login?return_to=%2Fdashboard",
    )
    .await;
}

#[tokio::test]
async fn authenticated_login_bypasses_to_default_post_login_path() {
    let app = build_test_app_with_login_routes(
        None,
        "bridge-secret",
        LoginRouteConfig {
            custom_login_enabled: true,
            login_fallback_enabled: true,
            kakao_enabled: false,
            kakao_oidc_ready: false,
            kakao_linking_ready: false,
            google_oauth_enabled: false,
            google_oauth_ready: false,
            apple_oauth_enabled: false,
            apple_oauth_ready: false,
            default_post_login_path: "/dashboard".to_owned(),
        },
    )
    .await
    .expect("test app");

    let (status, headers, _) = app
        .request_response_with_auth(Method::GET, "/login", "alice@example.com", "password123")
        .await
        .expect("authenticated request");

    assert!(
        status.is_redirection(),
        "expected redirect status, got {status}"
    );
    assert_eq!(
        headers
            .get(axum::http::header::LOCATION)
            .and_then(|value| value.to_str().ok()),
        Some("/dashboard"),
    );

    let (status, headers, _) = app
        .request_response_with_auth(
            Method::GET,
            "/login?return_to=%2Fteams%3Ftab%3Dops&next=%2Fignored",
            "alice@example.com",
            "password123",
        )
        .await
        .expect("authenticated request with return_to");

    assert!(
        status.is_redirection(),
        "expected redirect status, got {status}"
    );
    assert_eq!(
        headers
            .get(axum::http::header::LOCATION)
            .and_then(|value| value.to_str().ok()),
        Some("/teams?tab=ops"),
    );

    let (status, headers, _) = app
        .request_response_with_auth(
            Method::GET,
            "/login?return_to=%2Fteams&view=auth0",
            "alice@example.com",
            "password123",
        )
        .await
        .expect("authenticated request with ignored view");

    assert!(
        status.is_redirection(),
        "expected redirect status, got {status}"
    );
    assert_eq!(
        headers
            .get(axum::http::header::LOCATION)
            .and_then(|value| value.to_str().ok()),
        Some("/teams"),
    );
}

#[tokio::test]
async fn authenticated_root_uses_trailbase_login_when_fallback_enabled() {
    let app = build_test_app_with_login_routes(
        None,
        "bridge-secret",
        LoginRouteConfig {
            custom_login_enabled: true,
            login_fallback_enabled: true,
            kakao_enabled: false,
            kakao_oidc_ready: false,
            kakao_linking_ready: false,
            google_oauth_enabled: false,
            google_oauth_ready: false,
            apple_oauth_enabled: false,
            apple_oauth_ready: false,
            default_post_login_path: "/dashboard".to_owned(),
        },
    )
    .await
    .expect("test app");

    let (status, headers, _) = app
        .request_response_with_auth(Method::GET, "/", "alice@example.com", "password123")
        .await
        .expect("authenticated request");

    assert!(
        status.is_redirection(),
        "expected redirect status, got {status}"
    );
    assert_eq!(
        headers
            .get(axum::http::header::LOCATION)
            .and_then(|value| value.to_str().ok()),
        Some("/_/auth/login"),
    );
}

#[tokio::test]
async fn public_dir_preserves_bridge_root_redirects() {
    let temp_dir = tempfile::tempdir().expect("public dir");
    let asset_path = temp_dir.path().join("login-assets");
    fs::create_dir_all(&asset_path).expect("asset dir");
    fs::write(asset_path.join("hello.txt"), "bridge static asset").expect("asset file");

    let app = build_test_app_with_login_routes(
        Some(temp_dir.path().to_path_buf()),
        "bridge-secret",
        LoginRouteConfig {
            custom_login_enabled: true,
            login_fallback_enabled: false,
            kakao_enabled: false,
            kakao_oidc_ready: false,
            kakao_linking_ready: false,
            google_oauth_enabled: false,
            google_oauth_ready: false,
            apple_oauth_enabled: false,
            apple_oauth_ready: false,
            default_post_login_path: "/post-login".to_owned(),
        },
    )
    .await
    .expect("test app");

    assert_redirect_location(&app, Method::GET, "/", "/login").await;
}

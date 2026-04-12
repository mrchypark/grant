use axum::http::{Method, StatusCode, header};
use trailbase_auth_bridge::{LoginRouteConfig, build_test_app_with_login_routes};

fn extract_cookie_value(headers: &axum::http::HeaderMap, cookie_name: &str) -> String {
    let set_cookie = headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find(|value| value.starts_with(&format!("{cookie_name}=")))
        .expect("expected csrf cookie");

    set_cookie
        .split(';')
        .next()
        .and_then(|pair| pair.split_once('='))
        .map(|(_, value)| value.to_owned())
        .expect("cookie value")
}

fn extract_embedded_token(body: &str) -> String {
    extract_meta_content(body, "login-csrf-token")
}

fn extract_meta_content(body: &str, meta_name: &str) -> String {
    let marker = format!("meta name=\"{meta_name}\" content=\"");
    let start = body.find(&marker).expect("meta tag") + marker.len();
    let rest = &body[start..];
    let end = rest.find('"').expect("csrf token terminator");
    rest[..end].to_owned()
}

fn active_custom_login() -> LoginRouteConfig {
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

fn kakao_login_routes(
    kakao_enabled: bool,
    kakao_oidc_ready: bool,
    kakao_linking_ready: bool,
) -> LoginRouteConfig {
    LoginRouteConfig {
        custom_login_enabled: true,
        login_fallback_enabled: false,
        kakao_enabled,
        kakao_oidc_ready,
        kakao_linking_ready,
        google_oauth_enabled: false,
        google_oauth_ready: false,
        apple_oauth_enabled: false,
        apple_oauth_ready: false,
        default_post_login_path: "/post-login".to_owned(),
    }
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

async fn kakao_anchor_for(
    kakao_enabled: bool,
    kakao_oidc_ready: bool,
    kakao_linking_ready: bool,
) -> String {
    let app = build_test_app_with_login_routes(
        None,
        "bridge-secret",
        kakao_login_routes(kakao_enabled, kakao_oidc_ready, kakao_linking_ready),
    )
    .await
    .expect("test app");

    let (status, _, body) = app
        .request_response(Method::GET, "/login")
        .await
        .expect("request");

    assert_eq!(status, StatusCode::OK);

    let body = String::from_utf8(body).expect("utf8");
    provider_anchor(&body, "kakao").to_owned()
}

#[tokio::test]
async fn shell_and_js_contract_use_same_origin_json_login_fetch() {
    let app = build_test_app_with_login_routes(None, "bridge-secret", active_custom_login())
        .await
        .expect("test app");

    let (html_status, _, html_body) = app
        .request_response(Method::GET, "/login")
        .await
        .expect("login shell");
    let (js_status, _, js_body) = app
        .request_response(Method::GET, "/login-assets/login.v1.js")
        .await
        .expect("login script");

    assert_eq!(html_status, StatusCode::OK);
    assert_eq!(js_status, StatusCode::OK);

    let html = String::from_utf8(html_body).expect("utf8");
    let js = String::from_utf8(js_body).expect("utf8");

    assert!(html.contains("meta name=\"login-csrf-token\""));
    assert!(html.contains("/login-assets/login.v1.js"));
    assert!(html.contains("meta name=\"post-login-path\" content=\"/post-login\""));

    for expected in [
        "fetch('/api/auth/v1/login', {",
        "method: 'POST'",
        "credentials: 'same-origin'",
        "'content-type': 'application/json'",
        "'x-login-csrf': token",
        "JSON.stringify",
        "meta[name=\"post-login-path\"]",
        "window.location.assign(postLoginPath)",
    ] {
        assert!(
            js.contains(expected),
            "expected script to contain {expected:?}, got: {js}",
        );
    }
}

#[tokio::test]
async fn login_shell_uses_single_contract_and_ignores_legacy_view_query() {
    let app = build_test_app_with_login_routes(None, "bridge-secret", active_custom_login())
        .await
        .expect("test app");

    let (status, _, body) = app
        .request_response(Method::GET, "/login?return_to=%2Fteams%3Ftab%3Dqa&view=auth0")
        .await
        .expect("login shell");
    let (js_status, _, js_body) = app
        .request_response(Method::GET, "/login-assets/login.v1.js")
        .await
        .expect("login script");

    assert_eq!(status, StatusCode::OK);
    assert_eq!(js_status, StatusCode::OK);

    let html = String::from_utf8(body).expect("utf8");
    let js = String::from_utf8(js_body).expect("utf8");

    assert!(html.contains("Sign in to continue"));
    assert!(html.contains("/login-assets/login.v1.css"));
    assert_eq!(extract_meta_content(&html, "post-login-path"), "/teams?tab=qa");
    assert!(!html.contains("Editorial"));
    assert!(!html.contains("Auth0"));
    assert!(js.contains("fetch('/api/auth/v1/login', {"));
    assert!(js.contains("'x-login-csrf': token"));
}

#[tokio::test]
async fn login_shell_assets_do_not_use_browser_storage() {
    let app = build_test_app_with_login_routes(None, "bridge-secret", active_custom_login())
        .await
        .expect("test app");

    let (_, _, html_body) = app
        .request_response(Method::GET, "/login")
        .await
        .expect("login shell");
    let (_, _, js_body) = app
        .request_response(Method::GET, "/login-assets/login.v1.js")
        .await
        .expect("login script");

    let html = String::from_utf8(html_body).expect("utf8");
    let js = String::from_utf8(js_body).expect("utf8");

    for forbidden in ["localStorage", "sessionStorage"] {
        assert!(
            !html.contains(forbidden),
            "html should not contain {forbidden}"
        );
        assert!(
            !js.contains(forbidden),
            "script should not contain {forbidden}"
        );
    }
}

#[tokio::test]
async fn post_login_rejects_missing_csrf_token_in_active_custom_mode() {
    let app = build_test_app_with_login_routes(None, "bridge-secret", active_custom_login())
        .await
        .expect("test app");

    let (status, _, body) = app
        .request_response_with_headers(
            Method::POST,
            "/api/auth/v1/login",
            &[("content-type", "application/json")],
            br#"{"email":"alice@example.com","password":"password123"}"#,
        )
        .await
        .expect("login request");

    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(
        String::from_utf8(body)
            .expect("utf8")
            .contains("login csrf"),
        "expected csrf rejection body",
    );
}

#[tokio::test]
async fn post_login_accepts_matching_csrf_token_in_active_custom_mode() {
    let app = build_test_app_with_login_routes(None, "bridge-secret", active_custom_login())
        .await
        .expect("test app");

    let (_, headers, body) = app
        .request_response(Method::GET, "/login")
        .await
        .expect("login shell");

    let html = String::from_utf8(body).expect("utf8");
    let embedded_token = extract_embedded_token(&html);
    let cookie_token = extract_cookie_value(&headers, "login_csrf");
    assert_eq!(embedded_token, cookie_token);

    let cookie_header = format!("login_csrf={cookie_token}");
    let (status, _, _) = app
        .request_response_with_headers(
            Method::POST,
            "/api/auth/v1/login",
            &[
                ("content-type", "application/json"),
                ("x-login-csrf", embedded_token.as_str()),
                ("cookie", cookie_header.as_str()),
            ],
            br#"{"email":"alice@example.com","password":"password123"}"#,
        )
        .await
        .expect("login request");

    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn fallback_mode_bypasses_custom_login_csrf_guard() {
    let app = build_test_app_with_login_routes(
        None,
        "bridge-secret",
        LoginRouteConfig {
            custom_login_enabled: true,
            login_fallback_enabled: true,
            kakao_enabled: false,
            kakao_oidc_ready: false,
            kakao_linking_ready: false,
            default_post_login_path: "/post-login".to_owned(),
            google_oauth_enabled: false,
            google_oauth_ready: false,
            apple_oauth_enabled: false,
            apple_oauth_ready: false,
        },
    )
    .await
    .expect("test app");

    let (status, _, _) = app
        .request_response_with_headers(
            Method::POST,
            "/api/auth/v1/login",
            &[("content-type", "application/json")],
            br#"{"email":"alice@example.com","password":"password123"}"#,
        )
        .await
        .expect("login request");

    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn login_shell_escapes_post_login_path_meta_value() {
    let app = build_test_app_with_login_routes(
        None,
        "bridge-secret",
        LoginRouteConfig {
            custom_login_enabled: true,
            login_fallback_enabled: false,
            kakao_enabled: false,
            kakao_oidc_ready: false,
            kakao_linking_ready: false,
            default_post_login_path: "/post-login?tab=1&mode=2".to_owned(),
            google_oauth_enabled: false,
            google_oauth_ready: false,
            apple_oauth_enabled: false,
            apple_oauth_ready: false,
        },
    )
    .await
    .expect("test app");

    let (_, _, body) = app
        .request_response(Method::GET, "/login")
        .await
        .expect("login shell");
    let html = String::from_utf8(body).expect("utf8");

    assert!(
        html.contains("meta name=\"post-login-path\" content=\"/post-login?tab=1&amp;mode=2\""),
        "expected escaped post-login path meta value, got: {html}",
    );
}

#[tokio::test]
async fn login_shell_prefers_return_to_over_next_when_both_are_valid() {
    let app = build_test_app_with_login_routes(None, "bridge-secret", active_custom_login())
        .await
        .expect("test app");

    let (_, _, body) = app
        .request_response(Method::GET, "/login?return_to=%2Fteams%3Ftab%3Dqa&next=%2Fignored")
        .await
        .expect("login shell");

    let html = String::from_utf8(body).expect("utf8");
    assert_eq!(
        extract_meta_content(&html, "post-login-path"),
        "/teams?tab=qa",
    );
}

#[tokio::test]
async fn login_shell_falls_back_to_valid_next_when_return_to_is_invalid() {
    let app = build_test_app_with_login_routes(None, "bridge-secret", active_custom_login())
        .await
        .expect("test app");

    let (_, _, body) = app
        .request_response(
            Method::GET,
            "/login?return_to=https%3A%2F%2Fevil.example&next=%2Fteams",
        )
        .await
        .expect("login shell");

    let html = String::from_utf8(body).expect("utf8");
    assert_eq!(extract_meta_content(&html, "post-login-path"), "/teams");
}

#[tokio::test]
async fn login_shell_rejects_encoded_protocol_relative_return_to() {
    let app = build_test_app_with_login_routes(None, "bridge-secret", active_custom_login())
        .await
        .expect("test app");

    let (_, _, body) = app
        .request_response(Method::GET, "/login?return_to=%2F%2Fevil.example")
        .await
        .expect("login shell");

    let html = String::from_utf8(body).expect("utf8");
    assert_eq!(extract_meta_content(&html, "post-login-path"), "/post-login");
}

#[tokio::test]
async fn login_shell_rejects_auth_surface_return_to_targets() {
    let app = build_test_app_with_login_routes(None, "bridge-secret", active_custom_login())
        .await
        .expect("test app");

    let (_, _, body) = app
        .request_response(Method::GET, "/login?return_to=%2F_%2Fauth%2Flogin")
        .await
        .expect("login shell");

    let html = String::from_utf8(body).expect("utf8");
    assert_eq!(extract_meta_content(&html, "post-login-path"), "/post-login");
}

#[tokio::test]
async fn login_shell_rejects_dot_segment_return_to_targets() {
    let app = build_test_app_with_login_routes(None, "bridge-secret", active_custom_login())
        .await
        .expect("test app");

    let (_, _, body) = app
        .request_response(Method::GET, "/login?return_to=%2Fworkspaces%2F..%2Flogin")
        .await
        .expect("login shell");

    let html = String::from_utf8(body).expect("utf8");
    assert_eq!(extract_meta_content(&html, "post-login-path"), "/post-login");
}

#[tokio::test]
async fn provider_gating_applies_to_the_single_login_shell() {
    let app = build_test_app_with_login_routes(
        None,
        "bridge-secret",
        LoginRouteConfig {
            custom_login_enabled: true,
            login_fallback_enabled: false,
            kakao_enabled: true,
            kakao_oidc_ready: true,
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

    let (_, _, body) = app
        .request_response(Method::GET, "/login?view=auth0")
        .await
        .expect("login shell");
    let html = String::from_utf8(body).expect("utf8");

    assert!(
        provider_anchor(&html, "google").contains(" hidden"),
        "expected google hidden in the single login shell when readiness is false, got: {html}",
    );
    assert!(
        provider_anchor(&html, "kakao").contains(" hidden"),
        "expected kakao hidden in the single login shell when linking gate is false, got: {html}",
    );
    assert!(
        provider_anchor(&html, "apple").contains(" hidden"),
        "expected apple hidden in the single login shell when readiness is false, got: {html}",
    );
}

#[tokio::test]
async fn kakao_button_stays_hidden_when_kakao_rollout_is_disabled() {
    let kakao_anchor = kakao_anchor_for(false, true, true).await;
    assert!(
        kakao_anchor.contains("data-provider-rollout-enabled=\"false\"")
            && kakao_anchor.contains("data-provider-oidc-ready=\"true\"")
            && kakao_anchor.contains("data-provider-linking-ready=\"true\"")
            && kakao_anchor.contains("href=\"/api/auth/v1/oauth/oidc0/login\"")
            && kakao_anchor.contains(" hidden"),
        "kakao oauth should stay hidden when rollout is disabled, got anchor: {kakao_anchor}",
    );
}

#[tokio::test]
async fn kakao_button_is_visible_only_when_all_kakao_safety_gates_are_true() {
    let app = build_test_app_with_login_routes(
        None,
        "bridge-secret",
        kakao_login_routes(true, true, true),
    )
    .await
    .expect("test app");

    let (status, _, body) = app
        .request_response(Method::GET, "/login")
        .await
        .expect("request");

    assert_eq!(status, StatusCode::OK);

    let body = String::from_utf8(body).expect("utf8");
    let kakao_anchor = provider_anchor(&body, "kakao");
    assert!(
        kakao_anchor.contains("data-provider-rollout-enabled=\"true\"")
            && kakao_anchor.contains("data-provider-oidc-ready=\"true\"")
            && kakao_anchor.contains("data-provider-linking-ready=\"true\"")
            && kakao_anchor.contains("href=\"/api/auth/v1/oauth/oidc0/login\"")
            && !kakao_anchor.contains(" hidden")
            && body.contains("Continue with Kakao"),
        "kakao oauth should be visible only when rollout, OIDC proof, and linking compatibility are all enabled, got body: {body}",
    );
}

#[tokio::test]
async fn kakao_button_stays_hidden_until_oidc_proof_is_complete() {
    let kakao_anchor = kakao_anchor_for(true, false, true).await;

    assert!(
        kakao_anchor.contains("data-provider-rollout-enabled=\"true\"")
            && kakao_anchor.contains("data-provider-oidc-ready=\"false\"")
            && kakao_anchor.contains("data-provider-linking-ready=\"true\"")
            && kakao_anchor.contains(" hidden"),
        "kakao oauth should stay hidden until OIDC proof is complete, got anchor: {kakao_anchor}",
    );
}

#[tokio::test]
async fn kakao_button_stays_hidden_until_linking_policy_is_verified() {
    let kakao_anchor = kakao_anchor_for(true, true, false).await;

    assert!(
        kakao_anchor.contains("data-provider-rollout-enabled=\"true\"")
            && kakao_anchor.contains("data-provider-oidc-ready=\"true\"")
            && kakao_anchor.contains("data-provider-linking-ready=\"false\"")
            && kakao_anchor.contains(" hidden"),
        "kakao oauth should stay hidden until linking policy is verified, got anchor: {kakao_anchor}",
    );
}

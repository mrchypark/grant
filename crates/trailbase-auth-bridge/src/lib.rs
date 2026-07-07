use anyhow::{Context, Result, anyhow};
use axum::{
    Json, Router,
    extract::{FromRef, Path, State},
    http::{HeaderMap, Method, StatusCode, Uri, header},
    middleware::{self, Next},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path as FsPath, PathBuf},
    sync::Arc,
};
use tower::util::ServiceExt;
use trailbase::constants::COOKIE_AUTH_TOKEN;
use trailbase::{AppState, DataDir, Server, ServerOptions, User};
use trailbase_sqlite::params;
use uuid::Uuid;

pub const INTROSPECT_PATH: &str = "/api/grant/introspect";
pub const INTERNAL_SECRET_HEADER: &str = "x-grant-internal-secret";
pub const LOGIN_CSS_PATH: &str = "/login-assets/login.v1.css";
pub const LOGIN_JS_PATH: &str = "/login-assets/login.v1.js";
const LOGIN_CSRF_COOKIE_NAME: &str = "login_csrf";
const LOGIN_CSRF_HEADER_NAME: &str = "x-login-csrf";
const LOGIN_CSRF_TEMPLATE_MARKER: &str = "__LOGIN_CSRF_TOKEN__";
const POST_LOGIN_PATH_TEMPLATE_MARKER: &str = "__POST_LOGIN_PATH__";
const GOOGLE_OAUTH_ROLLOUT_ENABLED_TEMPLATE_MARKER: &str = "__GOOGLE_OAUTH_ROLLOUT_ENABLED__";
const GOOGLE_OAUTH_READY_TEMPLATE_MARKER: &str = "__GOOGLE_OAUTH_READY__";
const GOOGLE_OAUTH_HIDDEN_TEMPLATE_MARKER: &str = "__GOOGLE_OAUTH_HIDDEN__";
const KAKAO_OAUTH_ROLLOUT_ENABLED_TEMPLATE_MARKER: &str = "__KAKAO_OAUTH_ROLLOUT_ENABLED__";
const KAKAO_OAUTH_OIDC_READY_TEMPLATE_MARKER: &str = "__KAKAO_OIDC_READY__";
const KAKAO_OAUTH_LINKING_READY_TEMPLATE_MARKER: &str = "__KAKAO_LINKING_READY__";
const KAKAO_OAUTH_HIDDEN_TEMPLATE_MARKER: &str = "__KAKAO_OAUTH_HIDDEN__";
const APPLE_OAUTH_ROLLOUT_ENABLED_TEMPLATE_MARKER: &str = "__APPLE_OAUTH_ROLLOUT_ENABLED__";
const APPLE_OAUTH_READY_TEMPLATE_MARKER: &str = "__APPLE_OAUTH_READY__";
const APPLE_OAUTH_HIDDEN_TEMPLATE_MARKER: &str = "__APPLE_OAUTH_HIDDEN__";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeConfig {
    pub data_dir: PathBuf,
    pub address: String,
    pub admin_address: Option<String>,
    pub internal_secret: String,
    pub dev: bool,
    pub log_responses: bool,
    pub public_dir: Option<PathBuf>,
    pub custom_login_enabled: bool,
    pub login_fallback_enabled: bool,
    pub kakao_enabled: bool,
    pub kakao_oidc_ready: bool,
    pub kakao_linking_ready: bool,
    pub google_oauth_enabled: bool,
    pub google_oauth_ready: bool,
    pub apple_oauth_enabled: bool,
    pub apple_oauth_ready: bool,
    pub default_post_login_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginRouteConfig {
    pub custom_login_enabled: bool,
    pub login_fallback_enabled: bool,
    pub google_oauth_enabled: bool,
    pub google_oauth_ready: bool,
    pub kakao_enabled: bool,
    pub kakao_oidc_ready: bool,
    pub kakao_linking_ready: bool,
    pub apple_oauth_enabled: bool,
    pub apple_oauth_ready: bool,
    pub default_post_login_path: String,
}

impl Default for LoginRouteConfig {
    fn default() -> Self {
        Self {
            custom_login_enabled: false,
            login_fallback_enabled: true,
            kakao_enabled: false,
            kakao_oidc_ready: false,
            kakao_linking_ready: false,
            google_oauth_enabled: false,
            google_oauth_ready: false,
            apple_oauth_enabled: false,
            apple_oauth_ready: false,
            default_post_login_path: "/app".to_owned(),
        }
    }
}

impl From<&BridgeConfig> for LoginRouteConfig {
    fn from(value: &BridgeConfig) -> Self {
        Self {
            custom_login_enabled: value.custom_login_enabled,
            login_fallback_enabled: value.login_fallback_enabled,
            kakao_enabled: value.kakao_enabled,
            kakao_oidc_ready: value.kakao_oidc_ready,
            kakao_linking_ready: value.kakao_linking_ready,
            google_oauth_enabled: value.google_oauth_enabled,
            google_oauth_ready: value.google_oauth_ready,
            apple_oauth_enabled: value.apple_oauth_enabled,
            apple_oauth_ready: value.apple_oauth_ready,
            default_post_login_path: value.default_post_login_path.clone(),
        }
    }
}

impl BridgeConfig {
    pub fn from_env() -> Result<Self> {
        let internal_secret = std::env::var("TRAILBASE_GRANT_INTERNAL_SECRET")
            .context("missing TRAILBASE_GRANT_INTERNAL_SECRET")?;
        if internal_secret.trim().is_empty() {
            anyhow::bail!("TRAILBASE_GRANT_INTERNAL_SECRET must not be empty");
        }
        let default_post_login_path = normalize_post_login_path(
            &std::env::var("TRAILBASE_DEFAULT_POST_LOGIN_PATH")
                .unwrap_or_else(|_| "/app".to_owned()),
            "TRAILBASE_DEFAULT_POST_LOGIN_PATH",
        )?;

        Ok(Self {
            data_dir: std::env::var("TRAILBASE_DATA_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("./traildepot")),
            address: std::env::var("TRAILBASE_ADDRESS")
                .unwrap_or_else(|_| "0.0.0.0:4000".to_owned()),
            admin_address: std::env::var("TRAILBASE_ADMIN_ADDRESS").ok(),
            internal_secret,
            dev: env_bool("TRAILBASE_DEV", false),
            log_responses: env_bool("TRAILBASE_LOG_RESPONSES", true),
            public_dir: std::env::var("TRAILBASE_PUBLIC_DIR")
                .ok()
                .map(PathBuf::from),
            custom_login_enabled: env_bool_strict("TRAILBASE_CUSTOM_LOGIN_ENABLED", false)?,
            login_fallback_enabled: env_bool_strict("TRAILBASE_LOGIN_FALLBACK_ENABLED", true)?,
            kakao_enabled: env_bool_strict("TRAILBASE_KAKAO_ENABLED", false)?,
            kakao_oidc_ready: env_bool_strict("TRAILBASE_KAKAO_OIDC_READY", false)?,
            kakao_linking_ready: env_bool_strict("TRAILBASE_KAKAO_LINKING_READY", false)?,
            google_oauth_enabled: env_bool_strict("TRAILBASE_GOOGLE_OAUTH_ENABLED", false)?,
            google_oauth_ready: env_bool_strict("TRAILBASE_GOOGLE_OAUTH_READY", false)?,
            apple_oauth_enabled: env_bool_strict("TRAILBASE_APPLE_OAUTH_ENABLED", false)?,
            apple_oauth_ready: env_bool_strict("TRAILBASE_APPLE_OAUTH_READY", false)?,
            default_post_login_path,
        })
    }
}

#[derive(Clone)]
struct BridgeState {
    app_state: AppState,
    jwt: Arc<trailbase::api::JwtHelper>,
    internal_secret: String,
    login_routes: LoginRouteConfig,
}

impl FromRef<BridgeState> for AppState {
    fn from_ref(value: &BridgeState) -> Self {
        value.app_state.clone()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IntrospectionResponse {
    pub valid: bool,
    pub trailbase_user_id: String,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub verified: bool,
    pub active: bool,
    pub expires_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct VerifiedRow {
    verified: bool,
}

pub async fn run(config: BridgeConfig) -> Result<()> {
    ensure_auth_ui_component(&config.data_dir)?;
    let login_routes = LoginRouteConfig::from(&config);
    let Server {
        state,
        main_router,
        admin_router,
        tls,
    } = Server::init(ServerOptions {
        data_dir: DataDir(config.data_dir),
        address: config.address,
        admin_address: config.admin_address,
        public_dir: config.public_dir.clone(),
        log_responses: config.log_responses,
        dev: config.dev,
        cors_allowed_origins: vec![],
        ..Default::default()
    })
    .await?;
    ensure_core_auth_schema(&state).await?;
    ensure_session_auth_schema(&state).await?;

    let router = (
        main_router.0,
        build_router(state, main_router.1, config.internal_secret, login_routes).await?,
    );

    trailbase::api::serve(router, admin_router, tls)
        .await
        .map_err(|err| anyhow!(err.to_string()))?;
    Ok(())
}

pub async fn seed_user(data_dir: PathBuf, email: &str, password: &str) -> Result<()> {
    let server = Server::init(ServerOptions {
        data_dir: DataDir(data_dir),
        address: "127.0.0.1:0".to_owned(),
        admin_address: None,
        public_dir: None,
        log_responses: false,
        dev: true,
        cors_allowed_origins: vec![],
        ..Default::default()
    })
    .await
    .context("init trailbase for seed-user")?;
    ensure_core_auth_schema(&server.state).await?;
    ensure_session_auth_schema(&server.state).await?;

    trailbase::api::cli::add_user(server.state.user_conn(), email, password)
        .await
        .with_context(|| format!("seed trailbase user {email}"))?;
    Ok(())
}

pub async fn build_router(
    app_state: AppState,
    main_router: Router,
    internal_secret: String,
    login_routes: LoginRouteConfig,
) -> Result<Router> {
    let jwt = Arc::new(
        trailbase::api::JwtHelper::init_from_path(app_state.data_dir())
            .await
            .context("initialize auth bridge JWT helper")?,
    );
    let state = BridgeState {
        app_state,
        jwt,
        internal_secret,
        login_routes,
    };
    let main_router = main_router.layer(middleware::from_fn_with_state(
        state.clone(),
        require_login_security,
    ));
    Ok(Router::new()
        .route("/healthz", get(health_handler))
        .route("/", get(root_entry_handler))
        .route("/login", get(login_entry_handler))
        .route(LOGIN_CSS_PATH, get(login_stylesheet_handler))
        .route(LOGIN_JS_PATH, get(login_script_handler))
        .route(
            INTROSPECT_PATH,
            post(introspect_handler).route_layer(middleware::from_fn_with_state(
                state.clone(),
                require_internal_secret,
            )),
        )
        .with_state(state)
        .merge(main_router))
}

async fn health_handler() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

async fn root_entry_handler(
    State(state): State<BridgeState>,
    headers: HeaderMap,
    uri: Uri,
) -> Response {
    let target = resolve_post_login_target(uri.query(), &state.login_routes.default_post_login_path);

    if state.login_routes.login_fallback_enabled || !state.login_routes.custom_login_enabled {
        return with_login_entry_cache_headers(
            Redirect::to(&login_entry_location("/_/auth/login", &target)).into_response(),
        );
    }

    if is_authenticated(&state, &headers) {
        return with_login_entry_cache_headers(Redirect::to(&target.path).into_response());
    }

    with_login_entry_cache_headers(Redirect::to(&login_entry_location("/login", &target)).into_response())
}

async fn login_entry_handler(
    State(state): State<BridgeState>,
    headers: HeaderMap,
    uri: Uri,
) -> Response {
    let target = resolve_post_login_target(uri.query(), &state.login_routes.default_post_login_path);

    if is_authenticated(&state, &headers) {
        return with_login_entry_cache_headers(Redirect::to(&target.path).into_response());
    }

    if state.login_routes.login_fallback_enabled {
        return with_login_entry_cache_headers(
            Redirect::to(&login_entry_location("/_/auth/login", &target)).into_response(),
        );
    }

    if state.login_routes.custom_login_enabled {
        return login_shell_handler(&state.login_routes, &target.path);
    }

    with_login_entry_cache_headers(
        Redirect::to(&login_entry_location("/_/auth/login", &target)).into_response(),
    )
}

fn login_shell_handler(login_routes: &LoginRouteConfig, post_login_path: &str) -> Response {
    let csrf_token = Uuid::new_v4().simple().to_string();
    let csrf_cookie = format!(
        "{LOGIN_CSRF_COOKIE_NAME}={csrf_token}; Path=/api/auth/v1/login; HttpOnly; SameSite=Strict"
    );
    let google_visible = login_routes.google_oauth_enabled && login_routes.google_oauth_ready;
    let kakao_visible = login_routes.kakao_enabled
        && login_routes.kakao_oidc_ready
        && login_routes.kakao_linking_ready;
    let apple_visible = login_routes.apple_oauth_enabled && login_routes.apple_oauth_ready;
    let rendered_html = LOGIN_SHELL_HTML
        .replace(LOGIN_CSRF_TEMPLATE_MARKER, &csrf_token)
        .replace(
            POST_LOGIN_PATH_TEMPLATE_MARKER,
            &escape_html_attribute(post_login_path),
        )
        .replace(
            GOOGLE_OAUTH_ROLLOUT_ENABLED_TEMPLATE_MARKER,
            bool_html_attribute(login_routes.google_oauth_enabled),
        )
        .replace(
            GOOGLE_OAUTH_READY_TEMPLATE_MARKER,
            bool_html_attribute(login_routes.google_oauth_ready),
        )
        .replace(
            GOOGLE_OAUTH_HIDDEN_TEMPLATE_MARKER,
            hidden_html_attribute(google_visible),
        )
        .replace(
            KAKAO_OAUTH_ROLLOUT_ENABLED_TEMPLATE_MARKER,
            bool_html_attribute(login_routes.kakao_enabled),
        )
        .replace(
            KAKAO_OAUTH_OIDC_READY_TEMPLATE_MARKER,
            bool_html_attribute(login_routes.kakao_oidc_ready),
        )
        .replace(
            KAKAO_OAUTH_LINKING_READY_TEMPLATE_MARKER,
            bool_html_attribute(login_routes.kakao_linking_ready),
        )
        .replace(
            KAKAO_OAUTH_HIDDEN_TEMPLATE_MARKER,
            hidden_html_attribute(kakao_visible),
        )
        .replace(
            APPLE_OAUTH_ROLLOUT_ENABLED_TEMPLATE_MARKER,
            bool_html_attribute(login_routes.apple_oauth_enabled),
        )
        .replace(
            APPLE_OAUTH_READY_TEMPLATE_MARKER,
            bool_html_attribute(login_routes.apple_oauth_ready),
        )
        .replace(
            APPLE_OAUTH_HIDDEN_TEMPLATE_MARKER,
            hidden_html_attribute(apple_visible),
        );
    let mut response = (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("text/html; charset=utf-8"),
        )],
        rendered_html,
    )
        .into_response();
    response.headers_mut().append(
        header::SET_COOKIE,
        header::HeaderValue::from_str(&csrf_cookie)
            .expect("login csrf cookie must serialize to a valid header value"),
    );
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        header::HeaderValue::from_static(
            "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'",
        ),
    );
    response.headers_mut().insert(
        header::X_FRAME_OPTIONS,
        header::HeaderValue::from_static("DENY"),
    );
    with_login_entry_cache_headers(response)
}

async fn login_stylesheet_handler() -> Response {
    with_versioned_asset_cache_headers(
        (
            StatusCode::OK,
            [(
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("text/css; charset=utf-8"),
            )],
            LOGIN_SHELL_CSS,
        )
            .into_response(),
    )
}

async fn login_script_handler() -> Response {
    with_versioned_asset_cache_headers(
        (
            StatusCode::OK,
            [(
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("application/javascript; charset=utf-8"),
            )],
            LOGIN_SHELL_JS,
        )
            .into_response(),
    )
}

fn with_login_entry_cache_headers(mut response: Response) -> Response {
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("private, no-store"),
    );
    response.headers_mut().insert(
        header::VARY,
        header::HeaderValue::from_static("authorization, cookie"),
    );
    response
}

fn with_versioned_asset_cache_headers(mut response: Response) -> Response {
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("public, max-age=31536000, immutable"),
    );
    response
}

async fn require_internal_secret(
    State(state): State<BridgeState>,
    headers: HeaderMap,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let provided = headers
        .get(INTERNAL_SECRET_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .unwrap_or_default();

    if provided.is_empty() || provided != state.internal_secret {
        return (StatusCode::UNAUTHORIZED, "invalid internal secret").into_response();
    }

    next.run(request).await
}

async fn require_login_security(
    State(state): State<BridgeState>,
    headers: HeaderMap,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    if request.method() != Method::POST
        || request.uri().path() != "/api/auth/v1/login"
        || state.login_routes.login_fallback_enabled
        || !state.login_routes.custom_login_enabled
    {
        return next.run(request).await;
    }

    if !is_json_content_type(&headers) {
        return (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "content-type must be application/json",
        )
            .into_response();
    }

    if !origin_matches_host(&headers) {
        return (StatusCode::FORBIDDEN, "origin must match host").into_response();
    }

    let csrf_header = headers
        .get(LOGIN_CSRF_HEADER_NAME)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let csrf_cookie = request_cookie(headers.get(header::COOKIE), LOGIN_CSRF_COOKIE_NAME);

    match (csrf_cookie.as_deref(), csrf_header) {
        (Some(cookie), Some(header_value)) if cookie == header_value => next.run(request).await,
        _ => (StatusCode::FORBIDDEN, "login csrf token mismatch").into_response(),
    }
}

fn is_authenticated(state: &BridgeState, headers: &HeaderMap) -> bool {
    let Some(auth_token) = request_auth_token(headers) else {
        return false;
    };

    trailbase::api::AuthTokenClaims::from_auth_token(state.jwt.as_ref(), &auth_token).is_ok()
}

fn request_auth_token(headers: &HeaderMap) -> Option<String> {
    if let Some(token) = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
    {
        return Some(token.to_owned());
    }

    let cookie_header = headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())?;

    for cookie in cookie_header.split(';') {
        let cookie = cookie.trim();
        if let Some(token) = cookie.strip_prefix(&format!("{COOKIE_AUTH_TOKEN}=")) {
            return Some(token.to_owned());
        }
    }

    None
}

fn request_cookie(
    cookie_header: Option<&header::HeaderValue>,
    cookie_name: &str,
) -> Option<String> {
    let cookie_header = cookie_header.and_then(|value| value.to_str().ok())?;

    for cookie in cookie_header.split(';') {
        let cookie = cookie.trim();
        if let Some(token) = cookie.strip_prefix(&format!("{cookie_name}=")) {
            return Some(token.to_owned());
        }
    }

    None
}

fn is_json_content_type(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .map(|value| value.eq_ignore_ascii_case("application/json"))
        .unwrap_or(false)
}

fn origin_matches_host(headers: &HeaderMap) -> bool {
    let Some(origin) = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    else {
        return true;
    };
    let Some(host) = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    let Some(origin) = origin
        .strip_prefix("https://")
        .or_else(|| origin.strip_prefix("http://"))
    else {
        return false;
    };
    let origin_host = origin.split('/').next().unwrap_or(origin);
    origin_host.eq_ignore_ascii_case(host)
}

fn normalize_post_login_path(value: &str, key: &str) -> Result<String> {
    let path = value.trim();
    if path.is_empty() {
        anyhow::bail!("{key} must not be empty");
    }
    if !path.starts_with('/') || path.starts_with("//") {
        anyhow::bail!("{key} must be a same-origin absolute path");
    }
    let path_component = path.split(['?', '#']).next().unwrap_or(path);
    if path_component == "/login"
        || path_component.starts_with("/_/auth/")
        || path_component.starts_with("/api/auth/")
    {
        anyhow::bail!("{key} must not target auth entry routes");
    }
    if path.split('/').any(|segment| segment == "." || segment == "..") {
        anyhow::bail!("{key} must not contain dot segments");
    }
    if path.contains('\\') {
        anyhow::bail!("{key} must not contain backslashes");
    }
    if path.chars().any(|ch| ch.is_control()) {
        anyhow::bail!("{key} must not contain control characters");
    }

    Ok(path.to_owned())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedPostLoginTarget {
    path: String,
    from_query: bool,
}

fn resolve_post_login_target(query: Option<&str>, default_path: &str) -> ResolvedPostLoginTarget {
    let query: Vec<(String, String)> =
        serde_urlencoded::from_str(query.unwrap_or("")).unwrap_or_default();

    if let Some(path) = normalized_query_target(&query, "return_to") {
        return ResolvedPostLoginTarget {
            path,
            from_query: true,
        };
    }
    if let Some(path) = normalized_query_target(&query, "next") {
        return ResolvedPostLoginTarget {
            path,
            from_query: true,
        };
    }

    ResolvedPostLoginTarget {
        path: default_path.to_owned(),
        from_query: false,
    }
}

fn normalized_query_target(values: &[(String, String)], key: &str) -> Option<String> {
    let matches: Vec<&str> = values
        .iter()
        .filter(|(candidate, _)| candidate == key)
        .map(|(_, value)| value.as_str())
        .collect();
    if matches.len() != 1 {
        return None;
    }

    normalize_post_login_path(matches[0], key).ok()
}

fn login_entry_location(base_path: &str, target: &ResolvedPostLoginTarget) -> String {
    if !target.from_query {
        return base_path.to_owned();
    }

    let query = serde_urlencoded::to_string([("return_to", target.path.as_str())])
        .expect("validated login target query must serialize");
    format!("{base_path}?{query}")
}

fn escape_html_attribute(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

fn bool_html_attribute(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

fn hidden_html_attribute(visible: bool) -> &'static str {
    if visible { "" } else { "hidden" }
}

async fn introspect_handler(
    State(state): State<BridgeState>,
    user: User,
) -> Result<Json<IntrospectionResponse>, StatusCode> {
    let verified = state
        .app_state
        .user_conn()
        .read_query_value::<VerifiedRow>(
            r#"SELECT verified FROM "_user" WHERE id = $1"#,
            params!(user.uuid.into_bytes()),
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map(|row| row.verified)
        .unwrap_or(false);

    Ok(Json(IntrospectionResponse {
        valid: true,
        trailbase_user_id: user.id,
        email: Some(user.email.clone()),
        display_name: Some(user.email),
        verified,
        active: true,
        expires_at: None,
    }))
}

pub struct TestApp {
    router: Router,
    state: AppState,
    _base: std::sync::Arc<BaseTestServer>,
}

static TEST_SERVER_INIT_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> =
    std::sync::OnceLock::new();
static BASE_TEST_SERVER: std::sync::OnceLock<std::sync::Arc<BaseTestServer>> =
    std::sync::OnceLock::new();

fn test_server_init_lock() -> std::sync::MutexGuard<'static, ()> {
    TEST_SERVER_INIT_LOCK
        .get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct BaseTestServer {
    state: AppState,
    _root_dir: PathBuf,
}

async fn base_test_server() -> Result<std::sync::Arc<BaseTestServer>> {
    if let Some(server) = BASE_TEST_SERVER.get() {
        return Ok(server.clone());
    }

    let _guard = test_server_init_lock();
    if let Some(server) = BASE_TEST_SERVER.get() {
        return Ok(server.clone());
    }

    let root_dir = tempfile::tempdir().context("create bridge test root dir")?;
    let root_dir = root_dir.keep();
    let data_dir = root_dir.join("traildepot");
    let public_dir = root_dir.join("public");
    ensure_auth_ui_component(&data_dir)?;
    fs::create_dir_all(&public_dir).context("create bridge test public dir")?;
    let server = Server::init(ServerOptions {
        data_dir: DataDir(data_dir),
        address: "127.0.0.1:0".to_owned(),
        admin_address: None,
        public_dir: Some(public_dir.clone()),
        log_responses: false,
        dev: true,
        cors_allowed_origins: vec![],
        ..Default::default()
    })
    .await
    .context("init trailbase test server")?;
    ensure_core_auth_schema(&server.state).await?;
    ensure_session_auth_schema(&server.state).await?;

    trailbase::api::cli::add_user(server.state.user_conn(), "alice@example.com", "password123")
        .await
        .context("create trailbase test user")?;

    let base = std::sync::Arc::new(BaseTestServer {
        state: server.state,
        _root_dir: root_dir,
    });
    let _ = BASE_TEST_SERVER.set(base.clone());
    Ok(base)
}

impl TestApp {
    async fn execute_request(
        &self,
        request: axum::http::Request<axum::body::Body>,
    ) -> Result<(StatusCode, HeaderMap, Vec<u8>), String> {
        let response = self
            .router
            .clone()
            .oneshot(request)
            .await
            .map_err(|err| err.to_string())?;
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = http_body_util::BodyExt::collect(response.into_body())
            .await
            .map_err(|err| err.to_string())?
            .to_bytes()
            .to_vec();

        Ok((status, headers, bytes))
    }

    pub async fn introspect(
        &self,
        email: &str,
        password: &str,
        internal_secret: &str,
    ) -> Result<IntrospectionResponse, String> {
        let token = mint_auth_token_for_email(&self.state, email, password)
            .await
            .map_err(|err| err.to_string())?;

        let request = axum::http::Request::builder()
            .method("POST")
            .uri(INTROSPECT_PATH)
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header(INTERNAL_SECRET_HEADER, internal_secret)
            .body(axum::body::Body::empty())
            .map_err(|err| err.to_string())?;

        let (status, _, bytes) = self.execute_request(request).await?;
        let bytes = bytes;
        if !status.is_success() {
            return Err(format!("{status}: {}", String::from_utf8_lossy(&bytes)));
        }
        serde_json::from_slice(&bytes).map_err(|err| err.to_string())
    }

    pub async fn request_response(
        &self,
        method: Method,
        uri: &str,
    ) -> Result<(StatusCode, HeaderMap, Vec<u8>), String> {
        let request = axum::http::Request::builder()
            .method(method)
            .uri(uri)
            .body(axum::body::Body::empty())
            .map_err(|err| err.to_string())?;

        self.execute_request(request).await
    }

    pub async fn request_response_with_headers(
        &self,
        method: Method,
        uri: &str,
        headers: &[(&str, &str)],
        body: &[u8],
    ) -> Result<(StatusCode, HeaderMap, Vec<u8>), String> {
        let mut request = axum::http::Request::builder().method(method).uri(uri);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let request = request
            .body(axum::body::Body::from(body.to_vec()))
            .map_err(|err| err.to_string())?;

        self.execute_request(request).await
    }

    pub async fn request_bytes(
        &self,
        method: Method,
        uri: &str,
    ) -> Result<(StatusCode, Vec<u8>), String> {
        let (status, _, bytes) = self.request_response(method, uri).await?;
        Ok((status, bytes))
    }

    pub async fn request_response_with_auth(
        &self,
        method: Method,
        uri: &str,
        email: &str,
        password: &str,
    ) -> Result<(StatusCode, HeaderMap, Vec<u8>), String> {
        let token = mint_auth_token_for_email(&self.state, email, password)
            .await
            .map_err(|err| err.to_string())?;

        let request = axum::http::Request::builder()
            .method(method)
            .uri(uri)
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .map_err(|err| err.to_string())?;

        self.execute_request(request).await
    }
}

pub async fn build_test_app(internal_secret: &str) -> Result<TestApp> {
    build_test_app_with_login_routes(None, internal_secret, LoginRouteConfig::default()).await
}

pub async fn build_test_app_with_public_dir(
    public_dir: Option<PathBuf>,
    internal_secret: &str,
) -> Result<TestApp> {
    build_test_app_with_login_routes(public_dir, internal_secret, LoginRouteConfig::default()).await
}

pub async fn build_test_app_with_login_routes(
    public_dir: Option<PathBuf>,
    internal_secret: &str,
    login_routes: LoginRouteConfig,
) -> Result<TestApp> {
    let base = base_test_server().await?;
    let router = build_router(
        base.state.clone(),
        build_test_main_router(public_dir),
        internal_secret.to_owned(),
        login_routes,
    )
    .await?;

    Ok(TestApp {
        router,
        state: base.state.clone(),
        _base: base,
    })
}

fn build_test_main_router(public_dir: Option<PathBuf>) -> Router {
    let router = Router::new()
        .route("/api/auth/v1/login", post(|| async { StatusCode::OK }))
        .route("/_/auth/login", get(|| async { StatusCode::OK }));

    if let Some(public_dir) = public_dir {
        router.route(
            "/login-assets/{*path}",
            get(move |Path(path): Path<String>| {
                let public_dir = public_dir.clone();
                async move {
                    let file_path = public_dir.join("login-assets").join(path);
                    match tokio::fs::read(file_path).await {
                        Ok(bytes) => (StatusCode::OK, bytes).into_response(),
                        Err(_) => (StatusCode::NOT_FOUND, "Not found").into_response(),
                    }
                }
            }),
        )
    } else {
        router
    }
}

fn ensure_auth_ui_component(data_dir: &FsPath) -> Result<()> {
    let wasm_dir = data_dir.join("wasm");
    let wasm_path = wasm_dir.join("auth_ui_component.wasm");
    if wasm_path.exists() {
        return Ok(());
    }
    fs::create_dir_all(&wasm_dir).context("create TrailBase wasm dir")?;
    fs::write(&wasm_path, AUTH_UI_COMPONENT).context("write auth_ui_component.wasm")?;
    Ok(())
}

const AUTH_UI_COMPONENT: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/auth_ui_component.wasm"));
const LOGIN_SHELL_HTML: &str = include_str!("../static/login/index.html");
const LOGIN_SHELL_CSS: &str = include_str!("../static/login/login.v1.css");
const LOGIN_SHELL_JS: &str = include_str!("../static/login/login.v1.js");

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cached_jwt_helper_survives_key_file_rotation() {
        let root_dir = tempfile::tempdir().expect("temp dir");
        let data_dir = root_dir.path().join("traildepot");
        let public_dir = root_dir.path().join("public");
        ensure_auth_ui_component(&data_dir).expect("auth ui component");
        fs::create_dir_all(&public_dir).expect("create public dir");

        let server = Server::init(ServerOptions {
            data_dir: DataDir(data_dir.clone()),
            address: "127.0.0.1:0".to_owned(),
            admin_address: None,
            public_dir: Some(public_dir),
            log_responses: false,
            dev: true,
            cors_allowed_origins: vec![],
            ..Default::default()
        })
        .await
        .expect("init trailbase test server");
        ensure_core_auth_schema(&server.state)
            .await
            .expect("core auth schema");
        ensure_session_auth_schema(&server.state)
            .await
            .expect("session auth schema");
        trailbase::api::cli::add_user(server.state.user_conn(), "alice@example.com", "password123")
            .await
            .expect("create trailbase test user");

        let router = build_router(
            server.state.clone(),
            build_test_main_router(None),
            "bridge-secret".to_owned(),
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
        .expect("build router");
        let token = mint_auth_token_for_email(&server.state, "alice@example.com", "password123")
            .await
            .expect("mint auth token");

        let key_dir = server.state.data_dir().key_path();
        std::fs::remove_file(key_dir.join("private_key.pem")).expect("remove private key");
        std::fs::remove_file(key_dir.join("public_key.pem")).expect("remove public key");

        let response = router
            .oneshot(
                axum::http::Request::builder()
                    .method(Method::GET)
                    .uri("/login")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(axum::body::Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            response
                .headers()
                .get(header::LOCATION)
                .and_then(|value| value.to_str().ok()),
            Some("/post-login"),
        );
    }
}

async fn mint_auth_token_for_email(
    app_state: &AppState,
    email: &str,
    password: &str,
) -> Result<String> {
    let _ = password;
    trailbase::api::cli::mint_auth_token(
        app_state.data_dir(),
        app_state.user_conn(),
        app_state.session_conn(),
        trailbase::api::cli::UserReference::Email(email.to_owned()),
    )
    .await
    .map_err(|err| anyhow!(err.to_string()))
}

fn env_bool(key: &str, default: bool) -> bool {
    std::env::var(key)
        .ok()
        .and_then(|value| match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        })
        .unwrap_or(default)
}

fn env_bool_strict(key: &str, default: bool) -> Result<bool> {
    match std::env::var(key) {
        Ok(value) => match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Ok(true),
            "0" | "false" | "no" | "off" => Ok(false),
            _ => anyhow::bail!("{key} must be a boolean value"),
        },
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(std::env::VarError::NotUnicode(_)) => anyhow::bail!("{key} must be a boolean value"),
    }
}

async fn ensure_core_auth_schema(state: &AppState) -> Result<()> {
    let has_user_table = state
        .user_conn()
        .read_query_row_f(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = '_user')",
            (),
            |row| row.get::<_, bool>(0),
        )
        .await
        .context("check _user table")?
        .unwrap_or(false);

    if has_user_table {
        let has_totp_secret = state
            .user_conn()
            .read_query_row_f(r#"SELECT EXISTS(SELECT 1 FROM pragma_table_info('_user') WHERE name = 'totp_secret')"#, (), |row| {
                row.get::<_, bool>(0)
            })
            .await
            .context("check _user.totp_secret column")?
            .unwrap_or(false);

        if has_totp_secret {
            return Ok(());
        }

        state
            .user_conn()
            .execute_batch(CORE_MAIN_USER_UPGRADE_SCHEMA.to_owned())
            .await
            .context("upgrade fallback TrailBase _user schema")?;
        return Ok(());
    }

    state
        .user_conn()
        .execute_batch(format!("{CORE_MAIN_SCHEMA}\n{CORE_BASE_SCHEMA}"))
        .await
        .context("apply fallback TrailBase core schema")?;

    Ok(())
}

async fn ensure_session_auth_schema(state: &AppState) -> Result<()> {
    let has_session_table = state
        .session_conn()
        .read_query_row_f(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = '_session')",
            (),
            |row| row.get::<_, bool>(0),
        )
        .await
        .context("check _session table")?
        .unwrap_or(false);

    if has_session_table {
        return Ok(());
    }

    state
        .session_conn()
        .execute_batch(CORE_SESSION_SCHEMA.to_owned())
        .await
        .context("apply fallback TrailBase session schema")?;

    Ok(())
}

const CORE_MAIN_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS _user (
  id                               BLOB PRIMARY KEY NOT NULL CHECK(is_uuid_v7(id)) DEFAULT (uuid_v7()),
  email                            TEXT NOT NULL CHECK(is_email(email)),
  password_hash                    TEXT DEFAULT '' NOT NULL,
  verified                         INTEGER DEFAULT FALSE NOT NULL,
  admin                            INTEGER DEFAULT FALSE NOT NULL,
  totp_secret                      TEXT,
  created                          INTEGER DEFAULT (UNIXEPOCH()) NOT NULL,
  updated                          INTEGER DEFAULT (UNIXEPOCH()) NOT NULL,
  provider_id                      INTEGER DEFAULT 0 NOT NULL,
  provider_user_id                 TEXT,
  provider_avatar_url              TEXT
) STRICT;

CREATE UNIQUE INDEX IF NOT EXISTS __user__email_index ON _user (email);
CREATE UNIQUE INDEX IF NOT EXISTS __user__provider_ids_index ON _user (provider_id, provider_user_id);

CREATE TRIGGER IF NOT EXISTS __user__updated_trigger AFTER UPDATE ON _user FOR EACH ROW
  BEGIN
    UPDATE _user SET updated = UNIXEPOCH() WHERE id = OLD.id;
  END;

CREATE TABLE IF NOT EXISTS _session (
  id                           INTEGER PRIMARY KEY NOT NULL,
  user                         BLOB NOT NULL REFERENCES _user(id) ON DELETE CASCADE,
  refresh_token                TEXT NOT NULL,
  updated                      INTEGER DEFAULT (UNIXEPOCH()) NOT NULL
) STRICT;

CREATE TRIGGER IF NOT EXISTS __session__updated_trigger AFTER UPDATE ON _session FOR EACH ROW
  BEGIN
    UPDATE _session SET updated = UNIXEPOCH() WHERE user = OLD.user;
  END;

CREATE UNIQUE INDEX IF NOT EXISTS __session__refresh_token_index ON _session (refresh_token);
CREATE INDEX IF NOT EXISTS __session__user_index ON _session (user);

CREATE TABLE IF NOT EXISTS _user_avatar (
  user                         BLOB PRIMARY KEY NOT NULL REFERENCES _user(id) ON DELETE CASCADE,
  file                         TEXT CHECK(jsonschema('std.FileUpload', file, 'image/png, image/jpeg')) NOT NULL,
  updated                      INTEGER DEFAULT (UNIXEPOCH()) NOT NULL
) STRICT;

CREATE TRIGGER IF NOT EXISTS __user_avatar__updated_trigger AFTER UPDATE ON _user_avatar FOR EACH ROW
  BEGIN
    UPDATE _user_avatar SET updated = UNIXEPOCH() WHERE user = OLD.user;
  END;
"#;

const CORE_MAIN_USER_UPGRADE_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS _new_with_otp (
  id                               BLOB PRIMARY KEY NOT NULL CHECK(is_uuid(id)) DEFAULT (uuid_v4()),
  email                            TEXT NOT NULL CHECK(is_email(email)),
  password_hash                    TEXT DEFAULT '' NOT NULL,
  verified                         INTEGER DEFAULT FALSE NOT NULL,
  admin                            INTEGER DEFAULT FALSE NOT NULL,
  totp_secret                      TEXT,
  created                          INTEGER DEFAULT (UNIXEPOCH()) NOT NULL,
  updated                          INTEGER DEFAULT (UNIXEPOCH()) NOT NULL,
  provider_id                      INTEGER DEFAULT 0 NOT NULL,
  provider_user_id                 TEXT,
  provider_avatar_url              TEXT
) STRICT;

INSERT INTO _new_with_otp(
    id,
    email,
    password_hash,
    verified,
    admin,
    created,
    updated,
    provider_id,
    provider_user_id,
    provider_avatar_url
  )
  SELECT
    id,
    email,
    password_hash,
    verified,
    admin,
    created,
    updated,
    provider_id,
    provider_user_id,
    provider_avatar_url
  FROM _user;

PRAGMA legacy_alter_table=ON;
DROP TABLE _user;
ALTER TABLE _new_with_otp RENAME TO _user;
PRAGMA legacy_alter_table=OFF;

CREATE UNIQUE INDEX IF NOT EXISTS __user__email_index ON _user (email);
CREATE UNIQUE INDEX IF NOT EXISTS __user__provider_ids_index ON _user (provider_id, provider_user_id);

CREATE TRIGGER IF NOT EXISTS __user__updated_trigger AFTER UPDATE ON _user FOR EACH ROW
  BEGIN
    UPDATE _user SET updated = UNIXEPOCH() WHERE id = OLD.id;
  END;
"#;

const CORE_BASE_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS _file_deletions (
  id                           INTEGER PRIMARY KEY NOT NULL,
  deleted                      INTEGER NOT NULL DEFAULT (UNIXEPOCH()),
  attempts                     INTEGER NOT NULL DEFAULT 0,
  errors                       TEXT,
  table_name                   TEXT NOT NULL,
  record_rowid                 INTEGER NOT NULL,
  column_name                  TEXT NOT NULL,
  json                         TEXT NOT NULL
) STRICT;
"#;

const CORE_SESSION_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS _session (
  id                           INTEGER PRIMARY KEY NOT NULL,
  user                         BLOB NOT NULL,
  refresh_token                TEXT NOT NULL,
  created                      INTEGER DEFAULT (UNIXEPOCH()) NOT NULL,
  expires                      INTEGER NOT NULL
) STRICT;

CREATE UNIQUE INDEX IF NOT EXISTS __session__refresh_token_index ON _session (refresh_token);
CREATE INDEX IF NOT EXISTS __session__user_index ON _session (user);

CREATE TABLE IF NOT EXISTS _authorization_code (
  id                           INTEGER PRIMARY KEY NOT NULL,
  user                         BLOB NOT NULL,
  authorization_code           TEXT NOT NULL,
  pkce_code_challenge          TEXT NOT NULL,
  created                      INTEGER DEFAULT (UNIXEPOCH()) NOT NULL,
  expires                      INTEGER NOT NULL
) STRICT;

CREATE UNIQUE INDEX IF NOT EXISTS __authorization_code__code ON _authorization_code (authorization_code);

CREATE TABLE IF NOT EXISTS _otp_code (
  id                           INTEGER PRIMARY KEY NOT NULL,
  user                         BLOB NOT NULL,
  email                        TEXT NOT NULL,
  otp_code                     TEXT NOT NULL,
  created                      INTEGER DEFAULT (UNIXEPOCH()) NOT NULL,
  expires                      INTEGER NOT NULL
) STRICT;

CREATE UNIQUE INDEX IF NOT EXISTS __otp_code__email ON _otp_code (email);
"#;

use anyhow::{Context, Result, anyhow};
use axum::{
    Json, Router,
    extract::{FromRef, State},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::{fs, path::{Path, PathBuf}};
use tempfile::TempDir;
use tower::util::ServiceExt;
use trailbase::{AppState, DataDir, Server, ServerOptions, User};
use trailbase_sqlite::params;

pub const INTROSPECT_PATH: &str = "/api/certd/introspect";
pub const INTERNAL_SECRET_HEADER: &str = "x-certd-internal-secret";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeConfig {
    pub data_dir: PathBuf,
    pub address: String,
    pub admin_address: Option<String>,
    pub internal_secret: String,
    pub dev: bool,
    pub log_responses: bool,
}

impl BridgeConfig {
    pub fn from_env() -> Result<Self> {
        let internal_secret = std::env::var("TRAILBASE_CERTD_INTERNAL_SECRET")
            .context("missing TRAILBASE_CERTD_INTERNAL_SECRET")?;
        if internal_secret.trim().is_empty() {
            anyhow::bail!("TRAILBASE_CERTD_INTERNAL_SECRET must not be empty");
        }

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
        })
    }
}

#[derive(Clone)]
struct BridgeState {
    app_state: AppState,
    internal_secret: String,
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
    let Server {
        state,
        main_router,
        admin_router,
        tls,
    } = Server::init(ServerOptions {
        data_dir: DataDir(config.data_dir),
        address: config.address,
        admin_address: config.admin_address,
        public_dir: None,
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
        build_router(state, main_router.1, config.internal_secret),
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

pub fn build_router(app_state: AppState, main_router: Router, internal_secret: String) -> Router {
    let state = BridgeState {
        app_state,
        internal_secret,
    };
    Router::new()
        .route("/healthz", get(health_handler))
        .route(
            INTROSPECT_PATH,
            post(introspect_handler).route_layer(middleware::from_fn_with_state(
                state.clone(),
                require_internal_secret,
            )),
        )
        .with_state(state)
        .merge(main_router)
}

async fn health_handler() -> impl IntoResponse {
    (StatusCode::OK, "ok")
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
    _tempdir: TempDir,
}

impl TestApp {
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

        let response = self
            .router
            .clone()
            .oneshot(request)
            .await
            .map_err(|err| err.to_string())?;
        let status = response.status();
        let bytes = http_body_util::BodyExt::collect(response.into_body())
            .await
            .map_err(|err| err.to_string())?
            .to_bytes();
        if !status.is_success() {
            return Err(format!("{status}: {}", String::from_utf8_lossy(&bytes)));
        }
        serde_json::from_slice(&bytes).map_err(|err| err.to_string())
    }
}

pub async fn build_test_app(internal_secret: &str) -> Result<TestApp> {
    let tempdir = tempfile::tempdir().context("create trailbase tempdir")?;
    let data_dir = tempdir.path().join("traildepot");
    ensure_auth_ui_component(&data_dir)?;
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
    .context("init trailbase test server")?;
    ensure_core_auth_schema(&server.state).await?;
    ensure_session_auth_schema(&server.state).await?;

    trailbase::api::cli::add_user(server.state.user_conn(), "alice@example.com", "password123")
        .await
        .context("create trailbase test user")?;

    let router = build_router(server.state.clone(), server.main_router.1, internal_secret.to_owned());

    Ok(TestApp {
        router,
        state: server.state,
        _tempdir: tempdir,
    })
}

fn ensure_auth_ui_component(data_dir: &Path) -> Result<()> {
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

async fn mint_auth_token_for_email(app_state: &AppState, email: &str, password: &str) -> Result<String> {
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

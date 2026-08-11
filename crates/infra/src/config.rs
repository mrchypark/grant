use anyhow::{Context, Result};
use std::env::{self, VarError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    pub bind_addr: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityConfig {
    pub master_key: String,
    pub app_auth_key: String,
    pub bootstrap_user_id: String,
    pub bootstrap_user_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RauthyAuthConfig {
    pub base_url: String,
    pub introspect_path: String,
    pub userinfo_path: String,
    pub client_id: String,
    pub client_secret: String,
    pub client_read_api_key: String,
    pub cache_ttl_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchDataAuthConfig {
    /// Internal Patch Data origin. The resolver always uses the fixed refresh route.
    pub base_url: String,
    /// The only Grant application accepted for Patch Data application authorization.
    pub exposure_application_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthProviderMode {
    Rauthy,
    PatchData,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthConfig {
    pub provider_mode: AuthProviderMode,
    pub rauthy: RauthyAuthConfig,
    pub patch_data: Option<PatchDataAuthConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeMode {
    Embedded,
    Remote,
    Split,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalNodeRole {
    Voter,
    Learner,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadConsistency {
    Local,
    Strong,
    PreferStrong,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HiqliteConfig {
    pub runtime_mode: RuntimeMode,
    pub local_node_role: LocalNodeRole,
    pub default_read_consistency: ReadConsistency,
    pub remote_nodes: Vec<String>,
    pub remote_tls: bool,
    pub remote_tls_no_verify: bool,
    pub remote_api_secret: Option<String>,
    pub remote_with_proxy: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub security: SecurityConfig,
    pub auth: AuthConfig,
    pub hiqlite: HiqliteConfig,
}

impl AppConfig {
    pub fn from_env() -> Result<Self> {
        let master_key = env_or("GRANT_MASTER_KEY", "dev-only-master-key");
        let app_auth_key = required_env("GRANT_APP_AUTH_KEY")?;
        if app_auth_key == master_key {
            anyhow::bail!("GRANT_APP_AUTH_KEY must differ from GRANT_MASTER_KEY");
        }
        let security = SecurityConfig {
            master_key,
            app_auth_key,
            bootstrap_user_id: env_or("GRANT_BOOTSTRAP_USER_ID", "conalog-grant"),
            bootstrap_user_name: env_or("GRANT_BOOTSTRAP_USER_NAME", "Conalog Grant"),
        };
        Ok(Self {
            server: ServerConfig {
                bind_addr: env_or("GRANT_BIND_ADDR", "0.0.0.0:8080"),
            },
            security: security.clone(),
            auth: auth_config()?,
            hiqlite: hiqlite_config()?,
        })
    }
}

fn hiqlite_config() -> Result<HiqliteConfig> {
    Ok(HiqliteConfig {
        runtime_mode: runtime_mode_from_env("GRANT_RUNTIME_MODE")?,
        local_node_role: local_node_role_from_env("GRANT_LOCAL_NODE_ROLE")?,
        default_read_consistency: read_consistency_from_env("GRANT_READ_CONSISTENCY")?,
        remote_nodes: env::var("GRANT_REMOTE_NODES")
            .ok()
            .map(|value| value.split(',').filter_map(normalize_remote_node).collect())
            .unwrap_or_default(),
        remote_tls: env_bool("GRANT_REMOTE_TLS", false),
        remote_tls_no_verify: env_bool("GRANT_REMOTE_TLS_NO_VERIFY", false),
        remote_api_secret: env::var("GRANT_REMOTE_API_SECRET").ok(),
        remote_with_proxy: env_bool("GRANT_REMOTE_WITH_PROXY", false),
    })
}

fn auth_config() -> Result<AuthConfig> {
    let provider_mode = match enum_env_value("GRANT_AUTH_PROVIDER", "rauthy")?.as_str() {
        "rauthy" => AuthProviderMode::Rauthy,
        "patch-data" => AuthProviderMode::PatchData,
        other => {
            anyhow::bail!("GRANT_AUTH_PROVIDER must be one of rauthy, patch-data; got {other}")
        }
    };
    let patch_data = matches!(provider_mode, AuthProviderMode::PatchData)
        .then(|| {
            let base_url = required_env("GRANT_PATCH_DATA_BASE_URL")?;
            validate_internal_http_base_url("GRANT_PATCH_DATA_BASE_URL", &base_url)?;
            Ok::<PatchDataAuthConfig, anyhow::Error>(PatchDataAuthConfig {
                base_url,
                exposure_application_id: required_env("GRANT_PATCH_DATA_EXPOSURE_APPLICATION_ID")?,
            })
        })
        .transpose()?;
    Ok(AuthConfig {
        provider_mode: provider_mode.clone(),
        rauthy: RauthyAuthConfig {
            base_url: env_or("GRANT_RAUTHY_BASE_URL", "http://127.0.0.1:8080"),
            introspect_path: env_or("GRANT_RAUTHY_INTROSPECT_PATH", "/auth/v1/oidc/introspect"),
            userinfo_path: env_or("GRANT_RAUTHY_USERINFO_PATH", "/auth/v1/oidc/userinfo"),
            client_id: required_for_rauthy(&provider_mode, "GRANT_RAUTHY_CLIENT_ID")?,
            client_secret: required_for_rauthy(&provider_mode, "GRANT_RAUTHY_CLIENT_SECRET")?,
            client_read_api_key: required_for_rauthy(
                &provider_mode,
                "GRANT_RAUTHY_CLIENT_READ_API_KEY",
            )?,
            cache_ttl_seconds: env::var("GRANT_RAUTHY_CACHE_TTL_SECONDS")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(30),
        },
        patch_data,
    })
}

fn required_for_rauthy(provider_mode: &AuthProviderMode, key: &str) -> Result<String> {
    if matches!(provider_mode, AuthProviderMode::Rauthy) {
        required_env(key)
    } else {
        Ok(env::var(key).unwrap_or_default())
    }
}

fn validate_internal_http_base_url(key: &str, value: &str) -> Result<()> {
    let url = reqwest::Url::parse(value)
        .with_context(|| format!("{key} must be an absolute http(s) URL"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        anyhow::bail!(
            "{key} must be an absolute http(s) origin without path, userinfo, query, or fragment"
        );
    }
    Ok(())
}

fn env_or(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_owned())
}

fn required_env(key: &str) -> Result<String> {
    let value =
        env::var(key).with_context(|| format!("missing required environment variable {key}"))?;
    if value.trim().is_empty() {
        anyhow::bail!("{key} must not be empty");
    }
    Ok(value)
}

fn env_bool(key: &str, default: bool) -> bool {
    env::var(key)
        .ok()
        .and_then(|value| match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        })
        .unwrap_or(default)
}

fn runtime_mode_from_env(key: &str) -> Result<RuntimeMode> {
    match enum_env_value(key, "embedded")?.as_str() {
        "embedded" => Ok(RuntimeMode::Embedded),
        "remote" => Ok(RuntimeMode::Remote),
        "split" => Ok(RuntimeMode::Split),
        other => anyhow::bail!("{key} must be one of embedded, remote, split; got {other}"),
    }
}

fn read_consistency_from_env(key: &str) -> Result<ReadConsistency> {
    match enum_env_value(key, "local")?.as_str() {
        "local" => Ok(ReadConsistency::Local),
        "strong" => Ok(ReadConsistency::Strong),
        "prefer-strong" | "prefer_strong" | "preferstrong" => Ok(ReadConsistency::PreferStrong),
        other => anyhow::bail!("{key} must be one of local, strong, prefer-strong; got {other}"),
    }
}

fn local_node_role_from_env(key: &str) -> Result<LocalNodeRole> {
    match enum_env_value(key, "voter")?.as_str() {
        "voter" => Ok(LocalNodeRole::Voter),
        "learner" | "learner-only" | "learner_only" => Ok(LocalNodeRole::Learner),
        other => anyhow::bail!("{key} must be one of voter, learner; got {other}"),
    }
}

fn enum_env_value(key: &str, default: &str) -> Result<String> {
    let value = match env::var(key) {
        Ok(value) => value,
        Err(VarError::NotPresent) => return Ok(default.to_owned()),
        Err(err) => return Err(err).with_context(|| format!("read {key}")),
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        anyhow::bail!("{key} must not be empty");
    }
    Ok(trimmed.to_ascii_lowercase())
}

fn normalize_remote_node(part: &str) -> Option<String> {
    let trimmed = part.trim();
    if trimmed.is_empty() {
        return None;
    }

    let without_scheme = trimmed
        .strip_prefix("http://")
        .or_else(|| trimmed.strip_prefix("https://"))
        .unwrap_or(trimmed);
    Some(without_scheme.trim_end_matches('/').to_owned())
}

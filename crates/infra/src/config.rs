use anyhow::{Context, Result};
use std::{env, path::PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    pub bind_addr: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityConfig {
    pub master_key: String,
    pub bootstrap_user_id: String,
    pub bootstrap_user_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrailbaseAuthConfig {
    pub base_url: String,
    pub introspect_path: String,
    pub internal_secret: String,
    pub cache_ttl_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthConfig {
    pub trailbase: TrailbaseAuthConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactBackendConfig {
    FileSystem {
        root_dir: PathBuf,
    },
    S3 {
        bucket: String,
        region: String,
        endpoint: String,
        path_style: bool,
        access_key: String,
        secret_key: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactConfig {
    pub backend: ArtifactBackendConfig,
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
    pub artifacts: ArtifactConfig,
    pub hiqlite: HiqliteConfig,
}

impl AppConfig {
    pub fn from_env() -> Result<Self> {
        let current_dir = env::current_dir().context("resolve current working directory")?;
        Ok(Self {
            server: ServerConfig {
                bind_addr: env_or("CERTD_BIND_ADDR", "0.0.0.0:8080"),
            },
            security: SecurityConfig {
                master_key: env_or("CERTD_MASTER_KEY", "dev-only-master-key"),
                bootstrap_user_id: env_or("CERTD_BOOTSTRAP_USER_ID", "conalog-account"),
                bootstrap_user_name: env_or("CERTD_BOOTSTRAP_USER_NAME", "Conalog Account"),
            },
            auth: auth_config()?,
            artifacts: ArtifactConfig {
                backend: artifact_backend(&current_dir)?,
            },
            hiqlite: hiqlite_config(),
        })
    }
}

fn hiqlite_config() -> HiqliteConfig {
    HiqliteConfig {
        runtime_mode: runtime_mode_from_env("CERTD_RUNTIME_MODE"),
        local_node_role: local_node_role_from_env("CERTD_LOCAL_NODE_ROLE"),
        default_read_consistency: read_consistency_from_env("CERTD_READ_CONSISTENCY"),
        remote_nodes: env::var("CERTD_REMOTE_NODES")
            .ok()
            .map(|value| {
                value
                    .split(',')
                    .filter_map(normalize_remote_node)
                    .collect()
            })
            .unwrap_or_default(),
        remote_tls: env_bool("CERTD_REMOTE_TLS", false),
        remote_tls_no_verify: env_bool("CERTD_REMOTE_TLS_NO_VERIFY", false),
        remote_api_secret: env::var("CERTD_REMOTE_API_SECRET").ok(),
        remote_with_proxy: env_bool("CERTD_REMOTE_WITH_PROXY", false),
    }
}

fn auth_config() -> Result<AuthConfig> {
    Ok(AuthConfig {
        trailbase: TrailbaseAuthConfig {
            base_url: env_or("CERTD_TRAILBASE_BASE_URL", "http://127.0.0.1:4000"),
            introspect_path: env_or("CERTD_TRAILBASE_INTROSPECT_PATH", "/api/certd/introspect"),
            internal_secret: required_env("CERTD_TRAILBASE_INTERNAL_SECRET")?,
            cache_ttl_seconds: env::var("CERTD_TRAILBASE_CACHE_TTL_SECONDS")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(30),
        },
    })
}

fn artifact_backend(current_dir: &PathBuf) -> Result<ArtifactBackendConfig> {
    let backend = env_or("ARTIFACTS_BACKEND", "fs");
    match backend.trim().to_ascii_lowercase().as_str() {
        "fs" | "filesystem" => {
            let root = PathBuf::from(env_or("ARTIFACTS_ROOT_DIR", "./artifacts"));
            let resolved = if root.is_absolute() {
                root
            } else {
                current_dir.join(root)
            };
            Ok(ArtifactBackendConfig::FileSystem { root_dir: resolved })
        }
        "s3" => Ok(ArtifactBackendConfig::S3 {
            bucket: env_or("ARTIFACTS_BUCKET", "cert-artifacts"),
            region: env_or("ARTIFACTS_REGION", "us-east-1"),
            endpoint: env_or("ARTIFACTS_ENDPOINT", "http://127.0.0.1:9000"),
            path_style: env_bool("ARTIFACTS_PATH_STYLE", true),
            access_key: env_or_fallback("ARTIFACTS_ACCESS_KEY", "HQL_S3_KEY", "minioadmin"),
            secret_key: env_or_fallback("ARTIFACTS_SECRET_KEY", "HQL_S3_SECRET", "minioadmin"),
        }),
        other => Err(anyhow::anyhow!("unsupported ARTIFACTS_BACKEND: {other}")),
    }
}

fn env_or(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_owned())
}

fn required_env(key: &str) -> Result<String> {
    let value = env::var(key).with_context(|| format!("missing required environment variable {key}"))?;
    if value.trim().is_empty() {
        anyhow::bail!("{key} must not be empty");
    }
    Ok(value)
}

fn env_or_fallback(primary: &str, fallback: &str, default: &str) -> String {
    env::var(primary)
        .or_else(|_| env::var(fallback))
        .unwrap_or_else(|_| default.to_owned())
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

fn runtime_mode_from_env(key: &str) -> RuntimeMode {
    match env::var(key)
        .unwrap_or_else(|_| "embedded".to_owned())
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "remote" => RuntimeMode::Remote,
        "split" => RuntimeMode::Split,
        _ => RuntimeMode::Embedded,
    }
}

fn read_consistency_from_env(key: &str) -> ReadConsistency {
    match env::var(key)
        .unwrap_or_else(|_| "local".to_owned())
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "strong" => ReadConsistency::Strong,
        "prefer-strong" | "prefer_strong" | "preferstrong" => ReadConsistency::PreferStrong,
        _ => ReadConsistency::Local,
    }
}

fn local_node_role_from_env(key: &str) -> LocalNodeRole {
    match env::var(key)
        .unwrap_or_else(|_| "voter".to_owned())
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "learner" | "learner-only" | "learner_only" => LocalNodeRole::Learner,
        _ => LocalNodeRole::Voter,
    }
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

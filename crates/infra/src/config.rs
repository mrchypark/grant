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
pub enum ArtifactBackendConfig {
    FileSystem { root_dir: PathBuf },
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
pub struct AppConfig {
    pub server: ServerConfig,
    pub security: SecurityConfig,
    pub artifacts: ArtifactConfig,
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
            artifacts: ArtifactConfig {
                backend: artifact_backend(&current_dir)?,
            },
        })
    }
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

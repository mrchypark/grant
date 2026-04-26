use crate::config::{ArtifactBackendConfig, ArtifactConfig};
use anyhow::{Context, Result};
use aws_config::BehaviorVersion;
use aws_credential_types::Credentials;
use aws_sdk_s3::{
    Client,
    config::{Builder as S3ConfigBuilder, Region},
    primitives::ByteStream,
};
use bytes::Bytes;
use std::path::{Path, PathBuf};
use tokio::fs;

#[derive(Clone)]
pub enum ArtifactStore {
    FileSystem { root_dir: PathBuf },
    S3 { client: Client, bucket: String },
}

#[derive(Debug, Clone)]
pub struct ArtifactObject {
    pub object_key: String,
    pub content_type: String,
}

impl ArtifactStore {
    pub async fn new(cfg: &ArtifactConfig) -> Result<Self> {
        match &cfg.backend {
            ArtifactBackendConfig::FileSystem { root_dir } => {
                fs::create_dir_all(root_dir)
                    .await
                    .with_context(|| format!("create artifact root {}", root_dir.display()))?;
                Ok(Self::FileSystem {
                    root_dir: root_dir.clone(),
                })
            }
            ArtifactBackendConfig::S3 {
                bucket,
                region,
                endpoint,
                path_style,
                access_key,
                secret_key,
            } => {
                let credentials =
                    Credentials::new(access_key.clone(), secret_key.clone(), None, None, "static");

                let shared = aws_config::defaults(BehaviorVersion::latest())
                    .region(Region::new(region.clone()))
                    .credentials_provider(credentials)
                    .load()
                    .await;

                let conf = S3ConfigBuilder::from(&shared)
                    .endpoint_url(endpoint.clone())
                    .force_path_style(*path_style)
                    .build();

                let store = Self::S3 {
                    client: Client::from_conf(conf),
                    bucket: bucket.clone(),
                };
                store.ensure_bucket().await?;
                Ok(store)
            }
        }
    }

    pub async fn ensure_bucket(&self) -> Result<()> {
        match self {
            Self::FileSystem { .. } => Ok(()),
            Self::S3 { client, bucket } => {
                let exists = client.head_bucket().bucket(bucket).send().await.is_ok();
                if exists {
                    return Ok(());
                }

                match client.create_bucket().bucket(bucket).send().await {
                    Ok(_) => Ok(()),
                    Err(err) => {
                        let text = err.to_string();
                        if text.contains("BucketAlreadyOwnedByYou")
                            || text.contains("BucketAlreadyExists")
                        {
                            Ok(())
                        } else {
                            Err(err).context("create artifact bucket")
                        }
                    }
                }
            }
        }
    }

    pub async fn put_file<P: AsRef<Path>>(
        &self,
        object_key: String,
        path: P,
        content_type: &str,
    ) -> Result<ArtifactObject> {
        validate_object_key(&object_key)?;
        match self {
            Self::FileSystem { root_dir } => {
                let dest = root_dir.join(&object_key);
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent)
                        .await
                        .with_context(|| format!("create artifact parent {}", parent.display()))?;
                }
                fs::copy(path.as_ref(), &dest)
                    .await
                    .with_context(|| format!("copy artifact to {}", dest.display()))?;
                Ok(ArtifactObject {
                    object_key,
                    content_type: content_type.to_owned(),
                })
            }
            Self::S3 { client, bucket } => {
                let metadata = fs::metadata(path.as_ref())
                    .await
                    .with_context(|| format!("stat artifact file {}", path.as_ref().display()))?;
                let body = ByteStream::from_path(path.as_ref())
                    .await
                    .with_context(|| {
                        format!("open artifact file stream {}", path.as_ref().display())
                    })?;
                client
                    .put_object()
                    .bucket(bucket)
                    .key(&object_key)
                    .content_type(content_type)
                    .content_length(metadata.len() as i64)
                    .body(body)
                    .send()
                    .await
                    .with_context(|| format!("put object {object_key}"))?;
                Ok(ArtifactObject {
                    object_key,
                    content_type: content_type.to_owned(),
                })
            }
        }
    }

    pub async fn put_bytes(
        &self,
        object_key: String,
        bytes: Bytes,
        content_type: &str,
    ) -> Result<ArtifactObject> {
        validate_object_key(&object_key)?;
        match self {
            Self::FileSystem { root_dir } => {
                let dest = root_dir.join(&object_key);
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent)
                        .await
                        .with_context(|| format!("create artifact parent {}", parent.display()))?;
                }
                fs::write(&dest, &bytes)
                    .await
                    .with_context(|| format!("write artifact to {}", dest.display()))?;
                Ok(ArtifactObject {
                    object_key,
                    content_type: content_type.to_owned(),
                })
            }
            Self::S3 { client, bucket } => {
                let body = ByteStream::from(bytes.clone());
                client
                    .put_object()
                    .bucket(bucket)
                    .key(&object_key)
                    .content_type(content_type)
                    .content_length(bytes.len() as i64)
                    .body(body)
                    .send()
                    .await
                    .with_context(|| format!("put object {object_key}"))?;
                Ok(ArtifactObject {
                    object_key,
                    content_type: content_type.to_owned(),
                })
            }
        }
    }

    pub async fn get_bytes(&self, object_key: &str) -> Result<Bytes> {
        validate_object_key(object_key)?;
        match self {
            Self::FileSystem { root_dir } => fs::read(root_dir.join(object_key))
                .await
                .map(Bytes::from)
                .with_context(|| format!("read object {}", root_dir.join(object_key).display())),
            Self::S3 { client, bucket } => {
                let resp = client
                    .get_object()
                    .bucket(bucket)
                    .key(object_key)
                    .send()
                    .await
                    .with_context(|| format!("get object {object_key}"))?;
                let data = resp
                    .body
                    .collect()
                    .await
                    .with_context(|| format!("collect object {object_key}"))?;
                Ok(data.into_bytes())
            }
        }
    }

    pub async fn delete_object(&self, object_key: &str) -> Result<()> {
        validate_object_key(object_key)?;
        match self {
            Self::FileSystem { root_dir } => {
                let path = root_dir.join(object_key);
                match fs::remove_file(&path).await {
                    Ok(()) => Ok(()),
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(err) => {
                        Err(err).with_context(|| format!("delete object {}", path.display()))
                    }
                }
            }
            Self::S3 { client, bucket } => {
                client
                    .delete_object()
                    .bucket(bucket)
                    .key(object_key)
                    .send()
                    .await
                    .with_context(|| format!("delete object {object_key}"))?;
                Ok(())
            }
        }
    }

    pub async fn list_keys(&self, prefix: &str) -> Result<Vec<String>> {
        validate_object_key_prefix(prefix)?;
        match self {
            Self::FileSystem { root_dir } => list_fs_keys(root_dir, prefix).await,
            Self::S3 { client, bucket } => {
                let resp = client
                    .list_objects_v2()
                    .bucket(bucket)
                    .prefix(prefix)
                    .send()
                    .await
                    .with_context(|| format!("list objects with prefix {prefix}"))?;
                Ok(resp
                    .contents()
                    .iter()
                    .filter_map(|obj| obj.key().map(ToOwned::to_owned))
                    .collect())
            }
        }
    }
}

fn validate_object_key(object_key: &str) -> Result<()> {
    validate_object_key_inner(object_key, false)
}

fn validate_object_key_prefix(prefix: &str) -> Result<()> {
    if prefix.is_empty() {
        return Ok(());
    }
    validate_object_key_inner(prefix, true)
}

fn validate_object_key_inner(value: &str, allow_trailing_slash: bool) -> Result<()> {
    if value.is_empty() || value.starts_with('/') || value.starts_with('\\') {
        anyhow::bail!("artifact object key must be a relative path");
    }
    if value.contains('\\') || value.chars().any(char::is_control) {
        anyhow::bail!("artifact object key contains unsupported characters");
    }
    if !allow_trailing_slash && value.ends_with('/') {
        anyhow::bail!("artifact object key must name an object");
    }
    for (index, segment) in value.split('/').enumerate() {
        if segment.is_empty() {
            if allow_trailing_slash && value.ends_with('/') && index == value.matches('/').count() {
                continue;
            }
            anyhow::bail!("artifact object key contains an empty path segment");
        }
        if segment == "." || segment == ".." {
            anyhow::bail!("artifact object key contains a traversal segment");
        }
    }
    Ok(())
}

async fn list_fs_keys(root_dir: &Path, prefix: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let start = root_dir.join(prefix);
    if fs::metadata(&start).await.is_err() {
        return Ok(out);
    }

    let mut stack = vec![start];
    while let Some(dir) = stack.pop() {
        let mut entries = fs::read_dir(&dir)
            .await
            .with_context(|| format!("read artifact dir {}", dir.display()))?;
        while let Some(entry) = entries
            .next_entry()
            .await
            .with_context(|| format!("read artifact entry {}", dir.display()))?
        {
            let path = entry.path();
            let file_type = entry
                .file_type()
                .await
                .with_context(|| format!("read artifact type {}", path.display()))?;
            if file_type.is_dir() {
                stack.push(path);
            } else if file_type.is_file() {
                let rel = path
                    .strip_prefix(root_dir)
                    .with_context(|| format!("strip prefix {}", path.display()))?;
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    out.sort();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_object_key_rejects_traversal_segments() {
        assert!(validate_object_key("../escape.pdf").is_err());
        assert!(validate_object_key("org/../escape.pdf").is_err());
        assert!(validate_object_key("/absolute.pdf").is_err());
        assert!(validate_object_key(r"org\escape.pdf").is_err());
    }

    #[test]
    fn validate_object_key_accepts_canonical_storage_keys() {
        validate_object_key("bootstrap-user/FAC-DOC/document-1.pdf").expect("valid object key");
        validate_object_key_prefix("bootstrap-user/FAC-DOC/").expect("valid object key prefix");
    }
}

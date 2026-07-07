use crate::{
    config::AuthConfig,
    config::ReadConsistency,
    database::{CachedExternalIdentity, Database},
};
use anyhow::{Context, Result, anyhow};
use chrono::{Duration as ChronoDuration, Utc};
use reqwest::Client;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedPrincipal {
    pub user_id: String,
    pub organization_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalIdentity {
    pub provider: String,
    pub external_user_id: String,
    pub email: Option<String>,
    pub display_name: String,
    pub active: bool,
}

#[derive(Clone)]
pub struct AuthResolver {
    trailbase: TrailbaseResolver,
}

#[derive(Clone)]
struct TrailbaseResolver {
    base_url: String,
    introspect_path: String,
    internal_secret: String,
    cache_ttl: Duration,
    client: Client,
    cache: Arc<Mutex<HashMap<String, CachedTrailbaseIdentity>>>,
}

#[derive(Debug, Clone)]
struct CachedTrailbaseIdentity {
    identity: TrailbaseIdentity,
    expires_at: Instant,
}

#[derive(Debug, Clone)]
struct TrailbaseIdentity {
    external_user_id: String,
    email: Option<String>,
    display_name: String,
    active: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TrailbaseIntrospectionResponse {
    valid: bool,
    trailbase_user_id: String,
    email: Option<String>,
    display_name: Option<String>,
    verified: bool,
    active: bool,
    expires_at: Option<String>,
}

impl AuthResolver {
    pub fn new(cfg: &AuthConfig) -> Self {
        Self {
            trailbase: TrailbaseResolver {
                base_url: cfg.trailbase.base_url.trim_end_matches('/').to_owned(),
                introspect_path: cfg.trailbase.introspect_path.clone(),
                internal_secret: cfg.trailbase.internal_secret.clone(),
                cache_ttl: Duration::from_secs(cfg.trailbase.cache_ttl_seconds),
                client: Client::new(),
                cache: Arc::new(Mutex::new(HashMap::new())),
            },
        }
    }

    pub async fn resolve(
        &self,
        db: &Database,
        bearer_token: Option<&str>,
        organization_id: Option<&str>,
        read_consistency: Option<ReadConsistency>,
    ) -> Result<AuthenticatedPrincipal> {
        let organization_id = organization_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("unauthorized: missing x-organization-id header"))?;

        let token = bearer_token
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("unauthorized: missing bearer token"))?;
        let effective_read_consistency =
            db.default_read_consistency_with_override(read_consistency.clone());
        let effective_read_consistency_override = Some(effective_read_consistency.clone());
        let should_sync = should_sync_identity(Some(&effective_read_consistency));
        let identity = self
            .trailbase
            .introspect(
                db,
                token,
                effective_read_consistency_override.clone(),
                should_sync,
            )
            .await?;
        let user = if should_sync {
            db.resolve_or_create_user_identity_with_consistency(
                "trailbase",
                &identity.external_user_id,
                identity.email.as_deref(),
                &identity.display_name,
                identity.active,
                effective_read_consistency_override.clone(),
            )
            .await
            .map_err(map_strong_identity_sync_error)?
        } else {
            db.resolve_existing_user_identity_with_consistency(
                "trailbase",
                &identity.external_user_id,
                effective_read_consistency_override.clone(),
            )
            .await?
            .ok_or_else(|| {
                anyhow!(
                    "forbidden: no local user identity available for trailbase user {}",
                    identity.external_user_id
                )
            })?
        };
        let is_member = db
            .has_active_organization_membership_with_consistency(
                &user.id,
                organization_id,
                effective_read_consistency_override,
            )
            .await?;
        if !is_member {
            anyhow::bail!(
                "forbidden: user {} is not an active member of organization {}",
                user.id,
                organization_id
            );
        }
        Ok(AuthenticatedPrincipal {
            user_id: user.id,
            organization_id: organization_id.to_owned(),
        })
    }

    pub async fn resolve_external_identity(
        &self,
        db: Option<&Database>,
        bearer_token: Option<&str>,
        read_consistency: Option<ReadConsistency>,
    ) -> Result<ExternalIdentity> {
        let token = bearer_token
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("unauthorized: missing bearer token"))?;
        let identity = if let Some(db) = db {
            let effective_read_consistency =
                db.default_read_consistency_with_override(read_consistency.clone());
            self.trailbase
                .introspect(
                    db,
                    token,
                    Some(effective_read_consistency.clone()),
                    should_sync_identity(Some(&effective_read_consistency)),
                )
                .await?
        } else {
            self.trailbase.introspect_without_db(token).await?
        };
        Ok(ExternalIdentity {
            provider: "trailbase".to_owned(),
            external_user_id: identity.external_user_id,
            email: identity.email,
            display_name: identity.display_name,
            active: identity.active,
        })
    }
}

impl TrailbaseResolver {
    async fn introspect(
        &self,
        db: &Database,
        token: &str,
        read_consistency: Option<ReadConsistency>,
        persist_shared_cache: bool,
    ) -> Result<TrailbaseIdentity> {
        let cache_key = token_cache_key(token);
        if let Some(identity) = self.read_cache(&cache_key).await {
            return Ok(identity);
        }

        let response = self.send_introspection(token).await;
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                if let Some(identity) = self
                    .read_shared_cache(db, &cache_key, read_consistency.clone())
                    .await?
                {
                    return Ok(identity);
                }
                return Err(error).context("call TrailBase introspect");
            }
        };
        reject_unauthorized_introspection(&response)?;
        let response = match response.error_for_status() {
            Ok(response) => response,
            Err(error) => {
                if let Some(identity) = self
                    .read_shared_cache(db, &cache_key, read_consistency.clone())
                    .await?
                {
                    return Ok(identity);
                }
                return Err(error).context("TrailBase introspect returned error status");
            }
        };
        let identity = parse_introspection_response(response).await?;
        self.write_cache(cache_key, identity.clone()).await;
        if persist_shared_cache {
            self.write_shared_cache(db, token, &identity).await?;
        }
        Ok(identity)
    }

    async fn introspect_without_db(&self, token: &str) -> Result<TrailbaseIdentity> {
        let cache_key = token_cache_key(token);
        if let Some(identity) = self.read_cache(&cache_key).await {
            return Ok(identity);
        }

        let response = self
            .send_introspection(token)
            .await
            .context("call TrailBase introspect")?;
        reject_unauthorized_introspection(&response)?;
        let response = response
            .error_for_status()
            .context("TrailBase introspect returned error status")?;
        let identity = parse_introspection_response(response).await?;
        self.write_cache(cache_key, identity.clone()).await;
        Ok(identity)
    }

    async fn send_introspection(&self, token: &str) -> reqwest::Result<reqwest::Response> {
        let url = format!("{}{}", self.base_url, self.introspect_path);
        self.client
            .post(url)
            .bearer_auth(token)
            .header("x-grant-internal-secret", &self.internal_secret)
            .send()
            .await
    }

    async fn read_cache(&self, key: &str) -> Option<TrailbaseIdentity> {
        let cache = self.cache.lock().await;
        let cached = cache.get(key)?;
        if cached.expires_at <= Instant::now() {
            return None;
        }
        Some(cached.identity.clone())
    }

    async fn write_cache(&self, key: String, identity: TrailbaseIdentity) {
        self.write_cache_until(key, identity, Instant::now() + self.cache_ttl)
            .await;
    }

    async fn write_cache_until(
        &self,
        key: String,
        identity: TrailbaseIdentity,
        expires_at: Instant,
    ) {
        let mut cache = self.cache.lock().await;
        cache.insert(
            key,
            CachedTrailbaseIdentity {
                identity,
                expires_at,
            },
        );
    }

    async fn read_shared_cache(
        &self,
        db: &Database,
        token_hash: &str,
        read_consistency: Option<ReadConsistency>,
    ) -> Result<Option<TrailbaseIdentity>> {
        let now = Utc::now();
        let cached = db
            .get_cached_external_identity(token_hash, now, read_consistency)
            .await?;
        let Some(cached) = cached else {
            return Ok(None);
        };
        let identity = TrailbaseIdentity {
            external_user_id: cached.external_user_id,
            email: cached.email,
            display_name: cached.display_name,
            active: cached.active,
        };
        let ttl = cached
            .expires_at
            .signed_duration_since(now)
            .to_std()
            .unwrap_or_else(|_| Duration::from_secs(0));
        self.write_cache_until(
            token_hash.to_owned(),
            identity.clone(),
            Instant::now() + ttl,
        )
        .await;
        Ok(Some(identity))
    }

    async fn write_shared_cache(
        &self,
        db: &Database,
        token: &str,
        identity: &TrailbaseIdentity,
    ) -> Result<()> {
        let token_hash = token_cache_key(token);
        let expires_at = Utc::now() + ChronoDuration::seconds(self.cache_ttl.as_secs() as i64);
        db.put_cached_external_identity(
            &token_hash,
            &CachedExternalIdentity {
                provider: "trailbase".to_owned(),
                external_user_id: identity.external_user_id.clone(),
                email: identity.email.clone(),
                display_name: identity.display_name.clone(),
                active: identity.active,
                expires_at,
            },
            expires_at,
        )
        .await
    }
}

fn should_sync_identity(read_consistency: Option<&ReadConsistency>) -> bool {
    matches!(
        read_consistency,
        Some(ReadConsistency::Strong | ReadConsistency::PreferStrong)
    )
}

fn reject_unauthorized_introspection(response: &reqwest::Response) -> Result<()> {
    if response.status() == reqwest::StatusCode::UNAUTHORIZED
        || response.status() == reqwest::StatusCode::FORBIDDEN
    {
        anyhow::bail!("unauthorized: trailbase token rejected");
    }
    Ok(())
}

async fn parse_introspection_response(response: reqwest::Response) -> Result<TrailbaseIdentity> {
    let payload: TrailbaseIntrospectionResponse = response
        .json()
        .await
        .context("decode TrailBase introspection response")?;
    if !payload.valid {
        anyhow::bail!("unauthorized: trailbase token is not valid");
    }
    if !payload.active {
        anyhow::bail!("unauthorized: trailbase user is inactive");
    }
    let _ = payload.verified;
    let _ = payload.expires_at.as_deref();
    Ok(TrailbaseIdentity {
        external_user_id: payload.trailbase_user_id,
        email: payload.email,
        display_name: payload
            .display_name
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "TrailBase User".to_owned()),
        active: payload.active,
    })
}

pub(crate) fn map_strong_identity_sync_error(err: anyhow::Error) -> anyhow::Error {
    let message = err.to_string();
    if message.contains("unauthorized:") || message.contains("forbidden:") {
        err
    } else {
        anyhow!(
            "unavailable: strong consistency requested but quorum-backed identity sync is unavailable"
        )
    }
}

fn token_cache_key(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    hex::encode(digest)
}

#[cfg(test)]
mod tests {
    use super::{map_strong_identity_sync_error, should_sync_identity};
    use crate::config::ReadConsistency;
    use anyhow::anyhow;

    #[test]
    fn wraps_internal_identity_sync_failures_with_quorum_message() {
        let err = map_strong_identity_sync_error(anyhow!("query user identity"));
        assert_eq!(
            err.to_string(),
            "unavailable: strong consistency requested but quorum-backed identity sync is unavailable"
        );
    }

    #[test]
    fn preserves_auth_failures_when_wrapping_identity_sync_errors() {
        let err = map_strong_identity_sync_error(anyhow!("forbidden: no local user identity"));
        assert_eq!(err.to_string(), "forbidden: no local user identity");
    }

    #[test]
    fn sync_gate_treats_prefer_strong_as_sync_capable() {
        assert!(should_sync_identity(Some(&ReadConsistency::Strong)));
        assert!(should_sync_identity(Some(&ReadConsistency::PreferStrong)));
        assert!(!should_sync_identity(Some(&ReadConsistency::Local)));
        assert!(!should_sync_identity(None));
    }
}

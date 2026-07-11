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
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const AUTH_PROVIDER: &str = "rauthy";

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
    rauthy: RauthyResolver,
}

#[derive(Clone)]
struct RauthyResolver {
    base_url: String,
    introspect_path: String,
    userinfo_path: String,
    client_id: String,
    client_secret: String,
    cache_ttl: Duration,
    client: Client,
    cache: Arc<Mutex<HashMap<String, CachedRauthyIdentity>>>,
}

#[derive(Debug, Clone)]
struct CachedRauthyIdentity {
    identity: RauthyIdentity,
    expires_at: Instant,
}

#[derive(Debug, Clone)]
struct RauthyIdentity {
    external_user_id: String,
    email: Option<String>,
    display_name: String,
    active: bool,
}

#[derive(Debug, Deserialize)]
struct RauthyIntrospectionResponse {
    sub: String,
    exp: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct RauthyUserinfoResponse {
    sub: String,
    name: Option<String>,
    email: Option<String>,
    preferred_username: Option<String>,
    given_name: Option<String>,
    family_name: Option<String>,
    email_verified: Option<bool>,
}

impl AuthResolver {
    pub fn new(cfg: &AuthConfig) -> Self {
        Self {
            rauthy: RauthyResolver {
                base_url: cfg.rauthy.base_url.trim_end_matches('/').to_owned(),
                introspect_path: cfg.rauthy.introspect_path.clone(),
                userinfo_path: cfg.rauthy.userinfo_path.clone(),
                client_id: cfg.rauthy.client_id.clone(),
                client_secret: cfg.rauthy.client_secret.clone(),
                cache_ttl: Duration::from_secs(cfg.rauthy.cache_ttl_seconds),
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
            .rauthy
            .introspect(
                db,
                token,
                effective_read_consistency_override.clone(),
                should_sync,
            )
            .await?;
        let user = if should_sync {
            db.resolve_or_create_user_identity_with_consistency(
                AUTH_PROVIDER,
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
                AUTH_PROVIDER,
                &identity.external_user_id,
                effective_read_consistency_override.clone(),
            )
            .await?
            .ok_or_else(|| {
                anyhow!(
                    "forbidden: no local user identity available for rauthy user {}",
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
            self.rauthy
                .introspect(
                    db,
                    token,
                    Some(effective_read_consistency.clone()),
                    should_sync_identity(Some(&effective_read_consistency)),
                )
                .await?
        } else {
            self.rauthy.introspect_without_db(token).await?
        };
        Ok(ExternalIdentity {
            provider: AUTH_PROVIDER.to_owned(),
            external_user_id: identity.external_user_id,
            email: identity.email,
            display_name: identity.display_name,
            active: identity.active,
        })
    }
}

impl RauthyResolver {
    async fn introspect(
        &self,
        db: &Database,
        token: &str,
        read_consistency: Option<ReadConsistency>,
        persist_shared_cache: bool,
    ) -> Result<RauthyIdentity> {
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
                return Err(error).context("call Rauthy introspect");
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
                return Err(error).context("Rauthy introspect returned error status");
            }
        };
        let (identity, ttl) = self.identity_from_introspection(token, response).await?;
        self.write_cache_until(cache_key, identity.clone(), Instant::now() + ttl)
            .await;
        if persist_shared_cache {
            self.write_shared_cache(db, token, &identity, ttl).await?;
        }
        Ok(identity)
    }

    async fn introspect_without_db(&self, token: &str) -> Result<RauthyIdentity> {
        let cache_key = token_cache_key(token);
        if let Some(identity) = self.read_cache(&cache_key).await {
            return Ok(identity);
        }

        let response = self
            .send_introspection(token)
            .await
            .context("call Rauthy introspect")?;
        reject_unauthorized_introspection(&response)?;
        let response = response
            .error_for_status()
            .context("Rauthy introspect returned error status")?;
        let (identity, ttl) = self.identity_from_introspection(token, response).await?;
        self.write_cache_until(cache_key, identity.clone(), Instant::now() + ttl)
            .await;
        Ok(identity)
    }

    async fn send_introspection(&self, token: &str) -> reqwest::Result<reqwest::Response> {
        let url = format!("{}{}", self.base_url, self.introspect_path);
        self.client
            .post(url)
            .basic_auth(&self.client_id, Some(&self.client_secret))
            .form(&[("token", token)])
            .send()
            .await
    }

    async fn identity_from_introspection(
        &self,
        token: &str,
        response: reqwest::Response,
    ) -> Result<(RauthyIdentity, Duration)> {
        let token_info = parse_introspection_response(response).await?;
        let ttl = cache_ttl_for_exp(self.cache_ttl, token_info.exp, Utc::now().timestamp())?;
        let identity = self.fetch_userinfo(token, &token_info.sub).await?;
        Ok((identity, ttl))
    }

    async fn fetch_userinfo(&self, token: &str, expected_sub: &str) -> Result<RauthyIdentity> {
        let url = format!("{}{}", self.base_url, self.userinfo_path);
        let response = self
            .client
            .get(url)
            .bearer_auth(token)
            .send()
            .await
            .context("call Rauthy userinfo")?;
        reject_unauthorized_introspection(&response)?;
        let response = response
            .error_for_status()
            .context("Rauthy userinfo returned error status")?;
        let payload: RauthyUserinfoResponse = response
            .json()
            .await
            .context("decode Rauthy userinfo response")?;
        if payload.sub != expected_sub {
            anyhow::bail!("unauthorized: rauthy userinfo subject mismatch");
        }
        let email = verified_email(payload.email, payload.email_verified);
        let composed_name = display_name_from_parts(
            payload.given_name.as_deref(),
            payload.family_name.as_deref(),
        );
        let display_name = first_non_empty([
            payload.name.as_deref(),
            payload.preferred_username.as_deref(),
            composed_name.as_deref(),
            email.as_deref(),
            Some(expected_sub),
        ])
        .unwrap_or_else(|| "Rauthy User".to_owned());
        Ok(RauthyIdentity {
            external_user_id: expected_sub.to_owned(),
            email,
            display_name,
            active: true,
        })
    }

    async fn read_cache(&self, key: &str) -> Option<RauthyIdentity> {
        let cache = self.cache.lock().expect("Rauthy auth cache poisoned");
        let cached = cache.get(key)?;
        if cached.expires_at <= Instant::now() {
            return None;
        }
        Some(cached.identity.clone())
    }

    async fn write_cache_until(&self, key: String, identity: RauthyIdentity, expires_at: Instant) {
        let mut cache = self.cache.lock().expect("Rauthy auth cache poisoned");
        cache.insert(
            key,
            CachedRauthyIdentity {
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
    ) -> Result<Option<RauthyIdentity>> {
        let now = Utc::now();
        let cached = db
            .get_cached_external_identity(token_hash, now, read_consistency)
            .await?;
        let Some(cached) = cached else {
            return Ok(None);
        };
        if cached.provider != AUTH_PROVIDER {
            return Ok(None);
        }
        let identity = RauthyIdentity {
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
        identity: &RauthyIdentity,
        ttl: Duration,
    ) -> Result<()> {
        let token_hash = token_cache_key(token);
        let expires_at = Utc::now() + ChronoDuration::seconds(ttl.as_secs() as i64);
        db.put_cached_external_identity(
            &token_hash,
            &CachedExternalIdentity {
                provider: AUTH_PROVIDER.to_owned(),
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
        anyhow::bail!("unauthorized: rauthy token rejected");
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
struct RauthyIntrospectionPayload {
    active: bool,
    sub: Option<String>,
    exp: Option<i64>,
}

async fn parse_introspection_response(
    response: reqwest::Response,
) -> Result<RauthyIntrospectionResponse> {
    let payload: RauthyIntrospectionPayload = response
        .json()
        .await
        .context("decode Rauthy introspection response")?;
    if !payload.active {
        anyhow::bail!("unauthorized: rauthy token is not valid");
    }
    let sub = payload
        .sub
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("unauthorized: rauthy token has no subject"))?
        .to_owned();
    Ok(RauthyIntrospectionResponse {
        sub,
        exp: payload.exp,
    })
}

fn first_non_empty<'a>(values: impl IntoIterator<Item = Option<&'a str>>) -> Option<String> {
    values
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn display_name_from_parts(given_name: Option<&str>, family_name: Option<&str>) -> Option<String> {
    let joined = [given_name, family_name]
        .into_iter()
        .flatten()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if joined.is_empty() {
        None
    } else {
        Some(joined)
    }
}

fn verified_email(email: Option<String>, verified: Option<bool>) -> Option<String> {
    verified
        .is_some_and(|verified| verified)
        .then_some(email)
        .flatten()
        .filter(|value| !value.trim().is_empty())
}

fn cache_ttl_for_exp(configured: Duration, exp: Option<i64>, now: i64) -> Result<Duration> {
    let Some(exp) = exp else {
        return Ok(configured);
    };
    let remaining = exp
        .checked_sub(now)
        .ok_or_else(|| anyhow!("unauthorized: invalid token expiration"))?;
    if remaining <= 0 {
        return Ok(Duration::ZERO);
    }
    Ok(configured.min(Duration::from_secs(remaining as u64)))
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
    use super::{
        cache_ttl_for_exp, map_strong_identity_sync_error, should_sync_identity, verified_email,
    };
    use crate::config::ReadConsistency;
    use anyhow::anyhow;
    use std::time::Duration;

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

    #[test]
    fn only_verified_email_is_eligible_for_identity_linking() {
        assert_eq!(
            verified_email(Some("user@example.com".to_owned()), Some(true)).as_deref(),
            Some("user@example.com")
        );
        assert_eq!(
            verified_email(Some("user@example.com".to_owned()), Some(false)),
            None
        );
    }

    #[test]
    fn token_expiry_caps_cache_ttl() {
        assert_eq!(
            cache_ttl_for_exp(Duration::from_secs(30), Some(110), 100).expect("ttl"),
            Duration::from_secs(10)
        );
        assert_eq!(
            cache_ttl_for_exp(Duration::from_secs(30), Some(100), 100).expect("ttl"),
            Duration::ZERO
        );
        assert!(cache_ttl_for_exp(Duration::from_secs(30), Some(i64::MAX), i64::MIN).is_err());
    }
}

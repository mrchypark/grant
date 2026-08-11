use crate::{
    config::ReadConsistency,
    config::{AuthConfig, AuthProviderMode, PatchDataAuthConfig},
    database::{CachedExternalIdentity, Database, ExternalIdentityMaterialization},
};
use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use futures_util::StreamExt;
use reqwest::Client;
use serde::{
    Deserialize,
    de::{self, MapAccess, Visitor},
};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const RAUTHY_AUTH_PROVIDER: &str = "rauthy";
const PATCH_DATA_AUTH_PROVIDER: &str = "patch-data";
const PATCH_DATA_AUTH_REFRESH_PATH: &str = "/api/collections/users/auth-refresh";
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_PATCH_DATA_AUTH_RESPONSE_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedPrincipal {
    pub user_id: String,
    pub organization_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalIdentity {
    pub provider: String,
    pub external_user_id: String,
    pub client_id: String,
    pub email: Option<String>,
    pub display_name: String,
    pub active: bool,
    pub organization_id: Option<String>,
}

#[derive(Clone)]
pub struct AuthResolver {
    rauthy: RauthyResolver,
    patch_data: Option<PatchDataResolver>,
    provider_mode: AuthProviderMode,
}

#[derive(Clone)]
struct PatchDataResolver {
    base_url: String,
    exposure_application_id: String,
    client: Client,
}

#[derive(Debug, Clone)]
struct PatchDataIdentity {
    external_user_id: String,
    email: String,
    display_name: String,
    organization_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PatchDataAuthRefreshResponse {
    #[serde(rename = "token")]
    _token: String,
    record: PatchDataAuthRefreshRecord,
}

#[derive(Debug)]
struct PatchDataAuthRefreshRecord {
    id: String,
    email: String,
    verified: bool,
    display_name: Option<String>,
    organization_id: String,
}

impl<'de> Deserialize<'de> for PatchDataAuthRefreshRecord {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct RecordVisitor;

        impl<'de> Visitor<'de> for RecordVisitor {
            type Value = PatchDataAuthRefreshRecord;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a Patch Data auth refresh record")
            }

            fn visit_map<A>(self, mut map: A) -> std::result::Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut id = None;
                let mut email = None;
                let mut verified = None;
                let mut display_name = None;
                let mut organization_id = None;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "id" => set_once(&mut id, map.next_value()?, "id")?,
                        "email" => set_once(&mut email, map.next_value()?, "email")?,
                        "verified" => set_once(&mut verified, map.next_value()?, "verified")?,
                        "displayName" | "name" => {
                            set_once(&mut display_name, map.next_value()?, "displayName")?
                        }
                        "organizationId" => {
                            set_once(&mut organization_id, map.next_value()?, "organizationId")?
                        }
                        _ => {
                            let _: de::IgnoredAny = map.next_value()?;
                        }
                    }
                }
                Ok(PatchDataAuthRefreshRecord {
                    id: id.ok_or_else(|| de::Error::missing_field("id"))?,
                    email: email.ok_or_else(|| de::Error::missing_field("email"))?,
                    verified: verified.ok_or_else(|| de::Error::missing_field("verified"))?,
                    display_name,
                    organization_id: organization_id
                        .ok_or_else(|| de::Error::missing_field("organizationId"))?,
                })
            }
        }

        deserializer.deserialize_map(RecordVisitor)
    }
}

fn set_once<T, E: de::Error>(
    slot: &mut Option<T>,
    value: T,
    field: &'static str,
) -> std::result::Result<(), E> {
    if slot.replace(value).is_some() {
        return Err(E::duplicate_field(field));
    }
    Ok(())
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
    client_id: String,
    email: Option<String>,
    display_name: String,
    active: bool,
}

#[derive(Debug, Deserialize)]
struct RauthyIntrospectionResponse {
    sub: String,
    client_id: String,
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
        Self::new_with_patch_data_request_timeout(cfg, HTTP_REQUEST_TIMEOUT)
    }

    /// Test seam: production always uses `HTTP_REQUEST_TIMEOUT`; callers that
    /// construct an isolated resolver can bound a deliberately stalled peer.
    pub fn new_with_patch_data_request_timeout(
        cfg: &AuthConfig,
        patch_data_request_timeout: Duration,
    ) -> Self {
        Self {
            rauthy: RauthyResolver {
                base_url: cfg.rauthy.base_url.trim_end_matches('/').to_owned(),
                introspect_path: cfg.rauthy.introspect_path.clone(),
                userinfo_path: cfg.rauthy.userinfo_path.clone(),
                client_id: cfg.rauthy.client_id.clone(),
                client_secret: cfg.rauthy.client_secret.clone(),
                cache_ttl: Duration::from_secs(cfg.rauthy.cache_ttl_seconds),
                client: Client::builder()
                    .connect_timeout(HTTP_CONNECT_TIMEOUT)
                    .timeout(HTTP_REQUEST_TIMEOUT)
                    .redirect(reqwest::redirect::Policy::none())
                    .build()
                    .expect("build Rauthy auth client"),
                cache: Arc::new(Mutex::new(HashMap::new())),
            },
            patch_data: cfg
                .patch_data
                .as_ref()
                .map(|config| PatchDataResolver::new(config, patch_data_request_timeout)),
            provider_mode: cfg.provider_mode.clone(),
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
            .resolve_identity(
                db,
                token,
                Some(organization_id),
                effective_read_consistency_override.clone(),
                should_sync,
            )
            .await?;
        let user = if should_sync {
            if matches!(self.provider_mode, AuthProviderMode::PatchData) {
                db.materialize_external_identity(ExternalIdentityMaterialization {
                    provider: &identity.provider,
                    external_user_id: &identity.external_user_id,
                    email: identity.email.as_deref(),
                    display_name: &identity.display_name,
                })
                .await
                .map_err(map_strong_identity_sync_error)?
            } else {
                db.resolve_or_create_user_identity_with_consistency(
                    &identity.provider,
                    &identity.external_user_id,
                    identity.email.as_deref(),
                    &identity.display_name,
                    effective_read_consistency_override.clone(),
                )
                .await
                .map_err(map_strong_identity_sync_error)?
            }
        } else {
            db.resolve_existing_user_identity_with_consistency(
                &identity.provider,
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
        if !user.active {
            anyhow::bail!("forbidden: user {} is inactive", user.id);
        }
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
        self.resolve_external_identity_for_organization(db, bearer_token, None, read_consistency)
            .await
    }

    pub async fn resolve_external_identity_for_organization(
        &self,
        db: Option<&Database>,
        bearer_token: Option<&str>,
        organization_id: Option<&str>,
        read_consistency: Option<ReadConsistency>,
    ) -> Result<ExternalIdentity> {
        let token = bearer_token
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("unauthorized: missing bearer token"))?;
        let identity = if let Some(db) = db {
            let effective_read_consistency =
                db.default_read_consistency_with_override(read_consistency.clone());
            self.resolve_identity(
                db,
                token,
                organization_id,
                Some(effective_read_consistency.clone()),
                should_sync_identity(Some(&effective_read_consistency)),
            )
            .await?
        } else {
            self.resolve_identity_without_db(token, organization_id)
                .await?
        };
        Ok(identity)
    }

    async fn resolve_identity(
        &self,
        db: &Database,
        token: &str,
        organization_id: Option<&str>,
        read_consistency: Option<ReadConsistency>,
        persist_shared_cache: bool,
    ) -> Result<ExternalIdentity> {
        match self.provider_mode {
            AuthProviderMode::Rauthy => {
                let token = bearer_value(token)?;
                let identity = self
                    .rauthy
                    .introspect(db, token, read_consistency, persist_shared_cache)
                    .await?;
                Ok(ExternalIdentity {
                    provider: RAUTHY_AUTH_PROVIDER.to_owned(),
                    external_user_id: identity.external_user_id,
                    client_id: identity.client_id,
                    email: identity.email,
                    display_name: identity.display_name,
                    active: identity.active,
                    organization_id: None,
                })
            }
            AuthProviderMode::PatchData => {
                self.patch_data
                    .as_ref()
                    .expect("Patch Data config validated")
                    .resolve(token, organization_id)
                    .await
            }
        }
    }

    async fn resolve_identity_without_db(
        &self,
        token: &str,
        organization_id: Option<&str>,
    ) -> Result<ExternalIdentity> {
        match self.provider_mode {
            AuthProviderMode::Rauthy => {
                let token = bearer_value(token)?;
                let identity = self.rauthy.introspect_without_db(token).await?;
                Ok(ExternalIdentity {
                    provider: RAUTHY_AUTH_PROVIDER.to_owned(),
                    external_user_id: identity.external_user_id,
                    client_id: identity.client_id,
                    email: identity.email,
                    display_name: identity.display_name,
                    active: identity.active,
                    organization_id: None,
                })
            }
            AuthProviderMode::PatchData => {
                self.patch_data
                    .as_ref()
                    .expect("Patch Data config validated")
                    .resolve(token, organization_id)
                    .await
            }
        }
    }
}

impl PatchDataResolver {
    fn new(cfg: &PatchDataAuthConfig, request_timeout: Duration) -> Self {
        Self {
            base_url: cfg.base_url.trim_end_matches('/').to_owned(),
            exposure_application_id: cfg.exposure_application_id.clone(),
            client: Client::builder()
                .connect_timeout(HTTP_CONNECT_TIMEOUT)
                .timeout(request_timeout)
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("build Patch Data auth client"),
        }
    }

    async fn resolve(
        &self,
        authorization_value: &str,
        expected_organization_id: Option<&str>,
    ) -> Result<ExternalIdentity> {
        let response = self
            .client
            .post(format!("{}{}", self.base_url, PATCH_DATA_AUTH_REFRESH_PATH))
            // Patch Data validates the credential. Grant deliberately does not decode or trust JWT claims.
            .header(reqwest::header::AUTHORIZATION, authorization_value)
            .send()
            .await
            .map_err(map_patch_data_request_error)?;
        if response.status() == reqwest::StatusCode::UNAUTHORIZED
            || response.status() == reqwest::StatusCode::FORBIDDEN
        {
            anyhow::bail!("unauthorized: Patch Data token rejected");
        }
        if !response.status().is_success() {
            // Patch Data is the authority for credentials only.  Every other
            // upstream response is an availability failure; do not expose its
            // status, body, or transport detail to Grant callers.
            anyhow::bail!("unavailable: Patch Data auth refresh is unavailable");
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_PATCH_DATA_AUTH_RESPONSE_BYTES as u64)
        {
            anyhow::bail!("unavailable: Patch Data auth refresh response is too large");
        }
        let mut bytes = Vec::with_capacity(
            response
                .content_length()
                .unwrap_or_default()
                .min(MAX_PATCH_DATA_AUTH_RESPONSE_BYTES as u64) as usize,
        );
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(map_patch_data_request_error)?;
            let remaining = MAX_PATCH_DATA_AUTH_RESPONSE_BYTES.saturating_sub(bytes.len());
            if chunk.len() > remaining {
                // Dropping the response stream aborts the connection immediately;
                // no chunked or dishonest Content-Length response can exceed this
                // in-memory bound.
                anyhow::bail!("unavailable: Patch Data auth refresh response is too large");
            }
            bytes.extend_from_slice(&chunk);
        }
        let payload: PatchDataAuthRefreshResponse = serde_json::from_slice(&bytes)
            .map_err(|_| anyhow!("unavailable: Patch Data auth refresh response was invalid"))?;
        if payload._token.trim().is_empty() {
            anyhow::bail!("unavailable: Patch Data auth refresh response was invalid");
        }
        if !payload.record.verified {
            anyhow::bail!("forbidden: Patch Data email is not verified");
        }
        let identity = PatchDataIdentity {
            external_user_id: normalized_required("Patch Data user id", &payload.record.id)?,
            email: normalized_email(&payload.record.email)?,
            display_name: payload
                .record
                .display_name
                .as_deref()
                .and_then(normalized_optional)
                .unwrap_or_else(|| "Patch Data User".to_owned()),
            organization_id: normalized_required(
                "Patch Data organization id",
                &payload.record.organization_id,
            )?,
        };
        // A refresh token is opaque to Grant. Do not cache it: the provider is
        // authoritative for expiry and revocation, so this cannot outlive it.
        self.external_identity(identity, expected_organization_id)
    }

    fn external_identity(
        &self,
        identity: PatchDataIdentity,
        expected_organization_id: Option<&str>,
    ) -> Result<ExternalIdentity> {
        if let Some(expected) = expected_organization_id
            && identity.organization_id != expected
        {
            anyhow::bail!(
                "forbidden: Patch Data user is not a member of the requested organization"
            );
        }
        Ok(ExternalIdentity {
            provider: PATCH_DATA_AUTH_PROVIDER.to_owned(),
            external_user_id: identity.external_user_id,
            client_id: self.exposure_application_id.clone(),
            email: Some(identity.email),
            display_name: identity.display_name,
            active: true,
            organization_id: Some(identity.organization_id),
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
        let cache_key = token_cache_key(RAUTHY_AUTH_PROVIDER, token);
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
                return Err(map_rauthy_request_error(
                    "introspection",
                    "call Rauthy introspect",
                    error,
                ));
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
        let (identity, local_expires_at, shared_expires_at) =
            self.identity_from_introspection(token, response).await?;
        self.write_cache_until(cache_key, identity.clone(), local_expires_at)
            .await;
        if persist_shared_cache {
            self.write_shared_cache(db, token, &identity, shared_expires_at)
                .await?;
        }
        Ok(identity)
    }

    async fn introspect_without_db(&self, token: &str) -> Result<RauthyIdentity> {
        let cache_key = token_cache_key(RAUTHY_AUTH_PROVIDER, token);
        if let Some(identity) = self.read_cache(&cache_key).await {
            return Ok(identity);
        }

        let response = self.send_introspection(token).await.map_err(|error| {
            map_rauthy_request_error("introspection", "call Rauthy introspect", error)
        })?;
        reject_unauthorized_introspection(&response)?;
        let response = response
            .error_for_status()
            .context("Rauthy introspect returned error status")?;
        let (identity, local_expires_at, _) =
            self.identity_from_introspection(token, response).await?;
        self.write_cache_until(cache_key, identity.clone(), local_expires_at)
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
    ) -> Result<(RauthyIdentity, Instant, DateTime<Utc>)> {
        let token_info = parse_introspection_response(response).await?;
        let identity = self
            .fetch_userinfo(token, &token_info.sub, &token_info.client_id)
            .await?;
        let monotonic_now = Instant::now();
        let wall_now = Utc::now();
        let (local_expires_at, shared_expires_at) =
            cache_deadlines(self.cache_ttl, token_info.exp, wall_now, monotonic_now)?;
        Ok((identity, local_expires_at, shared_expires_at))
    }

    async fn fetch_userinfo(
        &self,
        token: &str,
        expected_sub: &str,
        client_id: &str,
    ) -> Result<RauthyIdentity> {
        let url = format!("{}{}", self.base_url, self.userinfo_path);
        let response = self
            .client
            .get(url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|error| map_rauthy_request_error("userinfo", "call Rauthy userinfo", error))?;
        reject_unauthorized_introspection(&response)?;
        let response = response
            .error_for_status()
            .context("Rauthy userinfo returned error status")?;
        let payload: RauthyUserinfoResponse = response.json().await.map_err(|error| {
            map_rauthy_request_error("userinfo", "decode Rauthy userinfo response", error)
        })?;
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
            client_id: client_id.to_owned(),
            email,
            display_name,
            active: true,
        })
    }

    async fn read_cache(&self, key: &str) -> Option<RauthyIdentity> {
        self.read_cache_at(key, Instant::now()).await
    }

    async fn read_cache_at(&self, key: &str, now: Instant) -> Option<RauthyIdentity> {
        let mut cache = self.cache.lock().expect("Rauthy auth cache poisoned");
        let cached = cache.get(key)?;
        if cached.expires_at <= now {
            cache.remove(key);
            return None;
        }
        Some(cached.identity.clone())
    }

    async fn write_cache_until(&self, key: String, identity: RauthyIdentity, expires_at: Instant) {
        let mut cache = self.cache.lock().expect("Rauthy auth cache poisoned");
        let now = Instant::now();
        cache.retain(|_, cached| cached.expires_at > now);
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
        let query_now = Utc::now();
        let cached = db
            .get_cached_external_identity(token_hash, query_now, read_consistency)
            .await?;
        let Some(cached) = cached else {
            return Ok(None);
        };
        let monotonic_now = Instant::now();
        let wall_now = Utc::now();
        if cached.expires_at <= wall_now {
            return Ok(None);
        }
        if cached.provider != RAUTHY_AUTH_PROVIDER {
            return Ok(None);
        }
        let identity = RauthyIdentity {
            external_user_id: cached.external_user_id,
            client_id: cached.client_id,
            email: cached.email,
            display_name: cached.display_name,
            active: cached.active,
        };
        let ttl = cached
            .expires_at
            .signed_duration_since(wall_now)
            .to_std()
            .unwrap_or_else(|_| Duration::from_secs(0));
        self.write_cache_until(token_hash.to_owned(), identity.clone(), monotonic_now + ttl)
            .await;
        Ok(Some(identity))
    }

    async fn write_shared_cache(
        &self,
        db: &Database,
        token: &str,
        identity: &RauthyIdentity,
        expires_at: DateTime<Utc>,
    ) -> Result<()> {
        let token_hash = token_cache_key(RAUTHY_AUTH_PROVIDER, token);
        db.put_cached_external_identity(
            &token_hash,
            &CachedExternalIdentity {
                provider: RAUTHY_AUTH_PROVIDER.to_owned(),
                external_user_id: identity.external_user_id.clone(),
                client_id: identity.client_id.clone(),
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
    client_id: Option<String>,
    exp: Option<i64>,
}

async fn parse_introspection_response(
    response: reqwest::Response,
) -> Result<RauthyIntrospectionResponse> {
    let payload: RauthyIntrospectionPayload = response.json().await.map_err(|error| {
        map_rauthy_request_error(
            "introspection",
            "decode Rauthy introspection response",
            error,
        )
    })?;
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
    let client_id = payload
        .client_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("unauthorized: rauthy token has no client_id"))?
        .to_owned();
    Ok(RauthyIntrospectionResponse {
        sub,
        client_id,
        exp: payload.exp,
    })
}

fn map_rauthy_request_error(
    operation: &str,
    context: &'static str,
    error: reqwest::Error,
) -> anyhow::Error {
    if error.is_timeout() || error.is_connect() || error.is_request() || error.is_body() {
        anyhow!("unavailable: Rauthy {operation} request failed")
    } else {
        anyhow::Error::new(error).context(context)
    }
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

fn cache_deadlines(
    configured: Duration,
    exp: Option<i64>,
    wall_now: DateTime<Utc>,
    monotonic_now: Instant,
) -> Result<(Instant, DateTime<Utc>)> {
    let configured_expiry = wall_now
        .checked_add_signed(
            ChronoDuration::from_std(configured)
                .map_err(|_| anyhow!("unauthorized: invalid auth cache duration"))?,
        )
        .ok_or_else(|| anyhow!("unauthorized: invalid auth cache expiration"))?;
    let wall_expires_at = if let Some(exp) = exp {
        let token_expires_at = DateTime::<Utc>::from_timestamp(exp, 0)
            .ok_or_else(|| anyhow!("unauthorized: invalid token expiration"))?;
        if token_expires_at <= wall_now {
            anyhow::bail!("unauthorized: rauthy token is expired");
        }
        configured_expiry.min(token_expires_at)
    } else {
        configured_expiry
    };
    let remaining = wall_expires_at
        .signed_duration_since(wall_now)
        .to_std()
        .map_err(|_| anyhow!("unauthorized: rauthy token is expired"))?;
    let monotonic_expires_at = monotonic_now
        .checked_add(remaining)
        .ok_or_else(|| anyhow!("unauthorized: invalid auth cache expiration"))?;
    Ok((monotonic_expires_at, wall_expires_at))
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

fn token_cache_key(provider: &str, token: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(provider.as_bytes());
    digest.update([0]);
    digest.update(token.as_bytes());
    hex::encode(digest.finalize())
}

fn normalized_optional(value: &str) -> Option<String> {
    let normalized = value.trim();
    (!normalized.is_empty()).then(|| normalized.to_owned())
}

fn bearer_value(value: &str) -> Result<&str> {
    value
        .trim()
        .strip_prefix("Bearer ")
        .or_else(|| value.trim().strip_prefix("bearer "))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("unauthorized: missing bearer token"))
}

fn normalized_required(field: &str, value: &str) -> Result<String> {
    normalized_optional(value).ok_or_else(|| anyhow!("unauthorized: {field} is missing"))
}

fn normalized_email(value: &str) -> Result<String> {
    let email = normalized_required("Patch Data email", value)?.to_ascii_lowercase();
    if !email.contains('@') {
        anyhow::bail!("unauthorized: Patch Data email is invalid");
    }
    Ok(email)
}

fn map_patch_data_request_error(_error: reqwest::Error) -> anyhow::Error {
    // The response and all transport details belong to an internal upstream;
    // returning them would turn an auth boundary into an information oracle.
    anyhow!("unavailable: Patch Data auth refresh request failed")
}

#[cfg(test)]
mod tests {
    use super::{
        CachedRauthyIdentity, RauthyIdentity, cache_deadlines, map_strong_identity_sync_error,
        normalized_email, normalized_required, should_sync_identity, token_cache_key,
        verified_email,
    };
    use crate::config::{AuthConfig, AuthProviderMode, RauthyAuthConfig, ReadConsistency};
    use anyhow::anyhow;
    use chrono::{TimeZone, Utc};
    use std::time::{Duration, Instant};

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
    fn patch_data_identity_values_are_normalized_and_required() {
        assert_eq!(
            normalized_required("id", "  user-1  ").expect("id"),
            "user-1"
        );
        assert_eq!(
            normalized_email("  USER@example.test ").expect("email"),
            "user@example.test"
        );
        assert!(normalized_required("id", " \t ").is_err());
        assert!(normalized_email("not-an-email").is_err());
    }

    #[test]
    fn auth_cache_keys_are_provider_namespaced() {
        assert_ne!(
            token_cache_key("rauthy", "same-token"),
            token_cache_key("patch-data", "same-token")
        );
    }

    #[test]
    fn token_expiry_anchors_the_cache_deadline_without_subsecond_overrun() {
        let monotonic_now = Instant::now();
        let wall_now = Utc
            .timestamp_opt(100, 500_000_000)
            .single()
            .expect("wall clock");
        let (monotonic_expires_at, wall_expires_at) =
            cache_deadlines(Duration::from_secs(30), Some(101), wall_now, monotonic_now)
                .expect("deadlines");
        assert_eq!(
            monotonic_expires_at.duration_since(monotonic_now),
            Duration::from_millis(500)
        );
        assert_eq!(
            wall_expires_at,
            Utc.timestamp_opt(101, 0).single().expect("token expiry")
        );
        assert!(
            cache_deadlines(
                Duration::from_secs(30),
                Some(100),
                Utc.timestamp_opt(100, 0).single().expect("wall clock"),
                monotonic_now,
            )
            .expect_err("expired token")
            .to_string()
            .contains("token is expired")
        );
        assert!(
            cache_deadlines(
                Duration::from_secs(30),
                Some(i64::MAX),
                wall_now,
                monotonic_now,
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn cached_identity_is_rejected_at_its_anchored_expiry_without_sleeping() {
        let resolver = super::AuthResolver::new(&AuthConfig {
            provider_mode: AuthProviderMode::Rauthy,
            rauthy: RauthyAuthConfig {
                base_url: "http://localhost".to_owned(),
                introspect_path: "/introspect".to_owned(),
                userinfo_path: "/userinfo".to_owned(),
                client_id: "client".to_owned(),
                client_secret: "secret".to_owned(),
                client_read_api_key: "client-read".to_owned(),
                cache_ttl_seconds: 30,
            },
            patch_data: None,
        })
        .rauthy;
        let identity = RauthyIdentity {
            external_user_id: "user".to_owned(),
            client_id: "client".to_owned(),
            email: None,
            display_name: "User".to_owned(),
            active: true,
        };
        let monotonic_now = Instant::now();
        let wall_now = Utc
            .timestamp_opt(100, 500_000_000)
            .single()
            .expect("wall clock");
        let (expires_at, _) =
            cache_deadlines(Duration::from_secs(30), Some(101), wall_now, monotonic_now)
                .expect("anchored token deadline");
        resolver
            .write_cache_until("token".to_owned(), identity.clone(), expires_at)
            .await;

        let cached = resolver
            .read_cache_at("token", expires_at - Duration::from_nanos(1))
            .await
            .expect("identity before expiry");
        assert_eq!(cached.external_user_id, identity.external_user_id);
        assert_eq!(cached.client_id, identity.client_id);
        assert!(resolver.read_cache_at("token", expires_at).await.is_none());
        assert!(resolver.read_cache_at("token", expires_at).await.is_none());
    }

    #[tokio::test]
    async fn cache_write_sweeps_unqueried_expired_entries() {
        let resolver = super::AuthResolver::new(&AuthConfig {
            provider_mode: AuthProviderMode::Rauthy,
            rauthy: RauthyAuthConfig {
                base_url: "http://localhost".to_owned(),
                introspect_path: "/introspect".to_owned(),
                userinfo_path: "/userinfo".to_owned(),
                client_id: "client".to_owned(),
                client_secret: "secret".to_owned(),
                client_read_api_key: "client-read".to_owned(),
                cache_ttl_seconds: 30,
            },
            patch_data: None,
        })
        .rauthy;
        let identity = RauthyIdentity {
            external_user_id: "user".to_owned(),
            client_id: "client".to_owned(),
            email: None,
            display_name: "User".to_owned(),
            active: true,
        };
        resolver.cache.lock().expect("cache").insert(
            "expired".to_owned(),
            CachedRauthyIdentity {
                identity: identity.clone(),
                expires_at: Instant::now() - Duration::from_secs(1),
            },
        );

        resolver
            .write_cache_until(
                "fresh".to_owned(),
                identity,
                Instant::now() + Duration::from_secs(30),
            )
            .await;

        let cache = resolver.cache.lock().expect("cache");
        assert!(!cache.contains_key("expired"));
        assert!(cache.contains_key("fresh"));
    }
}

use crate::{
    auth::{
        AuthResolver, AuthenticatedPrincipal, ExternalIdentity, map_strong_identity_sync_error,
    },
    config::{AppConfig, AuthProviderMode, ReadConsistency},
    database::{
        ApplicationConsentWriteError, ApplicationSecurityMutationError,
        ApplicationTrustMutationError, BackupEntry, BindIdentityInput, Database,
        ExternalIdentityMaterialization, ExternalOwnerBootstrapCommand,
        ExternalOwnerBootstrapError, OrganizationMembershipWriteError, PatchDataAuthorityImport,
        PatchDataAuthorityImportResult, UserActionCheck,
    },
};
use anyhow::Result;
use chrono::Utc;
use domain::{
    Action, Agreement, AgreementStatus, AgreementType, Application, ApplicationAppType,
    ApplicationConsentPolicy, ApplicationGrant, ApplicationGrantStatus,
    ApplicationProvisioningStatus, ApplicationType, Group, GroupMember, HealthStatus,
    MembershipStatus, Organization, OrganizationMembership, Role, RoleBinding, RoleBindingState,
    RoleBindingSubjectType, ServiceProvider, User,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::net::IpAddr;
use std::{
    collections::HashSet,
    error::Error as StdError,
    fmt,
    future::Future,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

const MAX_APPLICATION_ACTION_IDS_PER_REQUEST: usize = 999;
const MAX_ACTION_IDS_PER_REQUEST: usize = 999;
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const APPLICATION_SECURITY_MUTATION_LEASE_MINUTES: i64 = 10;
const MAX_RAUTHY_ERROR_BODY_BYTES: usize = 4 * 1024;
const MAX_RAUTHY_ERROR_FIELD_CHARS: usize = 256;
// Schema shape changes only through controlled migrations. Keep this short so an
// out-of-band schema loss is still detected without issuing a Raft query per probe.
const SCHEMA_READINESS_CACHE_TTL: Duration = Duration::from_secs(30);
const EXPOSURE_READ_PROVIDER_ID: &str = "exposure";
const EXPOSURE_READ_ACTION_ID: &str = "exposure.read";
const PATCH_DATA_PLANT_READ_PROVIDER_ID: &str = "patch-data";
const PATCH_DATA_PLANT_READ_ACTION_ID: &str = "patch-data.plant.read";
const PATCH_DATA_PLANT_RESOURCE_TYPE: &str = "plant";
const EXPOSURE_DECISION_TTL_SECONDS: i64 = 5;
const EXPOSURE_PRINCIPAL_DIGEST_DOMAIN: &[u8] = b"grant.exposure-read.principal.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceErrorKind {
    BadRequest,
    Unauthorized,
    Forbidden,
    NotFound,
    Conflict,
    Unavailable,
}

#[derive(Debug)]
pub struct ServiceError {
    kind: ServiceErrorKind,
    message: String,
}

impl ServiceError {
    pub fn new(kind: ServiceErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> ServiceErrorKind {
        self.kind
    }
}

impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl StdError for ServiceError {}

#[derive(Clone)]
pub struct AppService {
    cfg: AppConfig,
    db: Database,
    auth: AuthResolver,
    http_client: reqwest::Client,
    proxy_client: reqwest::Client,
    schema_readiness: Arc<SchemaReadinessCache>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SchemaReadinessRoute {
    Local,
    Strong,
}

#[derive(Debug, Default)]
struct SchemaReadinessState {
    local_verified_at: Option<Instant>,
    strong_verified_at: Option<Instant>,
}

impl SchemaReadinessState {
    fn is_fresh(&self, route: SchemaReadinessRoute, now: Instant, ttl: Duration) -> bool {
        self.verified_at(route)
            .is_some_and(|verified_at| now.saturating_duration_since(verified_at) < ttl)
    }

    fn mark_verified(&mut self, route: SchemaReadinessRoute, verified_at: Instant) {
        match route {
            SchemaReadinessRoute::Local => self.local_verified_at = Some(verified_at),
            SchemaReadinessRoute::Strong => self.strong_verified_at = Some(verified_at),
        }
    }

    fn clear(&mut self) {
        self.local_verified_at = None;
        self.strong_verified_at = None;
    }

    fn verified_at(&self, route: SchemaReadinessRoute) -> Option<Instant> {
        match route {
            SchemaReadinessRoute::Local => self.local_verified_at,
            SchemaReadinessRoute::Strong => self.strong_verified_at,
        }
    }
}

#[derive(Debug)]
struct SchemaReadinessCache {
    ttl: Duration,
    state: Mutex<SchemaReadinessState>,
}

impl SchemaReadinessCache {
    fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            state: Mutex::new(SchemaReadinessState::default()),
        }
    }

    async fn verify<F, Fut>(&self, route: SchemaReadinessRoute, verify: F) -> bool
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<()>>,
    {
        let mut state = self.state.lock().await;
        if state.is_fresh(route, Instant::now(), self.ttl) {
            return true;
        }
        match verify().await {
            Ok(()) => {
                state.mark_verified(route, Instant::now());
                true
            }
            Err(_) => {
                state.clear();
                false
            }
        }
    }

    async fn while_invalidated<F, Fut, T>(&self, operation: F) -> Result<T>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        let mut state = self.state.lock().await;
        state.clear();
        let result = operation().await;
        state.clear();
        result
    }
}

#[derive(Debug, Clone)]
pub struct CreateApplicationInput {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub application_type: ApplicationType,
    pub app_type: ApplicationAppType,
    pub active: bool,
    pub redirect_uri: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ApplicationRead {
    pub application: Application,
    pub redirect_uri: Option<String>,
}

#[derive(Debug, Clone)]
pub struct UpsertGroupInput {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone)]
pub struct UpsertAgreementInput {
    pub id: String,
    pub agreement_type: AgreementType,
    pub grantor_organization_id: String,
    pub grantee_organization_id: String,
    pub status: AgreementStatus,
    pub title: String,
    pub reference_no: Option<String>,
    pub starts_at: Option<String>,
    pub ends_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CreateApplicationGrantInput {
    pub id: String,
    pub user_id: String,
    pub application_id: String,
    pub action_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RauthyClientResponse {
    id: String,
    name: Option<String>,
    enabled: bool,
    confidential: bool,
    redirect_uris: Vec<String>,
    flows_enabled: Vec<String>,
    challenges: Option<Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct CreateRoleInput {
    pub id: String,
    pub code: String,
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CreateRoleBindingInput {
    pub id: String,
    pub subject_type: RoleBindingSubjectType,
    pub subject_id: String,
    pub subject_organization_id: String,
    pub provider_id: String,
    pub resource_type: String,
    pub resource_id: String,
    pub role_id: String,
    pub agreement_id: Option<String>,
    pub state: RoleBindingState,
}

#[derive(Debug, Clone)]
pub struct AuthorizationSubject {
    pub subject_type: String,
    pub id: String,
    pub organization_id: String,
}

#[derive(Debug, Clone)]
pub struct AuthorizationResource {
    pub provider_id: String,
    pub resource_type: String,
    pub id: String,
}

#[derive(Debug, Clone)]
pub struct AuthorizationCheck {
    pub subject: AuthorizationSubject,
    pub action: String,
    pub resource: AuthorizationResource,
}

#[derive(Debug, Clone)]
pub struct ApplicationAuthorizationCheck {
    pub action: String,
    pub resource: AuthorizationResource,
}

#[derive(Debug, Clone)]
pub struct AuthorizationDecision {
    pub allowed: bool,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct ExposureReadAuthorizationDecision {
    pub allowed: bool,
    pub reason: String,
    pub principal_digest: String,
    pub issued_at: String,
    pub expires_at: String,
}

struct ApplicationAuthorizationContext {
    user_id: String,
    organization_id: String,
    application_id: String,
}

#[derive(Debug, Clone)]
pub struct BackupInfo {
    pub name: String,
    pub last_modified: i64,
    pub size: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct CurrentUserView {
    pub provider: String,
    pub external_user_id: String,
    pub user: User,
    pub organizations: Vec<Organization>,
    pub active_organization_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializedUserView {
    pub user_id: String,
    pub organization_id: String,
    pub membership_status: String,
}

impl AppService {
    pub fn new(cfg: AppConfig, db: Database) -> Self {
        let auth = AuthResolver::new(&cfg.auth);
        let http_client = reqwest::Client::builder()
            .connect_timeout(HTTP_CONNECT_TIMEOUT)
            .timeout(HTTP_REQUEST_TIMEOUT)
            .build()
            .expect("build Rauthy HTTP client");
        let proxy_client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(HTTP_CONNECT_TIMEOUT)
            .timeout(HTTP_REQUEST_TIMEOUT)
            .build()
            .expect("build Rauthy proxy client");
        Self {
            cfg,
            db,
            auth,
            http_client,
            proxy_client,
            schema_readiness: Arc::new(SchemaReadinessCache::new(SCHEMA_READINESS_CACHE_TTL)),
        }
    }

    pub async fn migrate_schema(&self) -> Result<()> {
        self.db
            .verify_coarse_deployment_compatible(ReadConsistency::Strong)
            .await?;
        self.schema_readiness
            .while_invalidated(|| self.db.ensure_schema_only())
            .await?;
        self.db
            .verify_coarse_deployment_compatible(ReadConsistency::Strong)
            .await
    }

    pub async fn initialize(&self) -> Result<()> {
        self.db
            .verify_coarse_deployment_compatible(ReadConsistency::Strong)
            .await?;
        self.schema_readiness
            .while_invalidated(|| self.db.ensure_schema())
            .await?;
        self.db
            .verify_coarse_deployment_compatible(ReadConsistency::Strong)
            .await
    }

    pub async fn bootstrap(&self) -> Result<()> {
        self.db
            .verify_coarse_deployment_compatible(ReadConsistency::Strong)
            .await?;
        self.db.ensure_bootstrap_data().await
    }

    pub async fn verify_coarse_deployment_compatibility(
        &self,
        consistency: ReadConsistency,
    ) -> Result<()> {
        self.db
            .verify_coarse_deployment_compatible(consistency)
            .await
    }

    pub async fn health_status(&self) -> HealthStatus {
        self.db.health_status().await
    }

    pub async fn is_ready(&self) -> bool {
        // Cluster/database health remains uncached and is checked on every probe.
        let readiness = self.db.readiness().await;
        let (consistency, route) = if readiness.read_ready {
            (ReadConsistency::Local, SchemaReadinessRoute::Local)
        } else if readiness.write_ready {
            (ReadConsistency::Strong, SchemaReadinessRoute::Strong)
        } else {
            return false;
        };
        self.schema_readiness
            .verify(route, || self.db.verify_schema_ready(consistency))
            .await
    }

    pub fn master_key(&self) -> &str {
        &self.cfg.security.master_key
    }

    pub fn app_auth_key(&self) -> &str {
        &self.cfg.security.app_auth_key
    }

    pub fn rauthy_base_url(&self) -> &str {
        &self.cfg.auth.rauthy.base_url
    }

    pub fn auth_provider_mode(&self) -> &AuthProviderMode {
        &self.cfg.auth.provider_mode
    }

    pub async fn import_patch_data_authority(
        &self,
        command: PatchDataAuthorityImport,
    ) -> Result<PatchDataAuthorityImportResult> {
        self.db.import_patch_data_authority(command).await
    }

    pub fn rauthy_proxy_client(&self) -> &reqwest::Client {
        &self.proxy_client
    }

    pub async fn authenticate_request(
        &self,
        bearer_token: Option<&str>,
        organization_id: Option<&str>,
        read_consistency: Option<ReadConsistency>,
    ) -> Result<AuthenticatedPrincipal> {
        self.auth
            .resolve(&self.db, bearer_token, organization_id, read_consistency)
            .await
    }

    pub async fn authenticate_request_for_inactive_organization(
        &self,
        bearer_token: Option<&str>,
        target_organization_id: &str,
        read_consistency: Option<ReadConsistency>,
    ) -> Result<AuthenticatedPrincipal> {
        let target_organization_id = require_non_empty("organizationId", target_organization_id)?;
        if matches!(self.cfg.auth.provider_mode, AuthProviderMode::PatchData) {
            // Patch Data verifies the bearer here; Grant's local membership
            // remains authoritative even while the organization is inactive.
            let identity = self
                .auth
                .resolve_external_identity_for_organization(
                    Some(&self.db),
                    bearer_token,
                    Some(&target_organization_id),
                    read_consistency.clone(),
                )
                .await?;
            let user = self
                .db
                .materialize_external_identity(ExternalIdentityMaterialization {
                    provider: &identity.provider,
                    external_user_id: &identity.external_user_id,
                    email: identity.email.as_deref(),
                    display_name: &identity.display_name,
                })
                .await
                .map_err(map_strong_identity_sync_error)?;
            if !user.active {
                return Err(forbidden_error("forbidden: local user is inactive"));
            }
            if !self
                .db
                .has_organization_membership_with_consistency(
                    &user.id,
                    &target_organization_id,
                    read_consistency,
                )
                .await?
            {
                return Err(forbidden_error(format!(
                    "forbidden: user {} is not an active member of organization {}",
                    user.id, target_organization_id
                )));
            }
            return Ok(AuthenticatedPrincipal {
                user_id: user.id,
                organization_id: target_organization_id,
            });
        }
        let current = self
            .current_user(bearer_token, None, read_consistency.clone())
            .await?;
        let has_membership = self
            .db
            .has_organization_membership_with_consistency(
                &current.user.id,
                &target_organization_id,
                read_consistency,
            )
            .await?;
        if !has_membership {
            return Err(forbidden_error(format!(
                "forbidden: user {} is not an active member of organization {}",
                current.user.id, target_organization_id
            )));
        }
        Ok(AuthenticatedPrincipal {
            user_id: current.user.id,
            organization_id: target_organization_id,
        })
    }

    pub async fn bootstrap_rauthy_owner(
        &self,
        bearer_token: Option<&str>,
    ) -> Result<AuthenticatedPrincipal> {
        if !matches!(self.cfg.auth.provider_mode, AuthProviderMode::Rauthy) {
            return Err(forbidden_error(
                "forbidden: Rauthy owner bootstrap is unavailable for the configured auth provider",
            ));
        }
        let identity = self
            .auth
            .resolve_external_identity(None, bearer_token, None)
            .await?;
        let user = self
            .db
            .bind_identity_to_user(BindIdentityInput {
                user_id: &self.cfg.security.bootstrap_user_id,
                provider: &identity.provider,
                external_user_id: &identity.external_user_id,
                email: identity.email.as_deref(),
                display_name: &identity.display_name,
                active: identity.active,
                allow_rebind: true,
            })
            .await?;
        if !user.active {
            return Err(forbidden_error("forbidden: local user is inactive"));
        }
        Ok(AuthenticatedPrincipal {
            user_id: user.id,
            organization_id: self.cfg.security.bootstrap_user_id.clone(),
        })
    }

    /// Establishes the first externally-authenticated owner without assuming an
    /// Rauthy client. This is intentionally master-key protected and idempotent.
    pub async fn bootstrap_external_owner(
        &self,
        bearer_token: Option<&str>,
        organization_id: Option<&str>,
    ) -> Result<AuthenticatedPrincipal> {
        let identity = self
            .auth
            .resolve_external_identity_for_bootstrap_organization(bearer_token, organization_id)
            .await?;
        let organization_id = identity.organization_id.clone().ok_or_else(|| {
            forbidden_error("forbidden: configured auth provider did not verify an organization")
        })?;
        if !identity.active {
            return Err(forbidden_error("forbidden: external user is inactive"));
        }
        let application_id = match self.cfg.auth.provider_mode {
            AuthProviderMode::PatchData => {
                &self
                    .cfg
                    .auth
                    .patch_data
                    .as_ref()
                    .expect("Patch Data config validated")
                    .exposure_application_id
            }
            // This route is for externally-scoped providers. Do not allow a
            // Rauthy caller to bypass its dedicated bootstrap route.
            AuthProviderMode::Rauthy => {
                return Err(forbidden_error(
                    "forbidden: external owner bootstrap is unavailable for Rauthy",
                ));
            }
        };
        let bootstrapped = self
            .db
            .bootstrap_external_owner_atomic(ExternalOwnerBootstrapCommand {
                identity: ExternalIdentityMaterialization {
                    provider: &identity.provider,
                    external_user_id: &identity.external_user_id,
                    email: identity.email.as_deref(),
                    display_name: &identity.display_name,
                },
                organization_id: &organization_id,
                application_id,
            })
            .await
            .map_err(map_external_owner_bootstrap_error)?;
        Ok(AuthenticatedPrincipal {
            user_id: bootstrapped.user_id,
            organization_id: bootstrapped.organization_id,
        })
    }

    /// Creates only the local user/identity pair for a verified external
    /// principal. Enrollment remains a separate owner-authorized operation.
    pub async fn materialize_current_user(
        &self,
        bearer_token: Option<&str>,
        organization_id: Option<&str>,
    ) -> Result<MaterializedUserView> {
        let organization_id = organization_id
            .ok_or_else(|| forbidden_error("forbidden: organization is required"))?;
        let identity = self
            .auth
            .resolve_external_identity_for_organization(
                Some(&self.db),
                bearer_token,
                Some(organization_id),
                Some(ReadConsistency::Strong),
            )
            .await?;
        if !identity.active {
            return Err(forbidden_error("forbidden: external user is inactive"));
        }
        let user = self
            .db
            .materialize_external_identity(ExternalIdentityMaterialization {
                provider: &identity.provider,
                external_user_id: &identity.external_user_id,
                email: identity.email.as_deref(),
                display_name: &identity.display_name,
            })
            .await
            .map_err(|_| forbidden_error("forbidden: identity materialization failed"))?;
        if !user.active {
            return Err(forbidden_error("forbidden: local user is inactive"));
        }
        let membership_status = match self
            .db
            .organization_membership_status_with_consistency(
                &user.id,
                organization_id,
                Some(ReadConsistency::Strong),
            )
            .await?
        {
            None => "none",
            Some(MembershipStatus::Active) => "active",
            Some(MembershipStatus::Disabled) => "disabled",
        }
        .to_owned();
        Ok(MaterializedUserView {
            user_id: user.id,
            organization_id: organization_id.to_owned(),
            membership_status,
        })
    }

    pub async fn current_user(
        &self,
        bearer_token: Option<&str>,
        active_organization_id: Option<&str>,
        read_consistency: Option<ReadConsistency>,
    ) -> Result<CurrentUserView> {
        let effective_read_consistency = self
            .db
            .default_read_consistency_with_override(read_consistency.clone());
        let effective_read_consistency_override = Some(effective_read_consistency.clone());
        let identity = self
            .auth
            .resolve_external_identity_for_organization(
                Some(&self.db),
                bearer_token,
                active_organization_id,
                effective_read_consistency_override.clone(),
            )
            .await?;
        let user = if matches!(
            effective_read_consistency,
            ReadConsistency::Strong | ReadConsistency::PreferStrong
        ) {
            if matches!(self.cfg.auth.provider_mode, AuthProviderMode::PatchData) {
                self.db
                    .materialize_external_identity(ExternalIdentityMaterialization {
                        provider: &identity.provider,
                        external_user_id: &identity.external_user_id,
                        email: identity.email.as_deref(),
                        display_name: &identity.display_name,
                    })
                    .await
                    .map_err(map_strong_identity_sync_error)?
            } else {
                self.db
                    .resolve_or_create_user_identity_with_consistency(
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
            self.db
                .resolve_existing_user_identity_with_consistency(
                    &identity.provider,
                    &identity.external_user_id,
                    effective_read_consistency_override.clone(),
                )
                .await?
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "forbidden: no local user identity available for {}:{}",
                        identity.provider,
                        identity.external_user_id
                    )
                })?
        };
        if !user.active {
            return Err(forbidden_error("forbidden: local user is inactive"));
        }
        let mut organizations = self
            .db
            .list_active_organizations_for_user_with_consistency(
                &user.id,
                effective_read_consistency_override,
            )
            .await?;
        if let Some(external_organization_id) = identity.organization_id.as_deref() {
            organizations.retain(|organization| organization.id == external_organization_id);
            if organizations.is_empty() {
                return Err(forbidden_error(
                    "forbidden: Patch Data user has no active local membership for the requested organization",
                ));
            }
        }
        let active_organization_id =
            if identity.organization_id.is_some() && active_organization_id.is_none() {
                identity.organization_id.clone()
            } else {
                normalize_active_organization(active_organization_id, &identity, &organizations)?
            };
        Ok(CurrentUserView {
            provider: identity.provider,
            external_user_id: identity.external_user_id,
            user,
            organizations,
            active_organization_id,
        })
    }

    pub async fn list_applications_authorized(
        &self,
        user_id: &str,
        organization_id: &str,
        read_consistency: Option<ReadConsistency>,
    ) -> Result<Vec<Application>> {
        self.authorize_organization_action(
            user_id,
            organization_id,
            organization_id,
            "organization.application.manage",
            "manage organization applications",
            read_consistency.clone(),
        )
        .await?;
        self.db
            .list_applications_with_consistency(organization_id, read_consistency)
            .await
    }

    pub async fn list_service_providers(
        &self,
        read_consistency: Option<ReadConsistency>,
    ) -> Result<Vec<ServiceProvider>> {
        self.db
            .list_service_providers_with_consistency(read_consistency)
            .await
    }

    pub async fn upsert_service_provider_admin(
        &self,
        provider: ServiceProvider,
    ) -> Result<ServiceProvider> {
        let provider = ServiceProvider {
            id: require_non_empty("id", &provider.id)?,
            name: require_non_empty("name", &provider.name)?,
            description: normalize_optional_text(provider.description),
            active: provider.active,
        };
        self.db.upsert_service_provider(&provider).await?;
        Ok(provider)
    }

    pub async fn set_user_active_admin(&self, user_id: &str, active: bool) -> Result<User> {
        let user_id = require_non_empty("userId", user_id)?;
        self.db.set_user_active(&user_id, active).await
    }

    pub async fn replace_provider_actions_admin(
        &self,
        provider_id: &str,
        actions: Vec<Action>,
    ) -> Result<Vec<Action>> {
        let provider_id = require_non_empty("providerId", provider_id)?;
        if actions.len() > MAX_ACTION_IDS_PER_REQUEST {
            return Err(bad_request_error(format!(
                "actions must contain at most {MAX_ACTION_IDS_PER_REQUEST} items"
            )));
        }
        let mut normalized_actions = Vec::with_capacity(actions.len());
        let mut ids = HashSet::new();
        let mut codes = HashSet::new();
        for action in actions {
            if action.visibility_granting {
                return Err(bad_request_error(
                    "action.visibilityGranting=true is unavailable in the coarse deployment baseline",
                ));
            }
            let code = require_non_empty("action.code", &action.code)?;
            let id = normalize_action_id(&provider_id, &code, &action.id)?;
            if action.provider_id != provider_id {
                return Err(bad_request_error(format!(
                    "action {id} providerId must match path providerId"
                )));
            }
            if !codes.insert(code.clone()) {
                return Err(bad_request_error(format!(
                    "duplicate action code in provider manifest: {code}"
                )));
            }
            if !ids.insert(id.clone()) {
                return Err(bad_request_error(format!(
                    "duplicate action id in provider manifest: {id}"
                )));
            }
            normalized_actions.push(Action {
                id,
                provider_id: provider_id.clone(),
                code,
                name: require_non_empty("action.name", &action.name)?,
                description: normalize_optional_text(action.description),
                risk_level: action.risk_level,
                visibility_granting: action.visibility_granting,
                grantable_to_applications: action.grantable_to_applications,
                consent_label: normalize_optional_text(action.consent_label),
                consent_description: normalize_optional_text(action.consent_description),
            });
        }
        self.db
            .replace_provider_actions(&provider_id, &normalized_actions)
            .await
    }

    pub async fn get_application_authorized(
        &self,
        user_id: &str,
        organization_id: &str,
        application_id: &str,
        read_consistency: Option<ReadConsistency>,
    ) -> Result<Application> {
        self.authorize_organization_action(
            user_id,
            organization_id,
            organization_id,
            "organization.application.manage",
            "manage organization applications",
            read_consistency.clone(),
        )
        .await?;
        let application = self
            .db
            .get_application_with_consistency(application_id, read_consistency)
            .await?;
        if application.organization_id != organization_id {
            return Err(not_found_error(format!(
                "application not found: {} in organization {}",
                application_id, organization_id
            )));
        }
        Ok(application)
    }

    pub async fn get_application_detail_authorized(
        &self,
        user_id: &str,
        organization_id: &str,
        application_id: &str,
        read_consistency: Option<ReadConsistency>,
    ) -> Result<ApplicationRead> {
        let application = self
            .get_application_authorized(
                user_id,
                organization_id,
                application_id,
                read_consistency.clone(),
            )
            .await?;
        let redirect_uri = self
            .application_redirect_uri_for_read(application_id, read_consistency)
            .await?;
        Ok(ApplicationRead {
            application,
            redirect_uri,
        })
    }

    pub async fn create_application(
        &self,
        user_id: &str,
        organization_id: &str,
        input: CreateApplicationInput,
    ) -> Result<ApplicationRead> {
        self.authorize_organization_action(
            user_id,
            organization_id,
            organization_id,
            "organization.application.manage",
            "manage organization applications",
            Some(ReadConsistency::Strong),
        )
        .await?;
        let application_id = require_non_empty("id", &input.id)?;
        let application = Application {
            id: application_id.clone(),
            organization_id: organization_id.to_owned(),
            slug: require_non_empty("slug", &input.slug)?,
            name: require_non_empty("name", &input.name)?,
            description: normalize_optional_text(input.description),
            application_type: input.application_type,
            app_type: input.app_type,
            first_party: false,
            consent_policy: ApplicationConsentPolicy::Explicit,
            active: input.active,
            provisioning_status: ApplicationProvisioningStatus::Unprovisioned,
            rauthy_client_id: Some(format!("grant-{}", uuid::Uuid::new_v4().simple())),
            created_by_user_id: user_id.to_owned(),
        };
        let redirect_uri = input
            .redirect_uri
            .as_deref()
            .map(canonical_redirect_uri)
            .transpose()?;
        self.db
            .insert_application_with_optional_redirect_uri(&application, redirect_uri.as_deref())
            .await?;
        Ok(ApplicationRead {
            application,
            redirect_uri,
        })
    }

    async fn acquire_application_security_mutation(
        &self,
        organization_id: &str,
        application_id: &str,
    ) -> Result<(String, Application)> {
        let lease_token = uuid::Uuid::new_v4().simple().to_string();
        let acquired_at = Utc::now();
        let expires_at =
            acquired_at + chrono::Duration::minutes(APPLICATION_SECURITY_MUTATION_LEASE_MINUTES);
        if !self
            .db
            .try_acquire_application_security_mutation(
                application_id,
                organization_id,
                &lease_token,
                &acquired_at.to_rfc3339(),
                &expires_at.to_rfc3339(),
            )
            .await?
        {
            let current = self
                .db
                .get_application_with_consistency(application_id, Some(ReadConsistency::Strong))
                .await?;
            ensure_organization_application_mutable(&current)?;
            return Err(service_error(
                ServiceErrorKind::Conflict,
                "application security mutation is already in progress",
            ));
        }
        let application = self
            .db
            .get_application_with_consistency(application_id, Some(ReadConsistency::Strong))
            .await?;
        Ok((lease_token, application))
    }

    pub async fn verify_application_provisioning_admin(
        &self,
        organization_id: &str,
        application_id: &str,
    ) -> Result<Application> {
        let application = self
            .db
            .get_application_with_consistency(application_id, Some(ReadConsistency::Strong))
            .await?;
        if application.organization_id != organization_id {
            return Err(not_found_error(format!(
                "application not found: {application_id} in organization {organization_id}"
            )));
        }
        let client_id = rauthy_client_id(&application)?;
        let response = self
            .fetch_rauthy_application_client(
                &self.rauthy_admin_url(&format!("/auth/v1/clients/{client_id}")),
            )
            .await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(service_error(
                ServiceErrorKind::Conflict,
                "Rauthy client is not provisioned",
            ));
        }
        if !response.status().is_success() {
            return Err(rauthy_admin_response_error("rauthy client fetch", response).await);
        }
        let client: RauthyClientResponse = response.json().await.map_err(|err| {
            service_error(
                ServiceErrorKind::Unavailable,
                format!("invalid Rauthy client response: {err}"),
            )
        })?;
        let expected_confidential =
            matches!(application.application_type, ApplicationType::Confidential);
        let expected_flows = if expected_confidential {
            vec!["authorization_code", "client_credentials", "refresh_token"]
        } else {
            vec!["authorization_code", "refresh_token"]
        };
        let expected_flows = expected_flows.into_iter().collect::<HashSet<_>>();
        let redirect_uris = self
            .db
            .list_application_redirect_uris_with_consistency(
                application_id,
                Some(ReadConsistency::Strong),
            )
            .await?;
        if redirect_uris.len() > 1 {
            return Err(service_error(
                ServiceErrorKind::Conflict,
                "local application redirect URI manifest has more than one entry",
            ));
        }
        let expected_redirect_uris = redirect_uris
            .into_iter()
            .map(|redirect_uri| {
                let canonical = canonical_redirect_uri(&redirect_uri.redirect_uri)?;
                if canonical != redirect_uri.redirect_uri {
                    return Err(bad_request_error("stored redirect URI is not canonical"));
                }
                Ok(redirect_uri.redirect_uri)
            })
            .collect::<Result<Vec<_>>>()
            .map_err(|_| {
                service_error(
                    ServiceErrorKind::Conflict,
                    "local application redirect URI manifest is invalid",
                )
            })?;
        let actual_flows = client
            .flows_enabled
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        if client.id != client_id
            || client.name.as_deref() != Some(application.name.as_str())
            || client.confidential != expected_confidential
            || client.enabled != application.active
            || client.redirect_uris != expected_redirect_uris
            || actual_flows != expected_flows
            || (matches!(application.application_type, ApplicationType::Public)
                && client.challenges.as_deref() != Some(&["S256".to_owned()]))
        {
            return Err(service_error(
                ServiceErrorKind::Conflict,
                "Rauthy client configuration does not match the local application reservation",
            ));
        }
        self.db
            .mark_application_provisioning_ready(application_id, organization_id)
            .await
    }

    async fn application_redirect_uri_for_read(
        &self,
        application_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Option<String>> {
        let redirect_uris = self
            .db
            .list_application_redirect_uris_with_consistency(application_id, consistency)
            .await?;
        match redirect_uris.as_slice() {
            [] => Ok(None),
            [redirect_uri] => Ok(Some(redirect_uri.redirect_uri.clone())),
            _ => Err(service_error(
                ServiceErrorKind::Conflict,
                "local application redirect URI manifest has more than one entry",
            )),
        }
    }

    async fn fetch_rauthy_application_client(&self, url: &str) -> Result<reqwest::Response> {
        self.http_client
            .get(url)
            .header(
                "authorization",
                format!("API-Key {}", self.cfg.auth.rauthy.client_read_api_key),
            )
            .send()
            .await
            .map_err(|err| {
                service_error(
                    ServiceErrorKind::Unavailable,
                    format!("rauthy client fetch failed: {err}"),
                )
            })
    }

    fn rauthy_admin_url(&self, path: &str) -> String {
        format!("{}{}", self.rauthy_base_url().trim_end_matches('/'), path)
    }

    pub async fn list_application_actions_authorized(
        &self,
        user_id: &str,
        organization_id: &str,
        application_id: &str,
        read_consistency: Option<ReadConsistency>,
    ) -> Result<Vec<Action>> {
        self.get_application_authorized(
            user_id,
            organization_id,
            application_id,
            read_consistency.clone(),
        )
        .await?;
        self.db
            .list_application_actions_with_consistency(application_id, read_consistency)
            .await
    }

    pub async fn replace_application_actions_authorized(
        &self,
        user_id: &str,
        organization_id: &str,
        application_id: &str,
        action_ids: &[String],
    ) -> Result<Vec<Action>> {
        self.get_application_authorized(
            user_id,
            organization_id,
            application_id,
            Some(ReadConsistency::Strong),
        )
        .await?;
        if action_ids.len() > MAX_APPLICATION_ACTION_IDS_PER_REQUEST {
            return Err(bad_request_error(format!(
                "actionIds must contain at most {MAX_APPLICATION_ACTION_IDS_PER_REQUEST} items"
            )));
        }
        let (lease_token, _) = self
            .acquire_application_security_mutation(organization_id, application_id)
            .await?;
        let result = self
            .db
            .replace_application_actions_with_security_lease(
                application_id,
                action_ids,
                &lease_token,
            )
            .await;
        let _ = self
            .db
            .release_application_security_mutation(application_id, &lease_token)
            .await;
        result.map_err(map_application_security_mutation_error)
    }

    pub async fn list_application_grants_authorized(
        &self,
        user_id: &str,
        organization_id: &str,
        read_consistency: Option<ReadConsistency>,
    ) -> Result<Vec<ApplicationGrant>> {
        self.authorize_organization_action(
            user_id,
            organization_id,
            organization_id,
            "organization.application.manage",
            "manage organization application grants",
            read_consistency.clone(),
        )
        .await?;
        self.db
            .list_application_grants_for_organization_with_consistency(
                organization_id,
                read_consistency,
            )
            .await
    }

    pub async fn create_application_grant_authorized(
        &self,
        user_id: &str,
        organization_id: &str,
        input: CreateApplicationGrantInput,
    ) -> Result<ApplicationGrant> {
        self.authorize_organization_action(
            user_id,
            organization_id,
            organization_id,
            "organization.application.manage",
            "manage organization application grants",
            Some(ReadConsistency::Strong),
        )
        .await?;
        let target_user_id = require_non_empty("userId", &input.user_id)?;
        if !self
            .db
            .has_organization_membership_with_consistency(
                &target_user_id,
                organization_id,
                Some(ReadConsistency::Strong),
            )
            .await?
        {
            return Err(forbidden_error(format!(
                "forbidden: user {target_user_id} is not a member of organization {organization_id}"
            )));
        }
        let application_id = require_non_empty("applicationId", &input.application_id)?;
        let application = self
            .db
            .get_application_with_consistency(&application_id, Some(ReadConsistency::Strong))
            .await?;
        if application.organization_id != organization_id {
            return Err(not_found_error(format!(
                "application not found: {} in organization {}",
                application_id, organization_id
            )));
        }
        let now = Utc::now().to_rfc3339();
        let grant = ApplicationGrant {
            id: require_non_empty("id", &input.id)?,
            user_id: target_user_id,
            application_id,
            organization_id: organization_id.to_owned(),
            status: ApplicationGrantStatus::Active,
            consented_at: now.clone(),
            revoked_at: None,
            revoked_by_user_id: None,
            created_at: now.clone(),
            updated_at: now,
        };
        self.db
            .validate_application_grant_action_subset(&grant.application_id, &input.action_ids)
            .await?;
        let grant_id = self.db.insert_application_grant(&grant).await?;
        self.db
            .replace_application_grant_actions(&grant_id, &input.action_ids)
            .await?;
        self.db
            .get_application_grant_with_consistency(&grant_id, Some(ReadConsistency::Strong))
            .await
    }

    pub async fn replace_application_grant_actions_authorized(
        &self,
        user_id: &str,
        organization_id: &str,
        grant_id: &str,
        action_ids: &[String],
    ) -> Result<Vec<Action>> {
        self.authorize_organization_action(
            user_id,
            organization_id,
            organization_id,
            "organization.application.manage",
            "manage organization application grants",
            Some(ReadConsistency::Strong),
        )
        .await?;
        if action_ids.len() > MAX_ACTION_IDS_PER_REQUEST {
            return Err(bad_request_error(format!(
                "actionIds must contain at most {MAX_ACTION_IDS_PER_REQUEST} items"
            )));
        }
        let grant = self
            .db
            .get_application_grant_with_consistency(grant_id, Some(ReadConsistency::Strong))
            .await?;
        ensure_application_grant_org_scope(&grant, organization_id)?;
        self.db
            .replace_application_grant_actions(grant_id, action_ids)
            .await
    }

    pub async fn revoke_application_grant_authorized(
        &self,
        user_id: &str,
        organization_id: &str,
        grant_id: &str,
    ) -> Result<ApplicationGrant> {
        self.authorize_organization_action(
            user_id,
            organization_id,
            organization_id,
            "organization.application.manage",
            "manage organization application grants",
            Some(ReadConsistency::Strong),
        )
        .await?;
        let grant = self
            .db
            .get_application_grant_with_consistency(grant_id, Some(ReadConsistency::Strong))
            .await?;
        ensure_application_grant_org_scope(&grant, organization_id)?;
        self.db.revoke_application_grant(grant_id, user_id).await
    }

    pub async fn get_application_consent(
        &self,
        user_id: &str,
        organization_id: &str,
        application_id: &str,
    ) -> Result<Option<(ApplicationGrant, Vec<Action>)>> {
        let application = self
            .db
            .get_application_with_consistency(application_id, Some(ReadConsistency::Strong))
            .await?;
        if application.organization_id != organization_id {
            return Err(not_found_error(format!(
                "application not found: {application_id} in organization {organization_id}"
            )));
        }
        let Some(grant) = self
            .db
            .find_active_application_consent_with_consistency(
                user_id,
                application_id,
                organization_id,
                Some(ReadConsistency::Strong),
            )
            .await?
        else {
            return Ok(None);
        };
        let actions = self
            .db
            .list_application_grant_actions_with_consistency(
                &grant.id,
                Some(ReadConsistency::Strong),
            )
            .await?;
        Ok(Some((grant, actions)))
    }

    pub async fn replace_application_consent(
        &self,
        user_id: &str,
        organization_id: &str,
        application_id: &str,
        action_ids: &[String],
    ) -> Result<(ApplicationGrant, Vec<Action>)> {
        if action_ids.is_empty() || action_ids.len() > MAX_ACTION_IDS_PER_REQUEST {
            return Err(bad_request_error(format!(
                "actionIds must contain between 1 and {MAX_ACTION_IDS_PER_REQUEST} items"
            )));
        }
        let mut normalized_action_ids = Vec::with_capacity(action_ids.len());
        let mut seen_action_ids = HashSet::with_capacity(action_ids.len());
        for action_id in action_ids {
            let action_id = require_non_empty("actionIds[]", action_id)?;
            if seen_action_ids.insert(action_id.clone()) {
                normalized_action_ids.push(action_id);
            }
        }
        let application = self
            .db
            .get_application_with_consistency(application_id, Some(ReadConsistency::Strong))
            .await?;
        if application.organization_id != organization_id {
            return Err(not_found_error(format!(
                "application not found: {application_id} in organization {organization_id}"
            )));
        }
        if !application.active {
            return Err(forbidden_error("forbidden: application is inactive"));
        }
        if application.consent_policy != ApplicationConsentPolicy::Explicit {
            return Err(bad_request_error(
                "explicit consent cannot be written for an auto-approve application",
            ));
        }
        if !self
            .db
            .has_active_organization_membership_with_consistency(
                user_id,
                organization_id,
                Some(ReadConsistency::Strong),
            )
            .await?
        {
            return Err(forbidden_error(format!(
                "forbidden: user {user_id} is not an active member of organization {organization_id}"
            )));
        }
        let now = Utc::now().to_rfc3339();
        let grant = ApplicationGrant {
            id: uuid::Uuid::new_v4().simple().to_string(),
            user_id: user_id.to_owned(),
            application_id: application_id.to_owned(),
            organization_id: organization_id.to_owned(),
            status: ApplicationGrantStatus::Active,
            consented_at: now.clone(),
            revoked_at: None,
            revoked_by_user_id: None,
            created_at: now.clone(),
            updated_at: now,
        };
        let grant = self
            .db
            .replace_active_application_consent(&grant, &normalized_action_ids)
            .await
            .map_err(map_application_consent_write_error)?;
        let actions = self
            .db
            .list_application_grant_actions_with_consistency(
                &grant.id,
                Some(ReadConsistency::Strong),
            )
            .await?;
        if actions.len() != normalized_action_ids.len() {
            return Err(service_error(
                ServiceErrorKind::Conflict,
                "application actions changed while consent was being written",
            ));
        }
        Ok((grant, actions))
    }

    pub async fn revoke_application_consent(
        &self,
        user_id: &str,
        organization_id: &str,
        application_id: &str,
    ) -> Result<()> {
        let application = self
            .db
            .get_application_with_consistency(application_id, Some(ReadConsistency::Strong))
            .await?;
        if application.organization_id != organization_id {
            return Err(not_found_error(format!(
                "application not found: {application_id} in organization {organization_id}"
            )));
        }
        self.db
            .revoke_active_application_consent(user_id, application_id, organization_id)
            .await
    }

    pub async fn set_application_trust_admin(
        &self,
        organization_id: &str,
        application_id: &str,
        trusted: bool,
    ) -> Result<Application> {
        self.db
            .set_application_trust(organization_id, application_id, trusted)
            .await
            .map_err(map_application_trust_mutation_error)
    }

    pub async fn list_groups_authorized(
        &self,
        user_id: &str,
        organization_id: &str,
        read_consistency: Option<ReadConsistency>,
    ) -> Result<Vec<Group>> {
        self.authorize_organization_action(
            user_id,
            organization_id,
            organization_id,
            "organization.group.manage",
            "manage organization groups",
            read_consistency.clone(),
        )
        .await?;
        self.db
            .list_groups_with_consistency(organization_id, read_consistency)
            .await
    }

    pub async fn get_group_authorized(
        &self,
        user_id: &str,
        organization_id: &str,
        group_id: &str,
        read_consistency: Option<ReadConsistency>,
    ) -> Result<Group> {
        self.authorize_organization_action(
            user_id,
            organization_id,
            organization_id,
            "organization.group.manage",
            "manage organization groups",
            read_consistency.clone(),
        )
        .await?;
        self.db
            .find_group_with_consistency(group_id, organization_id, read_consistency)
            .await?
            .ok_or_else(|| not_found_error(format!("group not found: {group_id}")))
    }

    pub async fn create_group_authorized(
        &self,
        user_id: &str,
        organization_id: &str,
        input: UpsertGroupInput,
    ) -> Result<Group> {
        self.authorize_organization_action(
            user_id,
            organization_id,
            organization_id,
            "organization.group.manage",
            "manage organization groups",
            Some(ReadConsistency::Strong),
        )
        .await?;
        let group = Group {
            id: require_non_empty("id", &input.id)?,
            organization_id: organization_id.to_owned(),
            name: require_non_empty("name", &input.name)?,
            description: normalize_optional_text(input.description),
        };
        self.db.insert_group(&group).await?;
        Ok(group)
    }

    pub async fn update_group_authorized(
        &self,
        user_id: &str,
        organization_id: &str,
        group_id: &str,
        input: UpsertGroupInput,
    ) -> Result<Group> {
        let existing = self
            .get_group_authorized(
                user_id,
                organization_id,
                group_id,
                Some(ReadConsistency::Strong),
            )
            .await?;
        let group = Group {
            id: existing.id,
            organization_id: existing.organization_id,
            name: require_non_empty("name", &input.name)?,
            description: normalize_optional_text(input.description),
        };
        self.db.update_group(&group).await?;
        Ok(group)
    }

    pub async fn delete_group_authorized(
        &self,
        user_id: &str,
        organization_id: &str,
        group_id: &str,
    ) -> Result<()> {
        self.get_group_authorized(
            user_id,
            organization_id,
            group_id,
            Some(ReadConsistency::Strong),
        )
        .await?;
        if !self
            .db
            .delete_group_if_unbound(group_id, organization_id)
            .await?
        {
            return Err(service_error(
                ServiceErrorKind::Conflict,
                "group cannot be deleted while role bindings reference it",
            ));
        }
        Ok(())
    }

    pub async fn list_group_members_authorized(
        &self,
        user_id: &str,
        organization_id: &str,
        group_id: &str,
        read_consistency: Option<ReadConsistency>,
    ) -> Result<Vec<GroupMember>> {
        self.get_group_authorized(user_id, organization_id, group_id, read_consistency.clone())
            .await?;
        self.db
            .list_group_members_with_consistency(group_id, organization_id, read_consistency)
            .await
    }

    pub async fn add_group_member_authorized(
        &self,
        user_id: &str,
        organization_id: &str,
        group_id: &str,
        target_user_id: &str,
    ) -> Result<GroupMember> {
        self.get_group_authorized(
            user_id,
            organization_id,
            group_id,
            Some(ReadConsistency::Strong),
        )
        .await?;
        let target_user_id = require_non_empty("userId", target_user_id)?;
        self.db
            .add_group_member_if_active(group_id, organization_id, &target_user_id)
            .await?
            .ok_or_else(|| {
                forbidden_error(format!(
                    "forbidden: user {target_user_id} is not an active member of organization {organization_id}"
                ))
            })
    }

    pub async fn remove_group_member_authorized(
        &self,
        user_id: &str,
        organization_id: &str,
        group_id: &str,
        target_user_id: &str,
    ) -> Result<()> {
        self.get_group_authorized(
            user_id,
            organization_id,
            group_id,
            Some(ReadConsistency::Strong),
        )
        .await?;
        self.db
            .remove_group_member(group_id, organization_id, target_user_id)
            .await
    }

    pub async fn enroll_organization_member_authorized(
        &self,
        user_id: &str,
        organization_id: &str,
        target_user_id: &str,
    ) -> Result<OrganizationMembership> {
        self.authorize_organization_action(
            user_id,
            organization_id,
            organization_id,
            "organization.member.manage",
            "manage organization members",
            Some(ReadConsistency::Strong),
        )
        .await?;
        let target_user_id = require_non_empty("userId", target_user_id)?;
        self.db
            .enroll_active_member(organization_id, &target_user_id)
            .await
            .map_err(|error| {
                match error.downcast_ref::<OrganizationMembershipWriteError>() {
                    Some(OrganizationMembershipWriteError::TargetUserNotFound) => {
                        not_found_error(format!("user not found: {target_user_id}"))
                    }
                    Some(OrganizationMembershipWriteError::TargetUserInactive) => {
                        forbidden_error(format!("forbidden: user {target_user_id} is inactive"))
                    }
                    Some(OrganizationMembershipWriteError::OrganizationInactive) => {
                        forbidden_error(format!("forbidden: organization {organization_id} is inactive"))
                    }
                    Some(OrganizationMembershipWriteError::ExistingNonMemberRole) => service_error(
                        ServiceErrorKind::Conflict,
                        format!(
                            "organization membership for user {target_user_id} cannot be changed to member"
                        ),
                    ),
                    Some(OrganizationMembershipWriteError::PreconditionsChanged) => service_error(
                        ServiceErrorKind::Conflict,
                        "organization membership enrollment preconditions changed",
                    ),
                    None => error,
                }
            })
    }

    pub async fn list_agreements_authorized(
        &self,
        user_id: &str,
        organization_id: &str,
        read_consistency: Option<ReadConsistency>,
    ) -> Result<Vec<Agreement>> {
        self.authorize_organization_action(
            user_id,
            organization_id,
            organization_id,
            "organization.permission.manage",
            "manage organization agreements",
            read_consistency.clone(),
        )
        .await?;
        self.db
            .list_agreements_for_organization_with_consistency(organization_id, read_consistency)
            .await
    }

    pub async fn get_agreement_authorized(
        &self,
        user_id: &str,
        organization_id: &str,
        agreement_id: &str,
        read_consistency: Option<ReadConsistency>,
    ) -> Result<Agreement> {
        self.authorize_organization_action(
            user_id,
            organization_id,
            organization_id,
            "organization.permission.manage",
            "manage organization agreements",
            read_consistency.clone(),
        )
        .await?;
        let agreement = self
            .db
            .get_agreement_with_consistency(agreement_id, read_consistency)
            .await?;
        ensure_agreement_org_scope(&agreement, organization_id)?;
        Ok(agreement)
    }

    pub async fn upsert_agreement_authorized(
        &self,
        user_id: &str,
        organization_id: &str,
        input: UpsertAgreementInput,
    ) -> Result<Agreement> {
        self.authorize_organization_action(
            user_id,
            organization_id,
            organization_id,
            "organization.permission.manage",
            "manage organization agreements",
            Some(ReadConsistency::Strong),
        )
        .await?;
        let agreement = Agreement {
            id: require_non_empty("id", &input.id)?,
            agreement_type: input.agreement_type,
            grantor_organization_id: require_non_empty(
                "grantorOrganizationId",
                &input.grantor_organization_id,
            )?,
            grantee_organization_id: require_non_empty(
                "granteeOrganizationId",
                &input.grantee_organization_id,
            )?,
            status: input.status,
            title: require_non_empty("title", &input.title)?,
            reference_no: normalize_optional_text(input.reference_no),
            starts_at: normalize_optional_text(input.starts_at),
            ends_at: normalize_optional_text(input.ends_at),
        };
        ensure_agreement_org_scope(&agreement, organization_id)?;
        self.db
            .get_organization(&agreement.grantor_organization_id)
            .await?;
        self.db
            .get_organization(&agreement.grantee_organization_id)
            .await?;
        self.db.upsert_agreement(&agreement).await?;
        Ok(agreement)
    }

    pub async fn create_agreement_authorized(
        &self,
        user_id: &str,
        organization_id: &str,
        input: UpsertAgreementInput,
    ) -> Result<Agreement> {
        if self
            .db
            .find_agreement_by_id_with_consistency(&input.id, Some(ReadConsistency::Strong))
            .await?
            .is_some()
        {
            return Err(service_error(
                ServiceErrorKind::Conflict,
                format!("agreement already exists: {}", input.id),
            ));
        }
        if input.grantor_organization_id != organization_id {
            return Err(forbidden_error(format!(
                "forbidden: only grantor organization {} can create agreement {}",
                input.grantor_organization_id, input.id
            )));
        }
        self.upsert_agreement_authorized(user_id, organization_id, input)
            .await
    }

    pub async fn update_agreement_authorized(
        &self,
        user_id: &str,
        organization_id: &str,
        agreement_id: &str,
        input: UpsertAgreementInput,
    ) -> Result<Agreement> {
        let existing = self
            .get_agreement_authorized(
                user_id,
                organization_id,
                agreement_id,
                Some(ReadConsistency::Strong),
            )
            .await?;
        if input.id != agreement_id {
            return Err(bad_request_error("agreement id must match path id"));
        }
        if input.grantor_organization_id != existing.grantor_organization_id
            || input.grantee_organization_id != existing.grantee_organization_id
        {
            return Err(bad_request_error(
                "agreement grantorOrganizationId and granteeOrganizationId cannot be changed",
            ));
        }
        if existing.grantor_organization_id != organization_id
            && (input.status != existing.status
                || input.starts_at != existing.starts_at
                || input.ends_at != existing.ends_at)
        {
            return Err(forbidden_error(format!(
                "forbidden: only grantor organization {} can change agreement authorization terms",
                existing.grantor_organization_id
            )));
        }
        self.upsert_agreement_authorized(user_id, organization_id, input)
            .await
    }

    pub async fn list_roles_authorized(
        &self,
        user_id: &str,
        organization_id: &str,
        read_consistency: Option<ReadConsistency>,
    ) -> Result<Vec<Role>> {
        self.authorize_organization_action(
            user_id,
            organization_id,
            organization_id,
            "organization.permission.manage",
            "manage organization roles",
            read_consistency.clone(),
        )
        .await?;
        self.db
            .list_roles_for_organization_with_consistency(organization_id, read_consistency)
            .await
    }

    pub async fn create_role_authorized(
        &self,
        user_id: &str,
        organization_id: &str,
        input: CreateRoleInput,
    ) -> Result<Role> {
        self.authorize_organization_action(
            user_id,
            organization_id,
            organization_id,
            "organization.permission.manage",
            "manage organization roles",
            Some(ReadConsistency::Strong),
        )
        .await?;
        let role = Role {
            id: require_non_empty("id", &input.id)?,
            organization_id: Some(organization_id.to_owned()),
            code: require_non_empty("code", &input.code)?,
            name: require_non_empty("name", &input.name)?,
            description: normalize_optional_text(input.description),
        };
        self.db.insert_role(&role).await?;
        Ok(role)
    }

    pub async fn replace_role_actions_authorized(
        &self,
        user_id: &str,
        organization_id: &str,
        role_id: &str,
        action_ids: &[String],
    ) -> Result<Vec<Action>> {
        self.authorize_organization_action(
            user_id,
            organization_id,
            organization_id,
            "organization.permission.manage",
            "manage organization roles",
            Some(ReadConsistency::Strong),
        )
        .await?;
        if action_ids.len() > MAX_ACTION_IDS_PER_REQUEST {
            return Err(bad_request_error(format!(
                "actionIds must contain at most {MAX_ACTION_IDS_PER_REQUEST} items"
            )));
        }
        let role = self
            .db
            .get_role_with_consistency(role_id, Some(ReadConsistency::Strong))
            .await?;
        if role.organization_id.as_deref() != Some(organization_id) {
            return Err(not_found_error(format!(
                "role not found: {} in organization {}",
                role_id, organization_id
            )));
        }
        self.db.replace_role_actions(role_id, action_ids).await
    }

    pub async fn list_role_bindings_authorized(
        &self,
        user_id: &str,
        organization_id: &str,
        read_consistency: Option<ReadConsistency>,
    ) -> Result<Vec<RoleBinding>> {
        self.authorize_organization_action(
            user_id,
            organization_id,
            organization_id,
            "organization.permission.manage",
            "manage organization role bindings",
            read_consistency.clone(),
        )
        .await?;
        self.db
            .list_role_bindings_for_organization_with_consistency(organization_id, read_consistency)
            .await
    }

    pub async fn create_role_binding_authorized(
        &self,
        user_id: &str,
        organization_id: &str,
        input: CreateRoleBindingInput,
    ) -> Result<RoleBinding> {
        self.authorize_organization_action(
            user_id,
            organization_id,
            organization_id,
            "organization.permission.manage",
            "manage organization role bindings",
            Some(ReadConsistency::Strong),
        )
        .await?;
        if input.subject_organization_id != organization_id {
            return Err(forbidden_error(format!(
                "forbidden: role binding subject organization {} does not match path organization {}",
                input.subject_organization_id, organization_id
            )));
        }
        let now = Utc::now().to_rfc3339();
        let binding = RoleBinding {
            id: require_non_empty("id", &input.id)?,
            subject_type: input.subject_type,
            subject_id: require_non_empty("subjectId", &input.subject_id)?,
            subject_organization_id: organization_id.to_owned(),
            provider_id: require_non_empty("providerId", &input.provider_id)?,
            resource_type: require_non_empty("resourceType", &input.resource_type)?,
            resource_id: require_non_empty("resourceId", &input.resource_id)?,
            role_id: require_non_empty("roleId", &input.role_id)?,
            agreement_id: normalize_optional_text(input.agreement_id),
            state: input.state,
            created_by_user_id: user_id.to_owned(),
            created_at: now.clone(),
            updated_at: now,
        };
        self.db.insert_role_binding(&binding).await?;
        Ok(binding)
    }

    pub async fn delete_role_binding_authorized(
        &self,
        user_id: &str,
        organization_id: &str,
        binding_id: &str,
    ) -> Result<()> {
        self.authorize_organization_action(
            user_id,
            organization_id,
            organization_id,
            "organization.permission.manage",
            "manage organization role bindings",
            Some(ReadConsistency::Strong),
        )
        .await?;
        let binding = self
            .db
            .get_role_binding_with_consistency(binding_id, Some(ReadConsistency::Strong))
            .await?;
        if binding.subject_organization_id != organization_id {
            return Err(not_found_error(format!(
                "role binding not found: {} in organization {}",
                binding_id, organization_id
            )));
        }
        self.db.delete_role_binding(binding_id).await
    }

    pub async fn check_authorization(
        &self,
        subject: &AuthorizationSubject,
        action: &str,
        resource: &AuthorizationResource,
        read_consistency: Option<ReadConsistency>,
    ) -> Result<AuthorizationDecision> {
        self.check_authorizations(
            &[AuthorizationCheck {
                subject: subject.clone(),
                action: action.to_owned(),
                resource: resource.clone(),
            }],
            read_consistency,
        )
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| service_error(ServiceErrorKind::Unavailable, "authorization check failed"))
    }

    pub async fn check_authorizations(
        &self,
        checks: &[AuthorizationCheck],
        read_consistency: Option<ReadConsistency>,
    ) -> Result<Vec<AuthorizationDecision>> {
        self.evaluate_authorizations(checks, read_consistency, None)
            .await
    }

    async fn evaluate_authorizations(
        &self,
        checks: &[AuthorizationCheck],
        read_consistency: Option<ReadConsistency>,
        application_id: Option<&str>,
    ) -> Result<Vec<AuthorizationDecision>> {
        let normalized = checks
            .iter()
            .map(normalize_authorization_check)
            .collect::<Result<Vec<_>>>()?;
        let db_checks = normalized
            .iter()
            .map(|check| UserActionCheck {
                user_id: &check.subject_id,
                organization_id: &check.organization_id,
                provider_id: &check.provider_id,
                resource_type: &check.resource_type,
                resource_id: &check.resource_id,
                action_id: &check.action_id,
                application_id,
                consistency: read_consistency.clone(),
            })
            .collect::<Vec<_>>();
        self.db
            .evaluate_user_actions(&db_checks)
            .await?
            .into_iter()
            .map(|decision| {
                Ok(AuthorizationDecision {
                    allowed: decision.allowed,
                    reason: if decision.allowed {
                        if application_id.is_none() {
                            "role_binding"
                        } else if decision.trusted_application {
                            "trusted_application"
                        } else {
                            "explicit_consent"
                        }
                    } else if decision.rbac_allowed && application_id.is_some() {
                        "application_consent_required"
                    } else {
                        "no_matching_binding"
                    }
                    .to_owned(),
                })
            })
            .collect()
    }

    pub async fn check_application_authorizations(
        &self,
        bearer_token: Option<&str>,
        organization_id: &str,
        checks: &[ApplicationAuthorizationCheck],
    ) -> Result<Vec<AuthorizationDecision>> {
        let context = self
            .resolve_application_authorization_context(bearer_token, organization_id)
            .await?;
        let exact_checks = checks
            .iter()
            .map(|check| AuthorizationCheck {
                subject: AuthorizationSubject {
                    subject_type: "user".to_owned(),
                    id: context.user_id.clone(),
                    organization_id: context.organization_id.clone(),
                },
                action: check.action.clone(),
                resource: check.resource.clone(),
            })
            .collect::<Vec<_>>();
        self.evaluate_authorizations(
            &exact_checks,
            Some(ReadConsistency::Strong),
            Some(&context.application_id),
        )
        .await
    }

    pub async fn check_exposure_read_authorization(
        &self,
        bearer_token: Option<&str>,
        organization_id: &str,
        exposure_id: &str,
        resource_id: &str,
    ) -> Result<ExposureReadAuthorizationDecision> {
        let context = self
            .resolve_application_authorization_context(bearer_token, organization_id)
            .await?;
        let decision = self
            .evaluate_authorizations(
                &[AuthorizationCheck {
                    subject: AuthorizationSubject {
                        subject_type: "user".to_owned(),
                        id: context.user_id.clone(),
                        organization_id: context.organization_id.clone(),
                    },
                    action: EXPOSURE_READ_ACTION_ID.to_owned(),
                    resource: AuthorizationResource {
                        provider_id: EXPOSURE_READ_PROVIDER_ID.to_owned(),
                        resource_type: exposure_id.to_owned(),
                        id: resource_id.to_owned(),
                    },
                }],
                Some(ReadConsistency::Strong),
                Some(&context.application_id),
            )
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| {
                service_error(
                    ServiceErrorKind::Unavailable,
                    "exposure read authorization check failed",
                )
            })?;
        let issued_at = Utc::now();
        let expires_at = issued_at + chrono::Duration::seconds(EXPOSURE_DECISION_TTL_SECONDS);
        Ok(ExposureReadAuthorizationDecision {
            allowed: decision.allowed,
            reason: decision.reason,
            principal_digest: exposure_principal_digest(
                &context.user_id,
                &context.organization_id,
                &context.application_id,
            ),
            issued_at: issued_at.to_rfc3339(),
            expires_at: expires_at.to_rfc3339(),
        })
    }

    pub async fn check_patch_data_plant_read_authorization(
        &self,
        bearer_token: Option<&str>,
        organization_id: &str,
        plant_id: &str,
    ) -> Result<ExposureReadAuthorizationDecision> {
        let context = self
            .resolve_application_authorization_context(bearer_token, organization_id)
            .await?;
        let decision = self
            .evaluate_authorizations(
                &[AuthorizationCheck {
                    subject: AuthorizationSubject {
                        subject_type: "user".to_owned(),
                        id: context.user_id.clone(),
                        organization_id: context.organization_id.clone(),
                    },
                    action: PATCH_DATA_PLANT_READ_ACTION_ID.to_owned(),
                    resource: AuthorizationResource {
                        provider_id: PATCH_DATA_PLANT_READ_PROVIDER_ID.to_owned(),
                        resource_type: PATCH_DATA_PLANT_RESOURCE_TYPE.to_owned(),
                        id: plant_id.to_owned(),
                    },
                }],
                Some(ReadConsistency::Strong),
                Some(&context.application_id),
            )
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| {
                service_error(
                    ServiceErrorKind::Unavailable,
                    "Patch Data plant read authorization check failed",
                )
            })?;
        let issued_at = Utc::now();
        let expires_at = issued_at + chrono::Duration::seconds(EXPOSURE_DECISION_TTL_SECONDS);
        Ok(ExposureReadAuthorizationDecision {
            allowed: decision.allowed,
            reason: decision.reason,
            principal_digest: exposure_principal_digest(
                &context.user_id,
                &context.organization_id,
                &context.application_id,
            ),
            issued_at: issued_at.to_rfc3339(),
            expires_at: expires_at.to_rfc3339(),
        })
    }

    async fn resolve_application_authorization_context(
        &self,
        bearer_token: Option<&str>,
        organization_id: &str,
    ) -> Result<ApplicationAuthorizationContext> {
        let organization_id = require_non_empty("organizationId", organization_id)?;
        let identity = self
            .auth
            .resolve_external_identity_for_organization(
                Some(&self.db),
                bearer_token,
                Some(&organization_id),
                Some(ReadConsistency::Strong),
            )
            .await?;
        let user = match self.cfg.auth.provider_mode {
            // Patch Data identities are provider-qualified.  In particular,
            // never reconcile a verified Patch Data principal by email or a
            // bare local id: that would inherit another principal's access.
            AuthProviderMode::PatchData => self
                .db
                .materialize_external_identity(ExternalIdentityMaterialization {
                    provider: &identity.provider,
                    external_user_id: &identity.external_user_id,
                    email: identity.email.as_deref(),
                    display_name: &identity.display_name,
                })
                .await
                .map_err(map_strong_identity_sync_error)?,
            // Rauthy retains its established legacy reconciliation behavior.
            AuthProviderMode::Rauthy => self
                .db
                .resolve_or_create_user_identity_with_consistency(
                    &identity.provider,
                    &identity.external_user_id,
                    identity.email.as_deref(),
                    &identity.display_name,
                    Some(ReadConsistency::Strong),
                )
                .await
                .map_err(map_strong_identity_sync_error)?,
        };
        if !user.active {
            return Err(forbidden_error("forbidden: local user is inactive"));
        }
        let application = match self.cfg.auth.provider_mode {
            AuthProviderMode::Rauthy => self
                .db
                .find_active_application_by_client_for_organization(&identity.client_id, &organization_id)
                .await?
                .ok_or_else(|| forbidden_error(format!(
                    "forbidden: token client {} is not an active application in organization {}",
                    identity.client_id, organization_id
                )))?,
            AuthProviderMode::PatchData => {
                let configured_id = &self.cfg.auth.patch_data.as_ref().expect("Patch Data config validated").exposure_application_id;
                let application = self.db.get_application_with_consistency(configured_id, Some(ReadConsistency::Strong)).await
                    .map_err(|_| forbidden_error("forbidden: configured Patch Data exposure application is not ready"))?;
                if application.organization_id != organization_id
                    || !application.active
                    || application.provisioning_status != ApplicationProvisioningStatus::Ready
                {
                    return Err(forbidden_error("forbidden: configured Patch Data exposure application is not active for this organization"));
                }
                application
            }
        };
        Ok(ApplicationAuthorizationContext {
            user_id: user.id,
            organization_id,
            application_id: application.id,
        })
    }

    pub async fn deactivate_organization(
        &self,
        user_id: &str,
        organization_id: &str,
        target_organization_id: &str,
    ) -> Result<Organization> {
        self.authorize_organization_action(
            user_id,
            organization_id,
            target_organization_id,
            "organization.permission.manage",
            &format!("deactivate organization {}", target_organization_id),
            Some(ReadConsistency::Strong),
        )
        .await?;
        self.db
            .deactivate_organization(target_organization_id)
            .await
    }

    pub async fn reactivate_organization(
        &self,
        user_id: &str,
        organization_id: &str,
        target_organization_id: &str,
    ) -> Result<Organization> {
        if organization_id != target_organization_id
            || !self
                .db
                .has_user_organization_reactivation_action_with_consistency(
                    user_id,
                    target_organization_id,
                    Some(ReadConsistency::Strong),
                )
                .await?
        {
            return Err(forbidden_error(format!(
                "forbidden: user {user_id} cannot reactivate organization {target_organization_id}"
            )));
        }
        self.db
            .reactivate_organization(target_organization_id)
            .await
    }

    pub async fn trigger_backup(&self) -> Result<()> {
        self.db.trigger_backup().await
    }

    pub async fn save_role_binding(&self, binding: &RoleBinding) -> Result<()> {
        self.db.save_role_binding(binding).await
    }

    pub async fn list_role_bindings_for_subject(
        &self,
        subject_type: RoleBindingSubjectType,
        subject_id: &str,
        subject_organization_id: Option<&str>,
    ) -> Result<Vec<RoleBinding>> {
        self.db
            .list_role_bindings_for_subject(subject_type, subject_id, subject_organization_id)
            .await
    }

    pub async fn list_role_bindings_for_resource(
        &self,
        provider_id: &str,
        resource_type: &str,
        resource_id: &str,
    ) -> Result<Vec<RoleBinding>> {
        self.db
            .list_role_bindings_for_resource(provider_id, resource_type, resource_id)
            .await
    }

    pub async fn list_backups_local(&self) -> Result<Vec<BackupInfo>> {
        map_backups(self.db.list_backup_local().await?)
    }

    pub async fn list_backups_s3(&self) -> Result<Vec<BackupInfo>> {
        map_backups(self.db.list_backup_s3().await?)
    }

    async fn authorize_organization_action(
        &self,
        user_id: &str,
        organization_id: &str,
        target_organization_id: &str,
        action_id: &str,
        action_label: &str,
        read_consistency: Option<ReadConsistency>,
    ) -> Result<()> {
        let allowed = self
            .db
            .has_user_organization_action_with_consistency(
                user_id,
                organization_id,
                target_organization_id,
                action_id,
                read_consistency,
            )
            .await?;
        if !allowed {
            return Err(forbidden_error(format!(
                "forbidden: user {} in organization {} cannot {}",
                user_id, organization_id, action_label
            )));
        }
        Ok(())
    }
}

fn map_backups(items: Vec<BackupEntry>) -> Result<Vec<BackupInfo>> {
    Ok(items
        .into_iter()
        .map(|item| BackupInfo {
            name: item.name,
            last_modified: item.last_modified,
            size: item.size,
        })
        .collect())
}

struct NormalizedAuthorizationCheck {
    subject_id: String,
    organization_id: String,
    provider_id: String,
    resource_type: String,
    resource_id: String,
    action_id: String,
}

fn normalize_authorization_check(
    check: &AuthorizationCheck,
) -> Result<NormalizedAuthorizationCheck> {
    if check.subject.subject_type != "user" {
        return Err(bad_request_error("only user authz subjects are supported"));
    }
    Ok(NormalizedAuthorizationCheck {
        subject_id: require_non_empty("subject.id", &check.subject.id)?,
        organization_id: require_non_empty("subject.orgId", &check.subject.organization_id)?,
        provider_id: require_non_empty("resource.provider", &check.resource.provider_id)?,
        resource_type: require_non_empty("resource.type", &check.resource.resource_type)?,
        resource_id: require_non_empty("resource.id", &check.resource.id)?,
        action_id: require_non_empty("action", &check.action)?,
    })
}

fn normalize_active_organization(
    requested_organization_id: Option<&str>,
    identity: &ExternalIdentity,
    organizations: &[Organization],
) -> Result<Option<String>> {
    let requested = requested_organization_id
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let Some(requested) = requested else {
        return Ok(None);
    };
    if organizations.iter().any(|org| org.id == requested) {
        return Ok(Some(requested.to_owned()));
    }
    Err(forbidden_error(format!(
        "forbidden: rauthy user {} is not an active member of organization {}",
        identity.external_user_id, requested
    )))
}

fn require_non_empty(field_name: &str, value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(bad_request_error(format!("{field_name} must not be empty")));
    }
    Ok(trimmed.to_owned())
}

fn exposure_principal_digest(user_id: &str, organization_id: &str, application_id: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(EXPOSURE_PRINCIPAL_DIGEST_DOMAIN);
    for value in [user_id, organization_id, application_id] {
        digest.update((value.len() as u64).to_be_bytes());
        digest.update(value.as_bytes());
    }
    hex::encode(digest.finalize())
}

fn normalize_action_id(provider_id: &str, code: &str, provided_id: &str) -> Result<String> {
    let provided_id = require_non_empty("action.id", provided_id)?;
    let normalized = action_id_for_provider(provider_id, code);
    if provided_id == code || provided_id == normalized {
        return Ok(normalized);
    }
    Err(bad_request_error(format!(
        "action.id must match normalized action id {normalized}"
    )))
}

fn action_id_for_provider(provider_id: &str, code: &str) -> String {
    if provider_id == "grant" || code.starts_with(&format!("{provider_id}.")) {
        code.to_owned()
    } else {
        format!("{provider_id}.{code}")
    }
}

fn normalize_optional_text(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        }
    })
}

fn rauthy_client_id(application: &Application) -> Result<&str> {
    application
        .rauthy_client_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| bad_request_error("application has no rauthyClientId"))
}

fn ensure_agreement_org_scope(agreement: &Agreement, organization_id: &str) -> Result<()> {
    if agreement.grantor_organization_id == organization_id
        || agreement.grantee_organization_id == organization_id
    {
        return Ok(());
    }
    Err(not_found_error(format!(
        "agreement not found: {} in organization {}",
        agreement.id, organization_id
    )))
}

fn ensure_application_grant_org_scope(
    grant: &ApplicationGrant,
    organization_id: &str,
) -> Result<()> {
    if grant.organization_id == organization_id {
        return Ok(());
    }
    Err(not_found_error(format!(
        "application grant not found: {} in organization {}",
        grant.id, organization_id
    )))
}

fn service_error(kind: ServiceErrorKind, message: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(ServiceError::new(kind, message))
}

fn map_external_owner_bootstrap_error(error: anyhow::Error) -> anyhow::Error {
    if error
        .downcast_ref::<ExternalOwnerBootstrapError>()
        .is_some()
    {
        service_error(
            ServiceErrorKind::Conflict,
            "external owner bootstrap conflict",
        )
    } else {
        service_error(
            ServiceErrorKind::Unavailable,
            "external owner bootstrap unavailable",
        )
    }
}

fn map_application_consent_write_error(err: anyhow::Error) -> anyhow::Error {
    if err.downcast_ref::<ApplicationConsentWriteError>()
        == Some(&ApplicationConsentWriteError::PreconditionsChanged)
    {
        return service_error(
            ServiceErrorKind::Conflict,
            "application consent preconditions changed while consent was being written",
        );
    }
    err
}

fn map_application_trust_mutation_error(err: anyhow::Error) -> anyhow::Error {
    if matches!(
        err.downcast_ref::<ApplicationTrustMutationError>(),
        Some(ApplicationTrustMutationError::SecurityMutationInProgress { .. })
    ) {
        return service_error(
            ServiceErrorKind::Conflict,
            "application security mutation is already in progress",
        );
    }
    err
}

fn map_application_security_mutation_error(err: anyhow::Error) -> anyhow::Error {
    match err.downcast_ref::<ApplicationSecurityMutationError>() {
        Some(error @ ApplicationSecurityMutationError::TrustedApplication { .. }) => {
            service_error(ServiceErrorKind::Forbidden, error.to_string())
        }
        Some(error @ ApplicationSecurityMutationError::ApplicationNotFound { .. }) => {
            service_error(ServiceErrorKind::NotFound, error.to_string())
        }
        Some(error @ ApplicationSecurityMutationError::LeaseExpiredOrInvalid { .. }) => {
            service_error(ServiceErrorKind::Conflict, error.to_string())
        }
        None => err,
    }
}

async fn rauthy_admin_response_error(
    operation: &str,
    mut response: reqwest::Response,
) -> anyhow::Error {
    let status = response.status();
    let mut body = Vec::new();
    while body.len() < MAX_RAUTHY_ERROR_BODY_BYTES {
        let Ok(Some(chunk)) = response.chunk().await else {
            break;
        };
        let remaining = MAX_RAUTHY_ERROR_BODY_BYTES - body.len();
        body.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
    }

    let detail = serde_json::from_slice::<serde_json::Value>(&body)
        .ok()
        .and_then(|body| {
            ["message", "error"].into_iter().find_map(|field| {
                body.get(field)
                    .and_then(serde_json::Value::as_str)
                    .and_then(sanitize_rauthy_error_field)
                    .map(|value| format!("; {field}: {value}"))
            })
        })
        .unwrap_or_default();
    service_error(
        ServiceErrorKind::Unavailable,
        format!("{operation} failed with {status}{detail}"),
    )
}

fn sanitize_rauthy_error_field(value: &str) -> Option<String> {
    let value = value
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .take(MAX_RAUTHY_ERROR_FIELD_CHARS)
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if value.is_empty() {
        return None;
    }

    let words = value.split_whitespace().collect::<Vec<_>>();
    let mut sanitized = Vec::new();
    let mut index = 0;
    while index < words.len() {
        let word = words[index];
        if word.eq_ignore_ascii_case("bearer") {
            sanitized.push(word.to_owned());
            index += 1;
            redact_following_rauthy_value(&words, &mut index, &mut sanitized, false);
            continue;
        }

        if let Some(separator) = word.find(['=', ':']) {
            let label = &word[..separator];
            if is_sensitive_rauthy_label(label) {
                let field_value = &word[separator + 1..];
                let authorization = label.eq_ignore_ascii_case("authorization");
                if authorization && is_rauthy_auth_scheme(field_value) {
                    sanitized.push(word.to_owned());
                    index += 1;
                    redact_following_rauthy_value(&words, &mut index, &mut sanitized, false);
                } else if field_value.is_empty() {
                    sanitized.push(word.to_owned());
                    index += 1;
                    redact_following_rauthy_value(
                        &words,
                        &mut index,
                        &mut sanitized,
                        authorization,
                    );
                } else {
                    sanitized.push(format!("{}[redacted]", &word[..=separator]));
                    index += 1;
                }
                continue;
            }
        }

        if is_sensitive_rauthy_label(word)
            && words
                .get(index + 1)
                .is_some_and(|separator| matches!(*separator, "=" | ":"))
        {
            sanitized.push(word.to_owned());
            sanitized.push(words[index + 1].to_owned());
            index += 2;
            redact_following_rauthy_value(
                &words,
                &mut index,
                &mut sanitized,
                word.eq_ignore_ascii_case("authorization"),
            );
            continue;
        }

        sanitized.push(word.to_owned());
        index += 1;
    }
    Some(sanitized.join(" "))
}

fn is_sensitive_rauthy_label(label: &str) -> bool {
    let label = label.to_ascii_lowercase();
    matches!(
        label.as_str(),
        "token" | "secret" | "password" | "api-key" | "api_key" | "authorization"
    ) || label.ends_with("_token")
        || label.ends_with("_secret")
}

fn is_rauthy_auth_scheme(value: &str) -> bool {
    ["bearer", "basic", "api-key"]
        .iter()
        .any(|scheme| value.eq_ignore_ascii_case(scheme))
}

fn redact_following_rauthy_value(
    words: &[&str],
    index: &mut usize,
    sanitized: &mut Vec<String>,
    preserve_auth_scheme: bool,
) {
    if preserve_auth_scheme
        && words
            .get(*index)
            .is_some_and(|value| is_rauthy_auth_scheme(value))
    {
        sanitized.push(words[*index].to_owned());
        *index += 1;
    }
    if *index < words.len() {
        sanitized.push("[redacted]".to_owned());
        *index += 1;
    }
}

fn bad_request_error(message: impl Into<String>) -> anyhow::Error {
    service_error(ServiceErrorKind::BadRequest, message)
}

fn canonical_redirect_uri(value: &str) -> Result<String> {
    if value.trim().is_empty() || value.len() > 2_048 || value.contains('*') {
        return Err(bad_request_error(
            "redirectUri must be a non-empty URL of at most 2048 bytes",
        ));
    }
    let url = reqwest::Url::parse(value)
        .map_err(|_| bad_request_error("redirectUri must be an absolute URL"))?;
    let authority = value
        .split_once("://")
        .map(|(_, rest)| rest.split(['/', '?', '#']).next().unwrap_or_default())
        .unwrap_or_default();
    if authority.contains('@')
        || url.fragment().is_some()
        || url.query().is_some()
        || url.host_str().is_none()
    {
        return Err(bad_request_error(
            "redirectUri has unsupported authority, fragment, query, or host",
        ));
    }
    match url.scheme() {
        "https" => {}
        "http" => {
            let Some(host) = url.host_str() else {
                return Err(bad_request_error(
                    "redirectUri HTTP host must be a loopback IP",
                ));
            };
            if !matches!(host.trim_matches(['[', ']']).parse::<IpAddr>(), Ok(ip) if ip.is_loopback())
                || url.port().is_none()
            {
                return Err(bad_request_error(
                    "redirectUri HTTP must use a loopback IP with an explicit port",
                ));
            }
        }
        _ => {
            return Err(bad_request_error(
                "redirectUri scheme must be https or loopback http",
            ));
        }
    }
    let canonical: String = url.into();
    if canonical.len() > 2_048 || canonical.contains('*') {
        return Err(bad_request_error(
            "redirectUri must be a non-empty URL of at most 2048 bytes",
        ));
    }
    Ok(canonical)
}

fn not_found_error(message: impl Into<String>) -> anyhow::Error {
    service_error(ServiceErrorKind::NotFound, message)
}

fn forbidden_error(message: impl Into<String>) -> anyhow::Error {
    service_error(ServiceErrorKind::Forbidden, message)
}

fn ensure_organization_application_mutable(application: &Application) -> Result<()> {
    if application.first_party
        && application.consent_policy == ApplicationConsentPolicy::AutoApprove
    {
        return Err(forbidden_error(
            "forbidden: trusted applications may only be changed by the master administrator",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        SCHEMA_READINESS_CACHE_TTL, SchemaReadinessCache, SchemaReadinessRoute,
        SchemaReadinessState, ServiceError, ServiceErrorKind, exposure_principal_digest,
        map_application_consent_write_error, map_application_security_mutation_error,
        map_application_trust_mutation_error, map_external_owner_bootstrap_error,
        sanitize_rauthy_error_field,
    };
    use crate::database::{
        ApplicationConsentWriteError, ApplicationSecurityMutationError,
        ApplicationTrustMutationError, ExternalOwnerBootstrapError,
    };
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::{Duration, Instant},
    };

    #[test]
    fn exposure_principal_digest_is_stable_and_length_framed() {
        let digest = exposure_principal_digest("user", "organization", "application");
        assert_eq!(digest.len(), 64);
        assert!(
            digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        );
        assert_eq!(
            digest,
            exposure_principal_digest("user", "organization", "application")
        );
        assert_ne!(
            exposure_principal_digest("ab", "c", "d"),
            exposure_principal_digest("a", "bc", "d")
        );
    }

    #[test]
    fn schema_readiness_cache_is_route_scoped_and_expires() {
        let started_at = Instant::now();
        let mut state = SchemaReadinessState::default();
        state.mark_verified(SchemaReadinessRoute::Local, started_at);

        assert!(state.is_fresh(
            SchemaReadinessRoute::Local,
            started_at + SCHEMA_READINESS_CACHE_TTL - Duration::from_millis(1),
            SCHEMA_READINESS_CACHE_TTL,
        ));
        assert!(!state.is_fresh(
            SchemaReadinessRoute::Strong,
            started_at,
            SCHEMA_READINESS_CACHE_TTL,
        ));
        assert!(!state.is_fresh(
            SchemaReadinessRoute::Local,
            started_at + SCHEMA_READINESS_CACHE_TTL,
            SCHEMA_READINESS_CACHE_TTL,
        ));

        state.clear();
        assert!(!state.is_fresh(
            SchemaReadinessRoute::Local,
            started_at,
            SCHEMA_READINESS_CACHE_TTL,
        ));
    }

    #[test]
    fn maps_only_typed_database_mutation_errors_to_service_errors() {
        let conflict = map_external_owner_bootstrap_error(anyhow::Error::new(
            ExternalOwnerBootstrapError::PreconditionsChanged,
        ));
        assert_eq!(
            conflict
                .downcast_ref::<ServiceError>()
                .expect("typed bootstrap conflict")
                .kind(),
            ServiceErrorKind::Conflict
        );
        let unavailable =
            map_external_owner_bootstrap_error(anyhow::anyhow!("raft quorum unavailable"));
        assert_eq!(
            unavailable
                .downcast_ref::<ServiceError>()
                .expect("bootstrap availability error")
                .kind(),
            ServiceErrorKind::Unavailable
        );

        let consent_error = anyhow::Error::new(ApplicationConsentWriteError::PreconditionsChanged)
            .context("replace active application consent transaction");
        let consent_error = map_application_consent_write_error(consent_error);
        let consent_service_error = consent_error
            .downcast_ref::<ServiceError>()
            .expect("typed consent conflict maps to a service error");
        assert_eq!(consent_service_error.kind(), ServiceErrorKind::Conflict);

        let trust_error =
            anyhow::Error::new(ApplicationTrustMutationError::SecurityMutationInProgress {
                application_id: "application-1".to_owned(),
            })
            .context("set application trust transaction");
        let trust_error = map_application_trust_mutation_error(trust_error);
        let trust_service_error = trust_error
            .downcast_ref::<ServiceError>()
            .expect("typed trust conflict maps to a service error");
        assert_eq!(trust_service_error.kind(), ServiceErrorKind::Conflict);

        for (error, expected_kind) in [
            (
                ApplicationSecurityMutationError::TrustedApplication {
                    application_id: "application-trusted".to_owned(),
                },
                ServiceErrorKind::Forbidden,
            ),
            (
                ApplicationSecurityMutationError::ApplicationNotFound {
                    application_id: "application-missing".to_owned(),
                },
                ServiceErrorKind::NotFound,
            ),
            (
                ApplicationSecurityMutationError::LeaseExpiredOrInvalid {
                    application_id: "application-conflict".to_owned(),
                },
                ServiceErrorKind::Conflict,
            ),
        ] {
            let mapped = map_application_security_mutation_error(
                anyhow::Error::new(error).context("application security mutation"),
            );
            assert_eq!(
                mapped
                    .downcast_ref::<ServiceError>()
                    .expect("typed application security mutation maps to a service error")
                    .kind(),
                expected_kind
            );
        }
    }

    #[test]
    fn leaves_unexpected_database_errors_internal_even_when_messages_match() {
        let consent_message =
            "application consent preconditions changed while consent was being written";
        let consent_error = map_application_consent_write_error(
            anyhow::anyhow!(consent_message).context("unexpected database failure"),
        );
        assert!(consent_error.downcast_ref::<ServiceError>().is_none());
        assert_eq!(consent_error.to_string(), "unexpected database failure");
        assert!(
            consent_error
                .chain()
                .any(|cause| cause.to_string() == consent_message)
        );

        let trust_message = "application security mutation already in progress";
        let trust_error = map_application_trust_mutation_error(
            anyhow::anyhow!(trust_message).context("unexpected database failure"),
        );
        assert!(trust_error.downcast_ref::<ServiceError>().is_none());
        assert_eq!(trust_error.to_string(), "unexpected database failure");
        assert!(
            trust_error
                .chain()
                .any(|cause| cause.to_string() == trust_message)
        );

        let security_message = "application security lease expired or invalid: application-1";
        let security_error = map_application_security_mutation_error(
            anyhow::anyhow!(security_message).context("unexpected database failure"),
        );
        assert!(security_error.downcast_ref::<ServiceError>().is_none());
        assert_eq!(security_error.to_string(), "unexpected database failure");
        assert!(
            security_error
                .chain()
                .any(|cause| cause.to_string() == security_message)
        );
    }

    #[tokio::test]
    async fn schema_readiness_cache_single_flights_and_invalidates() {
        let cache = Arc::new(SchemaReadinessCache::new(Duration::from_secs(30)));
        let verification_count = Arc::new(AtomicUsize::new(0));

        let verify = |cache: Arc<SchemaReadinessCache>, count: Arc<AtomicUsize>| {
            tokio::spawn(async move {
                cache
                    .verify(SchemaReadinessRoute::Local, || async move {
                        count.fetch_add(1, Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_millis(20)).await;
                        Ok(())
                    })
                    .await
            })
        };
        let first = verify(cache.clone(), verification_count.clone());
        let second = verify(cache.clone(), verification_count.clone());
        let (first, second) = tokio::join!(first, second);
        assert!(first.expect("first cache task"));
        assert!(second.expect("second cache task"));
        assert_eq!(verification_count.load(Ordering::SeqCst), 1);

        cache
            .while_invalidated(|| async { Ok(()) })
            .await
            .expect("invalidate cache");
        assert!(
            cache
                .verify(SchemaReadinessRoute::Local, || async {
                    verification_count.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                })
                .await
        );
        assert_eq!(verification_count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn schema_readiness_failures_are_not_cached() {
        let cache = SchemaReadinessCache::new(Duration::from_secs(30));
        let verification_count = AtomicUsize::new(0);

        for _ in 0..2 {
            assert!(
                !cache
                    .verify(SchemaReadinessRoute::Strong, || async {
                        verification_count.fetch_add(1, Ordering::SeqCst);
                        anyhow::bail!("schema unavailable")
                    })
                    .await
            );
        }
        assert_eq!(verification_count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn preserves_benign_rauthy_error_messages() {
        for message in [
            "token expired",
            "invalid_token",
            "missing authorization header",
        ] {
            assert_eq!(
                sanitize_rauthy_error_field(message).as_deref(),
                Some(message)
            );
        }
    }

    #[test]
    fn redacts_only_sensitive_rauthy_error_values() {
        for (message, expected) in [
            ("token=abc", "token=[redacted]"),
            ("password: abc", "password: [redacted]"),
            ("api_key=abc", "api_key=[redacted]"),
            (
                "Authorization: Bearer abc",
                "Authorization: Bearer [redacted]",
            ),
            (
                "Authorization: Basic abc",
                "Authorization: Basic [redacted]",
            ),
            (
                "Authorization: API-Key abc",
                "Authorization: API-Key [redacted]",
            ),
            ("Authorization: abc", "Authorization: [redacted]"),
            ("Authorization=abc", "Authorization=[redacted]"),
            ("Bearer abc", "Bearer [redacted]"),
            ("token = abc", "token = [redacted]"),
            (
                "Authorization : Basic abc",
                "Authorization : Basic [redacted]",
            ),
            (
                "request rejected: token=abc",
                "request rejected: token=[redacted]",
            ),
        ] {
            let sanitized = sanitize_rauthy_error_field(message).expect("sanitized message");
            assert_eq!(sanitized, expected);
            assert!(!sanitized.contains("abc"));
        }
    }
}

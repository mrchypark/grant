use crate::{
    auth::{
        AuthResolver, AuthenticatedPrincipal, ExternalIdentity, map_strong_identity_sync_error,
    },
    config::{AppConfig, ReadConsistency},
    database::{BackupEntry, BindIdentityInput, Database, UserActionCheck},
};
use anyhow::Result;
use chrono::Utc;
use domain::{
    Action, Agreement, AgreementStatus, AgreementType, Application, ApplicationAppType,
    ApplicationConsentPolicy, ApplicationGrant, ApplicationGrantStatus, ApplicationRedirectUri,
    ApplicationType, HealthStatus, Organization, Role, RoleBinding, RoleBindingState,
    RoleBindingSubjectType, ServiceProvider, User,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, error::Error as StdError, fmt, time::Duration};

const MAX_APPLICATION_ACTION_IDS_PER_REQUEST: usize = 999;
const MAX_ACTION_IDS_PER_REQUEST: usize = 999;
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_RAUTHY_ERROR_BODY_BYTES: usize = 4 * 1024;
const MAX_RAUTHY_ERROR_FIELD_CHARS: usize = 256;

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
}

#[derive(Debug, Clone)]
pub struct CreateApplicationInput {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub application_type: ApplicationType,
    pub app_type: ApplicationAppType,
    pub first_party: bool,
    pub consent_policy: ApplicationConsentPolicy,
    pub active: bool,
}

#[derive(Debug, Clone)]
pub struct UpdateApplicationInput {
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub application_type: ApplicationType,
    pub app_type: ApplicationAppType,
    pub first_party: bool,
    pub consent_policy: ApplicationConsentPolicy,
    pub active: bool,
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

#[derive(Debug, Clone)]
pub struct RotateClientSecretResult {
    pub client_secret: String,
    pub secret_rotated_at: String,
}

#[derive(Debug, Serialize)]
struct RauthyNewClientRequest {
    id: String,
    name: Option<String>,
    confidential: bool,
    redirect_uris: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RauthyClientResponse {
    id: String,
    name: Option<String>,
    enabled: bool,
    confidential: bool,
    redirect_uris: Vec<String>,
    post_logout_redirect_uris: Option<Vec<String>>,
    allowed_origins: Option<Vec<String>>,
    flows_enabled: Vec<String>,
    access_token_alg: String,
    id_token_alg: String,
    auth_code_lifetime: i32,
    access_token_lifetime: i32,
    scopes: Vec<String>,
    default_scopes: Vec<String>,
    challenges: Option<Vec<String>>,
    force_mfa: bool,
    client_uri: Option<String>,
    contacts: Option<Vec<String>>,
    backchannel_logout_uri: Option<String>,
    restrict_group_prefix: Option<String>,
    claims: Option<serde_json::Value>,
    claims_at_root: bool,
    allowed_resources: Option<Vec<String>>,
    default_aud: Option<Vec<String>>,
    scim: Option<RauthyScimClient>,
}

#[derive(Debug, Serialize)]
struct RauthyClientUpdateRequest {
    name: Option<String>,
    enabled: bool,
    confidential: bool,
    redirect_uris: Vec<String>,
    post_logout_redirect_uris: Option<Vec<String>>,
    allowed_origins: Option<Vec<String>>,
    flows_enabled: Vec<String>,
    access_token_alg: String,
    id_token_alg: String,
    auth_code_lifetime: i32,
    access_token_lifetime: i32,
    scopes: Vec<String>,
    default_scopes: Vec<String>,
    challenges: Option<Vec<String>>,
    force_mfa: bool,
    client_uri: Option<String>,
    contacts: Option<Vec<String>>,
    backchannel_logout_uri: Option<String>,
    restrict_group_prefix: Option<String>,
    claims: Option<serde_json::Value>,
    claims_at_root: bool,
    allowed_resources: Option<Vec<String>>,
    default_aud: Option<Vec<String>>,
    scim: Option<RauthyScimClient>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RauthyScimClient {
    bearer_token: String,
    base_uri: String,
    sync_groups: bool,
    group_sync_prefix: Option<String>,
}

impl From<RauthyClientResponse> for RauthyClientUpdateRequest {
    fn from(client: RauthyClientResponse) -> Self {
        Self {
            name: client.name,
            enabled: client.enabled,
            confidential: client.confidential,
            redirect_uris: client.redirect_uris,
            post_logout_redirect_uris: client.post_logout_redirect_uris,
            allowed_origins: client.allowed_origins,
            flows_enabled: client.flows_enabled,
            access_token_alg: client.access_token_alg,
            id_token_alg: client.id_token_alg,
            auth_code_lifetime: client.auth_code_lifetime,
            access_token_lifetime: client.access_token_lifetime,
            scopes: client.scopes,
            default_scopes: client.default_scopes,
            challenges: client.challenges,
            force_mfa: client.force_mfa,
            client_uri: client.client_uri,
            contacts: client.contacts,
            backchannel_logout_uri: client.backchannel_logout_uri,
            restrict_group_prefix: client.restrict_group_prefix,
            claims: client.claims,
            claims_at_root: client.claims_at_root,
            allowed_resources: client.allowed_resources,
            default_aud: client.default_aud,
            scim: client.scim,
        }
    }
}

#[derive(Debug, Serialize)]
struct RauthyClientSecretRequest {}

#[derive(Debug, Deserialize)]
struct RauthyClientSecretResponse {
    secret: Option<String>,
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
pub struct AuthorizationDecision {
    pub allowed: bool,
    pub reason: String,
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
        }
    }

    pub async fn migrate_schema(&self) -> Result<()> {
        self.db.ensure_schema_only().await
    }

    pub async fn initialize(&self) -> Result<()> {
        self.db.ensure_schema().await
    }

    pub async fn bootstrap(&self) -> Result<()> {
        self.db.ensure_bootstrap_data().await
    }

    pub async fn health_status(&self) -> HealthStatus {
        self.db.health_status().await
    }

    pub fn master_key(&self) -> &str {
        &self.cfg.security.master_key
    }

    pub fn rauthy_base_url(&self) -> &str {
        &self.cfg.auth.rauthy.base_url
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
        Ok(AuthenticatedPrincipal {
            user_id: user.id,
            organization_id: self.cfg.security.bootstrap_user_id.clone(),
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
            .resolve_external_identity(
                Some(&self.db),
                bearer_token,
                effective_read_consistency_override.clone(),
            )
            .await?;
        let user = if matches!(
            effective_read_consistency,
            ReadConsistency::Strong | ReadConsistency::PreferStrong
        ) {
            self.db
                .resolve_or_create_user_identity_with_consistency(
                    &identity.provider,
                    &identity.external_user_id,
                    identity.email.as_deref(),
                    &identity.display_name,
                    identity.active,
                    effective_read_consistency_override.clone(),
                )
                .await
                .map_err(map_strong_identity_sync_error)?
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
        let organizations = self
            .db
            .list_active_organizations_for_user_with_consistency(
                &user.id,
                effective_read_consistency_override,
            )
            .await?;
        let active_organization_id =
            normalize_active_organization(active_organization_id, &identity, &organizations)?;
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

    pub async fn create_application(
        &self,
        user_id: &str,
        organization_id: &str,
        input: CreateApplicationInput,
    ) -> Result<Application> {
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
            first_party: input.first_party,
            consent_policy: input.consent_policy,
            active: input.active,
            rauthy_client_id: Some(application_id),
            secret_rotated_at: None,
            created_by_user_id: user_id.to_owned(),
        };
        self.db.insert_application(&application).await?;
        Ok(application)
    }

    pub async fn update_application(
        &self,
        user_id: &str,
        organization_id: &str,
        application_id: &str,
        input: UpdateApplicationInput,
    ) -> Result<Application> {
        let existing = self
            .get_application_authorized(
                user_id,
                organization_id,
                application_id,
                Some(ReadConsistency::Strong),
            )
            .await?;
        let previous = existing.clone();
        let slug = require_non_empty("slug", &input.slug)?;
        let slug_changed = existing.slug != slug;
        let application = Application {
            id: existing.id,
            organization_id: existing.organization_id,
            slug,
            name: require_non_empty("name", &input.name)?,
            description: normalize_optional_text(input.description),
            application_type: input.application_type,
            app_type: input.app_type,
            first_party: input.first_party,
            consent_policy: input.consent_policy,
            active: input.active,
            rauthy_client_id: existing.rauthy_client_id,
            secret_rotated_at: existing.secret_rotated_at,
            created_by_user_id: existing.created_by_user_id,
        };
        let redirect_uris = self
            .db
            .list_application_redirect_uris_with_consistency(
                application_id,
                Some(ReadConsistency::Strong),
            )
            .await?
            .into_iter()
            .map(|item| item.redirect_uri)
            .collect::<Vec<_>>();
        self.db
            .update_application(&application, slug_changed)
            .await?;
        let sync_result = if redirect_uris.is_empty() {
            Ok(())
        } else {
            self.sync_rauthy_application_client(&application, redirect_uris, false)
                .await
        };
        if let Err(err) = sync_result {
            let _ = self.db.update_application(&previous, slug_changed).await;
            return Err(err);
        }
        Ok(application)
    }

    pub async fn list_application_redirect_uris_authorized(
        &self,
        user_id: &str,
        organization_id: &str,
        application_id: &str,
        read_consistency: Option<ReadConsistency>,
    ) -> Result<Vec<ApplicationRedirectUri>> {
        self.get_application_authorized(
            user_id,
            organization_id,
            application_id,
            read_consistency.clone(),
        )
        .await?;
        self.db
            .list_application_redirect_uris_with_consistency(application_id, read_consistency)
            .await
    }

    pub async fn create_application_redirect_uri_authorized(
        &self,
        user_id: &str,
        organization_id: &str,
        application_id: &str,
        redirect_uri: &str,
    ) -> Result<ApplicationRedirectUri> {
        let application = self
            .get_application_authorized(
                user_id,
                organization_id,
                application_id,
                Some(ReadConsistency::Strong),
            )
            .await?;
        let redirect_uri = normalize_redirect_uri(redirect_uri)?;
        let existing = self
            .db
            .list_application_redirect_uris_with_consistency(
                application_id,
                Some(ReadConsistency::Strong),
            )
            .await?;
        if existing
            .iter()
            .any(|item| item.redirect_uri == redirect_uri)
        {
            return Err(service_error(
                ServiceErrorKind::Conflict,
                "redirectUri already exists for this application",
            ));
        }
        let create_client = existing.is_empty();
        let item = ApplicationRedirectUri {
            application_id: application_id.to_owned(),
            redirect_uri: redirect_uri.clone(),
            created_at: Utc::now().to_rfc3339(),
        };
        self.db.insert_application_redirect_uri(&item).await?;
        let mut redirect_uris: Vec<String> =
            existing.into_iter().map(|item| item.redirect_uri).collect();
        redirect_uris.push(redirect_uri);
        if let Err(err) = self
            .sync_rauthy_application_client(&application, redirect_uris, create_client)
            .await
        {
            let _ = self
                .db
                .delete_application_redirect_uri(application_id, &item.redirect_uri)
                .await;
            return Err(err);
        }
        Ok(item)
    }

    pub async fn delete_application_redirect_uri_authorized(
        &self,
        user_id: &str,
        organization_id: &str,
        application_id: &str,
        redirect_uri: &str,
    ) -> Result<()> {
        let application = self
            .get_application_authorized(
                user_id,
                organization_id,
                application_id,
                Some(ReadConsistency::Strong),
            )
            .await?;
        let redirect_uri = normalize_redirect_uri(redirect_uri)?;
        let existing = self
            .db
            .list_application_redirect_uris_with_consistency(
                application_id,
                Some(ReadConsistency::Strong),
            )
            .await?;
        let mut removed = None;
        let redirect_uris = existing
            .into_iter()
            .filter_map(|item| {
                if item.redirect_uri == redirect_uri {
                    removed = Some(item);
                    None
                } else {
                    Some(item.redirect_uri)
                }
            })
            .collect::<Vec<_>>();
        self.db
            .delete_application_redirect_uri(application_id, &redirect_uri)
            .await?;
        if removed.is_none() {
            return Ok(());
        }
        if let Err(err) = self
            .sync_rauthy_application_client(&application, redirect_uris, false)
            .await
        {
            if let Some(item) = removed {
                let _ = self.db.insert_application_redirect_uri(&item).await;
            }
            return Err(err);
        }
        Ok(())
    }

    pub async fn rotate_application_client_secret_authorized(
        &self,
        user_id: &str,
        organization_id: &str,
        application_id: &str,
    ) -> Result<RotateClientSecretResult> {
        let application = self
            .get_application_authorized(
                user_id,
                organization_id,
                application_id,
                Some(ReadConsistency::Strong),
            )
            .await?;
        let redirect_uris = self
            .db
            .list_application_redirect_uris_with_consistency(
                application_id,
                Some(ReadConsistency::Strong),
            )
            .await?;
        if redirect_uris.is_empty() {
            return Err(bad_request_error(
                "application must have at least one redirectUri before rotating client secret",
            ));
        }
        let client_secret = self
            .rotate_rauthy_application_client_secret(&application)
            .await?;
        let secret_rotated_at = Utc::now().to_rfc3339();
        self.db
            .update_application_secret_rotated_at(application_id, &secret_rotated_at)
            .await?;
        Ok(RotateClientSecretResult {
            client_secret,
            secret_rotated_at,
        })
    }

    async fn sync_rauthy_application_client(
        &self,
        application: &Application,
        redirect_uris: Vec<String>,
        create: bool,
    ) -> Result<()> {
        let client_id = rauthy_client_id(application)?;
        let flows_enabled = if matches!(application.application_type, ApplicationType::Confidential)
        {
            vec!["authorization_code", "client_credentials", "refresh_token"]
        } else {
            vec!["authorization_code", "refresh_token"]
        };
        let response = if create {
            self.http_client
                .post(self.rauthy_admin_url("/auth/v1/clients"))
                .header(
                    "authorization",
                    format!("API-Key {}", self.cfg.auth.rauthy.admin_api_key),
                )
                .json(&RauthyNewClientRequest {
                    id: client_id.to_owned(),
                    name: Some(application.name.clone()),
                    confidential: matches!(
                        application.application_type,
                        ApplicationType::Confidential
                    ),
                    redirect_uris,
                })
                .send()
                .await
        } else {
            let url = self.rauthy_admin_url(&format!("/auth/v1/clients/{client_id}"));
            let response = self
                .http_client
                .get(&url)
                .header(
                    "authorization",
                    format!("API-Key {}", self.cfg.auth.rauthy.admin_api_key),
                )
                .send()
                .await
                .map_err(|err| {
                    service_error(
                        ServiceErrorKind::Unavailable,
                        format!("rauthy client fetch failed: {err}"),
                    )
                })?;
            if !response.status().is_success() {
                return Err(rauthy_admin_response_error("rauthy client fetch", response).await);
            }
            let existing: RauthyClientResponse = response.json().await.map_err(|err| {
                service_error(
                    ServiceErrorKind::Unavailable,
                    format!("invalid rauthy client response: {err}"),
                )
            })?;
            if existing.id != client_id {
                return Err(service_error(
                    ServiceErrorKind::Unavailable,
                    "invalid rauthy client response: client id mismatch",
                ));
            }
            let mut request = RauthyClientUpdateRequest::from(existing);
            request.name = Some(application.name.clone());
            request.confidential =
                matches!(application.application_type, ApplicationType::Confidential);
            request.redirect_uris = redirect_uris;
            request.enabled = application.active;
            request.flows_enabled = flows_enabled.into_iter().map(str::to_owned).collect();
            self.http_client
                .put(url)
                .header(
                    "authorization",
                    format!("API-Key {}", self.cfg.auth.rauthy.admin_api_key),
                )
                .json(&request)
                .send()
                .await
        }
        .map_err(|err| {
            service_error(
                ServiceErrorKind::Unavailable,
                format!("rauthy client sync failed: {err}"),
            )
        })?;
        if !response.status().is_success() {
            return Err(rauthy_admin_response_error("rauthy client sync", response).await);
        }
        Ok(())
    }

    async fn rotate_rauthy_application_client_secret(
        &self,
        application: &Application,
    ) -> Result<String> {
        let client_id = rauthy_client_id(application)?;
        let response = self
            .http_client
            .put(self.rauthy_admin_url(&format!("/auth/v1/clients/{client_id}/secret")))
            .header(
                "authorization",
                format!("API-Key {}", self.cfg.auth.rauthy.admin_api_key),
            )
            .json(&RauthyClientSecretRequest {})
            .send()
            .await
            .map_err(|err| {
                service_error(
                    ServiceErrorKind::Unavailable,
                    format!("rauthy client secret rotation failed: {err}"),
                )
            })?;
        if !response.status().is_success() {
            return Err(
                rauthy_admin_response_error("rauthy client secret rotation", response).await,
            );
        }
        let body: RauthyClientSecretResponse = response.json().await.map_err(|err| {
            service_error(
                ServiceErrorKind::Unavailable,
                format!("invalid rauthy client secret response: {err}"),
            )
        })?;
        body.secret.ok_or_else(|| {
            service_error(
                ServiceErrorKind::Unavailable,
                "rauthy client secret response did not include a secret",
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
        self.db
            .replace_application_actions(application_id, action_ids)
            .await
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
                consistency: read_consistency.clone(),
            })
            .collect::<Vec<_>>();
        self.db
            .has_user_actions(&db_checks)
            .await?
            .into_iter()
            .map(|allowed| {
                Ok(AuthorizationDecision {
                    allowed,
                    reason: if allowed {
                        "role_binding".to_owned()
                    } else {
                        "no_matching_binding".to_owned()
                    },
                })
            })
            .collect()
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
        self.authorize_organization_action(
            user_id,
            organization_id,
            target_organization_id,
            "organization.permission.manage",
            &format!("reactivate organization {}", target_organization_id),
            Some(ReadConsistency::Strong),
        )
        .await?;
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

fn normalize_redirect_uri(value: &str) -> Result<String> {
    let redirect_uri = require_non_empty("redirectUri", value)?;
    let authority = redirect_uri
        .strip_prefix("http://")
        .or_else(|| redirect_uri.strip_prefix("https://"))
        .ok_or_else(|| {
            bad_request_error("redirectUri must use http or https scheme and include a host")
        })?;
    if authority.starts_with('/') {
        return Err(bad_request_error(
            "redirectUri must use http or https scheme and include a host",
        ));
    }
    let parsed = reqwest::Url::parse(&redirect_uri)
        .map_err(|_| bad_request_error("redirectUri must be a valid absolute URL"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(bad_request_error(
            "redirectUri must use http or https scheme and include a host",
        ));
    }
    if parsed.host_str().is_none_or(|host| host.trim().is_empty()) {
        return Err(bad_request_error(
            "redirectUri must use http or https scheme and include a host",
        ));
    }
    if parsed.fragment().is_some() {
        return Err(bad_request_error("redirectUri must not contain a fragment"));
    }
    Ok(parsed.to_string())
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

fn not_found_error(message: impl Into<String>) -> anyhow::Error {
    service_error(ServiceErrorKind::NotFound, message)
}

fn forbidden_error(message: impl Into<String>) -> anyhow::Error {
    service_error(ServiceErrorKind::Forbidden, message)
}

#[cfg(test)]
mod tests {
    use super::sanitize_rauthy_error_field;

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

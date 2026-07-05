use crate::{
    artifacts::ArtifactStore,
    auth::{
        AuthResolver, AuthenticatedPrincipal, ExternalIdentity, map_strong_identity_sync_error,
    },
    config::{AppConfig, ReadConsistency},
    database::{BackupEntry, Database},
};
use anyhow::Result;
use domain::{
    Action, Application, ApplicationAppType, ApplicationConsentPolicy, ApplicationType,
    HealthStatus, Organization, RoleBinding, RoleBindingResourceType, RoleBindingSubjectType, User,
};
use std::{error::Error as StdError, fmt};

const MAX_APPLICATION_ACTION_IDS_PER_REQUEST: usize = 999;

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
    pub fn new(cfg: AppConfig, db: Database, _artifacts: ArtifactStore) -> Self {
        let auth = AuthResolver::new(&cfg.auth);
        Self { cfg, db, auth }
    }

    pub async fn migrate_schema(&self) -> Result<()> {
        self.db.ensure_schema_only().await
    }

    pub async fn initialize(&self) -> Result<()> {
        self.db.ensure_schema().await
    }

    pub async fn health_status(&self) -> HealthStatus {
        self.db.health_status().await
    }

    pub fn master_key(&self) -> &str {
        &self.cfg.security.master_key
    }

    pub fn trailbase_base_url(&self) -> &str {
        &self.cfg.auth.trailbase.base_url
    }

    pub fn db(&self) -> &Database {
        &self.db
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

    pub async fn bootstrap_trailbase_owner(
        &self,
        bearer_token: Option<&str>,
    ) -> Result<AuthenticatedPrincipal> {
        let identity = self
            .auth
            .resolve_external_identity(None, bearer_token, None)
            .await?;
        let user = self
            .db
            .bind_identity_to_user(
                &self.cfg.security.bootstrap_user_id,
                &identity.provider,
                &identity.external_user_id,
                identity.email.as_deref(),
                &identity.display_name,
                identity.active,
                true,
            )
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
        let application = Application {
            id: require_non_empty("id", &input.id)?,
            organization_id: organization_id.to_owned(),
            slug: require_non_empty("slug", &input.slug)?,
            name: require_non_empty("name", &input.name)?,
            description: normalize_optional_text(input.description),
            application_type: input.application_type,
            app_type: input.app_type,
            first_party: input.first_party,
            consent_policy: input.consent_policy,
            active: input.active,
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
            created_by_user_id: existing.created_by_user_id,
        };
        self.db
            .update_application(&application, slug_changed)
            .await?;
        Ok(application)
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
        resource_type: RoleBindingResourceType,
        resource_id: &str,
    ) -> Result<Vec<RoleBinding>> {
        self.db
            .list_role_bindings_for_resource(resource_type, resource_id)
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
            anyhow::bail!(
                "forbidden: user {} in organization {} cannot {}",
                user_id,
                organization_id,
                action_label
            );
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
        "forbidden: trailbase user {} is not an active member of organization {}",
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

fn service_error(kind: ServiceErrorKind, message: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(ServiceError::new(kind, message))
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

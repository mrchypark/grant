use crate::{
    artifacts::ArtifactStore,
    auth::{
        AuthResolver, AuthenticatedPrincipal, ExternalIdentity, map_strong_identity_sync_error,
    },
    bridge::{self, BridgeDocument, BridgeOptions, BridgeRateRecord},
    config::{AppConfig, ReadConsistency},
    crypto::{
        CertificateBundle, decrypt_bundle_with_key, encrypt_bundle_with_key, extract_business_no,
        extract_common_name, generate_data_encryption_key, normalize_business_no,
    },
    database::{
        CertGatewayChallenge, Database, DatabaseLookupError, KrRpsSyncMaterialization,
        PersistedDocument, PlantSummary, StoredCertificate, StoredDocument, StoredRateRecord,
    },
    kms::{EnvKmsProvider, KmsProvider},
    npki::{validate_certificate_bundle, verify_challenge_signature},
};
use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Datelike, Duration as ChronoDuration, Utc};
use chrono_tz::Asia::Seoul;
use domain::{
    Action, Client, ClientAppType, ClientConsentPolicy, ClientType, DocumentArtifact,
    DocumentCategory, GenerationRecord, HealthStatus, Organization, Plant, RateRecord, RecRecord,
    Role, RoleBinding, RoleBindingResourceType, RoleBindingState, RoleBindingSubjectType, SyncMode,
    SyncRun, User,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error as StdError,
    fmt,
    future::Future,
    path::Path,
    pin::Pin,
    sync::Arc,
};
use tracing::{error, info, warn};
use uuid::Uuid;

type CollectSnapshotFuture = Pin<Box<dyn Future<Output = Result<bridge::BridgeRun>> + Send>>;
type SnapshotCollector = Arc<dyn Fn(BridgeOptions) -> CollectSnapshotFuture + Send + Sync>;
const MAX_CLIENT_ACTION_IDS_PER_REQUEST: usize = 999;
const STALE_OBJECT_DELETE_CONCURRENCY: usize = 8;

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
    artifacts: ArtifactStore,
    auth: AuthResolver,
    collector: SnapshotCollector,
}

#[derive(Debug, Clone)]
pub struct SyncInput {
    pub organization_id: String,
    pub facility_code: Option<String>,
    pub start_month: Option<String>,
    pub end_month: Option<String>,
    pub mode: SyncMode,
}

#[derive(Debug, Clone)]
pub struct CreatePlantInput {
    pub facility_code: String,
    pub name: String,
    pub contract_no: String,
    pub address: String,
    pub installed_kw: f64,
    pub rec_start_date: String,
    pub confirm_issued: String,
    pub owner_organization_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CreateClientInput {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub client_type: ClientType,
    pub app_type: ClientAppType,
    pub first_party: bool,
    pub consent_policy: ClientConsentPolicy,
    pub active: bool,
}

#[derive(Debug, Clone)]
pub struct UpdateClientInput {
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub client_type: ClientType,
    pub app_type: ClientAppType,
    pub first_party: bool,
    pub consent_policy: ClientConsentPolicy,
    pub active: bool,
}

#[derive(Debug, Clone)]
pub struct PlantView {
    pub plant: Plant,
    pub last_synced_at: Option<String>,
    pub last_sync: Option<SyncRun>,
    pub generation_count: i64,
    pub generation_total_kwh: f64,
    pub rates_count: i64,
    pub rec_count: i64,
    pub rec_total: i64,
    pub document_count: i64,
    pub document_categories: BTreeMap<String, i64>,
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

#[derive(Debug, Clone)]
pub struct CreateCertGatewayChallengeInput {
    pub organization_id: String,
    pub mode: String,
    pub facility_code: Option<String>,
    pub created_by_user_id: String,
    pub ttl_seconds: i64,
}

#[derive(Debug, Clone)]
pub struct CertGatewayChallengeView {
    pub id: String,
    pub mode: String,
    pub nonce: String,
    pub payload: String,
    pub expires_at: String,
}

#[derive(Debug, Clone)]
pub struct RegisterCertificateFromProofInput {
    pub organization_id: String,
    pub requested_by_user_id: Option<String>,
    pub challenge_id: String,
    pub challenge_payload: String,
    pub challenge_signature_base64: String,
    pub bundle: CertificateBundle,
}

#[derive(Debug, Clone)]
pub struct ExecuteEphemeralCrawlInput {
    pub organization_id: String,
    pub requested_by_user_id: Option<String>,
    pub challenge_id: String,
    pub challenge_payload: String,
    pub challenge_signature_base64: String,
    pub bundle: CertificateBundle,
    pub facility_code: Option<String>,
    pub start_month: Option<String>,
    pub end_month: Option<String>,
    pub mode: SyncMode,
}

#[derive(Debug, Clone)]
pub struct IssueCrawlTaskTokenInput {
    pub organization_id: String,
    pub certificate_id: String,
    pub facility_code: Option<String>,
    pub start_month: Option<String>,
    pub end_month: Option<String>,
    pub mode: SyncMode,
    pub created_by_user_id: String,
    pub ttl_seconds: i64,
}

#[derive(Debug, Clone)]
pub struct IssuedCrawlTaskToken {
    pub token: String,
    pub expires_at: String,
}

#[derive(Debug, Clone)]
pub struct StartCrawlWithTaskTokenInput {
    pub organization_id: String,
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CrawlTaskScope {
    certificate_id: String,
    facility_code: Option<String>,
    start_month: Option<String>,
    end_month: Option<String>,
    mode: String,
}

impl AppService {
    pub fn new(cfg: AppConfig, db: Database, artifacts: ArtifactStore) -> Self {
        Self::new_with_collector(cfg, db, artifacts, bridge::collect_snapshot)
    }

    pub fn new_with_collector<F, Fut>(
        cfg: AppConfig,
        db: Database,
        artifacts: ArtifactStore,
        collector: F,
    ) -> Self
    where
        F: Fn(BridgeOptions) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<bridge::BridgeRun>> + Send + 'static,
    {
        let auth = AuthResolver::new(&cfg.auth);
        Self {
            cfg,
            db,
            artifacts,
            auth,
            collector: Arc::new(move |opts| Box::pin(collector(opts))),
        }
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

    pub async fn create_cert_gateway_challenge(
        &self,
        input: CreateCertGatewayChallengeInput,
    ) -> Result<CertGatewayChallengeView> {
        let organization_id = require_non_empty("organizationId", &input.organization_id)?;
        let mode = parse_cert_gateway_mode(&input.mode)?;
        let created_by_user_id = require_non_empty("createdByUserId", &input.created_by_user_id)?;
        let facility_code = normalize_optional_text(input.facility_code);
        if mode == "stored" && facility_code.is_some() {
            return Err(bad_request_error(
                "stored certificate challenges cannot be facility-scoped",
            ));
        }
        let nonce = Uuid::new_v4().simple().to_string();
        let expires_at = (Utc::now() + ChronoDuration::seconds(input.ttl_seconds)).to_rfc3339();
        let payload = build_challenge_payload(
            &organization_id,
            &mode,
            &nonce,
            facility_code.as_deref(),
            &expires_at,
        );
        let challenge = self
            .db
            .create_cert_gateway_challenge(
                &organization_id,
                mode,
                &nonce,
                &hash_hex(&payload),
                facility_code.as_deref(),
                &expires_at,
                &created_by_user_id,
            )
            .await?;
        Ok(CertGatewayChallengeView {
            id: challenge.id,
            mode: challenge.mode,
            nonce: challenge.nonce,
            payload,
            expires_at: challenge.expires_at,
        })
    }

    pub async fn register_certificate_from_proof(
        &self,
        input: RegisterCertificateFromProofInput,
    ) -> Result<StoredCertificate> {
        let organization_id = require_non_empty("organizationId", &input.organization_id)?;
        let business_no = self
            .validate_bundle_for_organization(&organization_id, None, &input.bundle)
            .await?;
        let challenge = self
            .validate_cert_gateway_challenge(
                &input.challenge_id,
                &input.challenge_payload,
                &input.challenge_signature_base64,
                &input.bundle.sign_cert_der,
                "stored",
                &organization_id,
            )
            .await?;
        if challenge.facility_code.is_some() {
            return Err(bad_request_error(
                "stored certificate challenges cannot be facility-scoped",
            ));
        }
        if let Some(requested_by_user_id) = input
            .requested_by_user_id
            .as_deref()
            .map(|value| require_non_empty("requestedByUserId", value))
            .transpose()?
        {
            if requested_by_user_id != challenge.created_by_user_id {
                anyhow::bail!(
                    "forbidden: challenge creator {} does not match requester {}",
                    challenge.created_by_user_id,
                    requested_by_user_id
                );
            }
        }
        self.db
            .consume_cert_gateway_challenge(&input.challenge_id)
            .await?;
        self.store_certificate_bundle(&organization_id, business_no, input.bundle)
            .await
    }

    pub async fn authorize_certificate_registration(
        &self,
        user_id: &str,
        organization_id: &str,
    ) -> Result<()> {
        self.authorize_organization_action(
            user_id,
            organization_id,
            organization_id,
            "organization.permission.manage",
            "register stored certificates",
            Some(ReadConsistency::Strong),
        )
        .await
    }

    pub async fn list_stored_certificates(
        &self,
        organization_id: &str,
    ) -> Result<Vec<StoredCertificate>> {
        let organization_id = require_non_empty("organizationId", organization_id)?;
        self.db.list_certificates(&organization_id).await
    }

    pub async fn execute_ephemeral_crawl(
        &self,
        input: ExecuteEphemeralCrawlInput,
    ) -> Result<SyncRun> {
        let organization_id = require_non_empty("organizationId", &input.organization_id)?;
        let requested_by_user_id = input
            .requested_by_user_id
            .as_deref()
            .map(|value| require_non_empty("requestedByUserId", value))
            .transpose()?;
        let business_no = self
            .validate_bundle_for_organization(&organization_id, None, &input.bundle)
            .await?;
        let challenge = self
            .validate_cert_gateway_challenge(
                &input.challenge_id,
                &input.challenge_payload,
                &input.challenge_signature_base64,
                &input.bundle.sign_cert_der,
                "ephemeral",
                &organization_id,
            )
            .await?;
        if let Some(requested_by_user_id) = requested_by_user_id {
            if requested_by_user_id != challenge.created_by_user_id {
                anyhow::bail!(
                    "forbidden: challenge creator {} does not match requester {}",
                    challenge.created_by_user_id,
                    requested_by_user_id
                );
            }
        }
        let effective_facility_code = match (
            challenge.facility_code.as_deref(),
            input.facility_code.as_deref(),
        ) {
            (None, Some(requested_facility_code)) => {
                anyhow::bail!(
                    "challenge facilityCode mismatch: signed <none>, requested {}",
                    requested_facility_code
                );
            }
            (Some(challenge_facility_code), Some(requested_facility_code))
                if challenge_facility_code != requested_facility_code =>
            {
                anyhow::bail!(
                    "challenge facilityCode mismatch: signed {}, requested {}",
                    challenge_facility_code,
                    requested_facility_code
                );
            }
            (Some(challenge_facility_code), _) => Some(challenge_facility_code.to_owned()),
            (None, None) => None,
        };
        let authorized_facility_code = self
            .authorize_gateway_sync_scope(
                &challenge.created_by_user_id,
                &organization_id,
                effective_facility_code.as_deref(),
            )
            .await?;
        validate_sync_request_shape(
            &input.mode,
            input.start_month.as_deref(),
            input.end_month.as_deref(),
        )?;
        self.db
            .consume_cert_gateway_challenge(&input.challenge_id)
            .await?;

        self.run_sync_with_bundle(
            &organization_id,
            Some(challenge.created_by_user_id.as_str()),
            false,
            business_no,
            input.bundle,
            authorized_facility_code,
            input.start_month,
            input.end_month,
            input.mode,
        )
        .await
    }

    pub async fn issue_crawl_task_token(
        &self,
        input: IssueCrawlTaskTokenInput,
    ) -> Result<IssuedCrawlTaskToken> {
        let organization_id = require_non_empty("organizationId", &input.organization_id)?;
        let certificate_id = require_non_empty("certificateId", &input.certificate_id)?;
        let created_by_user_id = require_non_empty("createdByUserId", &input.created_by_user_id)?;
        let certificate = self
            .db
            .get_certificate_by_id(&certificate_id)
            .await
            .map_err(map_missing_certificate)?;
        let organization = self.db.get_organization(&organization_id).await?;
        if organization.inactivated_at.is_some() {
            return Err(forbidden_error(format!(
                "forbidden: organization {organization_id} is inactive"
            )));
        }
        if organization.party_id.as_deref() != Some(certificate.party_id.as_str()) {
            return Err(bad_request_error(
                "certificate does not belong to organization",
            ));
        }
        if certificate.active != 1 {
            return Err(bad_request_error("certificate is inactive"));
        }
        validate_sync_request_shape(
            &input.mode,
            input.start_month.as_deref(),
            input.end_month.as_deref(),
        )?;
        let authorized_facility_code = self
            .authorize_gateway_sync_scope(
                &created_by_user_id,
                &organization_id,
                input.facility_code.as_deref(),
            )
            .await?;
        if let Some(facility_code) = authorized_facility_code.as_deref() {
            if self
                .db
                .get_plant_by_facility_code(facility_code)
                .await
                .is_ok()
            {
                self.authorize_stored_certificate_use_for_sync(
                    &created_by_user_id,
                    &organization_id,
                    facility_code,
                )
                .await?;
            }
        }

        let scope = CrawlTaskScope {
            certificate_id: certificate.id,
            facility_code: authorized_facility_code,
            start_month: input.start_month,
            end_month: input.end_month,
            mode: sync_mode_label(&input.mode).to_owned(),
        };
        let scope_json = serde_json::to_string(&scope).context("serialize crawl task scope")?;
        let token = format!("{}.{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let token_hash = hash_hex(&token);
        let expires_at = (Utc::now() + ChronoDuration::seconds(input.ttl_seconds)).to_rfc3339();
        let issued = self
            .db
            .issue_crawl_task_token(
                &organization_id,
                Some(&certificate_id),
                &token_hash,
                &scope_json,
                &expires_at,
                &created_by_user_id,
            )
            .await?;
        Ok(IssuedCrawlTaskToken {
            token,
            expires_at: issued.expires_at,
        })
    }

    pub async fn authorize_gateway_sync_scope(
        &self,
        user_id: &str,
        organization_id: &str,
        facility_code: Option<&str>,
    ) -> Result<Option<String>> {
        match facility_code
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(facility_code) => match self.db.get_plant_by_facility_code(facility_code).await {
                Ok(_) => {
                    self.authorize_sync(user_id, organization_id, Some(facility_code))
                        .await?;
                    Ok(Some(facility_code.to_owned()))
                }
                Err(err)
                    if matches!(
                        err.downcast_ref::<DatabaseLookupError>(),
                        Some(DatabaseLookupError::PlantNotFound)
                    ) =>
                {
                    self.authorize_organization_action(
                        user_id,
                        organization_id,
                        organization_id,
                        "organization.plant.create",
                        &format!("create plant {}", facility_code),
                        Some(ReadConsistency::Strong),
                    )
                    .await?;
                    Ok(Some(facility_code.to_owned()))
                }
                Err(err) => Err(err),
            },
            None => {
                let executable = self
                    .db
                    .list_user_plant_action_facility_codes_with_consistency(
                        user_id,
                        organization_id,
                        "plant.sync.execute",
                        Some(ReadConsistency::Strong),
                    )
                    .await?;

                match executable.len() {
                    0 => {
                        self.authorize_organization_action(
                            user_id,
                            organization_id,
                            organization_id,
                            "organization.plant.create",
                            "create plants",
                            Some(ReadConsistency::Strong),
                        )
                        .await?;
                        Ok(None)
                    }
                    1 => Ok(executable.into_iter().next()),
                    _ => Err(bad_request_error(
                        "facilityCode is required when multiple executable plants are visible",
                    )),
                }
            }
        }
    }

    pub async fn consume_crawl_task_token_and_start_crawl(
        &self,
        input: StartCrawlWithTaskTokenInput,
    ) -> Result<SyncRun> {
        let organization_id = require_non_empty("organizationId", &input.organization_id)?;
        let token = require_non_empty("token", &input.token)?;
        let token_hash = hash_hex(&token);
        let issued = self
            .db
            .get_crawl_task_token_by_hash(&token_hash)
            .await
            .map_err(map_missing_crawl_task_token)?;
        if is_expired(&issued.expires_at)? {
            return Err(bad_request_error("crawl task token has expired"));
        }
        if issued.organization_id != organization_id {
            return Err(forbidden_error(format!(
                "forbidden: crawl task token organization {} does not match request organization {}",
                issued.organization_id, organization_id
            )));
        }
        if issued.used_at.is_some() {
            anyhow::bail!("crawl task token already used");
        }
        let organization = self.db.get_organization(&organization_id).await?;
        if organization.inactivated_at.is_some() {
            return Err(forbidden_error(format!(
                "forbidden: organization {organization_id} is inactive"
            )));
        }
        let scope: CrawlTaskScope =
            serde_json::from_str(&issued.scope_json).context("parse crawl task scope")?;
        let mode = parse_sync_mode(&scope.mode)?;
        validate_sync_request_shape(
            &mode,
            scope.start_month.as_deref(),
            scope.end_month.as_deref(),
        )?;
        let authorized_facility_code = self
            .authorize_gateway_sync_scope(
                &issued.created_by_user_id,
                &organization_id,
                scope.facility_code.as_deref(),
            )
            .await?;
        if let Some(facility_code) = authorized_facility_code.as_deref() {
            if self
                .db
                .get_plant_by_facility_code(facility_code)
                .await
                .is_ok()
            {
                self.authorize_stored_certificate_use_for_sync(
                    &issued.created_by_user_id,
                    &organization_id,
                    facility_code,
                )
                .await?;
            }
        }
        let certificate = if let Some(certificate_id) = issued.certificate_id.as_deref() {
            let certificate = self
                .db
                .get_certificate_by_id(certificate_id)
                .await
                .map_err(map_missing_certificate)?;
            if organization.party_id.as_deref() != Some(certificate.party_id.as_str()) {
                anyhow::bail!("crawl task token certificate party mismatch");
            }
            if certificate.active != 1 {
                return Err(bad_request_error("certificate is inactive"));
            }
            Some(certificate)
        } else {
            None
        };
        let consumed = self.db.consume_crawl_task_token(&token_hash).await?;

        if let Some(certificate) = certificate {
            let business_no = self
                .resolve_organization_business_no(&consumed.organization_id, None)
                .await?;
            let kms = EnvKmsProvider::from_config(&self.cfg.kms)?;
            let dek = kms.unwrap(&certificate.kms_key_id, &certificate.wrapped_dek)?;
            let bundle = decrypt_bundle_with_key(&dek, &certificate.encrypted_blob)?;
            self.db
                .touch_certificate_last_used_at_if_stale(&certificate)
                .await?;

            return self
                .run_sync_with_bundle(
                    &consumed.organization_id,
                    Some(issued.created_by_user_id.as_str()),
                    true,
                    business_no,
                    bundle,
                    authorized_facility_code,
                    scope.start_month,
                    scope.end_month,
                    mode,
                )
                .await;
        }
        anyhow::bail!("crawl task token missing certificate");
    }

    pub async fn sync(&self, input: SyncInput) -> Result<SyncRun> {
        self.sync_with_initiator(input, None).await
    }

    async fn sync_with_initiator(
        &self,
        input: SyncInput,
        created_by_user_id: Option<&str>,
    ) -> Result<SyncRun> {
        let organization_id = require_non_empty("organizationId", &input.organization_id)?;
        let business_no = self
            .resolve_organization_business_no(&organization_id, None)
            .await?;
        let cert = self.db.load_active_certificate(&organization_id).await?;
        self.db
            .touch_certificate_last_used_at_if_stale(&cert)
            .await?;
        let kms = EnvKmsProvider::from_config(&self.cfg.kms)?;
        let dek = kms.unwrap(&cert.kms_key_id, &cert.wrapped_dek)?;
        let bundle = decrypt_bundle_with_key(&dek, &cert.encrypted_blob)?;

        self.run_sync_with_bundle(
            &organization_id,
            created_by_user_id,
            true,
            business_no,
            bundle,
            input.facility_code,
            input.start_month,
            input.end_month,
            input.mode,
        )
        .await
    }

    async fn finish_failed_sync_run(
        &self,
        run_id: &str,
        duration_ms: i64,
        facility_code: Option<&str>,
        asset_id: Option<&str>,
        message: &str,
    ) {
        if let Some(facility_code) = facility_code {
            if let Err(err) = self
                .db
                .upsert_sync_run_item(
                    run_id,
                    asset_id,
                    facility_code,
                    "failed",
                    0,
                    0,
                    0,
                    0,
                    Some(message),
                )
                .await
            {
                warn!(
                    run_id,
                    facility_code,
                    error = %err,
                    "failed to record sync run item failure"
                );
            }
        }
        if let Err(err) = self
            .db
            .finish_sync_run(run_id, "failed", duration_ms, Some(message))
            .await
        {
            warn!(run_id, error = %err, "failed to mark sync run failed");
        }
    }

    async fn run_sync_with_bundle(
        &self,
        organization_id: &str,
        created_by_user_id: Option<&str>,
        requires_stored_credential: bool,
        business_no: String,
        bundle: CertificateBundle,
        facility_code: Option<String>,
        start_month: Option<String>,
        end_month: Option<String>,
        requested_mode: SyncMode,
    ) -> Result<SyncRun> {
        let input = SyncInput {
            organization_id: organization_id.to_owned(),
            facility_code,
            start_month,
            end_month,
            mode: requested_mode,
        };

        let (effective_mode, start_month, end_month, skip_documents) =
            self.resolve_sync_plan(&input).await?;
        info!(
            business_no,
            facility_code = input.facility_code.as_deref().unwrap_or(""),
            mode = ?effective_mode,
            start_month = start_month.as_deref().unwrap_or(""),
            end_month = end_month.as_deref().unwrap_or(""),
            skip_documents,
            "starting sync"
        );
        let requested_plant_id = match input.facility_code.as_deref() {
            Some(facility_code) => self
                .db
                .get_plant_by_facility_code_with_consistency(
                    facility_code,
                    Some(ReadConsistency::Strong),
                )
                .await
                .ok()
                .map(|plant| plant.id),
            None => None,
        };
        let run = self
            .db
            .start_sync_run(
                &organization_id,
                requested_plant_id.as_deref(),
                effective_mode.clone(),
            )
            .await?;

        let started = std::time::Instant::now();
        let collect = self
            .collect_snapshot(
                business_no.clone(),
                input.facility_code.clone(),
                start_month.clone(),
                end_month.clone(),
                skip_documents,
                bundle,
            )
            .await;

        match collect {
            Ok(run_output) => {
                let snapshot = run_output.snapshot;
                let mut item_facility_code = input
                    .facility_code
                    .clone()
                    .unwrap_or_else(|| snapshot.facility.code.clone());
                let mut item_asset_id: Option<String> = None;
                let materialize_result: Result<SyncRun> = async {
                    if let Some(requested_facility_code) = input.facility_code.as_deref() {
                        if snapshot.facility.code != requested_facility_code {
                            anyhow::bail!(
                                "facility code mismatch: requested {}, collector returned {}",
                                requested_facility_code,
                                snapshot.facility.code
                            );
                        }
                    }
                    item_facility_code = snapshot.facility.code.clone();
                    info!(
                        business_no,
                        plant_code = snapshot.facility.code,
                        generation_count = snapshot.generation.len(),
                        rate_count = snapshot.rates.len(),
                        rec_count = snapshot.rec.len(),
                        document_count = snapshot.documents.len(),
                        "collector finished"
                    );
                    let existing_plant = match self
                        .db
                        .get_plant_by_facility_code_with_consistency(
                            &snapshot.facility.code,
                            Some(ReadConsistency::Strong),
                        )
                        .await
                    {
                        Ok(plant) => Some(plant),
                        Err(err)
                            if matches!(
                                err.downcast_ref::<DatabaseLookupError>(),
                                Some(DatabaseLookupError::PlantNotFound)
                            ) =>
                        {
                            None
                        }
                        Err(err) => Err(err)?,
                    };
                    if let Some(created_by_user_id) = created_by_user_id {
                        match existing_plant.as_ref() {
                            Some(_) => {
                                self.authorize_sync(
                                    created_by_user_id,
                                    organization_id,
                                    Some(&snapshot.facility.code),
                                )
                                .await?;
                                if requires_stored_credential {
                                    self.authorize_stored_certificate_use_for_sync(
                                        created_by_user_id,
                                        organization_id,
                                        &snapshot.facility.code,
                                    )
                                    .await?;
                                }
                            }
                            None => {
                                self.authorize_organization_action(
                                    created_by_user_id,
                                    organization_id,
                                    organization_id,
                                    "organization.plant.create",
                                    &format!("create plant {}", snapshot.facility.code),
                                    Some(ReadConsistency::Strong),
                                )
                                .await?;
                            }
                        }
                    }
                    let plant = Plant {
                        id: existing_plant
                            .as_ref()
                            .map(|plant| plant.id.clone())
                            .unwrap_or_else(|| kr_rps_plant_id(&snapshot.facility.code)),
                        facility_code: snapshot.facility.code.clone(),
                        name: snapshot.facility.name.clone(),
                        owner_organization_id: existing_plant
                            .as_ref()
                            .map(|plant| plant.owner_organization_id.clone())
                            .unwrap_or_else(|| organization_id.to_owned()),
                        created_by_organization_id: existing_plant
                            .as_ref()
                            .and_then(|plant| plant.created_by_organization_id.clone())
                            .or_else(|| Some(organization_id.to_owned())),
                        contract_no: snapshot.facility.contract_no.clone(),
                        address: snapshot.facility.address.clone(),
                        installed_kw: snapshot.facility.installed_kw,
                        rec_start_date: snapshot.facility.rec_start_date.clone(),
                        confirm_issued: snapshot.facility.confirm_issued.clone(),
                    };
                    let generation: Vec<GenerationRecord> = snapshot
                        .generation
                        .iter()
                        .map(|row| GenerationRecord {
                            month: row.month.clone(),
                            generation_kwh: row.generation_kwh,
                            self_consumption_kwh: row.self_consumption_kwh,
                            supply_kwh: row.supply_kwh,
                            application_deadline: row.application_deadline.clone(),
                        })
                        .collect();
                    let rates: Vec<StoredRateRecord> =
                        snapshot.rates.iter().map(map_rate_record).collect();
                    let rec: Vec<RecRecord> = snapshot.rec.iter().map(map_rec_record).collect();
                    let registry_account_scope = if plant.owner_organization_id == organization_id {
                        Some((organization_id, business_no.as_str()))
                    } else {
                        None
                    };
                    let (existing_objects, persisted_docs) = if effective_mode == SyncMode::Full {
                        let existing_objects = self.db.list_document_objects(&plant.id).await?;
                        let existing_object_keys =
                            existing_objects.iter().map(String::as_str).collect();
                        info!(
                            plant_code = plant.facility_code,
                            document_count = snapshot.documents.len(),
                            "persisting documents to object storage"
                        );
                        let persisted_docs = self
                            .persist_documents(
                                organization_id,
                                &plant.id,
                                &snapshot.documents,
                                &existing_object_keys,
                            )
                            .await?;
                        info!(
                            plant_code = plant.facility_code,
                            document_count = persisted_docs.len(),
                            "writing document rows"
                        );
                        (existing_objects, persisted_docs)
                    } else {
                        info!(plant_code = plant.facility_code, "upserting delta records");
                        (Vec::new(), Vec::new())
                    };
                    let duration_ms = started.elapsed().as_millis() as i64;
                    let asset_id = match self
                        .db
                        .materialize_kr_rps_sync(KrRpsSyncMaterialization {
                            run_id: &run.id,
                            plant: &plant,
                            registry_account_organization_id: registry_account_scope
                                .map(|(organization_id, _)| organization_id),
                            business_no: registry_account_scope.map(|(_, business_no)| business_no),
                            mode: &effective_mode,
                            generation: &generation,
                            rates: &rates,
                            rec: &rec,
                            documents: &persisted_docs,
                            duration_ms,
                        })
                        .await
                    {
                        Ok(asset_id) => asset_id,
                        Err(err) => {
                            if effective_mode == SyncMode::Full {
                                let existing_object_keys =
                                    existing_objects.iter().map(String::as_str).collect();
                                self.cleanup_unmaterialized_documents(
                                    &persisted_docs,
                                    &existing_object_keys,
                                )
                                .await;
                            }
                            return Err(err);
                        }
                    };
                    if existing_plant.is_none() {
                        if let Some(created_by_user_id) = created_by_user_id {
                            let persisted_plant = self
                                .db
                                .get_plant_by_id_with_consistency(
                                    &plant.id,
                                    Some(ReadConsistency::Strong),
                                )
                                .await?;
                            if persisted_plant.owner_organization_id == organization_id {
                                self.bootstrap_created_plant_bindings(
                                    created_by_user_id,
                                    organization_id,
                                    &persisted_plant.owner_organization_id,
                                    None,
                                    &persisted_plant,
                                    &asset_id,
                                )
                                .await?;
                            }
                        }
                    }
                    if effective_mode == SyncMode::Full {
                        let current_objects: BTreeSet<&str> = persisted_docs
                            .iter()
                            .map(|doc| doc.object_key.as_str())
                            .collect();
                        let mut stale_deletes = tokio::task::JoinSet::new();
                        let mut active_deletes = 0usize;
                        for object_key in existing_objects {
                            if current_objects.contains(object_key.as_str()) {
                                continue;
                            }
                            while active_deletes >= STALE_OBJECT_DELETE_CONCURRENCY {
                                match stale_deletes.join_next().await {
                                    Some(result) => {
                                        active_deletes = active_deletes.saturating_sub(1);
                                        log_stale_delete_result(result);
                                    }
                                    None => {
                                        active_deletes = 0;
                                    }
                                }
                            }
                            let artifacts = self.artifacts.clone();
                            stale_deletes.spawn(async move {
                                let result = artifacts.delete_object(&object_key).await;
                                (object_key, result)
                            });
                            active_deletes += 1;
                        }
                        while let Some(result) = stale_deletes.join_next().await {
                            log_stale_delete_result(result);
                        }
                    }
                    let duration_ms = started.elapsed().as_millis() as i64;
                    let finished_at = now_string();
                    if let Err(err) = self
                        .db
                        .refresh_completed_sync_run_timing(&run.id, duration_ms, &finished_at)
                        .await
                    {
                        warn!(
                            run_id = run.id.as_str(),
                            error = %err,
                            "failed to refresh completed sync run timing"
                        );
                    }
                    item_asset_id = Some(asset_id.clone());
                    info!(
                        "sync completed organization={} plant={} mode={:?} duration_ms={}",
                        organization_id, plant.facility_code, effective_mode, duration_ms
                    );

                    Ok(SyncRun {
                        id: run.id.clone(),
                        organization_id: organization_id.to_owned(),
                        plant_id: Some(plant.id.clone()),
                        mode: effective_mode.clone(),
                        status: "completed".to_owned(),
                        started_at: run.started_at.clone(),
                        finished_at: Some(finished_at),
                        duration_ms,
                    })
                }
                .await;

                match materialize_result {
                    Ok(sync_run) => Ok(sync_run),
                    Err(err) => {
                        let duration_ms = started.elapsed().as_millis() as i64;
                        let message = format!("{err:#}");
                        error!(
                            business_no,
                            facility_code = item_facility_code.as_str(),
                            mode = ?effective_mode,
                            duration_ms,
                            error = %message,
                            "sync failed"
                        );
                        self.finish_failed_sync_run(
                            &run.id,
                            duration_ms,
                            Some(&item_facility_code),
                            item_asset_id.as_deref(),
                            &message,
                        )
                        .await;
                        Err(err)
                    }
                }
            }
            Err(err) => {
                let duration_ms = started.elapsed().as_millis() as i64;
                let message = format!("{err:#}");
                error!(
                    business_no,
                    facility_code = input.facility_code.as_deref().unwrap_or(""),
                    mode = ?effective_mode,
                    duration_ms,
                    error = %message,
                    "sync failed"
                );
                self.finish_failed_sync_run(
                    &run.id,
                    duration_ms,
                    input.facility_code.as_deref(),
                    None,
                    &message,
                )
                .await;
                Err(err)
            }
        }
    }

    pub async fn sync_authorized(
        &self,
        user_id: &str,
        organization_id: &str,
        input: SyncInput,
    ) -> Result<SyncRun> {
        self.authorize_sync(user_id, organization_id, input.facility_code.as_deref())
            .await?;
        if let Some(facility_code) = input.facility_code.as_deref() {
            self.authorize_stored_certificate_use_for_sync(user_id, organization_id, facility_code)
                .await?;
        }
        self.sync_with_initiator(input, Some(user_id)).await
    }

    pub async fn authorize_sync(
        &self,
        user_id: &str,
        organization_id: &str,
        facility_code: Option<&str>,
    ) -> Result<()> {
        let facility_code =
            facility_code.ok_or_else(|| anyhow::anyhow!("facilityCode is required"))?;
        self.authorize_plant_action_by_facility_code(
            user_id,
            organization_id,
            facility_code,
            "plant.sync.execute",
            "execute sync for plant",
            Some(ReadConsistency::Strong),
        )
        .await?;
        Ok(())
    }

    async fn authorize_stored_certificate_use_for_sync(
        &self,
        user_id: &str,
        organization_id: &str,
        facility_code: &str,
    ) -> Result<()> {
        let asset_id = self
            .db
            .get_kr_rps_asset_id_by_facility_code_with_consistency(
                facility_code,
                Some(ReadConsistency::Strong),
            )
            .await?;
        let allowed = self
            .db
            .has_user_asset_action_with_consistency(
                user_id,
                organization_id,
                &asset_id,
                "asset.credential.use",
                Some(ReadConsistency::Strong),
            )
            .await?;
        if !allowed {
            anyhow::bail!(
                "forbidden: user {} in organization {} cannot use stored credential for plant {}",
                user_id,
                organization_id,
                facility_code
            );
        }
        let has_credential = self
            .db
            .has_usable_kr_rps_certificate_credential_with_consistency(
                organization_id,
                facility_code,
                Some(ReadConsistency::Strong),
            )
            .await?;
        if !has_credential {
            anyhow::bail!(
                "forbidden: organization {} has no usable stored KR RPS credential for plant {}",
                organization_id,
                facility_code
            );
        }
        Ok(())
    }

    pub async fn list_visible_plants(
        &self,
        user_id: &str,
        organization_id: &str,
        read_consistency: Option<ReadConsistency>,
    ) -> Result<Vec<Plant>> {
        self.db
            .list_visible_plants_with_consistency(user_id, organization_id, read_consistency)
            .await
    }

    pub async fn get_plant(&self, facility_code: &str) -> Result<PlantView> {
        let summary = self.db.get_plant_summary(facility_code).await?;
        Ok(map_plant_summary(summary))
    }

    pub async fn get_plant_profile(
        &self,
        user_id: &str,
        organization_id: &str,
        facility_code: &str,
        read_consistency: Option<ReadConsistency>,
    ) -> Result<PlantView> {
        let plant = self
            .authorize_plant_action_by_facility_code(
                user_id,
                organization_id,
                facility_code,
                "plant.profile.read",
                "read plant profile",
                read_consistency.clone(),
            )
            .await?;

        let summary = self
            .db
            .get_plant_summary_by_id_with_consistency(&plant.id, read_consistency)
            .await?;
        Ok(map_plant_summary(summary))
    }

    pub async fn create_plant(
        &self,
        user_id: &str,
        organization_id: &str,
        input: CreatePlantInput,
    ) -> Result<Plant> {
        let facility_code = require_non_empty("facilityCode", &input.facility_code)?;
        let name = require_non_empty("name", &input.name)?;
        let contract_no = require_non_empty("contractNo", &input.contract_no)?;
        let address = require_non_empty("address", &input.address)?;
        let rec_start_date = require_non_empty("recStartDate", &input.rec_start_date)?;
        let confirm_issued = require_non_empty("confirmIssued", &input.confirm_issued)?;
        let owner_organization_id = input
            .owner_organization_id
            .as_deref()
            .map(|value| require_non_empty("ownerOrganizationId", value))
            .transpose()?
            .unwrap_or_else(|| organization_id.to_owned());

        let can_create = self
            .db
            .has_user_organization_action(
                user_id,
                organization_id,
                organization_id,
                "organization.plant.create",
            )
            .await?;
        if !can_create {
            anyhow::bail!(
                "forbidden: user {} in organization {} cannot create plants",
                user_id,
                organization_id
            );
        }

        if owner_organization_id != organization_id {
            let can_create_for_owner = self
                .db
                .has_user_organization_action(
                    user_id,
                    organization_id,
                    organization_id,
                    "organization.plant.create_for_owner",
                )
                .await?;
            if !can_create_for_owner {
                anyhow::bail!(
                    "forbidden: user {} in organization {} cannot create plants for owner {}",
                    user_id,
                    organization_id,
                    owner_organization_id
                );
            }
        }

        if self
            .db
            .get_plant_by_facility_code(&facility_code)
            .await
            .is_ok()
        {
            anyhow::bail!("plant {} already exists", facility_code);
        }

        let owner_agreement_id = if owner_organization_id == organization_id {
            None
        } else {
            self.db
                .find_active_agreement_id(&owner_organization_id, organization_id)
                .await?
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "forbidden: no active agreement from owner {} to organization {}",
                        owner_organization_id,
                        organization_id
                    )
                })?
                .into()
        };

        let plant = Plant {
            id: kr_rps_plant_id(&facility_code),
            facility_code,
            name,
            owner_organization_id: owner_organization_id.clone(),
            created_by_organization_id: Some(organization_id.to_owned()),
            contract_no,
            address,
            installed_kw: input.installed_kw,
            rec_start_date,
            confirm_issued,
        };

        self.db.upsert_plant(&plant).await?;
        let asset_id = self
            .db
            .upsert_kr_rps_asset_projection(&plant, None, None, &[], &[])
            .await?;
        self.bootstrap_created_plant_bindings(
            user_id,
            organization_id,
            &owner_organization_id,
            owner_agreement_id.as_deref(),
            &plant,
            &asset_id,
        )
        .await?;

        Ok(plant)
    }

    pub async fn list_clients_authorized(
        &self,
        user_id: &str,
        organization_id: &str,
        read_consistency: Option<ReadConsistency>,
    ) -> Result<Vec<Client>> {
        self.authorize_organization_action(
            user_id,
            organization_id,
            organization_id,
            "organization.client.manage",
            "manage organization clients",
            read_consistency.clone(),
        )
        .await?;
        self.db
            .list_clients_with_consistency(organization_id, read_consistency)
            .await
    }

    pub async fn get_client_authorized(
        &self,
        user_id: &str,
        organization_id: &str,
        client_id: &str,
        read_consistency: Option<ReadConsistency>,
    ) -> Result<Client> {
        self.authorize_organization_action(
            user_id,
            organization_id,
            organization_id,
            "organization.client.manage",
            "manage organization clients",
            read_consistency.clone(),
        )
        .await?;
        let client = self
            .db
            .get_client_with_consistency(client_id, read_consistency)
            .await?;
        if client.organization_id != organization_id {
            return Err(not_found_error(format!(
                "client not found: {} in organization {}",
                client_id, organization_id
            )));
        }
        Ok(client)
    }

    pub async fn create_client(
        &self,
        user_id: &str,
        organization_id: &str,
        input: CreateClientInput,
    ) -> Result<Client> {
        self.authorize_organization_action(
            user_id,
            organization_id,
            organization_id,
            "organization.client.manage",
            "manage organization clients",
            Some(ReadConsistency::Strong),
        )
        .await?;
        let client_id = require_non_empty("id", &input.id)?;
        let slug = require_non_empty("slug", &input.slug)?;
        let client = Client {
            id: client_id,
            organization_id: organization_id.to_owned(),
            slug,
            name: require_non_empty("name", &input.name)?,
            description: normalize_optional_text(input.description),
            client_type: input.client_type,
            app_type: input.app_type,
            first_party: input.first_party,
            consent_policy: input.consent_policy,
            active: input.active,
            created_by_user_id: user_id.to_owned(),
        };
        self.db.insert_client(&client).await?;
        Ok(client)
    }

    pub async fn update_client(
        &self,
        user_id: &str,
        organization_id: &str,
        client_id: &str,
        input: UpdateClientInput,
    ) -> Result<Client> {
        let existing = self
            .get_client_authorized(
                user_id,
                organization_id,
                client_id,
                Some(ReadConsistency::Strong),
            )
            .await?;
        let slug = require_non_empty("slug", &input.slug)?;
        let client = Client {
            id: existing.id,
            organization_id: existing.organization_id,
            slug,
            name: require_non_empty("name", &input.name)?,
            description: normalize_optional_text(input.description),
            client_type: input.client_type,
            app_type: input.app_type,
            first_party: input.first_party,
            consent_policy: input.consent_policy,
            active: input.active,
            created_by_user_id: existing.created_by_user_id,
        };
        self.db.update_client(&client).await?;
        Ok(client)
    }

    pub async fn list_client_actions_authorized(
        &self,
        user_id: &str,
        organization_id: &str,
        client_id: &str,
        read_consistency: Option<ReadConsistency>,
    ) -> Result<Vec<Action>> {
        self.get_client_authorized(
            user_id,
            organization_id,
            client_id,
            read_consistency.clone(),
        )
        .await?;
        self.db
            .list_client_actions_with_consistency(client_id, read_consistency)
            .await
    }

    pub async fn replace_client_actions_authorized(
        &self,
        user_id: &str,
        organization_id: &str,
        client_id: &str,
        action_ids: &[String],
    ) -> Result<Vec<Action>> {
        self.get_client_authorized(
            user_id,
            organization_id,
            client_id,
            Some(ReadConsistency::Strong),
        )
        .await?;
        if action_ids.len() > MAX_CLIENT_ACTION_IDS_PER_REQUEST {
            return Err(bad_request_error(format!(
                "actionIds must contain at most {MAX_CLIENT_ACTION_IDS_PER_REQUEST} items"
            )));
        }
        self.db.replace_client_actions(client_id, action_ids).await
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
    pub async fn list_generation_authorized(
        &self,
        user_id: &str,
        organization_id: &str,
        facility_code: &str,
        read_consistency: Option<ReadConsistency>,
    ) -> Result<Vec<GenerationRecord>> {
        let plant = self
            .authorize_plant_action_by_facility_code(
                user_id,
                organization_id,
                facility_code,
                "plant.telemetry.read",
                "read plant generation",
                read_consistency.clone(),
            )
            .await?;
        self.db
            .list_generation_with_consistency(&plant.id, read_consistency)
            .await
    }

    pub async fn list_rates_authorized(
        &self,
        user_id: &str,
        organization_id: &str,
        facility_code: &str,
        read_consistency: Option<ReadConsistency>,
    ) -> Result<Vec<RateRecord>> {
        let plant = self
            .authorize_plant_action_by_facility_code(
                user_id,
                organization_id,
                facility_code,
                "plant.rate.read",
                "read plant rates",
                read_consistency.clone(),
            )
            .await?;
        self.db
            .list_rates_with_consistency(&plant.id, read_consistency)
            .await
    }

    pub async fn list_rec_authorized(
        &self,
        user_id: &str,
        organization_id: &str,
        facility_code: &str,
        read_consistency: Option<ReadConsistency>,
    ) -> Result<Vec<RecRecord>> {
        let plant = self
            .authorize_plant_action_by_facility_code(
                user_id,
                organization_id,
                facility_code,
                "plant.rec.read",
                "read plant rec",
                read_consistency.clone(),
            )
            .await?;
        self.db
            .list_rec_with_consistency(&plant.id, read_consistency)
            .await
    }

    pub async fn list_documents_authorized(
        &self,
        user_id: &str,
        organization_id: &str,
        facility_code: &str,
        read_consistency: Option<ReadConsistency>,
    ) -> Result<Vec<DocumentArtifact>> {
        let plant = self
            .authorize_plant_action_by_facility_code(
                user_id,
                organization_id,
                facility_code,
                "plant.document.read",
                "read plant documents",
                read_consistency.clone(),
            )
            .await?;
        let docs = self
            .db
            .list_documents_with_consistency(&plant.id, read_consistency)
            .await?;
        Ok(docs
            .into_iter()
            .map(|mut doc| {
                doc.artifact.download_url =
                    Some(format!("/api/v1/documents/{}/download", doc.artifact.id));
                doc.artifact
            })
            .collect())
    }

    pub async fn download_document_authorized(
        &self,
        user_id: &str,
        organization_id: &str,
        document_id: &str,
        read_consistency: Option<ReadConsistency>,
    ) -> Result<(StoredDocument, bytes::Bytes)> {
        let doc = self
            .db
            .get_document_with_consistency(document_id, read_consistency.clone())
            .await?;
        self.authorize_plant_action_by_id_with_asset_fallback(
            user_id,
            organization_id,
            &doc.plant_id,
            "plant.document.read",
            &format!("read document {}", doc.artifact.file_name),
            read_consistency,
        )
        .await?;
        let bytes = self.artifacts.get_bytes(&doc.object_key).await?;
        Ok((doc, bytes))
    }

    pub async fn trigger_backup(&self) -> Result<()> {
        self.db.trigger_backup().await
    }

    async fn authorize_plant_action_by_facility_code(
        &self,
        user_id: &str,
        organization_id: &str,
        facility_code: &str,
        action_id: &str,
        action_label: &str,
        read_consistency: Option<ReadConsistency>,
    ) -> Result<Plant> {
        let plant = self
            .db
            .get_plant_by_facility_code_with_consistency(facility_code, read_consistency.clone())
            .await?;
        let plant_allowed = self
            .db
            .has_user_plant_action_with_consistency(
                user_id,
                organization_id,
                &plant.id,
                action_id,
                read_consistency.clone(),
            )
            .await?;
        if plant_allowed {
            return Ok(plant);
        }
        let asset_action_id = kr_rps_plant_action_to_asset_action(action_id);
        let asset_allowed = if let Some(asset_action_id) = asset_action_id {
            match self
                .db
                .get_kr_rps_asset_id_by_facility_code_with_consistency(
                    facility_code,
                    read_consistency.clone(),
                )
                .await
            {
                Ok(asset_id) => {
                    self.db
                        .has_user_asset_action_with_consistency(
                            user_id,
                            organization_id,
                            &asset_id,
                            asset_action_id,
                            read_consistency.clone(),
                        )
                        .await?
                }
                Err(err)
                    if matches!(
                        err.downcast_ref::<DatabaseLookupError>(),
                        Some(DatabaseLookupError::PlantNotFound)
                    ) =>
                {
                    false
                }
                Err(err) => return Err(err),
            }
        } else {
            false
        };
        if !asset_allowed {
            anyhow::bail!(
                "forbidden: user {} in organization {} cannot {}",
                user_id,
                organization_id,
                &format!("{action_label} {}", facility_code)
            );
        }
        Ok(plant)
    }

    async fn authorize_plant_action_by_id_with_asset_fallback(
        &self,
        user_id: &str,
        organization_id: &str,
        plant_id: &str,
        action_id: &str,
        target_label: &str,
        read_consistency: Option<ReadConsistency>,
    ) -> Result<()> {
        let allowed = self
            .db
            .has_user_plant_action_with_consistency(
                user_id,
                organization_id,
                plant_id,
                action_id,
                read_consistency.clone(),
            )
            .await?;
        if allowed {
            return Ok(());
        }

        let plant = self
            .db
            .get_plant_by_id_with_consistency(plant_id, read_consistency.clone())
            .await?;
        let asset_action_id = kr_rps_plant_action_to_asset_action(action_id);
        let asset_allowed = if let Some(asset_action_id) = asset_action_id {
            match self
                .db
                .get_kr_rps_asset_id_by_facility_code_with_consistency(
                    &plant.facility_code,
                    read_consistency.clone(),
                )
                .await
            {
                Ok(asset_id) => {
                    self.db
                        .has_user_asset_action_with_consistency(
                            user_id,
                            organization_id,
                            &asset_id,
                            asset_action_id,
                            read_consistency,
                        )
                        .await?
                }
                Err(err)
                    if matches!(
                        err.downcast_ref::<DatabaseLookupError>(),
                        Some(DatabaseLookupError::PlantNotFound)
                    ) =>
                {
                    false
                }
                Err(err) => return Err(err),
            }
        } else {
            false
        };
        if !asset_allowed {
            anyhow::bail!(
                "forbidden: user {} in organization {} cannot {}",
                user_id,
                organization_id,
                target_label
            );
        }
        Ok(())
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
        let items = self.db.list_backup_local().await?;
        Ok(items
            .into_iter()
            .map(|item| BackupInfo {
                name: item.name,
                last_modified: item.last_modified,
                size: item.size,
            })
            .collect())
    }

    pub async fn list_backups_s3(&self) -> Result<Vec<BackupInfo>> {
        let items = self.db.list_backup_s3().await?;
        Ok(items
            .into_iter()
            .map(|item| BackupInfo {
                name: item.name,
                last_modified: item.last_modified,
                size: item.size,
            })
            .collect())
    }

    async fn collect_snapshot(
        &self,
        business_no: String,
        facility_code: Option<String>,
        start_month: Option<String>,
        end_month: Option<String>,
        skip_documents: bool,
        bundle: CertificateBundle,
    ) -> Result<bridge::BridgeRun> {
        (self.collector)(BridgeOptions {
            business_no,
            facility_code,
            start_month,
            end_month,
            skip_documents,
            bundle,
        })
        .await
    }

    async fn resolve_sync_plan(
        &self,
        input: &SyncInput,
    ) -> Result<(SyncMode, Option<String>, Option<String>, bool)> {
        match input.mode {
            SyncMode::Full => {
                if input.start_month.is_some() || input.end_month.is_some() {
                    return Err(bad_request_error(
                        "full sync cannot be bounded by startMonth or endMonth",
                    ));
                }
                Ok((SyncMode::Full, None, None, false))
            }
            SyncMode::Delta => {
                let Some(facility_code) = input.facility_code.as_deref() else {
                    return Ok((SyncMode::Full, None, None, false));
                };
                let plant = match self
                    .db
                    .get_plant_by_facility_code_with_consistency(
                        facility_code,
                        Some(ReadConsistency::Strong),
                    )
                    .await
                {
                    Ok(plant) => plant,
                    Err(err)
                        if matches!(
                            err.downcast_ref::<DatabaseLookupError>(),
                            Some(DatabaseLookupError::PlantNotFound)
                        ) =>
                    {
                        return Ok((SyncMode::Full, None, None, false));
                    }
                    Err(err) => return Err(err),
                };
                let Some(latest_month) = self.db.latest_generation_month(&plant.id).await? else {
                    return Ok((SyncMode::Full, None, None, false));
                };
                let end_month = current_month_seoul();
                Ok((SyncMode::Delta, Some(latest_month), Some(end_month), true))
            }
        }
    }

    async fn bootstrap_created_plant_bindings(
        &self,
        user_id: &str,
        creator_organization_id: &str,
        owner_organization_id: &str,
        creator_agreement_id: Option<&str>,
        plant: &Plant,
        asset_id: &str,
    ) -> Result<()> {
        self.ensure_system_role(
            "role-system-plant-owner",
            "system.plant.owner",
            "System Plant Owner",
            &[
                "plant.profile.read",
                "plant.profile.write",
                "plant.sync.execute",
                "plant.permission.manage",
            ],
        )
        .await?;
        self.ensure_system_role(
            "role-system-plant-creator",
            "system.plant.creator",
            "System Plant Creator",
            &["plant.profile.read", "plant.sync.execute"],
        )
        .await?;
        self.ensure_system_role(
            "role-system-asset-owner",
            "system.asset.owner",
            "System Asset Owner",
            &[
                "asset.profile.read",
                "asset.profile.write",
                "asset.sync.execute",
                "asset.credential.use",
                "asset.permission.manage",
            ],
        )
        .await?;
        self.ensure_system_role(
            "role-system-asset-creator",
            "system.asset.creator",
            "System Asset Creator",
            &[
                "asset.profile.read",
                "asset.sync.execute",
                "asset.credential.use",
            ],
        )
        .await?;

        let now = Utc::now().to_rfc3339();
        self.db
            .save_role_binding(&RoleBinding {
                id: Uuid::new_v4().simple().to_string(),
                subject_type: RoleBindingSubjectType::Organization,
                subject_id: owner_organization_id.to_owned(),
                subject_organization_id: owner_organization_id.to_owned(),
                resource_type: RoleBindingResourceType::Plant,
                resource_id: plant.id.clone(),
                role_id: "role-system-plant-owner".to_owned(),
                agreement_id: None,
                state: RoleBindingState::Active,
                created_by_user_id: user_id.to_owned(),
                created_at: now.clone(),
                updated_at: now.clone(),
            })
            .await?;
        self.db
            .save_role_binding(&RoleBinding {
                id: Uuid::new_v4().simple().to_string(),
                subject_type: RoleBindingSubjectType::Organization,
                subject_id: owner_organization_id.to_owned(),
                subject_organization_id: owner_organization_id.to_owned(),
                resource_type: RoleBindingResourceType::Asset,
                resource_id: asset_id.to_owned(),
                role_id: "role-system-asset-owner".to_owned(),
                agreement_id: None,
                state: RoleBindingState::Active,
                created_by_user_id: user_id.to_owned(),
                created_at: now.clone(),
                updated_at: now.clone(),
            })
            .await?;

        if creator_organization_id != owner_organization_id {
            self.db
                .save_role_binding(&RoleBinding {
                    id: Uuid::new_v4().simple().to_string(),
                    subject_type: RoleBindingSubjectType::Organization,
                    subject_id: creator_organization_id.to_owned(),
                    subject_organization_id: creator_organization_id.to_owned(),
                    resource_type: RoleBindingResourceType::Plant,
                    resource_id: plant.id.clone(),
                    role_id: "role-system-plant-creator".to_owned(),
                    agreement_id: creator_agreement_id.map(ToOwned::to_owned),
                    state: RoleBindingState::Active,
                    created_by_user_id: user_id.to_owned(),
                    created_at: now.clone(),
                    updated_at: now.clone(),
                })
                .await?;
            self.db
                .save_role_binding(&RoleBinding {
                    id: Uuid::new_v4().simple().to_string(),
                    subject_type: RoleBindingSubjectType::Organization,
                    subject_id: creator_organization_id.to_owned(),
                    subject_organization_id: creator_organization_id.to_owned(),
                    resource_type: RoleBindingResourceType::Asset,
                    resource_id: asset_id.to_owned(),
                    role_id: "role-system-asset-creator".to_owned(),
                    agreement_id: creator_agreement_id.map(ToOwned::to_owned),
                    state: RoleBindingState::Active,
                    created_by_user_id: user_id.to_owned(),
                    created_at: now.clone(),
                    updated_at: now.clone(),
                })
                .await?;
        }

        self.db
            .save_role_binding(&RoleBinding {
                id: Uuid::new_v4().simple().to_string(),
                subject_type: RoleBindingSubjectType::User,
                subject_id: user_id.to_owned(),
                subject_organization_id: creator_organization_id.to_owned(),
                resource_type: RoleBindingResourceType::Plant,
                resource_id: plant.id.clone(),
                role_id: "role-system-plant-creator".to_owned(),
                agreement_id: None,
                state: RoleBindingState::Active,
                created_by_user_id: user_id.to_owned(),
                created_at: now.clone(),
                updated_at: now.clone(),
            })
            .await?;
        self.db
            .save_role_binding(&RoleBinding {
                id: Uuid::new_v4().simple().to_string(),
                subject_type: RoleBindingSubjectType::User,
                subject_id: user_id.to_owned(),
                subject_organization_id: creator_organization_id.to_owned(),
                resource_type: RoleBindingResourceType::Asset,
                resource_id: asset_id.to_owned(),
                role_id: "role-system-asset-creator".to_owned(),
                agreement_id: None,
                state: RoleBindingState::Active,
                created_by_user_id: user_id.to_owned(),
                created_at: now.clone(),
                updated_at: now,
            })
            .await?;

        Ok(())
    }

    async fn ensure_system_role(
        &self,
        role_id: &str,
        code: &str,
        name: &str,
        actions: &[&str],
    ) -> Result<()> {
        self.db
            .upsert_role(&Role {
                id: role_id.to_owned(),
                organization_id: None,
                code: code.to_owned(),
                name: name.to_owned(),
                description: None,
            })
            .await?;
        for action in actions {
            self.db.upsert_role_action(role_id, action).await?;
        }
        Ok(())
    }

    async fn resolve_organization_business_no(
        &self,
        organization_id: &str,
        requested_business_no: Option<&str>,
    ) -> Result<String> {
        let linked_business_no = self
            .db
            .get_organization_business_no(organization_id)
            .await?;
        normalize_business_no(select_organization_business_no(
            organization_id,
            requested_business_no,
            linked_business_no.as_deref(),
            None,
        )?)
    }

    async fn validate_bundle_for_organization(
        &self,
        organization_id: &str,
        requested_business_no: Option<&str>,
        bundle: &CertificateBundle,
    ) -> Result<String> {
        let linked_business_no = self
            .db
            .get_organization_business_no(organization_id)
            .await?;
        let cert_business_no = extract_business_no(&bundle.sign_cert_der)?;
        let business_no = normalize_business_no(select_organization_business_no(
            organization_id,
            requested_business_no,
            linked_business_no.as_deref(),
            None,
        )?)?;
        // Some supported certificates omit subject serialNumber entirely. In that case,
        // only a linked or explicitly requested organization business number can be used;
        // the request-controlled bundle field must not bootstrap legal identity.
        if let Some(cert_business_no) = cert_business_no {
            if cert_business_no != business_no {
                return Err(bad_request_error("certificate business number mismatch"));
            }
        }
        validate_certificate_bundle(bundle, self.cfg.security.allow_untrusted_certificates)?;
        Ok(business_no)
    }

    async fn store_certificate_bundle(
        &self,
        organization_id: &str,
        business_no: String,
        mut bundle: CertificateBundle,
    ) -> Result<StoredCertificate> {
        let common_name = extract_common_name(&bundle.sign_cert_der)?;
        bundle.business_no = business_no.clone();
        bundle.common_name = common_name.clone();

        let dek = generate_data_encryption_key();
        let encrypted = encrypt_bundle_with_key(&dek, &bundle)?;
        let kms = EnvKmsProvider::from_config(&self.cfg.kms)?;
        let wrapped_dek = kms.wrap(&dek)?;
        self.db
            .save_certificate(
                organization_id,
                Some(&business_no),
                &common_name,
                &encrypted,
                &wrapped_dek,
                kms.active_key_id(),
            )
            .await
    }

    async fn validate_cert_gateway_challenge(
        &self,
        challenge_id: &str,
        challenge_payload: &str,
        challenge_signature_base64: &str,
        sign_cert_der: &[u8],
        expected_mode: &str,
        organization_id: &str,
    ) -> Result<CertGatewayChallenge> {
        let challenge = self
            .db
            .get_cert_gateway_challenge(challenge_id)
            .await
            .map_err(|err| map_missing_cert_gateway_challenge(err, challenge_id))?;
        if challenge.organization_id != organization_id {
            return Err(bad_request_error(
                "cert gateway challenge organization mismatch",
            ));
        }
        if challenge.mode != expected_mode {
            return Err(bad_request_error(format!(
                "cert gateway challenge mode mismatch: expected {}, got {}",
                expected_mode, challenge.mode
            )));
        }
        if challenge.consumed_at.is_some() {
            return Err(bad_request_error("cert gateway challenge already consumed"));
        }
        if is_expired(&challenge.expires_at)? {
            return Err(bad_request_error("cert gateway challenge has expired"));
        }
        if hash_hex(challenge_payload) != challenge.payload_hash {
            return Err(bad_request_error("cert gateway challenge payload mismatch"));
        }
        let challenge_signature_base64 =
            require_non_empty("challengeSignatureBase64", challenge_signature_base64)?;
        verify_challenge_signature(
            sign_cert_der,
            challenge_payload,
            &challenge_signature_base64,
        )
        .map_err(|err| bad_request_error(err.to_string()))?;
        Ok(challenge)
    }

    async fn persist_documents(
        &self,
        organization_id: &str,
        plant_code: &str,
        docs: &[BridgeDocument],
        existing_object_keys: &BTreeSet<&str>,
    ) -> Result<Vec<PersistedDocument>> {
        let mut persisted = Vec::with_capacity(docs.len());
        for doc in docs {
            let id = stable_document_id(organization_id, plant_code, doc);
            let category = resolved_document_category(doc);
            let source_path = Path::new(&doc.source_path);
            let uploaded_sha256 = match sha256_file_hex(source_path).await {
                Ok(value) => value,
                Err(err) => {
                    self.cleanup_unmaterialized_documents(&persisted, existing_object_keys)
                        .await;
                    return Err(err).with_context(|| format!("hash document {}", doc.file_name));
                }
            };
            for (field_name, expected_sha256) in [
                ("outputSha256", doc.output_sha256.as_deref()),
                ("sourceSha256", doc.source_sha256.as_deref()),
            ] {
                let Some(expected_sha256) = expected_sha256 else {
                    continue;
                };
                if expected_sha256 == uploaded_sha256 {
                    continue;
                }
                self.cleanup_unmaterialized_documents(&persisted, existing_object_keys)
                    .await;
                return Err(bad_request_error(format!(
                    "document {} {} mismatch: expected {}, got {}",
                    doc.file_name, field_name, expected_sha256, uploaded_sha256
                )));
            }
            let source_sha256 = doc
                .source_sha256
                .clone()
                .unwrap_or_else(|| uploaded_sha256.clone());
            let output_sha256 = doc
                .output_sha256
                .clone()
                .unwrap_or_else(|| uploaded_sha256.clone());
            let object_version = uploaded_sha256.chars().take(16).collect::<String>();
            let object_key = format!("{organization_id}/{plant_code}/{id}-{object_version}.pdf");
            info!(
                plant_code,
                file_name = doc.file_name.as_str(),
                object_key = object_key.as_str(),
                "uploading document"
            );
            if let Err(err) = self
                .artifacts
                .put_file(object_key.clone(), source_path, "application/pdf")
                .await
            {
                self.delete_unmaterialized_object(&object_key, existing_object_keys)
                    .await;
                self.cleanup_unmaterialized_documents(&persisted, existing_object_keys)
                    .await;
                return Err(err).with_context(|| format!("upload document {}", doc.file_name));
            }
            persisted.push(PersistedDocument {
                id,
                category,
                attachment_code: doc.attachment_code.clone(),
                file_class: doc.file_class.clone(),
                file_sec_name: doc.file_sec_name.clone(),
                file_name: doc.file_name.clone(),
                required: doc.required,
                source_sha256: Some(source_sha256),
                output_sha256: Some(output_sha256),
                object_key,
                content_type: "application/pdf".to_owned(),
            });
        }
        Ok(persisted)
    }

    async fn cleanup_unmaterialized_documents(
        &self,
        docs: &[PersistedDocument],
        existing_object_keys: &BTreeSet<&str>,
    ) {
        for doc in docs {
            self.delete_unmaterialized_object(&doc.object_key, existing_object_keys)
                .await;
        }
    }

    async fn delete_unmaterialized_object(
        &self,
        object_key: &str,
        existing_object_keys: &BTreeSet<&str>,
    ) {
        if existing_object_keys.contains(object_key) {
            return;
        }
        if let Err(err) = self.artifacts.delete_object(object_key).await {
            warn!(
                object_key,
                error = %err,
                "failed to delete unmaterialized document object"
            );
        }
    }
}

async fn sha256_file_hex(path: &Path) -> Result<String> {
    use tokio::io::AsyncReadExt;

    let mut file = tokio::fs::File::open(path)
        .await
        .with_context(|| format!("open document {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];
    loop {
        let read = file
            .read(&mut buffer)
            .await
            .with_context(|| format!("read document {}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn log_stale_delete_result(
    result: std::result::Result<(String, Result<()>), tokio::task::JoinError>,
) {
    match result {
        Ok((_, Ok(()))) => {}
        Ok((object_key, Err(err))) => {
            warn!("delete stale object {} failed: {}", object_key, err);
        }
        Err(err) => warn!("delete stale object task failed: {}", err),
    }
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

fn stable_document_id(organization_id: &str, plant_code: &str, doc: &BridgeDocument) -> String {
    let mut hasher = Sha256::new();
    hasher.update(organization_id.as_bytes());
    hasher.update(b":");
    hasher.update(plant_code.as_bytes());
    hasher.update(b":");
    hasher.update(doc.attachment_code.as_bytes());
    hasher.update(b":");
    hasher.update(doc.file_name.as_bytes());
    let digest = hasher.finalize();
    hex::encode(&digest[..16])
}

fn map_rate_record(row: &BridgeRateRecord) -> StoredRateRecord {
    StoredRateRecord {
        record: RateRecord {
            month: row.month.clone(),
            self_rate: row.self_rate,
            national_average: row.national_average,
            nearby_average: row.nearby_average,
            nearby_highest: row.nearby_highest,
        },
        published: row.published,
        source_control_flag: row.source_control_flag.clone(),
    }
}

fn map_rec_record(row: &bridge::BridgeRecRecord) -> RecRecord {
    RecRecord {
        month: row.month.clone(),
        application_no: row.application_no.clone(),
        supply_kwh: row.supply_kwh,
        rec_quantity: row.rec_quantity,
        fee_krw: row.fee_krw,
        issued_date: row.issued_date.clone(),
        expiry_date: row.expiry_date.clone(),
        virtual_account: row.virtual_account.clone(),
        bank_name: row.bank_name.clone(),
        status: row.status.clone(),
    }
}

fn map_document_category(value: &str) -> DocumentCategory {
    match value {
        "plant_info" => DocumentCategory::PlantInfo,
        "permit" => DocumentCategory::Permit,
        "site_land" => DocumentCategory::SiteLand,
        "drawings" => DocumentCategory::Drawings,
        "photos" => DocumentCategory::Photos,
        "equipment" => DocumentCategory::Equipment,
        "finance" => DocumentCategory::Finance,
        "development" => DocumentCategory::Development,
        _ => DocumentCategory::Other,
    }
}

fn resolved_document_category(doc: &BridgeDocument) -> DocumentCategory {
    let explicit = map_document_category(doc.category.trim());
    if explicit != DocumentCategory::Other {
        return explicit;
    }
    classify_document(
        &format!("{} {}", doc.file_sec_name.trim(), doc.file_name.trim()),
        &doc.file_class,
    )
}

fn classify_document(name_hint: &str, file_class: &str) -> DocumentCategory {
    let name = name_hint.to_lowercase();
    let class = file_class.to_lowercase();

    if contains_any(&name, &["내부사진", "현장사진", "공사사진", "사진"]) {
        DocumentCategory::Photos
    } else if contains_any(&name, &["개발행위", "준공검사"]) {
        DocumentCategory::Development
    } else if contains_any(
        &name,
        &["설치도면", "단선결선도", "현황도", "결선도", "배치도"],
    ) {
        DocumentCategory::Drawings
    } else if contains_any(&name, &["토지대장", "지적공부", "건축물대장", "등록증명서"])
    {
        DocumentCategory::SiteLand
    } else if contains_any(
        &name,
        &["허가증", "허가서", "신고필증", "사용전검사", "상업운전개시"],
    ) {
        DocumentCategory::Permit
    } else if contains_any(
        &name,
        &["ks 인증서", "구조안전", "접속함", "모듈", "인버터"],
    ) {
        DocumentCategory::Equipment
    } else if contains_any(&name, &["견적서", "세금계산서", "비용 산출내역서"]) {
        DocumentCategory::Finance
    } else if contains_any(&name, &["사업자등록증"]) {
        DocumentCategory::PlantInfo
    } else if class.contains("소요자금") {
        DocumentCategory::Finance
    } else if class.contains("개발행위") {
        DocumentCategory::Development
    } else if class.contains("설비구성") {
        DocumentCategory::Equipment
    } else if class.contains("발전소정보") {
        DocumentCategory::PlantInfo
    } else if class.contains("발전방식") {
        DocumentCategory::SiteLand
    } else {
        DocumentCategory::Other
    }
}

fn contains_any(value: &str, needles: &[&str]) -> bool {
    needles
        .iter()
        .any(|needle| value.contains(&needle.to_lowercase()))
}

fn kr_rps_plant_action_to_asset_action(action_id: &str) -> Option<&'static str> {
    match action_id {
        "plant.profile.read" => Some("asset.profile.read"),
        "plant.profile.write" => Some("asset.profile.write"),
        "plant.sync.execute" => Some("asset.sync.execute"),
        "plant.telemetry.read" => Some("asset.generation.read"),
        "plant.document.read" => Some("asset.document.read"),
        "plant.rec.read" => Some("asset.certificate.read"),
        "plant.rate.read" => Some("asset.rate.read"),
        "plant.permission.manage" => Some("asset.permission.manage"),
        _ => None,
    }
}

fn map_plant_summary(summary: PlantSummary) -> PlantView {
    let last_synced_at = summary
        .last_sync
        .as_ref()
        .and_then(|run| run.finished_at.clone().or_else(|| run.started_at.clone()));
    let last_sync = summary.last_sync;
    let document_categories = summary
        .document_categories
        .into_iter()
        .map(|(category, count)| (document_category_str(&category).to_owned(), count))
        .collect();
    PlantView {
        plant: summary.plant,
        last_synced_at,
        last_sync,
        generation_count: summary.generation_count,
        generation_total_kwh: summary.generation_total_kwh,
        rates_count: summary.rates_count,
        rec_count: summary.rec_count,
        rec_total: summary.rec_total,
        document_count: summary.document_count,
        document_categories,
    }
}

fn document_category_str(category: &DocumentCategory) -> &'static str {
    match category {
        DocumentCategory::PlantInfo => "plant_info",
        DocumentCategory::Permit => "permit",
        DocumentCategory::SiteLand => "site_land",
        DocumentCategory::Drawings => "drawings",
        DocumentCategory::Photos => "photos",
        DocumentCategory::Equipment => "equipment",
        DocumentCategory::Finance => "finance",
        DocumentCategory::Development => "development",
        DocumentCategory::Other => "other",
    }
}

fn current_month_seoul() -> String {
    let now = Utc::now().with_timezone(&Seoul);
    format!("{:04}-{:02}", now.year(), now.month())
}

fn build_challenge_payload(
    organization_id: &str,
    mode: &str,
    nonce: &str,
    facility_code: Option<&str>,
    expires_at: &str,
) -> String {
    format!(
        "cert-gateway:{organization_id}:{mode}:{}:{nonce}:{expires_at}",
        facility_code.unwrap_or("")
    )
}

fn hash_hex(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    hex::encode(hasher.finalize())
}

fn kr_rps_plant_id(facility_code: &str) -> String {
    format!(
        "plant-{}",
        &hash_hex(&format!("kr_rps:{facility_code}"))[..32]
    )
}

fn parse_sync_mode(value: &str) -> Result<SyncMode> {
    match value.trim().to_ascii_lowercase().as_str() {
        "full" => Ok(SyncMode::Full),
        "delta" => Ok(SyncMode::Delta),
        other => Err(bad_request_error(format!("unsupported sync mode: {other}"))),
    }
}

fn parse_cert_gateway_mode(value: &str) -> Result<&'static str> {
    match require_non_empty("mode", value)?
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "stored" => Ok("stored"),
        "ephemeral" => Ok("ephemeral"),
        other => Err(bad_request_error(format!(
            "unsupported cert gateway mode: {other}"
        ))),
    }
}

fn sync_mode_label(mode: &SyncMode) -> &'static str {
    match mode {
        SyncMode::Full => "full",
        SyncMode::Delta => "delta",
    }
}

fn is_expired(expires_at: &str) -> Result<bool> {
    let expires_at = DateTime::parse_from_rfc3339(expires_at)
        .with_context(|| format!("parse expiresAt {expires_at}"))?
        .with_timezone(&Utc);
    Ok(Utc::now() > expires_at)
}

fn validate_sync_request_shape(
    mode: &SyncMode,
    start_month: Option<&str>,
    end_month: Option<&str>,
) -> Result<()> {
    if matches!(mode, SyncMode::Full) && (start_month.is_some() || end_month.is_some()) {
        return Err(bad_request_error(
            "full sync cannot be bounded by startMonth or endMonth",
        ));
    }
    Ok(())
}

fn now_string() -> String {
    Utc::now().to_rfc3339()
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

fn map_missing_cert_gateway_challenge(err: anyhow::Error, _challenge_id: &str) -> anyhow::Error {
    if err.chain().any(|cause| {
        matches!(
            cause.downcast_ref::<DatabaseLookupError>(),
            Some(DatabaseLookupError::CertGatewayChallengeNotFound)
        )
    }) || err.chain().any(|cause| {
        cause
            .to_string()
            .contains("no row returned: cert gateway challenge")
    }) {
        return not_found_error("cert gateway challenge not found");
    }
    err
}

fn map_missing_crawl_task_token(err: anyhow::Error) -> anyhow::Error {
    if err.chain().any(|cause| {
        matches!(
            cause.downcast_ref::<DatabaseLookupError>(),
            Some(DatabaseLookupError::CrawlTaskTokenNotFound)
        )
    }) || err.chain().any(|cause| {
        cause
            .to_string()
            .contains("no row returned: crawl task token")
    }) {
        return not_found_error("crawl task token not found");
    }
    err
}

fn map_missing_certificate(err: anyhow::Error) -> anyhow::Error {
    if err.chain().any(|cause| {
        matches!(
            cause.downcast_ref::<DatabaseLookupError>(),
            Some(DatabaseLookupError::CertificateNotFound)
        )
    }) || err.chain().any(|cause| {
        cause
            .to_string()
            .contains("no row returned: certificate by id")
    }) {
        return not_found_error("certificate not found");
    }
    err
}

fn select_organization_business_no<'a>(
    organization_id: &str,
    requested_business_no: Option<&'a str>,
    linked_business_no: Option<&'a str>,
    _certificate_business_no: Option<&'a str>,
) -> Result<&'a str> {
    if let Some(business_no) = requested_business_no.filter(|value| !value.trim().is_empty()) {
        return Ok(business_no);
    }
    if let Some(business_no) = linked_business_no {
        return Ok(business_no);
    }
    Err(anyhow!(
        "organization missing linked legal party business number: {organization_id}"
    ))
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

#[cfg(test)]
mod tests {
    use super::{normalize_optional_text, select_organization_business_no};

    #[test]
    fn requested_business_number_wins() {
        let selected = select_organization_business_no(
            "ops-seoul",
            Some("2768603291"),
            Some("9999999999"),
            Some("1111111111"),
        )
        .expect("selected business number");
        assert_eq!(selected, "2768603291");
    }

    #[test]
    fn linked_business_number_is_used_when_request_omits_it() {
        let selected = select_organization_business_no(
            "ops-seoul",
            None,
            Some("2768603291"),
            Some("1111111111"),
        )
        .expect("selected business number");
        assert_eq!(selected, "2768603291");
    }

    #[test]
    fn blank_requested_business_number_falls_back_to_linked_business_number() {
        let selected =
            select_organization_business_no("ops-seoul", Some("   "), Some("2768603291"), None)
                .expect("selected business number");
        assert_eq!(selected, "2768603291");
    }

    #[test]
    fn certificate_subject_business_number_does_not_bootstrap_unlinked_organization() {
        let err = select_organization_business_no("ops-seoul", None, None, Some("2768603291"))
            .expect_err("certificate subject must not bootstrap legal identity");
        assert!(
            err.to_string()
                .contains("organization missing linked legal party business number: ops-seoul")
        );
    }

    #[test]
    fn missing_business_number_returns_explicit_error() {
        let err = select_organization_business_no("ops-seoul", None, None, None)
            .expect_err("missing business number must fail");
        assert!(
            err.to_string()
                .contains("organization missing linked legal party business number: ops-seoul")
        );
    }

    #[test]
    fn blank_optional_text_normalizes_to_none() {
        assert_eq!(normalize_optional_text(Some("   ".to_owned())), None);
        assert_eq!(
            normalize_optional_text(Some("  hello  ".to_owned())).as_deref(),
            Some("hello")
        );
    }
}

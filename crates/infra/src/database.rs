use crate::{
    config::ReadConsistency,
    runtime::{HiqliteRuntime, Readiness},
};
use anyhow::{Context, Result, anyhow};
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, Datelike, Duration as ChronoDuration, NaiveDate, Utc};
use domain::{
    Action, ActionRiskLevel, Agreement, AgreementDocument, AgreementDocumentType, Client,
    ClientAction, ClientAppType, ClientConsentPolicy, ClientGrant, ClientGrantAction,
    ClientGrantStatus, ClientType, DocumentArtifact, DocumentCategory, GenerationRecord,
    HealthStatus, Organization, Party, Plant, RateRecord, RecRecord, Role, RoleBinding,
    RoleBindingResourceType, RoleBindingState, RoleBindingSubjectType, SyncMode, SyncRun, User,
};
use hiqlite::Row;
use sha2::{Digest, Sha256};
use std::borrow::Cow;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::{error::Error as StdError, fmt};
use tracing::warn;
use uuid::Uuid;

macro_rules! params {
    ($($value:expr),* $(,)?) => {
        vec![$(::hiqlite::Param::from($value)),*]
    };
}

const CERTIFICATE_LAST_USED_AT_WRITE_INTERVAL_SECS: i64 = 60;
const KR_RPS_GENERATION_METRIC_GROSS_GENERATION_KWH: &str = "gross_generation_kwh";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseLookupError {
    PlantNotFound,
    CertificateNotFound,
    CertGatewayChallengeNotFound,
    CrawlTaskTokenNotFound,
}

impl fmt::Display for DatabaseLookupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PlantNotFound => f.write_str("plant not found"),
            Self::CertificateNotFound => f.write_str("certificate not found"),
            Self::CertGatewayChallengeNotFound => f.write_str("cert gateway challenge not found"),
            Self::CrawlTaskTokenNotFound => f.write_str("crawl task token not found"),
        }
    }
}

impl StdError for DatabaseLookupError {}

#[derive(Clone)]
pub struct Database {
    runtime: HiqliteRuntime,
    bootstrap_user_id: String,
    bootstrap_user_name: String,
}

#[derive(Debug, Clone)]
pub struct StoredCertificate {
    pub id: String,
    pub party_id: String,
    pub business_no: Option<String>,
    pub common_name: String,
    pub encrypted_blob: String,
    pub wrapped_dek: Vec<u8>,
    pub kms_key_id: String,
    pub active: i64,
    pub last_used_at: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct CertGatewayChallenge {
    pub id: String,
    pub organization_id: String,
    pub mode: String,
    pub nonce: String,
    pub payload_hash: String,
    pub facility_code: Option<String>,
    pub expires_at: String,
    pub consumed_at: Option<String>,
    pub created_by_user_id: String,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct CrawlTaskToken {
    pub id: String,
    pub organization_id: String,
    pub certificate_id: Option<String>,
    pub token_hash: String,
    pub scope_json: String,
    pub expires_at: String,
    pub used_at: Option<String>,
    pub created_by_user_id: String,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct StoredDocument {
    pub plant_id: String,
    pub artifact: DocumentArtifact,
    pub object_key: String,
    pub content_type: String,
}

pub struct KrRpsSyncMaterialization<'a> {
    pub run_id: &'a str,
    pub plant: &'a Plant,
    pub registry_account_organization_id: Option<&'a str>,
    pub business_no: Option<&'a str>,
    pub mode: &'a SyncMode,
    pub generation: &'a [GenerationRecord],
    pub rates: &'a [StoredRateRecord],
    pub rec: &'a [RecRecord],
    pub documents: &'a [PersistedDocument],
    pub duration_ms: i64,
}

struct KrRpsProjectionContext {
    asset_id: String,
    data_source_id: String,
    rate_source_id: String,
    registry_project_id: String,
}

#[derive(Debug, Clone)]
pub struct BackupEntry {
    pub name: String,
    pub last_modified: i64,
    pub size: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct PlantSummary {
    pub plant: Plant,
    pub last_sync: Option<SyncRun>,
    pub generation_count: i64,
    pub generation_total_kwh: f64,
    pub rates_count: i64,
    pub rec_count: i64,
    pub rec_total: i64,
    pub document_count: i64,
    pub document_categories: Vec<(DocumentCategory, i64)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KrRpsProjectionCounts {
    pub asset_profiles: i64,
    pub registry_projects: i64,
    pub data_sources: i64,
    pub generation_data: i64,
    pub rate_data: i64,
    pub energy_certificates: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KrRpsRegistryProjectBinding {
    pub asset_owner_organization_id: String,
    pub registry_account_organization_id: Option<String>,
    pub registry_account_external_account_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncRunAudit {
    pub run_status: String,
    pub item_status: Option<String>,
    pub run_error_message: Option<String>,
    pub item_error_message: Option<String>,
}

#[derive(Debug, Clone)]
struct CountRow {
    count: i64,
}

#[derive(Debug, Clone)]
struct CountSumRow {
    count: i64,
    total: Option<f64>,
}

#[derive(Debug, Clone)]
struct CountSumIntRow {
    count: i64,
    total: Option<i64>,
}

#[derive(Debug, Clone)]
struct TextValueRow {
    value: String,
}

impl From<&mut Row<'_>> for TextValueRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            value: row.get("value"),
        }
    }
}

#[derive(Debug, Clone)]
struct MetricPeriodRow {
    metric_type: String,
    period_start: String,
    period_end: String,
}

impl From<&mut Row<'_>> for MetricPeriodRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            metric_type: row.get("metric_type"),
            period_start: row.get("period_start"),
            period_end: row.get("period_end"),
        }
    }
}

#[derive(Debug, Clone)]
struct StoredCertificateRow {
    id: String,
    party_id: String,
    business_no: Option<String>,
    common_name: String,
    encrypted_blob: String,
    wrapped_dek_b64: String,
    kms_key_id: String,
    active: i64,
    last_used_at: Option<String>,
    created_at: String,
}

#[derive(Debug, Clone)]
struct UserIdentityJoinRow {
    identity_id: String,
    user_id: String,
}

#[derive(Debug, Clone)]
struct UserIdentityLookupRow {
    user_id: String,
}

impl From<&mut Row<'_>> for UserIdentityLookupRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            user_id: row.get("user_id"),
        }
    }
}

#[derive(Debug, Clone)]
struct UserLookupRow {
    id: String,
    email: Option<String>,
    display_name: String,
    active: i64,
}

#[derive(Debug, Clone)]
pub struct CachedExternalIdentity {
    pub provider: String,
    pub external_user_id: String,
    pub email: Option<String>,
    pub display_name: String,
    pub active: bool,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
struct AuthIdentityCacheRow {
    provider: String,
    external_user_id: String,
    email: Option<String>,
    display_name: String,
    active: i64,
    expires_at: String,
}

#[derive(Debug, Clone)]
struct PlantRow {
    id: String,
    facility_code: String,
    owner_organization_id: String,
    created_by_organization_id: Option<String>,
    name: String,
    contract_no: String,
    address: String,
    installed_kw: f64,
    rec_start_date: String,
    confirm_issued: String,
}

#[derive(Debug, Clone)]
struct SyncRunRow {
    id: String,
    organization_id: String,
    plant_id: Option<String>,
    mode: String,
    status: String,
    started_at: String,
    finished_at: Option<String>,
    duration_ms: i64,
}

#[derive(Debug, Clone)]
struct RoleBindingRow {
    id: String,
    subject_type: String,
    subject_id: String,
    subject_organization_id: String,
    resource_type: String,
    resource_id: String,
    role_id: String,
    agreement_id: Option<String>,
    state: String,
    created_by_user_id: String,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Clone)]
struct RoleOrgRow {
    organization_id: Option<String>,
}

#[derive(Debug, Clone)]
struct PartyIDRow {
    party_id: Option<String>,
}

#[derive(Debug, Clone)]
struct OrganizationRow {
    id: String,
    party_id: Option<String>,
    business_no: Option<String>,
    name: String,
    inactivated_at: Option<String>,
}

#[derive(Debug, Clone)]
struct PartyRow {
    id: String,
    business_no: String,
    legal_name: String,
    active: i64,
}

#[derive(Debug, Clone)]
struct AgreementDocumentRow {
    id: String,
    agreement_id: String,
    document_type: String,
    file_name: String,
    object_key: String,
    content_type: String,
    document_no: Option<String>,
    uploaded_at: String,
}

#[derive(Debug, Clone)]
struct ActionRow {
    id: String,
    code: String,
    name: String,
    description: Option<String>,
    risk_level: String,
    visibility_granting: i64,
    grantable_to_clients: i64,
    consent_label: Option<String>,
    consent_description: Option<String>,
}

#[derive(Debug, Clone)]
struct ClientRow {
    id: String,
    organization_id: String,
    slug: String,
    name: String,
    description: Option<String>,
    client_type: String,
    app_type: String,
    first_party: i64,
    consent_policy: String,
    active: i64,
    created_by_user_id: String,
}

#[derive(Debug, Clone)]
struct ClientGrantLookupRow {
    id: String,
}

#[derive(Debug, Clone)]
struct ActionLookupRow {
    id: String,
}

#[derive(Debug, Clone)]
struct OrganizationPlantActionSeedRow {
    organization_id: String,
    plant_id: String,
    action_id: String,
}

#[derive(Debug, Clone)]
struct OrganizationMembershipSeedRow {
    id: String,
    organization_id: String,
    user_id: String,
}

#[derive(Debug, Clone)]
struct GroupMembershipSeedRow {
    group_id: String,
    organization_membership_id: String,
}

#[derive(Debug, Clone)]
struct GroupPlantActionSeedRow {
    group_id: String,
    organization_id: String,
    plant_id: String,
    action_id: String,
}

#[derive(Debug, Clone)]
struct UserPlantActionSeedRow {
    user_id: String,
    organization_id: String,
    plant_id: String,
    action_id: String,
}

#[derive(Debug, Clone)]
struct GenerationRow {
    month: String,
    generation_kwh: f64,
    self_consumption_kwh: Option<f64>,
    supply_kwh: f64,
    application_deadline: Option<String>,
}

#[derive(Debug, Clone)]
struct RateRow {
    month: String,
    self_rate: Option<f64>,
    national_average: Option<f64>,
    nearby_average: Option<f64>,
    nearby_highest: Option<f64>,
}

#[derive(Debug, Clone)]
struct RecRow {
    month: String,
    application_no: String,
    supply_kwh: i64,
    rec_quantity: i64,
    fee_krw: i64,
    issued_date: String,
    expiry_date: String,
    virtual_account: Option<String>,
    bank_name: Option<String>,
    status: Option<String>,
}

#[derive(Debug, Clone)]
struct CertGatewayChallengeRow {
    id: String,
    organization_id: String,
    mode: String,
    nonce: String,
    payload_hash: String,
    facility_code: Option<String>,
    expires_at: String,
    consumed_at: Option<String>,
    created_by_user_id: String,
    created_at: String,
}

#[derive(Debug, Clone)]
struct CrawlTaskTokenRow {
    id: String,
    organization_id: String,
    certificate_id: Option<String>,
    token_hash: String,
    scope_json: String,
    expires_at: String,
    used_at: Option<String>,
    created_by_user_id: String,
    created_at: String,
}

#[derive(Debug, Clone)]
struct DocumentRow {
    id: String,
    plant_id: String,
    category: String,
    file_class: String,
    file_sec_name: String,
    file_name: String,
    required: i64,
    source_sha256: Option<String>,
    output_sha256: Option<String>,
    object_key: String,
    content_type: String,
}

#[derive(Debug, Clone)]
struct CategoryCountRow {
    category: String,
    count: i64,
}

impl Database {
    pub fn new(
        runtime: HiqliteRuntime,
        bootstrap_user_id: String,
        bootstrap_user_name: String,
    ) -> Self {
        Self {
            runtime,
            bootstrap_user_id,
            bootstrap_user_name,
        }
    }

    async fn query_rows_with<T, S>(
        &self,
        consistency: ReadConsistency,
        stmt: S,
        params: Vec<hiqlite::Param>,
        label: &str,
    ) -> Result<Vec<T>>
    where
        T: for<'a, 'r> From<&'a mut Row<'r>> + Send + 'static,
        S: Into<Cow<'static, str>>,
    {
        self.runtime
            .query_rows::<T, S>(consistency, stmt, params)
            .await
            .with_context(|| format!("query rows: {label}"))
    }

    async fn query_row_with<T, S>(
        &self,
        consistency: ReadConsistency,
        stmt: S,
        params: Vec<hiqlite::Param>,
        label: &str,
    ) -> Result<T>
    where
        T: for<'a, 'r> From<&'a mut Row<'r>> + Send + 'static,
        S: Into<Cow<'static, str>>,
    {
        let rows: Vec<T> = self
            .query_rows_with::<T, S>(consistency, stmt, params, label)
            .await?;
        if rows.is_empty() {
            return Err(anyhow!("no row returned: {label}"));
        }
        if rows.len() > 1 {
            return Err(anyhow!("multiple rows returned: {label}, expected 1"));
        }
        rows.into_iter()
            .next()
            .ok_or_else(|| anyhow!("no row returned: {label}"))
    }

    async fn query_consistent_rows<T, S>(
        &self,
        stmt: S,
        params: Vec<hiqlite::Param>,
        label: &str,
    ) -> Result<Vec<T>>
    where
        T: for<'a, 'r> From<&'a mut Row<'r>> + Send + 'static,
        S: Into<Cow<'static, str>>,
    {
        self.query_rows_with(ReadConsistency::Strong, stmt, params, label)
            .await
    }

    async fn query_default_rows<T, S>(
        &self,
        stmt: S,
        params: Vec<hiqlite::Param>,
        label: &str,
    ) -> Result<Vec<T>>
    where
        T: for<'a, 'r> From<&'a mut Row<'r>> + Send + 'static,
        S: Into<Cow<'static, str>>,
    {
        self.query_rows_with(self.runtime.default_read_consistency(), stmt, params, label)
            .await
    }

    async fn query_count<S>(
        &self,
        consistency: ReadConsistency,
        stmt: S,
        params: Vec<hiqlite::Param>,
        label: &str,
    ) -> Result<i64>
    where
        S: Into<Cow<'static, str>>,
    {
        Ok(self
            .query_row_with::<CountRow, S>(consistency, stmt, params, label)
            .await?
            .count)
    }

    async fn exists_row_with<S>(
        &self,
        consistency: ReadConsistency,
        stmt: S,
        params: Vec<hiqlite::Param>,
        label: &str,
    ) -> Result<bool>
    where
        S: Into<Cow<'static, str>>,
    {
        Ok(self.query_count(consistency, stmt, params, label).await? > 0)
    }

    fn validation_read_consistency(&self) -> ReadConsistency {
        validation_read_consistency_for_mode(self.runtime.runtime_mode())
    }

    pub(crate) fn default_read_consistency_with_override(
        &self,
        override_consistency: Option<ReadConsistency>,
    ) -> ReadConsistency {
        override_consistency.unwrap_or_else(|| self.runtime.default_read_consistency())
    }

    fn validation_read_consistency_with_override(
        &self,
        override_consistency: Option<ReadConsistency>,
    ) -> ReadConsistency {
        override_consistency.unwrap_or_else(|| self.validation_read_consistency())
    }

    pub async fn health_status(&self) -> HealthStatus {
        self.runtime.health_status().await
    }

    pub async fn readiness(&self) -> Readiness {
        self.runtime.readiness().await
    }

    pub async fn ensure_schema_only(&self) -> Result<()> {
        self.runtime
            .batch(
                r#"
                CREATE TABLE IF NOT EXISTS users (
                    id TEXT PRIMARY KEY,
                    email TEXT,
                    display_name TEXT NOT NULL,
                    active INTEGER NOT NULL DEFAULT 1,
                    created_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS user_identities (
                    id TEXT PRIMARY KEY,
                    user_id TEXT NOT NULL,
                    provider TEXT NOT NULL,
                    external_user_id TEXT NOT NULL,
                    email_snapshot TEXT,
                    display_name_snapshot TEXT,
                    last_seen_at TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS auth_identity_cache (
                    token_hash TEXT PRIMARY KEY,
                    provider TEXT NOT NULL,
                    external_user_id TEXT NOT NULL,
                    email TEXT,
                    display_name TEXT NOT NULL,
                    active INTEGER NOT NULL,
                    expires_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS parties (
                    id TEXT PRIMARY KEY,
                    business_no TEXT NOT NULL UNIQUE,
                    legal_name TEXT NOT NULL,
                    active INTEGER NOT NULL DEFAULT 1,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS organizations (
                    id TEXT PRIMARY KEY,
                    business_no TEXT,
                    name TEXT NOT NULL,
                    inactivated_at TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS organization_party_links (
                    organization_id TEXT NOT NULL,
                    party_id TEXT NOT NULL,
                    relation_type TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    PRIMARY KEY (organization_id, party_id, relation_type)
                );
                CREATE TABLE IF NOT EXISTS organization_memberships (
                    id TEXT PRIMARY KEY,
                    organization_id TEXT NOT NULL,
                    user_id TEXT NOT NULL,
                    membership_role TEXT NOT NULL,
                    status TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    UNIQUE (organization_id, user_id)
                );
                CREATE TABLE IF NOT EXISTS groups (
                    id TEXT PRIMARY KEY,
                    organization_id TEXT NOT NULL,
                    name TEXT NOT NULL,
                    description TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    UNIQUE (organization_id, name)
                );
                CREATE TABLE IF NOT EXISTS group_memberships (
                    group_id TEXT NOT NULL,
                    organization_membership_id TEXT NOT NULL,
                    PRIMARY KEY (group_id, organization_membership_id)
                );
                CREATE TABLE IF NOT EXISTS agreements (
                    id TEXT PRIMARY KEY,
                    agreement_type TEXT NOT NULL,
                    grantor_organization_id TEXT NOT NULL,
                    grantee_organization_id TEXT NOT NULL,
                    status TEXT NOT NULL,
                    title TEXT NOT NULL,
                    reference_no TEXT,
                    starts_at TEXT,
                    ends_at TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS agreement_documents (
                    id TEXT PRIMARY KEY,
                    agreement_id TEXT NOT NULL,
                    document_type TEXT NOT NULL,
                    file_name TEXT NOT NULL,
                    object_key TEXT NOT NULL,
                    content_type TEXT NOT NULL,
                    document_no TEXT,
                    uploaded_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS actions (
                    id TEXT PRIMARY KEY,
                    code TEXT NOT NULL UNIQUE,
                    name TEXT NOT NULL,
                    description TEXT,
                    risk_level TEXT NOT NULL,
                    visibility_granting INTEGER NOT NULL DEFAULT 0,
                    grantable_to_clients INTEGER NOT NULL DEFAULT 0,
                    consent_label TEXT,
                    consent_description TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS roles (
                    id TEXT PRIMARY KEY,
                    organization_id TEXT,
                    code TEXT NOT NULL,
                    name TEXT NOT NULL,
                    description TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    UNIQUE (organization_id, code)
                );
                CREATE TABLE IF NOT EXISTS role_actions (
                    role_id TEXT NOT NULL,
                    action_id TEXT NOT NULL,
                    PRIMARY KEY (role_id, action_id)
                );
                CREATE TABLE IF NOT EXISTS role_bindings (
                    id TEXT PRIMARY KEY,
                    subject_type TEXT NOT NULL,
                    subject_id TEXT NOT NULL,
                    subject_organization_id TEXT NOT NULL,
                    resource_type TEXT NOT NULL,
                    resource_id TEXT NOT NULL,
                    role_id TEXT NOT NULL,
                    agreement_id TEXT,
                    state TEXT NOT NULL,
                    created_by_user_id TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS clients (
                    id TEXT PRIMARY KEY,
                    organization_id TEXT NOT NULL,
                    slug TEXT NOT NULL UNIQUE,
                    name TEXT NOT NULL,
                    description TEXT,
                    client_type TEXT NOT NULL,
                    app_type TEXT NOT NULL,
                    first_party INTEGER NOT NULL DEFAULT 0,
                    consent_policy TEXT NOT NULL,
                    active INTEGER NOT NULL DEFAULT 1,
                    created_by_user_id TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS client_actions (
                    client_id TEXT NOT NULL,
                    action_id TEXT NOT NULL,
                    PRIMARY KEY (client_id, action_id)
                );
                CREATE TABLE IF NOT EXISTS client_grants (
                    id TEXT PRIMARY KEY,
                    user_id TEXT NOT NULL,
                    client_id TEXT NOT NULL,
                    organization_id TEXT NOT NULL,
                    status TEXT NOT NULL,
                    consented_at TEXT NOT NULL,
                    revoked_at TEXT,
                    revoked_by_user_id TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS client_grant_actions (
                    client_grant_id TEXT NOT NULL,
                    action_id TEXT NOT NULL,
                    PRIMARY KEY (client_grant_id, action_id)
                );
                CREATE TABLE IF NOT EXISTS effective_organization_plant_actions (
                    organization_id TEXT NOT NULL,
                    plant_id TEXT NOT NULL,
                    action_id TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    PRIMARY KEY (organization_id, plant_id, action_id)
                );
                CREATE TABLE IF NOT EXISTS effective_user_plant_actions (
                    user_id TEXT NOT NULL,
                    organization_id TEXT NOT NULL,
                    plant_id TEXT NOT NULL,
                    action_id TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    PRIMARY KEY (user_id, organization_id, plant_id, action_id)
                );
                CREATE TABLE IF NOT EXISTS effective_user_plant_visibility (
                    user_id TEXT NOT NULL,
                    organization_id TEXT NOT NULL,
                    plant_id TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    PRIMARY KEY (user_id, organization_id, plant_id)
                );
                CREATE TABLE IF NOT EXISTS effective_organization_asset_actions (
                    organization_id TEXT NOT NULL,
                    asset_id TEXT NOT NULL,
                    action_id TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    PRIMARY KEY (organization_id, asset_id, action_id)
                );
                CREATE TABLE IF NOT EXISTS effective_user_asset_actions (
                    user_id TEXT NOT NULL,
                    organization_id TEXT NOT NULL,
                    asset_id TEXT NOT NULL,
                    action_id TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    PRIMARY KEY (user_id, organization_id, asset_id, action_id)
                );
                CREATE TABLE IF NOT EXISTS effective_user_asset_visibility (
                    user_id TEXT NOT NULL,
                    organization_id TEXT NOT NULL,
                    asset_id TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    PRIMARY KEY (user_id, organization_id, asset_id)
                );
                CREATE INDEX IF NOT EXISTS idx_organization_memberships_org_user
                    ON organization_memberships (organization_id, user_id);
                CREATE UNIQUE INDEX IF NOT EXISTS idx_organization_party_links_org_relation
                    ON organization_party_links (organization_id, relation_type);
                CREATE INDEX IF NOT EXISTS idx_organization_party_links_party
                    ON organization_party_links (party_id, relation_type);
                CREATE UNIQUE INDEX IF NOT EXISTS idx_parties_business_no
                    ON parties (business_no);
                CREATE UNIQUE INDEX IF NOT EXISTS idx_user_identities_provider_external
                    ON user_identities (provider, external_user_id);
                CREATE INDEX IF NOT EXISTS idx_auth_identity_cache_expires_at
                    ON auth_identity_cache (expires_at);
                CREATE INDEX IF NOT EXISTS idx_group_memberships_group_member
                    ON group_memberships (group_id, organization_membership_id);
                CREATE INDEX IF NOT EXISTS idx_agreements_grantor_grantee
                    ON agreements (grantor_organization_id, grantee_organization_id, status);
                CREATE INDEX IF NOT EXISTS idx_agreement_documents_agreement
                    ON agreement_documents (agreement_id, document_type, uploaded_at DESC);
                CREATE INDEX IF NOT EXISTS idx_actions_code
                    ON actions (code);
                CREATE UNIQUE INDEX IF NOT EXISTS idx_roles_global_code
                    ON roles (code)
                    WHERE organization_id IS NULL;
                CREATE INDEX IF NOT EXISTS idx_role_bindings_subject_resource_state
                    ON role_bindings (subject_type, subject_id, resource_type, resource_id, state);
                CREATE UNIQUE INDEX IF NOT EXISTS idx_role_bindings_logical_unique
                    ON role_bindings (
                        subject_type,
                        subject_id,
                        subject_organization_id,
                        resource_type,
                        resource_id,
                        role_id
                    );
                CREATE INDEX IF NOT EXISTS idx_client_grants_user_client
                    ON client_grants (user_id, client_id, organization_id, status);
                CREATE UNIQUE INDEX IF NOT EXISTS idx_client_grants_active_unique
                    ON client_grants (user_id, client_id, organization_id)
                    WHERE status = 'active';
                CREATE INDEX IF NOT EXISTS idx_effective_organization_plant_actions_org_plant
                    ON effective_organization_plant_actions (organization_id, plant_id, action_id);
                CREATE INDEX IF NOT EXISTS idx_effective_user_plant_actions_user_org
                    ON effective_user_plant_actions (user_id, organization_id, plant_id, action_id);
                CREATE INDEX IF NOT EXISTS idx_effective_user_plant_visibility_user_org
                    ON effective_user_plant_visibility (user_id, organization_id, plant_id);
                CREATE INDEX IF NOT EXISTS idx_effective_organization_asset_actions_org_asset
                    ON effective_organization_asset_actions (organization_id, asset_id, action_id);
                CREATE INDEX IF NOT EXISTS idx_effective_user_asset_actions_user_org
                    ON effective_user_asset_actions (user_id, organization_id, asset_id, action_id);
                CREATE INDEX IF NOT EXISTS idx_effective_user_asset_visibility_user_org
                    ON effective_user_asset_visibility (user_id, organization_id, asset_id);
                CREATE TABLE IF NOT EXISTS assets (
                    id TEXT PRIMARY KEY,
                    owner_organization_id TEXT NOT NULL,
                    created_by_organization_id TEXT,
                    name TEXT NOT NULL,
                    country_code TEXT NOT NULL,
                    address TEXT NOT NULL,
                    capacity_kw REAL NOT NULL,
                    timezone TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS asset_profiles (
                    id TEXT PRIMARY KEY,
                    asset_id TEXT NOT NULL,
                    country_code TEXT NOT NULL,
                    provider TEXT NOT NULL,
                    profile_type TEXT NOT NULL,
                    external_id TEXT NOT NULL,
                    metadata_json TEXT NOT NULL DEFAULT '{}',
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS registries (
                    id TEXT PRIMARY KEY,
                    registry_type TEXT NOT NULL,
                    country_code TEXT,
                    provider TEXT NOT NULL,
                    name TEXT NOT NULL,
                    metadata_json TEXT NOT NULL DEFAULT '{}',
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS registry_accounts (
                    id TEXT PRIMARY KEY,
                    registry_id TEXT NOT NULL,
                    organization_id TEXT NOT NULL,
                    external_account_id TEXT NOT NULL,
                    display_name TEXT,
                    status TEXT NOT NULL,
                    metadata_json TEXT NOT NULL DEFAULT '{}',
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    UNIQUE (registry_id, organization_id, external_account_id)
                );
                CREATE TABLE IF NOT EXISTS registry_projects (
                    id TEXT PRIMARY KEY,
                    registry_id TEXT NOT NULL,
                    asset_id TEXT NOT NULL,
                    registry_account_id TEXT,
                    external_project_id TEXT NOT NULL,
                    project_type TEXT NOT NULL,
                    status TEXT NOT NULL,
                    metadata_json TEXT NOT NULL DEFAULT '{}',
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    UNIQUE (registry_id, external_project_id)
                );
                CREATE TABLE IF NOT EXISTS data_sources (
                    id TEXT PRIMARY KEY,
                    asset_id TEXT NOT NULL,
                    registry_id TEXT,
                    registry_project_id TEXT,
                    source_type TEXT NOT NULL,
                    provider TEXT NOT NULL,
                    external_source_id TEXT,
                    unit TEXT NOT NULL,
                    metadata_json TEXT NOT NULL DEFAULT '{}',
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS generation_data (
                    id TEXT PRIMARY KEY,
                    asset_id TEXT NOT NULL,
                    source_id TEXT NOT NULL,
                    period_start TEXT NOT NULL,
                    period_end TEXT NOT NULL,
                    metric_type TEXT NOT NULL,
                    value REAL NOT NULL,
                    unit TEXT NOT NULL,
                    quality TEXT NOT NULL,
                    metadata_json TEXT NOT NULL DEFAULT '{}',
                    recorded_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    UNIQUE (asset_id, source_id, metric_type, period_start, period_end)
                );
                CREATE TABLE IF NOT EXISTS rate_data (
                    id TEXT PRIMARY KEY,
                    asset_id TEXT NOT NULL,
                    source_id TEXT NOT NULL,
                    period_start TEXT NOT NULL,
                    period_end TEXT NOT NULL,
                    metric_type TEXT NOT NULL,
                    value REAL NOT NULL,
                    unit TEXT NOT NULL,
                    currency TEXT,
                    quality TEXT NOT NULL,
                    metadata_json TEXT NOT NULL DEFAULT '{}',
                    recorded_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    UNIQUE (asset_id, source_id, metric_type, period_start, period_end)
                );
                CREATE TABLE IF NOT EXISTS energy_certificates (
                    id TEXT PRIMARY KEY,
                    certificate_type TEXT NOT NULL,
                    registry_id TEXT NOT NULL,
                    registry_project_id TEXT,
                    asset_id TEXT NOT NULL,
                    serial_number TEXT NOT NULL,
                    vintage_start TEXT,
                    vintage_end TEXT,
                    quantity_mwh REAL NOT NULL,
                    owner_account_id TEXT,
                    status TEXT NOT NULL,
                    issued_at TEXT,
                    retired_at TEXT,
                    metadata_json TEXT NOT NULL DEFAULT '{}',
                    updated_at TEXT NOT NULL,
                    UNIQUE (registry_id, serial_number)
                );
                CREATE TABLE IF NOT EXISTS auth_credentials (
                    id TEXT PRIMARY KEY,
                    organization_id TEXT NOT NULL,
                    credential_type TEXT NOT NULL,
                    country_code TEXT,
                    provider TEXT NOT NULL,
                    external_subject_id TEXT,
                    encrypted_blob TEXT NOT NULL,
                    wrapped_dek TEXT NOT NULL,
                    kms_key_id TEXT NOT NULL,
                    active INTEGER NOT NULL DEFAULT 1,
                    last_used_at TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS credential_bindings (
                    credential_id TEXT NOT NULL,
                    registry_account_id TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    PRIMARY KEY (credential_id, registry_account_id)
                );
                CREATE TABLE IF NOT EXISTS country_capabilities (
                    country_code TEXT PRIMARY KEY,
                    has_rps INTEGER NOT NULL DEFAULT 0,
                    has_rec_registry INTEGER NOT NULL DEFAULT 0,
                    has_utility_api INTEGER NOT NULL DEFAULT 0,
                    has_certificate_login INTEGER NOT NULL DEFAULT 0,
                    enabled_features_json TEXT NOT NULL DEFAULT '[]',
                    updated_at TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_assets_owner_country
                    ON assets (owner_organization_id, country_code);
                CREATE INDEX IF NOT EXISTS idx_asset_profiles_asset_type
                    ON asset_profiles (asset_id, profile_type);
                CREATE UNIQUE INDEX IF NOT EXISTS idx_asset_profiles_country_provider_type_external
                    ON asset_profiles (country_code, provider, profile_type, external_id);
                CREATE INDEX IF NOT EXISTS idx_registries_country_type
                    ON registries (country_code, registry_type, provider);
                CREATE UNIQUE INDEX IF NOT EXISTS idx_registries_country_provider_type
                    ON registries (country_code, provider, registry_type)
                    WHERE country_code IS NOT NULL;
                CREATE UNIQUE INDEX IF NOT EXISTS idx_registries_global_provider_type
                    ON registries (provider, registry_type)
                    WHERE country_code IS NULL;
                CREATE INDEX IF NOT EXISTS idx_registry_accounts_org_registry
                    ON registry_accounts (organization_id, registry_id, status);
                CREATE INDEX IF NOT EXISTS idx_registry_projects_asset_registry
                    ON registry_projects (asset_id, registry_id, status);
                CREATE INDEX IF NOT EXISTS idx_data_sources_asset_type
                    ON data_sources (asset_id, source_type, provider);
                CREATE INDEX IF NOT EXISTS idx_generation_data_asset_source_metric_period
                    ON generation_data (asset_id, source_id, metric_type, period_start, period_end);
                CREATE INDEX IF NOT EXISTS idx_rate_data_asset_source_metric_period
                    ON rate_data (asset_id, source_id, metric_type, period_start, period_end);
                CREATE INDEX IF NOT EXISTS idx_energy_certificates_asset_registry
                    ON energy_certificates (asset_id, registry_id, status);
                CREATE INDEX IF NOT EXISTS idx_auth_credentials_org_provider
                    ON auth_credentials (organization_id, provider, credential_type, active);
                CREATE INDEX IF NOT EXISTS idx_credential_bindings_account
                    ON credential_bindings (registry_account_id, credential_id);
                CREATE TABLE IF NOT EXISTS certificates (
                    id TEXT PRIMARY KEY,
                    party_id TEXT NOT NULL,
                    common_name TEXT NOT NULL,
                    encrypted_blob TEXT NOT NULL,
                    wrapped_dek TEXT NOT NULL,
                    kms_key_id TEXT NOT NULL,
                    active INTEGER NOT NULL DEFAULT 1,
                    last_used_at TEXT,
                    created_at TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_certificates_party_active
                    ON certificates (party_id, active, created_at DESC);
                CREATE TABLE IF NOT EXISTS cert_gateway_challenges (
                    id TEXT PRIMARY KEY,
                    organization_id TEXT NOT NULL,
                    mode TEXT NOT NULL,
                    nonce TEXT NOT NULL,
                    payload_hash TEXT NOT NULL,
                    facility_code TEXT,
                    expires_at TEXT NOT NULL,
                    consumed_at TEXT,
                    created_by_user_id TEXT NOT NULL,
                    created_at TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_cert_gateway_challenges_org_expires
                    ON cert_gateway_challenges (organization_id, expires_at DESC);
                CREATE TABLE IF NOT EXISTS crawl_task_tokens (
                    id TEXT PRIMARY KEY,
                    organization_id TEXT NOT NULL,
                    certificate_id TEXT,
                    token_hash TEXT NOT NULL,
                    scope_json TEXT NOT NULL,
                    expires_at TEXT NOT NULL,
                    used_at TEXT,
                    created_by_user_id TEXT NOT NULL,
                    created_at TEXT NOT NULL
                );
                CREATE UNIQUE INDEX IF NOT EXISTS idx_crawl_task_tokens_token_hash
                    ON crawl_task_tokens (token_hash);
                CREATE INDEX IF NOT EXISTS idx_crawl_task_tokens_org_expires
                    ON crawl_task_tokens (organization_id, expires_at DESC);
                CREATE TABLE IF NOT EXISTS plants (
                    id TEXT PRIMARY KEY,
                    facility_code TEXT NOT NULL UNIQUE,
                    owner_organization_id TEXT NOT NULL,
                    created_by_organization_id TEXT,
                    name TEXT NOT NULL,
                    contract_no TEXT NOT NULL,
                    address TEXT NOT NULL,
                    installed_kw REAL NOT NULL,
                    rec_start_date TEXT NOT NULL,
                    confirm_issued TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS sync_runs (
                    id TEXT PRIMARY KEY,
                    organization_id TEXT NOT NULL,
                    plant_id TEXT,
                    mode TEXT NOT NULL,
                    status TEXT NOT NULL,
                    started_at TEXT NOT NULL,
                    finished_at TEXT,
                    duration_ms INTEGER NOT NULL DEFAULT 0,
                    error_message TEXT
                );
                CREATE TABLE IF NOT EXISTS sync_run_items (
                    id TEXT PRIMARY KEY,
                    sync_run_id TEXT NOT NULL,
                    asset_id TEXT,
                    registry_project_id TEXT,
                    data_source_id TEXT,
                    facility_code TEXT NOT NULL,
                    status TEXT NOT NULL,
                    generation_count INTEGER NOT NULL DEFAULT 0,
                    rate_count INTEGER NOT NULL DEFAULT 0,
                    rec_count INTEGER NOT NULL DEFAULT 0,
                    document_count INTEGER NOT NULL DEFAULT 0,
                    error_message TEXT,
                    write_version TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    UNIQUE (sync_run_id, facility_code)
                );
                CREATE INDEX IF NOT EXISTS idx_plants_owner_facility
                    ON plants (owner_organization_id, facility_code);
                CREATE INDEX IF NOT EXISTS idx_sync_run_items_run_status
                    ON sync_run_items (sync_run_id, status);
                CREATE INDEX IF NOT EXISTS idx_sync_run_items_asset
                    ON sync_run_items (asset_id, sync_run_id);
                CREATE TABLE IF NOT EXISTS generation_records (
                    plant_id TEXT NOT NULL,
                    month TEXT NOT NULL,
                    generation_kwh REAL NOT NULL,
                    self_consumption_kwh REAL,
                    supply_kwh REAL NOT NULL,
                    application_deadline TEXT,
                    updated_at TEXT NOT NULL,
                    PRIMARY KEY (plant_id, month)
                );
                CREATE TABLE IF NOT EXISTS rate_records (
                    plant_id TEXT NOT NULL,
                    month TEXT NOT NULL,
                    self_rate REAL,
                    national_average REAL,
                    nearby_average REAL,
                    nearby_highest REAL,
                    published INTEGER NOT NULL DEFAULT 0,
                    source_control_flag TEXT,
                    updated_at TEXT NOT NULL,
                    PRIMARY KEY (plant_id, month)
                );
                CREATE TABLE IF NOT EXISTS rec_records (
                    plant_id TEXT NOT NULL,
                    application_no TEXT NOT NULL,
                    month TEXT NOT NULL,
                    supply_kwh INTEGER NOT NULL,
                    rec_quantity INTEGER NOT NULL,
                    fee_krw INTEGER NOT NULL,
                    issued_date TEXT NOT NULL,
                    expiry_date TEXT NOT NULL,
                    virtual_account TEXT,
                    bank_name TEXT,
                    status TEXT,
                    updated_at TEXT NOT NULL,
                    PRIMARY KEY (plant_id, application_no)
                );
                CREATE TABLE IF NOT EXISTS documents (
                    id TEXT PRIMARY KEY,
                    plant_id TEXT NOT NULL,
                    category TEXT NOT NULL,
                    attachment_code TEXT NOT NULL,
                    file_class TEXT NOT NULL,
                    file_sec_name TEXT NOT NULL,
                    file_name TEXT NOT NULL,
                    required INTEGER NOT NULL DEFAULT 1,
                    source_sha256 TEXT,
                    output_sha256 TEXT,
                    object_key TEXT NOT NULL,
                    content_type TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE UNIQUE INDEX IF NOT EXISTS idx_documents_unique
                    ON documents (plant_id, attachment_code, file_name);
                "#
                .to_owned(),
            )
            .await
            .context("create schema")?;
        self.runtime
            .execute(
                "CREATE INDEX IF NOT EXISTS idx_sync_runs_plant_started
                 ON sync_runs (plant_id, started_at DESC)",
                params![],
            )
            .await
            .context("create sync_runs plant index")?;
        Ok(())
    }

    pub async fn ensure_schema(&self) -> Result<()> {
        self.ensure_schema_only().await?;
        self.ensure_bootstrap_principals().await?;
        self.ensure_action_catalog().await?;
        self.ensure_bootstrap_authorization().await?;
        Ok(())
    }

    pub async fn ensure_bootstrap_principals(&self) -> Result<User> {
        let now = now_string();
        self.runtime
            .execute(
                "INSERT INTO users (id, email, display_name, active, created_at) VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT(id) DO UPDATE SET display_name = excluded.display_name, active = excluded.active",
                params!(
                    self.bootstrap_user_id.clone(),
                    Option::<String>::None,
                    self.bootstrap_user_name.clone(),
                    1_i64,
                    now.clone()
                ),
            )
            .await
            .context("upsert bootstrap user")?;
        self.runtime
            .execute(
                "INSERT INTO organizations (id, business_no, name, inactivated_at, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT(id) DO UPDATE SET
                    name = excluded.name,
                    updated_at = excluded.updated_at",
                params!(
                    self.bootstrap_user_id.clone(),
                    Option::<String>::None,
                    self.bootstrap_user_name.clone(),
                    Option::<String>::None,
                    now.clone(),
                    now.clone()
                ),
            )
            .await
            .context("upsert bootstrap organization")?;
        self.runtime
            .execute(
                "INSERT INTO organization_memberships (id, organization_id, user_id, membership_role, status, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)
                 ON CONFLICT(organization_id, user_id) DO UPDATE SET
                    membership_role = excluded.membership_role,
                    status = excluded.status,
                    updated_at = excluded.updated_at",
                params!(
                    Uuid::new_v4().simple().to_string(),
                    self.bootstrap_user_id.clone(),
                    self.bootstrap_user_id.clone(),
                    "owner",
                    "active",
                    now.clone(),
                    now.clone()
                ),
            )
            .await
            .context("upsert bootstrap organization membership")?;
        Ok(User {
            id: self.bootstrap_user_id.clone(),
            email: None,
            display_name: self.bootstrap_user_name.clone(),
            active: true,
        })
    }

    async fn ensure_bootstrap_authorization(&self) -> Result<()> {
        let now = now_string();
        let role = Role {
            id: "role-system-bootstrap-owner".to_owned(),
            organization_id: None,
            code: "system.bootstrap.owner".to_owned(),
            name: "System Bootstrap Owner".to_owned(),
            description: Some("Bootstrap organization owner capabilities".to_owned()),
        };
        self.upsert_role(&role).await?;
        for action in [
            "organization.plant.create",
            "organization.plant.create_for_owner",
            "organization.client.manage",
            "organization.member.manage",
            "organization.group.manage",
            "organization.role.manage",
            "organization.permission.manage",
        ] {
            self.upsert_role_action(&role.id, action).await?;
        }
        self.save_role_binding(&RoleBinding {
            id: "binding-system-bootstrap-owner".to_owned(),
            subject_type: RoleBindingSubjectType::User,
            subject_id: self.bootstrap_user_id.clone(),
            subject_organization_id: self.bootstrap_user_id.clone(),
            resource_type: RoleBindingResourceType::Organization,
            resource_id: self.bootstrap_user_id.clone(),
            role_id: role.id,
            agreement_id: None,
            state: RoleBindingState::Active,
            created_by_user_id: self.bootstrap_user_id.clone(),
            created_at: now.clone(),
            updated_at: now,
        })
        .await
    }

    async fn ensure_action_catalog(&self) -> Result<()> {
        let now = now_string();
        for (code, name, risk_level, visibility_granting, grantable_to_clients) in [
            (
                "organization.plant.create",
                "Create Plant",
                "medium",
                0_i64,
                0_i64,
            ),
            (
                "organization.plant.create_for_owner",
                "Create Plant For Owner",
                "high",
                0_i64,
                0_i64,
            ),
            (
                "organization.client.manage",
                "Manage Organization Clients",
                "high",
                0_i64,
                0_i64,
            ),
            (
                "organization.member.manage",
                "Manage Organization Members",
                "high",
                0_i64,
                0_i64,
            ),
            (
                "organization.group.manage",
                "Manage Organization Groups",
                "high",
                0_i64,
                0_i64,
            ),
            (
                "organization.role.manage",
                "Manage Organization Roles",
                "high",
                0_i64,
                0_i64,
            ),
            (
                "organization.permission.manage",
                "Manage Organization Permissions",
                "critical",
                0_i64,
                0_i64,
            ),
            (
                "plant.profile.read",
                "Read Plant Profile",
                "low",
                1_i64,
                1_i64,
            ),
            (
                "asset.profile.read",
                "Read Asset Profile",
                "low",
                1_i64,
                1_i64,
            ),
            (
                "plant.profile.write",
                "Write Plant Profile",
                "medium",
                0_i64,
                0_i64,
            ),
            (
                "asset.profile.write",
                "Write Asset Profile",
                "medium",
                0_i64,
                0_i64,
            ),
            (
                "plant.telemetry.read",
                "Read Plant Telemetry",
                "low",
                0_i64,
                1_i64,
            ),
            (
                "asset.generation.read",
                "Read Asset Generation",
                "low",
                0_i64,
                1_i64,
            ),
            ("plant.alert.read", "Read Plant Alerts", "low", 0_i64, 1_i64),
            (
                "plant.document.read",
                "Read Plant Documents",
                "low",
                0_i64,
                1_i64,
            ),
            (
                "asset.document.read",
                "Read Asset Documents",
                "low",
                0_i64,
                1_i64,
            ),
            (
                "plant.document.write",
                "Write Plant Documents",
                "medium",
                0_i64,
                0_i64,
            ),
            (
                "plant.report.read",
                "Read Plant Reports",
                "low",
                0_i64,
                1_i64,
            ),
            (
                "plant.report.write",
                "Write Plant Reports",
                "medium",
                0_i64,
                0_i64,
            ),
            ("plant.rec.read", "Read Plant REC", "low", 0_i64, 1_i64),
            (
                "asset.certificate.read",
                "Read Asset Energy Certificates",
                "low",
                0_i64,
                1_i64,
            ),
            ("plant.rate.read", "Read Plant Rates", "low", 0_i64, 1_i64),
            ("asset.rate.read", "Read Asset Rates", "low", 0_i64, 1_i64),
            (
                "plant.sync.execute",
                "Execute Plant Sync",
                "high",
                0_i64,
                0_i64,
            ),
            (
                "asset.sync.execute",
                "Execute Asset Sync",
                "high",
                0_i64,
                0_i64,
            ),
            (
                "plant.device.read",
                "Read Plant Device",
                "low",
                0_i64,
                1_i64,
            ),
            (
                "plant.device.control",
                "Control Plant Device",
                "critical",
                0_i64,
                0_i64,
            ),
            (
                "plant.certificate.use",
                "Use Plant Certificate",
                "critical",
                0_i64,
                0_i64,
            ),
            (
                "asset.credential.use",
                "Use Asset Credential",
                "critical",
                0_i64,
                0_i64,
            ),
            (
                "plant.permission.manage",
                "Manage Plant Permissions",
                "critical",
                0_i64,
                0_i64,
            ),
            (
                "asset.permission.manage",
                "Manage Asset Permissions",
                "critical",
                0_i64,
                0_i64,
            ),
            (
                "plant.ownership.transfer",
                "Transfer Plant Ownership",
                "critical",
                0_i64,
                0_i64,
            ),
        ] {
            self.runtime
                .execute(
                    "INSERT INTO actions (
                        id, code, name, description, risk_level, visibility_granting,
                        grantable_to_clients, consent_label, consent_description, created_at, updated_at
                     )
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
                     ON CONFLICT(code) DO UPDATE SET
                        name = excluded.name,
                        risk_level = excluded.risk_level,
                        visibility_granting = excluded.visibility_granting,
                        grantable_to_clients = excluded.grantable_to_clients,
                        consent_label = excluded.consent_label,
                        consent_description = excluded.consent_description,
                        updated_at = excluded.updated_at",
                    params!(
                        code.to_owned(),
                        code.to_owned(),
                        name.to_owned(),
                        Option::<String>::None,
                        risk_level.to_owned(),
                        visibility_granting,
                        grantable_to_clients,
                        Some(name.to_owned()),
                        Option::<String>::None,
                        now.clone(),
                        now.clone()
                    ),
                )
                .await
                .with_context(|| format!("seed action {code}"))?;
        }

        Ok(())
    }

    pub async fn upsert_organization_with_owner(
        &self,
        organization_id: &str,
        display_name: &str,
        business_no: Option<&str>,
        legal_name: &str,
    ) -> Result<Organization> {
        let now = now_string();
        self.runtime
            .execute(
                "INSERT INTO organizations (id, business_no, name, inactivated_at, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT(id) DO UPDATE SET
                    business_no = COALESCE(excluded.business_no, organizations.business_no),
                    name = excluded.name,
                    updated_at = excluded.updated_at",
                params!(
                    organization_id.to_owned(),
                    business_no.map(ToOwned::to_owned),
                    display_name.to_owned(),
                    Option::<String>::None,
                    now.clone(),
                    now.clone()
                ),
            )
            .await
            .context("upsert organization")?;
        self.runtime
            .execute(
                "INSERT INTO organization_memberships (id, organization_id, user_id, membership_role, status, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)
                 ON CONFLICT(organization_id, user_id) DO UPDATE SET membership_role = excluded.membership_role, status = excluded.status, updated_at = excluded.updated_at",
                params!(
                    Uuid::new_v4().simple().to_string(),
                    organization_id.to_owned(),
                    self.bootstrap_user_id.clone(),
                    "owner",
                    "active",
                    now.clone(),
                    now.clone()
                ),
            )
            .await
            .context("upsert organization membership")?;
        if let Some(business_no) = business_no {
            let party_id = self
                .upsert_party_by_business_no(business_no, legal_name, &now)
                .await?;
            self.upsert_organization_party_link(
                organization_id,
                &party_id,
                organization_party_relation_type_str_legal(),
                &now,
            )
            .await?;
        }
        self.get_organization(organization_id).await
    }

    pub async fn upsert_organization_identity(
        &self,
        organization_id: &str,
        business_no: Option<&str>,
        legal_name: &str,
    ) -> Result<Organization> {
        let now = now_string();
        self.runtime
            .execute(
                "INSERT INTO organizations (id, business_no, name, inactivated_at, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT(id) DO UPDATE SET
                    business_no = COALESCE(excluded.business_no, organizations.business_no),
                    updated_at = excluded.updated_at",
                params!(
                    organization_id.to_owned(),
                    business_no.map(ToOwned::to_owned),
                    organization_id.to_owned(),
                    Option::<String>::None,
                    now.clone(),
                    now.clone()
                ),
            )
            .await
            .context("upsert organization identity")?;
        if let Some(business_no) = business_no {
            let party_id = self
                .upsert_party_by_business_no(business_no, legal_name, &now)
                .await?;
            self.upsert_organization_party_link(
                organization_id,
                &party_id,
                organization_party_relation_type_str_legal(),
                &now,
            )
            .await?;
        }
        self.get_organization(organization_id).await
    }

    async fn upsert_party_by_business_no(
        &self,
        business_no: &str,
        legal_name: &str,
        now: &str,
    ) -> Result<String> {
        let normalized = business_no.to_owned();
        let party_id = format!("party-{normalized}");
        self.runtime
            .execute(
                "INSERT INTO parties (id, business_no, legal_name, active, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT(business_no) DO UPDATE SET
                    legal_name = excluded.legal_name,
                    active = excluded.active,
                    updated_at = excluded.updated_at",
                params!(
                    party_id.clone(),
                    normalized,
                    legal_name.to_owned(),
                    1_i64,
                    now.to_owned(),
                    now.to_owned()
                ),
            )
            .await
            .context("upsert party by business number")?;
        let rows: Vec<PartyRow> = self
            .query_default_rows(
                "SELECT id, business_no, legal_name, active
                 FROM parties
                 WHERE business_no = $1
                 LIMIT 1",
                params!(business_no.to_owned()),
                "party by business number",
            )
            .await?;
        rows.into_iter()
            .next()
            .map(|row| row.id)
            .ok_or_else(|| anyhow!("party not found after upsert: {business_no}"))
    }

    async fn find_party_id_by_business_no(&self, business_no: &str) -> Result<Option<String>> {
        let rows: Vec<PartyRow> = self
            .query_default_rows(
                "SELECT id, business_no, legal_name, active
                 FROM parties
                 WHERE business_no = $1
                 LIMIT 1",
                params!(business_no.to_owned()),
                "party by business number",
            )
            .await?;
        Ok(rows.into_iter().next().map(|row| row.id))
    }

    async fn upsert_organization_party_link(
        &self,
        organization_id: &str,
        party_id: &str,
        relation_type: &str,
        now: &str,
    ) -> Result<()> {
        self.runtime
            .execute(
                "INSERT INTO organization_party_links (organization_id, party_id, relation_type, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT(organization_id, relation_type) DO UPDATE SET
                    party_id = excluded.party_id,
                    updated_at = excluded.updated_at",
                params!(
                    organization_id.to_owned(),
                    party_id.to_owned(),
                    relation_type.to_owned(),
                    now.to_owned(),
                    now.to_owned()
                ),
            )
            .await
            .context("upsert organization party link")?;
        Ok(())
    }

    async fn find_organization_party_id(&self, organization_id: &str) -> Result<Option<String>> {
        let rows: Vec<PartyIDRow> = self
            .query_default_rows(
                "SELECT opl.party_id
                 FROM organizations o
                 LEFT JOIN organization_party_links opl
                   ON opl.organization_id = o.id
                  AND opl.relation_type = 'legal'
                 WHERE o.id = $1
                 LIMIT 1",
                params!(organization_id.to_owned()),
                "organization party id",
            )
            .await?;
        Ok(rows.into_iter().next().and_then(|row| row.party_id))
    }

    pub async fn resolve_or_create_user_identity(
        &self,
        provider: &str,
        external_user_id: &str,
        email: Option<&str>,
        display_name: &str,
        active: bool,
    ) -> Result<User> {
        self.resolve_or_create_user_identity_with_consistency(
            provider,
            external_user_id,
            email,
            display_name,
            active,
            None,
        )
        .await
    }

    pub async fn resolve_or_create_user_identity_with_consistency(
        &self,
        provider: &str,
        external_user_id: &str,
        email: Option<&str>,
        display_name: &str,
        active: bool,
        consistency: Option<ReadConsistency>,
    ) -> Result<User> {
        let now = now_string();
        let read_consistency = self.validation_read_consistency_with_override(consistency.clone());
        let existing: Option<UserIdentityJoinRow> = self
            .runtime
            .query_rows(
                read_consistency.clone(),
                "SELECT ui.id as identity_id,
                        ui.user_id as user_id
                 FROM user_identities ui
                 WHERE ui.provider = $1 AND ui.external_user_id = $2",
                params!(provider.to_owned(), external_user_id.to_owned()),
            )
            .await
            .context("query user identity")?
            .into_iter()
            .next();

        let email_user = if existing.is_none() {
            match email {
                Some(email) => {
                    self.find_user_by_identity_or_email(provider, email, consistency.clone())
                        .await?
                }
                None => None,
            }
        } else {
            None
        };
        let existing_user = if existing.is_none() && email_user.is_none() {
            self.find_user_by_id(external_user_id, consistency.clone())
                .await?
        } else {
            None
        };

        let user_id = existing
            .as_ref()
            .map(|row| row.user_id.clone())
            .or_else(|| email_user.as_ref().map(|user| user.id.clone()))
            .or_else(|| existing_user.as_ref().map(|user| user.id.clone()))
            .unwrap_or_else(|| format!("{provider}:{external_user_id}"));
        let identity_id = existing
            .as_ref()
            .map(|row| row.identity_id.clone())
            .unwrap_or_else(|| Uuid::new_v4().simple().to_string());

        self.runtime
            .execute(
                "INSERT INTO users (id, email, display_name, active, created_at)
                 VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT(id) DO UPDATE SET
                    email = excluded.email,
                    display_name = excluded.display_name,
                    active = excluded.active",
                params!(
                    user_id.clone(),
                    email.map(ToOwned::to_owned),
                    display_name.to_owned(),
                    if active { 1_i64 } else { 0_i64 },
                    now.clone()
                ),
            )
            .await
            .context("upsert user from identity")?;

        self.runtime
            .execute(
                "INSERT INTO user_identities (
                    id, user_id, provider, external_user_id, email_snapshot, display_name_snapshot, last_seen_at, created_at, updated_at
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                 ON CONFLICT(provider, external_user_id) DO UPDATE SET
                    user_id = excluded.user_id,
                    email_snapshot = excluded.email_snapshot,
                    display_name_snapshot = excluded.display_name_snapshot,
                    last_seen_at = excluded.last_seen_at,
                    updated_at = excluded.updated_at",
                params!(
                    identity_id,
                    user_id.clone(),
                    provider.to_owned(),
                    external_user_id.to_owned(),
                    email.map(ToOwned::to_owned),
                    Some(display_name.to_owned()),
                    Some(now.clone()),
                    now.clone(),
                    now.clone()
                ),
            )
            .await
            .context("upsert user identity")?;

        Ok(User {
            id: user_id,
            email: email.map(ToOwned::to_owned),
            display_name: display_name.to_owned(),
            active,
        })
    }

    pub async fn resolve_existing_user_identity_with_consistency(
        &self,
        provider: &str,
        external_user_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Option<User>> {
        let read_consistency = self.validation_read_consistency_with_override(consistency.clone());
        let existing: Option<UserIdentityJoinRow> = self
            .runtime
            .query_rows(
                read_consistency,
                "SELECT ui.id as identity_id,
                        ui.user_id as user_id
                 FROM user_identities ui
                 WHERE ui.provider = $1 AND ui.external_user_id = $2",
                params!(provider.to_owned(), external_user_id.to_owned()),
            )
            .await
            .context("query user identity")?
            .into_iter()
            .next();

        if let Some(existing) = existing {
            return self
                .find_user_by_id(&existing.user_id, consistency.clone())
                .await;
        }
        Ok(None)
    }

    pub async fn bind_identity_to_user(
        &self,
        user_id: &str,
        provider: &str,
        external_user_id: &str,
        email: Option<&str>,
        display_name: &str,
        active: bool,
        allow_rebind: bool,
    ) -> Result<User> {
        let now = now_string();
        let existing: Option<UserIdentityJoinRow> = self
            .runtime
            .query_rows(
                self.validation_read_consistency(),
                "SELECT ui.id as identity_id,
                        ui.user_id as user_id
                 FROM user_identities ui
                 WHERE ui.provider = $1 AND ui.external_user_id = $2",
                params!(provider.to_owned(), external_user_id.to_owned()),
            )
            .await
            .context("query user identity for binding")?
            .into_iter()
            .next();

        if !allow_rebind
            && let Some(existing) = existing.as_ref()
            && existing.user_id != user_id
        {
            anyhow::bail!(
                "identity {}:{} is already bound to another user {}",
                provider,
                external_user_id,
                existing.user_id
            );
        }

        let target_user = self
            .find_user_by_id(user_id, None)
            .await?
            .ok_or_else(|| anyhow::anyhow!("target user {} does not exist", user_id))?;

        let identity_id = existing
            .as_ref()
            .map(|row| row.identity_id.clone())
            .unwrap_or_else(|| Uuid::new_v4().simple().to_string());

        self.runtime
            .execute(
                "INSERT INTO users (id, email, display_name, active, created_at)
                 VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT(id) DO UPDATE SET
                    email = excluded.email,
                    display_name = excluded.display_name,
                    active = excluded.active",
                params!(
                    target_user.id.clone(),
                    email.map(ToOwned::to_owned).or(target_user.email.clone()),
                    display_name.to_owned(),
                    if active { 1_i64 } else { 0_i64 },
                    now.clone()
                ),
            )
            .await
            .context("upsert user for identity binding")?;

        self.runtime
            .execute(
                "INSERT INTO user_identities (
                    id, user_id, provider, external_user_id, email_snapshot, display_name_snapshot, last_seen_at, created_at, updated_at
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                 ON CONFLICT(provider, external_user_id) DO UPDATE SET
                    user_id = excluded.user_id,
                    email_snapshot = excluded.email_snapshot,
                    display_name_snapshot = excluded.display_name_snapshot,
                    last_seen_at = excluded.last_seen_at,
                    updated_at = excluded.updated_at",
                params!(
                    identity_id,
                    user_id.to_owned(),
                    provider.to_owned(),
                    external_user_id.to_owned(),
                    email.map(ToOwned::to_owned),
                    Some(display_name.to_owned()),
                    Some(now.clone()),
                    now.clone(),
                    now.clone()
                ),
            )
            .await
            .context("bind identity to user")?;

        Ok(User {
            id: user_id.to_owned(),
            email: email.map(ToOwned::to_owned).or(target_user.email),
            display_name: display_name.to_owned(),
            active,
        })
    }

    async fn find_user_by_id(
        &self,
        user_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Option<User>> {
        let rows: Vec<UserLookupRow> = self
            .runtime
            .query_rows(
                self.validation_read_consistency_with_override(consistency),
                "SELECT id, email, display_name, active
                 FROM users
                 WHERE id = $1
                 LIMIT 1",
                params!(user_id.to_owned()),
            )
            .await
            .context("find user by id")?;
        Ok(rows.into_iter().next().map(|row| User {
            id: row.id,
            email: row.email,
            display_name: row.display_name,
            active: row.active != 0,
        }))
    }

    async fn find_user_by_identity_or_email(
        &self,
        provider: &str,
        email: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Option<User>> {
        let read_consistency = self.validation_read_consistency_with_override(consistency.clone());
        let identity_rows: Vec<UserIdentityLookupRow> = self
            .runtime
            .query_rows(
                read_consistency.clone(),
                "SELECT ui.user_id
                 FROM user_identities ui
                 WHERE ui.provider = $1
                   AND ui.email_snapshot = $2
                 ORDER BY ui.updated_at DESC, ui.created_at DESC
                 LIMIT 2",
                params!(provider.to_owned(), email.to_owned()),
            )
            .await
            .context("find user identity by provider/email")?;
        if identity_rows.len() > 1 {
            anyhow::bail!(
                "multiple local users matched {} identity email {}",
                provider,
                email
            );
        }
        if let Some(row) = identity_rows.into_iter().next() {
            return self
                .find_user_by_id(&row.user_id, consistency.clone())
                .await;
        }

        let user_rows: Vec<UserLookupRow> = self
            .runtime
            .query_rows(
                read_consistency,
                "SELECT id, email, display_name, active
                 FROM users
                 WHERE email = $1
                 ORDER BY created_at ASC, id ASC
                 LIMIT 2",
                params!(email.to_owned()),
            )
            .await
            .context("find user by email")?;
        if user_rows.len() > 1 {
            anyhow::bail!("multiple local users matched email {}", email);
        }
        Ok(user_rows.into_iter().next().map(|row| User {
            id: row.id,
            email: row.email,
            display_name: row.display_name,
            active: row.active != 0,
        }))
    }

    pub async fn get_cached_external_identity(
        &self,
        token_hash: &str,
        now: DateTime<Utc>,
        consistency: Option<ReadConsistency>,
    ) -> Result<Option<CachedExternalIdentity>> {
        let rows: Vec<AuthIdentityCacheRow> = self
            .runtime
            .query_rows(
                self.validation_read_consistency_with_override(consistency),
                "SELECT provider, external_user_id, email, display_name, active, expires_at
                 FROM auth_identity_cache
                 WHERE token_hash = $1 AND expires_at > $2
                 LIMIT 1",
                params!(token_hash.to_owned(), now.to_rfc3339()),
            )
            .await
            .context("query shared auth identity cache")?;

        Ok(rows.into_iter().next().map(|row| CachedExternalIdentity {
            provider: row.provider,
            external_user_id: row.external_user_id,
            email: row.email,
            display_name: row.display_name,
            active: row.active != 0,
            expires_at: DateTime::parse_from_rfc3339(&row.expires_at)
                .map(|ts| ts.with_timezone(&Utc))
                .unwrap_or(now),
        }))
    }

    pub async fn put_cached_external_identity(
        &self,
        token_hash: &str,
        identity: &CachedExternalIdentity,
        expires_at: DateTime<Utc>,
    ) -> Result<()> {
        let now = now_string();
        self.runtime
            .execute(
                "DELETE FROM auth_identity_cache WHERE expires_at <= $1",
                params!(now.clone()),
            )
            .await
            .context("purge expired auth identity cache")?;
        self.runtime
            .execute(
                "INSERT INTO auth_identity_cache (
                    token_hash, provider, external_user_id, email, display_name, active, expires_at, updated_at
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                 ON CONFLICT(token_hash) DO UPDATE SET
                    provider = excluded.provider,
                    external_user_id = excluded.external_user_id,
                    email = excluded.email,
                    display_name = excluded.display_name,
                    active = excluded.active,
                    expires_at = excluded.expires_at,
                    updated_at = excluded.updated_at",
                params!(
                    token_hash.to_owned(),
                    identity.provider.clone(),
                    identity.external_user_id.clone(),
                    identity.email.clone(),
                    identity.display_name.clone(),
                    if identity.active { 1_i64 } else { 0_i64 },
                    expires_at.to_rfc3339(),
                    now.clone()
                ),
            )
            .await
            .context("upsert shared auth identity cache")?;
        Ok(())
    }

    pub async fn has_active_organization_membership(
        &self,
        user_id: &str,
        organization_id: &str,
    ) -> Result<bool> {
        self.has_active_organization_membership_with_consistency(user_id, organization_id, None)
            .await
    }

    pub async fn has_organization_membership_with_consistency(
        &self,
        user_id: &str,
        organization_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<bool> {
        let row: CountRow = self
            .runtime
            .query_rows(
                self.validation_read_consistency_with_override(consistency),
                "SELECT COUNT(*) as count
                 FROM organization_memberships
                 WHERE user_id = $1
                   AND organization_id = $2
                   AND status = 'active'",
                params!(user_id.to_owned(), organization_id.to_owned()),
            )
            .await
            .context("check organization membership including inactive organization")?
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("membership count query returned no row"))?;
        Ok(row.count > 0)
    }

    pub async fn has_active_organization_membership_with_consistency(
        &self,
        user_id: &str,
        organization_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<bool> {
        let row: CountRow = self
            .runtime
            .query_rows(
                self.validation_read_consistency_with_override(consistency),
                "SELECT COUNT(*) as count
                 FROM organization_memberships om
                 JOIN organizations o ON o.id = om.organization_id
                 WHERE om.user_id = $1
                   AND om.organization_id = $2
                   AND om.status = 'active'
                   AND o.inactivated_at IS NULL",
                params!(user_id.to_owned(), organization_id.to_owned()),
            )
            .await
            .context("check organization membership")?
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("membership count query returned no row"))?;
        Ok(row.count > 0)
    }

    pub async fn get_user(&self, user_id: &str) -> Result<User> {
        self.find_user_by_id(user_id, None)
            .await?
            .ok_or_else(|| anyhow!("user not found: {user_id}"))
    }

    pub async fn list_active_organizations_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<Organization>> {
        self.list_active_organizations_for_user_with_consistency(user_id, None)
            .await
    }

    pub async fn list_active_organizations_for_user_with_consistency(
        &self,
        user_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Vec<Organization>> {
        let rows: Vec<OrganizationRow> = self
            .runtime
            .query_rows(
                self.validation_read_consistency_with_override(consistency),
                "SELECT o.id, opl.party_id, o.business_no, o.name, o.inactivated_at
                 FROM organizations o
                 LEFT JOIN organization_party_links opl
                   ON opl.organization_id = o.id
                  AND opl.relation_type = 'legal'
                 JOIN organization_memberships om ON om.organization_id = o.id
                 WHERE om.user_id = $1
                   AND om.status = 'active'
                   AND o.inactivated_at IS NULL
                 ORDER BY o.name ASC, o.id ASC",
                params!(user_id.to_owned()),
            )
            .await
            .context("list active organizations for user")?;
        Ok(rows.into_iter().map(Organization::from).collect())
    }

    pub async fn upsert_role(&self, role: &Role) -> Result<()> {
        let now = now_string();
        self.runtime
            .execute(
                "INSERT INTO roles (id, organization_id, code, name, description, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)
                 ON CONFLICT(id) DO UPDATE SET
                    organization_id = excluded.organization_id,
                    code = excluded.code,
                    name = excluded.name,
                    description = excluded.description,
                    updated_at = excluded.updated_at",
                params!(
                    role.id.clone(),
                    role.organization_id.clone(),
                    role.code.clone(),
                    role.name.clone(),
                    role.description.clone(),
                    now.clone(),
                    now
                ),
            )
            .await
            .context("upsert role")?;
        Ok(())
    }

    pub async fn upsert_role_action(&self, role_id: &str, action_id: &str) -> Result<()> {
        self.runtime
            .execute(
                "INSERT INTO role_actions (role_id, action_id)
                 VALUES ($1, $2)
                 ON CONFLICT(role_id, action_id) DO NOTHING",
                params!(role_id.to_owned(), action_id.to_owned()),
            )
            .await
            .context("upsert role action")?;
        Ok(())
    }

    fn validate_client_references<'a>(
        &'a self,
        client: &'a Client,
    ) -> impl std::future::Future<Output = Result<()>> + 'a {
        async move {
            if !self
                .exists_row_with(
                    ReadConsistency::Strong,
                    "SELECT COUNT(*) as count FROM organizations WHERE id = $1",
                    params!(client.organization_id.clone()),
                    "client organization existence",
                )
                .await?
            {
                return Err(anyhow!(
                    "client organization {} does not exist",
                    client.organization_id
                ));
            }
            if !self
                .exists_row_with(
                    ReadConsistency::Strong,
                    "SELECT COUNT(*) as count FROM users WHERE id = $1",
                    params!(client.created_by_user_id.clone()),
                    "client creator existence",
                )
                .await?
            {
                return Err(anyhow!(
                    "client creator user {} does not exist",
                    client.created_by_user_id
                ));
            }
            Ok(())
        }
    }

    pub async fn insert_client(&self, client: &Client) -> Result<()> {
        self.validate_client_references(client).await?;
        let now = now_string();
        self.runtime
            .execute(
                "INSERT INTO clients (
                    id, organization_id, slug, name, description, client_type, app_type,
                    first_party, consent_policy, active, created_by_user_id, created_at, updated_at
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
                params!(
                    client.id.clone(),
                    client.organization_id.clone(),
                    client.slug.clone(),
                    client.name.clone(),
                    client.description.clone(),
                    client_type_str(&client.client_type),
                    client_app_type_str(&client.app_type),
                    if client.first_party { 1_i64 } else { 0_i64 },
                    client_consent_policy_str(&client.consent_policy),
                    if client.active { 1_i64 } else { 0_i64 },
                    client.created_by_user_id.clone(),
                    now.clone(),
                    now
                ),
            )
            .await
            .context("insert client")?;
        Ok(())
    }

    pub async fn update_client(&self, client: &Client) -> Result<()> {
        self.validate_client_references(client).await?;
        let conflict_count = self
            .query_count(
                ReadConsistency::Strong,
                "SELECT COUNT(*) as count
                 FROM clients
                 WHERE slug = $1
                   AND id <> $2",
                params!(client.slug.clone(), client.id.clone()),
                "client slug conflict",
            )
            .await?;
        if conflict_count > 0 {
            return Err(anyhow!("client slug already exists: {}", client.slug));
        }
        let now = now_string();
        let updated = self
            .runtime
            .execute(
                "UPDATE clients
                 SET organization_id = $2,
                     slug = $3,
                     name = $4,
                     description = $5,
                     client_type = $6,
                     app_type = $7,
                     first_party = $8,
                     consent_policy = $9,
                     active = $10,
                     created_by_user_id = $11,
                     updated_at = $12
                 WHERE id = $1",
                params!(
                    client.id.clone(),
                    client.organization_id.clone(),
                    client.slug.clone(),
                    client.name.clone(),
                    client.description.clone(),
                    client_type_str(&client.client_type),
                    client_app_type_str(&client.app_type),
                    if client.first_party { 1_i64 } else { 0_i64 },
                    client_consent_policy_str(&client.consent_policy),
                    if client.active { 1_i64 } else { 0_i64 },
                    client.created_by_user_id.clone(),
                    now
                ),
            )
            .await
            .context("update client")?;
        if updated == 0 {
            if self
                .find_client_by_id_with_consistency(&client.id, Some(ReadConsistency::Strong))
                .await?
                .is_none()
            {
                return Err(anyhow!("client not found: {}", client.id));
            }
        }
        Ok(())
    }

    pub async fn upsert_client(&self, client: &Client) -> Result<()> {
        self.validate_client_references(client).await?;
        let now = now_string();
        self.runtime
            .execute(
                "INSERT INTO clients (
                    id, organization_id, slug, name, description, client_type, app_type,
                    first_party, consent_policy, active, created_by_user_id, created_at, updated_at
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
                 ON CONFLICT(id) DO UPDATE SET
                    organization_id = excluded.organization_id,
                    slug = excluded.slug,
                    name = excluded.name,
                    description = excluded.description,
                    client_type = excluded.client_type,
                    app_type = excluded.app_type,
                    first_party = excluded.first_party,
                    consent_policy = excluded.consent_policy,
                    active = excluded.active,
                    created_by_user_id = excluded.created_by_user_id,
                    updated_at = excluded.updated_at",
                params!(
                    client.id.clone(),
                    client.organization_id.clone(),
                    client.slug.clone(),
                    client.name.clone(),
                    client.description.clone(),
                    client_type_str(&client.client_type),
                    client_app_type_str(&client.app_type),
                    if client.first_party { 1_i64 } else { 0_i64 },
                    client_consent_policy_str(&client.consent_policy),
                    if client.active { 1_i64 } else { 0_i64 },
                    client.created_by_user_id.clone(),
                    now.clone(),
                    now
                ),
            )
            .await
            .context("upsert client")?;
        Ok(())
    }

    pub async fn list_clients_with_consistency(
        &self,
        organization_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Vec<Client>> {
        let rows: Vec<ClientRow> = self
            .query_rows_with(
                self.validation_read_consistency_with_override(consistency),
                "SELECT id, organization_id, slug, name, description, client_type, app_type,
                        first_party, consent_policy, active, created_by_user_id
                 FROM clients
                 WHERE organization_id = $1
                 ORDER BY name ASC, slug ASC, id ASC",
                params!(organization_id.to_owned()),
                "clients by organization",
            )
            .await?;
        rows.into_iter().map(TryInto::try_into).collect()
    }

    pub async fn find_client_by_id_with_consistency(
        &self,
        client_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Option<Client>> {
        let rows: Vec<ClientRow> = self
            .query_rows_with(
                self.validation_read_consistency_with_override(consistency),
                "SELECT id, organization_id, slug, name, description, client_type, app_type,
                        first_party, consent_policy, active, created_by_user_id
                 FROM clients
                 WHERE id = $1
                 LIMIT 1",
                params!(client_id.to_owned()),
                "client by id optional",
            )
            .await?;
        rows.into_iter().next().map(TryInto::try_into).transpose()
    }

    pub async fn get_client_with_consistency(
        &self,
        client_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Client> {
        let client = self
            .find_client_by_id_with_consistency(client_id, consistency)
            .await?
            .ok_or_else(|| anyhow!("client not found: {client_id}"))?;
        Ok(client)
    }

    pub async fn find_client_by_slug_with_consistency(
        &self,
        slug: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Option<Client>> {
        let rows: Vec<ClientRow> = self
            .query_rows_with(
                self.validation_read_consistency_with_override(consistency),
                "SELECT id, organization_id, slug, name, description, client_type, app_type,
                        first_party, consent_policy, active, created_by_user_id
                 FROM clients
                 WHERE slug = $1
                 LIMIT 1",
                params!(slug.to_owned()),
                "client by slug optional",
            )
            .await?;
        rows.into_iter().next().map(TryInto::try_into).transpose()
    }

    pub async fn upsert_client_action(&self, client_action: &ClientAction) -> Result<()> {
        if !self
            .exists_row_with(
                ReadConsistency::Strong,
                "SELECT COUNT(*) as count FROM clients WHERE id = $1",
                params!(client_action.client_id.clone()),
                "client existence for action",
            )
            .await?
        {
            return Err(anyhow!("client {} does not exist", client_action.client_id));
        }
        let grantable_count = self
            .query_count(
                ReadConsistency::Strong,
                "SELECT COUNT(*) as count
                 FROM actions
                 WHERE id = $1
                   AND grantable_to_clients = 1",
                params!(client_action.action_id.clone()),
                "grantable client action",
            )
            .await?;
        if grantable_count == 0 {
            return Err(anyhow!(
                "action {} is not grantable to clients",
                client_action.action_id
            ));
        }
        self.runtime
            .execute(
                "INSERT INTO client_actions (client_id, action_id)
                 VALUES ($1, $2)
                 ON CONFLICT(client_id, action_id) DO NOTHING",
                params!(
                    client_action.client_id.clone(),
                    client_action.action_id.clone()
                ),
            )
            .await
            .context("upsert client action")?;
        Ok(())
    }

    pub async fn replace_client_actions(
        &self,
        client_id: &str,
        action_ids: &[String],
    ) -> Result<Vec<Action>> {
        if !self
            .exists_row_with(
                ReadConsistency::Strong,
                "SELECT COUNT(*) as count FROM clients WHERE id = $1",
                params!(client_id.to_owned()),
                "client existence for action replace",
            )
            .await?
        {
            return Err(anyhow!("client {client_id} does not exist"));
        }
        let mut deduped = Vec::new();
        let mut seen = HashSet::new();
        for action_id in action_ids {
            let trimmed = action_id.trim();
            if trimmed.is_empty() {
                return Err(anyhow!("client action ids must not be empty"));
            }
            if seen.insert(trimmed.to_owned()) {
                deduped.push(trimmed.to_owned());
            }
        }

        if !deduped.is_empty() {
            let placeholders = (1..=deduped.len())
                .map(|index| format!("${index}"))
                .collect::<Vec<_>>()
                .join(", ");
            let grantable_rows: Vec<ActionLookupRow> = self
                .query_rows_with(
                    ReadConsistency::Strong,
                    format!(
                        "SELECT id
                         FROM actions
                         WHERE grantable_to_clients = 1
                           AND id IN ({placeholders})"
                    ),
                    deduped
                        .iter()
                        .cloned()
                        .map(hiqlite::Param::from)
                        .collect::<Vec<_>>(),
                    "grantable client action replace set",
                )
                .await?;
            let grantable_ids = grantable_rows
                .into_iter()
                .map(|row| row.id)
                .collect::<HashSet<_>>();
            if let Some(missing) = deduped
                .iter()
                .find(|action_id| !grantable_ids.contains(*action_id))
            {
                return Err(anyhow!("action {missing} is not grantable to clients"));
            }
        }

        let mut queries = Vec::with_capacity(1 + deduped.len());
        queries.push((
            Cow::Borrowed("DELETE FROM client_actions WHERE client_id = $1"),
            params!(client_id.to_owned()),
        ));
        for action_id in &deduped {
            queries.push((
                Cow::Borrowed(
                    "INSERT INTO client_actions (client_id, action_id)
                     VALUES ($1, $2)
                     ON CONFLICT(client_id, action_id) DO NOTHING",
                ),
                params!(client_id.to_owned(), action_id.clone()),
            ));
        }

        self.runtime
            .transaction(queries)
            .await
            .context("replace client actions transaction")?;
        self.list_client_actions_with_consistency(client_id, Some(ReadConsistency::Strong))
            .await
    }

    pub async fn list_client_actions_with_consistency(
        &self,
        client_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Vec<Action>> {
        let rows: Vec<ActionRow> = self
            .query_rows_with(
                self.validation_read_consistency_with_override(consistency),
                "SELECT a.id, a.code, a.name, a.description, a.risk_level, a.visibility_granting,
                        a.grantable_to_clients, a.consent_label, a.consent_description
                 FROM client_actions ca
                 JOIN actions a ON a.id = ca.action_id
                 WHERE ca.client_id = $1
                 ORDER BY a.code ASC",
                params!(client_id.to_owned()),
                "client actions",
            )
            .await?;
        rows.into_iter().map(TryInto::try_into).collect()
    }

    pub async fn upsert_client_grant(&self, client_grant: &ClientGrant) -> Result<String> {
        if !self
            .exists_row_with(
                ReadConsistency::Strong,
                "SELECT COUNT(*) as count FROM users WHERE id = $1",
                params!(client_grant.user_id.clone()),
                "client grant user existence",
            )
            .await?
        {
            return Err(anyhow!(
                "client grant user {} does not exist",
                client_grant.user_id
            ));
        }
        if !self
            .exists_row_with(
                ReadConsistency::Strong,
                "SELECT COUNT(*) as count FROM clients WHERE id = $1",
                params!(client_grant.client_id.clone()),
                "client grant client existence",
            )
            .await?
        {
            return Err(anyhow!(
                "client grant client {} does not exist",
                client_grant.client_id
            ));
        }
        if !self
            .exists_row_with(
                ReadConsistency::Strong,
                "SELECT COUNT(*) as count FROM organizations WHERE id = $1",
                params!(client_grant.organization_id.clone()),
                "client grant organization existence",
            )
            .await?
        {
            return Err(anyhow!(
                "client grant organization {} does not exist",
                client_grant.organization_id
            ));
        }
        if client_grant.status == ClientGrantStatus::Active {
            let existing: Vec<ClientGrantLookupRow> = self
                .query_consistent_rows(
                    "SELECT id, created_at
                     FROM client_grants
                     WHERE user_id = $1
                       AND client_id = $2
                       AND organization_id = $3
                       AND status = 'active'
                     LIMIT 1",
                    params!(
                        client_grant.user_id.clone(),
                        client_grant.client_id.clone(),
                        client_grant.organization_id.clone()
                    ),
                    "active client grant lookup",
                )
                .await?;
            if let Some(existing) = existing.into_iter().next() {
                self.runtime
                    .transaction([
                        (
                            Cow::Borrowed(
                                "UPDATE client_grants
                                 SET user_id = $1,
                                     client_id = $2,
                                     organization_id = $3,
                                     status = $4,
                                     consented_at = $5,
                                     revoked_at = $6,
                                     revoked_by_user_id = $7,
                                     updated_at = $8
                                 WHERE id = $9",
                            ),
                            params!(
                                client_grant.user_id.clone(),
                                client_grant.client_id.clone(),
                                client_grant.organization_id.clone(),
                                client_grant_status_str(&client_grant.status),
                                client_grant.consented_at.clone(),
                                client_grant.revoked_at.clone(),
                                client_grant.revoked_by_user_id.clone(),
                                client_grant.updated_at.clone(),
                                existing.id.clone()
                            ),
                        ),
                        (
                            Cow::Borrowed(
                                "DELETE FROM client_grant_actions WHERE client_grant_id = $1",
                            ),
                            params!(existing.id.clone()),
                        ),
                    ])
                    .await
                    .context("reset existing active client grant transaction")?;
                return Ok(existing.id);
            }
        }

        self.runtime
            .execute(
                "INSERT INTO client_grants (
                    id, user_id, client_id, organization_id, status, consented_at,
                    revoked_at, revoked_by_user_id, created_at, updated_at
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                 ON CONFLICT(id) DO UPDATE SET
                    user_id = excluded.user_id,
                    client_id = excluded.client_id,
                    organization_id = excluded.organization_id,
                    status = excluded.status,
                    consented_at = excluded.consented_at,
                    revoked_at = excluded.revoked_at,
                    revoked_by_user_id = excluded.revoked_by_user_id,
                    created_at = excluded.created_at,
                    updated_at = excluded.updated_at",
                params!(
                    client_grant.id.clone(),
                    client_grant.user_id.clone(),
                    client_grant.client_id.clone(),
                    client_grant.organization_id.clone(),
                    client_grant_status_str(&client_grant.status),
                    client_grant.consented_at.clone(),
                    client_grant.revoked_at.clone(),
                    client_grant.revoked_by_user_id.clone(),
                    client_grant.created_at.clone(),
                    client_grant.updated_at.clone()
                ),
            )
            .await
            .context("upsert client grant")?;
        Ok(client_grant.id.clone())
    }

    pub async fn upsert_client_grant_action(
        &self,
        client_grant_action: &ClientGrantAction,
    ) -> Result<()> {
        let allowed_count = self
            .query_count(
                self.runtime.default_read_consistency(),
                "SELECT COUNT(*) as count
                 FROM client_grants cg
                 JOIN client_actions ca
                   ON ca.client_id = cg.client_id
                 WHERE cg.id = $1
                   AND ca.action_id = $2",
                params!(
                    client_grant_action.client_grant_id.clone(),
                    client_grant_action.action_id.clone()
                ),
                "client grant action subset",
            )
            .await?;
        if allowed_count == 0 {
            return Err(anyhow!(
                "client grant action must be included in client_actions for the parent client"
            ));
        }
        self.runtime
            .execute(
                "INSERT INTO client_grant_actions (client_grant_id, action_id)
                 VALUES ($1, $2)
                 ON CONFLICT(client_grant_id, action_id) DO NOTHING",
                params!(
                    client_grant_action.client_grant_id.clone(),
                    client_grant_action.action_id.clone()
                ),
            )
            .await
            .context("upsert client grant action")?;
        Ok(())
    }

    pub async fn upsert_agreement(&self, agreement: &Agreement) -> Result<()> {
        let now = now_string();
        self.runtime
            .execute(
                "INSERT INTO agreements (
                    id, agreement_type, grantor_organization_id, grantee_organization_id,
                    status, title, reference_no, starts_at, ends_at, created_at, updated_at
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
                 ON CONFLICT(id) DO UPDATE SET
                    agreement_type = excluded.agreement_type,
                    grantor_organization_id = excluded.grantor_organization_id,
                    grantee_organization_id = excluded.grantee_organization_id,
                    status = excluded.status,
                    title = excluded.title,
                    reference_no = excluded.reference_no,
                    starts_at = excluded.starts_at,
                    ends_at = excluded.ends_at,
                    updated_at = excluded.updated_at",
                params!(
                    agreement.id.clone(),
                    agreement_type_str(&agreement.agreement_type),
                    agreement.grantor_organization_id.clone(),
                    agreement.grantee_organization_id.clone(),
                    agreement_status_str(&agreement.status),
                    agreement.title.clone(),
                    agreement.reference_no.clone(),
                    agreement.starts_at.clone(),
                    agreement.ends_at.clone(),
                    now.clone(),
                    now
                ),
            )
            .await
            .context("upsert agreement")?;
        self.rebuild_organization_expansion().await?;
        Ok(())
    }

    pub async fn upsert_agreement_document(&self, document: &AgreementDocument) -> Result<()> {
        self.runtime
            .execute(
                "INSERT INTO agreement_documents (
                    id, agreement_id, document_type, file_name, object_key, content_type, document_no, uploaded_at
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                 ON CONFLICT(id) DO UPDATE SET
                    agreement_id = excluded.agreement_id,
                    document_type = excluded.document_type,
                    file_name = excluded.file_name,
                    object_key = excluded.object_key,
                    content_type = excluded.content_type,
                    document_no = excluded.document_no,
                    uploaded_at = excluded.uploaded_at",
                params!(
                    document.id.clone(),
                    document.agreement_id.clone(),
                    agreement_document_type_str(&document.document_type),
                    document.file_name.clone(),
                    document.object_key.clone(),
                    document.content_type.clone(),
                    document.document_no.clone(),
                    document.uploaded_at.clone()
                ),
            )
            .await
            .context("upsert agreement document")?;
        Ok(())
    }

    pub async fn list_agreement_documents(
        &self,
        agreement_id: &str,
    ) -> Result<Vec<AgreementDocument>> {
        let rows: Vec<AgreementDocumentRow> = self
            .query_default_rows(
                "SELECT id, agreement_id, document_type, file_name, object_key, content_type, document_no, uploaded_at
                 FROM agreement_documents
                 WHERE agreement_id = $1
                 ORDER BY uploaded_at DESC, file_name ASC",
                params!(agreement_id.to_owned()),
                "agreement documents",
            )
            .await
            .context("list agreement documents")?;
        rows.into_iter().map(TryInto::try_into).collect()
    }

    pub async fn get_agreement_document(&self, id: &str) -> Result<AgreementDocument> {
        let rows: Vec<AgreementDocumentRow> = self
            .query_default_rows(
                "SELECT id, agreement_id, document_type, file_name, object_key, content_type, document_no, uploaded_at
                 FROM agreement_documents
                 WHERE id = $1
                 LIMIT 1",
                params!(id.to_owned()),
                "agreement document",
            )
            .await
            .context("get agreement document")?;
        let row = rows
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("agreement document not found"))?;
        row.try_into()
    }

    pub async fn save_certificate(
        &self,
        organization_id: &str,
        business_no: Option<&str>,
        common_name: &str,
        encrypted_blob: &str,
        wrapped_dek: &[u8],
        kms_key_id: &str,
    ) -> Result<StoredCertificate> {
        self.upsert_organization_identity(organization_id, None, "")
            .await?;

        let mut party_id = self.find_organization_party_id(organization_id).await?;
        if party_id.is_none() {
            if let Some(business_no) = business_no {
                let now = now_string();
                let linked_party_id = match self.find_party_id_by_business_no(business_no).await? {
                    Some(existing_party_id) => existing_party_id,
                    None => {
                        self.upsert_party_by_business_no(business_no, common_name, &now)
                            .await?
                    }
                };
                self.upsert_organization_party_link(
                    organization_id,
                    &linked_party_id,
                    organization_party_relation_type_str_legal(),
                    &now,
                )
                .await?;
                party_id = Some(linked_party_id);
            }
        }
        let party_id = party_id.ok_or_else(|| {
            anyhow!(
                "organization missing legal party link; register legal identity before uploading a certificate"
            )
        })?;

        self.runtime
            .execute(
                "UPDATE certificates SET active = 0 WHERE party_id = $1",
                params!(party_id.clone()),
            )
            .await
            .context("deactivate prior certificates")?;
        self.runtime
            .execute(
                "UPDATE auth_credentials
                 SET active = 0, updated_at = $1
                 WHERE organization_id = $2
                   AND provider = 'korea_energy_rps'
                   AND credential_type = 'certificate'",
                params!(now_string(), organization_id.to_owned()),
            )
            .await
            .context("deactivate prior KR RPS auth credentials")?;

        let id = Uuid::new_v4().simple().to_string();
        let credential_id = format!("auth-credential-{id}");
        let now = now_string();
        self.runtime
            .execute(
                "INSERT INTO certificates (id, party_id, common_name, encrypted_blob, wrapped_dek, kms_key_id, active, last_used_at, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
                params!(
                    id.clone(),
                    party_id,
                    common_name.to_owned(),
                    encrypted_blob.to_owned(),
                    STANDARD.encode(wrapped_dek),
                    kms_key_id.to_owned(),
                    1_i64,
                    Option::<String>::None,
                    now.clone()
                ),
            )
            .await
            .context("insert certificate")?;
        self.runtime
            .execute(
                "INSERT INTO auth_credentials (
                    id, organization_id, credential_type, country_code, provider, external_subject_id,
                    encrypted_blob, wrapped_dek, kms_key_id, active, last_used_at, created_at, updated_at
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
                params!(
                    credential_id.clone(),
                    organization_id.to_owned(),
                    "certificate".to_owned(),
                    Some("KR".to_owned()),
                    "korea_energy_rps".to_owned(),
                    business_no.map(ToOwned::to_owned),
                    encrypted_blob.to_owned(),
                    STANDARD.encode(wrapped_dek),
                    kms_key_id.to_owned(),
                    1_i64,
                    Option::<String>::None,
                    now.clone(),
                    now.clone()
                ),
            )
            .await
            .context("insert KR RPS auth credential")?;
        self.runtime
            .execute(
                "INSERT OR IGNORE INTO credential_bindings (
                    credential_id, registry_account_id, created_at
                 )
                 SELECT $1, ra.id, $2
                 FROM registry_accounts ra
                 WHERE ra.organization_id = $3
                   AND ra.registry_id = 'registry-kr-rps'
                   AND ra.status = 'active'
                   AND ($4 IS NULL OR ra.external_account_id = $4)",
                params!(
                    credential_id,
                    now,
                    organization_id.to_owned(),
                    business_no.map(ToOwned::to_owned)
                ),
            )
            .await
            .context("bind KR RPS auth credential to existing registry accounts")?;
        self.load_active_certificate(organization_id).await
    }

    pub async fn get_organization(&self, organization_id: &str) -> Result<Organization> {
        let rows: Vec<OrganizationRow> = self
            .query_default_rows(
                "SELECT o.id, opl.party_id, o.business_no, o.name, o.inactivated_at
                 FROM organizations o
                 LEFT JOIN organization_party_links opl
                   ON opl.organization_id = o.id
                  AND opl.relation_type = 'legal'
                 WHERE o.id = $1
                 LIMIT 1",
                params!(organization_id.to_owned()),
                "organization",
            )
            .await
            .context("query organization")?;
        let row = rows
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("organization not found"))?;
        Ok(row.into())
    }

    pub async fn get_organization_business_no(
        &self,
        organization_id: &str,
    ) -> Result<Option<String>> {
        let rows: Vec<OrganizationRow> = self
            .query_default_rows(
                "SELECT o.id, opl.party_id, COALESCE(p.business_no, o.business_no) AS business_no, o.name, o.inactivated_at
                 FROM organizations o
                 LEFT JOIN organization_party_links opl
                   ON opl.organization_id = o.id
                  AND opl.relation_type = 'legal'
                 LEFT JOIN parties p ON p.id = opl.party_id
                 WHERE o.id = $1
                 LIMIT 1",
                params!(organization_id.to_owned()),
                "organization business number",
            )
            .await
            .context("query organization business number")?;
        Ok(rows.into_iter().next().and_then(|row| row.business_no))
    }

    pub async fn deactivate_organization(&self, organization_id: &str) -> Result<Organization> {
        let now = now_string();
        self.runtime
            .execute(
                "UPDATE organizations
                 SET inactivated_at = $1, updated_at = $2
                 WHERE id = $3",
                params!(now.clone(), now, organization_id.to_owned()),
            )
            .await
            .context("deactivate organization")?;
        self.get_organization(organization_id).await
    }

    pub async fn reactivate_organization(&self, organization_id: &str) -> Result<Organization> {
        self.runtime
            .execute(
                "UPDATE organizations
                 SET inactivated_at = $1, updated_at = $2
                 WHERE id = $3",
                params!(
                    Option::<String>::None,
                    now_string(),
                    organization_id.to_owned()
                ),
            )
            .await
            .context("reactivate organization")?;
        self.get_organization(organization_id).await
    }

    pub async fn load_active_certificate(
        &self,
        organization_id: &str,
    ) -> Result<StoredCertificate> {
        let rows: Vec<StoredCertificateRow> = self
            .query_consistent_rows(
                "SELECT c.id, c.party_id, p.business_no, c.common_name, c.encrypted_blob, c.wrapped_dek, c.kms_key_id, c.active, c.last_used_at, c.created_at
                 FROM organization_party_links opl
                 JOIN parties p ON p.id = opl.party_id
                 JOIN certificates c ON c.party_id = opl.party_id
                 JOIN organizations o ON o.id = opl.organization_id
                 WHERE opl.organization_id = $1
                   AND opl.relation_type = 'legal'
                   AND c.active = 1
                   AND o.inactivated_at IS NULL
                 ORDER BY c.last_used_at DESC NULLS LAST, c.created_at DESC
                 LIMIT 1",
                params!(organization_id.to_owned()),
                "active certificate",
            )
            .await
            .context("query active certificate")?;
        let cert: StoredCertificate = rows
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("certificate not found"))?
            .try_into()?;
        Ok(cert)
    }

    pub async fn touch_certificate_last_used_at_if_stale(
        &self,
        cert: &StoredCertificate,
    ) -> Result<()> {
        if should_refresh_certificate_last_used_at(cert.last_used_at.as_deref())? {
            self.runtime
                .execute(
                    "UPDATE certificates SET last_used_at = $1 WHERE id = $2",
                    params!(now_string(), cert.id.clone()),
                )
                .await
                .context("update certificate last_used_at")?;
        }
        Ok(())
    }

    pub async fn get_certificate_by_id(&self, certificate_id: &str) -> Result<StoredCertificate> {
        let rows: Vec<StoredCertificateRow> = self
            .query_rows_with(
                ReadConsistency::Strong,
                "SELECT c.id, c.party_id, p.business_no, c.common_name, c.encrypted_blob, c.wrapped_dek, c.kms_key_id, c.active, c.last_used_at, c.created_at
                 FROM certificates c
                 LEFT JOIN parties p ON p.id = c.party_id
                 WHERE c.id = $1
                 LIMIT 1",
                params!(certificate_id.to_owned()),
                "certificate by id",
            )
            .await
            .context("query certificate by id")?;
        if rows.is_empty() {
            return Err(DatabaseLookupError::CertificateNotFound.into());
        }
        if rows.len() > 1 {
            return Err(anyhow!(
                "multiple rows returned: certificate by id, expected 1"
            ));
        }
        rows.into_iter()
            .next()
            .expect("single certificate row")
            .try_into()
    }

    pub async fn list_certificates(&self, organization_id: &str) -> Result<Vec<StoredCertificate>> {
        let rows: Vec<StoredCertificateRow> = self
            .query_consistent_rows(
                "SELECT c.id, c.party_id, p.business_no, c.common_name, c.encrypted_blob, c.wrapped_dek, c.kms_key_id, c.active, c.last_used_at, c.created_at
                 FROM organization_party_links opl
                 JOIN organizations o ON o.id = opl.organization_id
                 JOIN parties p ON p.id = opl.party_id
                 JOIN certificates c ON c.party_id = opl.party_id
                 WHERE opl.organization_id = $1
                   AND opl.relation_type = 'legal'
                   AND o.inactivated_at IS NULL
                 ORDER BY c.active DESC, c.last_used_at DESC NULLS LAST, c.created_at DESC",
                params!(organization_id.to_owned()),
                "organization certificates",
            )
            .await
            .context("query organization certificates")?;
        rows.into_iter().map(TryInto::try_into).collect()
    }

    pub async fn create_cert_gateway_challenge(
        &self,
        organization_id: &str,
        mode: &str,
        nonce: &str,
        payload_hash: &str,
        facility_code: Option<&str>,
        expires_at: &str,
        created_by_user_id: &str,
    ) -> Result<CertGatewayChallenge> {
        let id = Uuid::new_v4().simple().to_string();
        let created_at = now_string();
        self.runtime
            .execute(
                "INSERT INTO cert_gateway_challenges (id, organization_id, mode, nonce, payload_hash, facility_code, expires_at, consumed_at, created_by_user_id, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
                params!(
                    id.clone(),
                    organization_id.to_owned(),
                    mode.to_owned(),
                    nonce.to_owned(),
                    payload_hash.to_owned(),
                    facility_code.map(str::to_owned),
                    expires_at.to_owned(),
                    Option::<String>::None,
                    created_by_user_id.to_owned(),
                    created_at
                ),
            )
            .await
            .context("insert cert gateway challenge")?;
        self.get_cert_gateway_challenge(&id).await
    }

    pub async fn consume_cert_gateway_challenge(
        &self,
        challenge_id: &str,
    ) -> Result<CertGatewayChallenge> {
        let consumed_at = now_string();
        let updated = self
            .runtime
            .execute(
                "UPDATE cert_gateway_challenges
                 SET consumed_at = $1
                 WHERE id = $2
                   AND consumed_at IS NULL",
                params!(consumed_at.clone(), challenge_id.to_owned()),
            )
            .await
            .context("consume cert gateway challenge")?;
        if updated == 0 {
            let challenge = self.get_cert_gateway_challenge(challenge_id).await?;
            if challenge.consumed_at.is_some() {
                anyhow::bail!("cert gateway challenge already consumed");
            }
            anyhow::bail!("cert gateway challenge not found");
        }
        let challenge = self.get_cert_gateway_challenge(challenge_id).await?;
        Ok(challenge)
    }

    pub async fn issue_crawl_task_token(
        &self,
        organization_id: &str,
        certificate_id: Option<&str>,
        token_hash: &str,
        scope_json: &str,
        expires_at: &str,
        created_by_user_id: &str,
    ) -> Result<CrawlTaskToken> {
        let id = Uuid::new_v4().simple().to_string();
        let created_at = now_string();
        self.runtime
            .execute(
                "INSERT INTO crawl_task_tokens (id, organization_id, certificate_id, token_hash, scope_json, expires_at, used_at, created_by_user_id, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
                params!(
                    id.clone(),
                    organization_id.to_owned(),
                    certificate_id.map(str::to_owned),
                    token_hash.to_owned(),
                    scope_json.to_owned(),
                    expires_at.to_owned(),
                    Option::<String>::None,
                    created_by_user_id.to_owned(),
                    created_at
                ),
            )
            .await
            .context("insert crawl task token")?;
        self.get_crawl_task_token_by_hash(token_hash).await
    }

    pub async fn consume_crawl_task_token(&self, token_hash: &str) -> Result<CrawlTaskToken> {
        let used_at = now_string();
        let updated = self
            .runtime
            .execute(
                "UPDATE crawl_task_tokens
                 SET used_at = $1
                 WHERE token_hash = $2
                   AND used_at IS NULL",
                params!(used_at.clone(), token_hash.to_owned()),
            )
            .await
            .context("consume crawl task token")?;
        if updated == 0 {
            let token = self.get_crawl_task_token_by_hash(token_hash).await?;
            if token.used_at.is_some() {
                anyhow::bail!("crawl task token already used");
            }
            anyhow::bail!("crawl task token not found");
        }
        let token = self.get_crawl_task_token_by_hash(token_hash).await?;
        Ok(token)
    }

    pub async fn get_cert_gateway_challenge(
        &self,
        challenge_id: &str,
    ) -> Result<CertGatewayChallenge> {
        let rows: Vec<CertGatewayChallengeRow> = self
            .query_rows_with(
                ReadConsistency::Strong,
                "SELECT id, organization_id, mode, nonce, payload_hash, facility_code, expires_at, consumed_at, created_by_user_id, created_at
                 FROM cert_gateway_challenges
                 WHERE id = $1
                 LIMIT 1",
                params!(challenge_id.to_owned()),
                "cert gateway challenge",
            )
            .await
            .context("query cert gateway challenge")?;
        if rows.is_empty() {
            return Err(DatabaseLookupError::CertGatewayChallengeNotFound.into());
        }
        if rows.len() > 1 {
            return Err(anyhow!(
                "multiple rows returned: cert gateway challenge, expected 1"
            ));
        }
        Ok(rows
            .into_iter()
            .next()
            .expect("single cert gateway challenge row")
            .into())
    }

    pub async fn get_crawl_task_token_by_hash(&self, token_hash: &str) -> Result<CrawlTaskToken> {
        let rows: Vec<CrawlTaskTokenRow> = self
            .query_rows_with(
                ReadConsistency::Strong,
                "SELECT id, organization_id, certificate_id, token_hash, scope_json, expires_at, used_at, created_by_user_id, created_at
                 FROM crawl_task_tokens
                 WHERE token_hash = $1
                 LIMIT 1",
                params!(token_hash.to_owned()),
                "crawl task token",
            )
            .await
            .context("query crawl task token")?;
        if rows.is_empty() {
            return Err(DatabaseLookupError::CrawlTaskTokenNotFound.into());
        }
        if rows.len() > 1 {
            return Err(anyhow!(
                "multiple rows returned: crawl task token, expected 1"
            ));
        }
        Ok(rows
            .into_iter()
            .next()
            .expect("single crawl task token row")
            .into())
    }

    pub async fn start_sync_run(
        &self,
        organization_id: &str,
        plant_id: Option<&str>,
        mode: SyncMode,
    ) -> Result<SyncRun> {
        let id = Uuid::new_v4().simple().to_string();
        let started_at = now_string();
        self.runtime
            .execute(
                "INSERT INTO sync_runs (id, organization_id, plant_id, mode, status, started_at, finished_at, duration_ms, error_message)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
                params!(
                    id.clone(),
                    organization_id.to_owned(),
                    plant_id.map(ToOwned::to_owned),
                    sync_mode_str(&mode),
                    "running",
                    started_at.clone(),
                    Option::<String>::None,
                    0_i64,
                    Option::<String>::None
                ),
            )
            .await
            .context("insert sync run")?;
        Ok(SyncRun {
            id,
            organization_id: organization_id.to_owned(),
            plant_id: plant_id.map(ToOwned::to_owned),
            mode,
            status: "running".to_owned(),
            started_at: Some(started_at),
            finished_at: None,
            duration_ms: 0,
        })
    }

    pub async fn finish_sync_run(
        &self,
        run_id: &str,
        status: &str,
        duration_ms: i64,
        error_message: Option<&str>,
    ) -> Result<()> {
        self.finish_sync_run_at(run_id, status, duration_ms, error_message, &now_string())
            .await
    }

    pub async fn finish_sync_run_at(
        &self,
        run_id: &str,
        status: &str,
        duration_ms: i64,
        error_message: Option<&str>,
        finished_at: &str,
    ) -> Result<()> {
        self.runtime
            .execute(
                "UPDATE sync_runs
                 SET status = $1, finished_at = $2, duration_ms = $3, error_message = $4
                 WHERE id = $5",
                params!(
                    status.to_owned(),
                    finished_at.to_owned(),
                    duration_ms,
                    error_message.map(ToOwned::to_owned),
                    run_id.to_owned()
                ),
            )
            .await
            .context("finish sync run")?;
        Ok(())
    }

    pub async fn refresh_completed_sync_run_timing(
        &self,
        run_id: &str,
        duration_ms: i64,
        finished_at: &str,
    ) -> Result<()> {
        self.runtime
            .execute(
                "UPDATE sync_runs
                 SET finished_at = $1, duration_ms = $2
                 WHERE id = $3 AND status = 'completed'",
                params!(finished_at.to_owned(), duration_ms, run_id.to_owned()),
            )
            .await
            .context("refresh completed sync run timing")?;
        Ok(())
    }

    pub async fn update_sync_run_target(&self, run_id: &str, plant_id: Option<&str>) -> Result<()> {
        self.runtime
            .execute(
                "UPDATE sync_runs SET plant_id = $1 WHERE id = $2",
                params!(plant_id.map(ToOwned::to_owned), run_id.to_owned()),
            )
            .await
            .context("update sync run target")?;
        Ok(())
    }

    pub async fn upsert_sync_run_item(
        &self,
        sync_run_id: &str,
        asset_id: Option<&str>,
        facility_code: &str,
        status: &str,
        generation_count: usize,
        rate_count: usize,
        rec_count: usize,
        document_count: usize,
        error_message: Option<&str>,
    ) -> Result<()> {
        let now = now_string();
        let id = stable_scoped_id("sync_run_item", &format!("{sync_run_id}:{facility_code}"));
        let registry_project_id =
            asset_id.map(|_| stable_scoped_id("registry_project:kr_rps", facility_code));
        let data_source_id =
            asset_id.map(|_| stable_scoped_id("data_source:kr_rps:generation", facility_code));
        self.runtime
            .execute(
                "INSERT INTO sync_run_items (
                    id, sync_run_id, asset_id, registry_project_id, data_source_id,
                    facility_code, status, generation_count, rate_count, rec_count,
                    document_count, error_message, write_version, created_at, updated_at
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
                 ON CONFLICT(sync_run_id, facility_code) DO UPDATE SET
                    asset_id = excluded.asset_id,
                    registry_project_id = excluded.registry_project_id,
                    data_source_id = excluded.data_source_id,
                    status = excluded.status,
                    generation_count = excluded.generation_count,
                    rate_count = excluded.rate_count,
                    rec_count = excluded.rec_count,
                    document_count = excluded.document_count,
                    error_message = excluded.error_message,
                    write_version = excluded.write_version,
                    updated_at = excluded.updated_at",
                params!(
                    id,
                    sync_run_id.to_owned(),
                    asset_id.map(ToOwned::to_owned),
                    registry_project_id,
                    data_source_id,
                    facility_code.to_owned(),
                    status.to_owned(),
                    generation_count as i64,
                    rate_count as i64,
                    rec_count as i64,
                    document_count as i64,
                    error_message.map(ToOwned::to_owned),
                    now.clone(),
                    now.clone(),
                    now
                ),
            )
            .await
            .context("upsert sync run item")?;
        Ok(())
    }

    fn push_upsert_plant_query(
        queries: &mut Vec<(String, Vec<hiqlite::Param>)>,
        plant: &Plant,
        now: &str,
    ) {
        queries.push(("INSERT INTO plants (id, facility_code, owner_organization_id, created_by_organization_id, name, contract_no, address, installed_kw, rec_start_date, confirm_issued, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
                 ON CONFLICT(id) DO UPDATE SET
                    facility_code = excluded.facility_code,
                    name = excluded.name,
                    contract_no = excluded.contract_no,
                    address = excluded.address,
                    installed_kw = excluded.installed_kw,
                    rec_start_date = excluded.rec_start_date,
                    confirm_issued = excluded.confirm_issued,
                    updated_at = excluded.updated_at".to_owned(),
                params!(
                    plant.id.clone(),
                    plant.facility_code.clone(),
                    plant.owner_organization_id.clone(),
                    plant.created_by_organization_id.clone(),
                    plant.name.clone(),
                    plant.contract_no.clone(),
                    plant.address.clone(),
                    plant.installed_kw,
                    plant.rec_start_date.clone(),
                    plant.confirm_issued.clone(),
                    now.to_owned(),
                    now.to_owned()
                ),
            ));
    }

    pub async fn upsert_plant(&self, plant: &Plant) -> Result<()> {
        let now = now_string();
        let mut queries = Vec::with_capacity(1);
        Self::push_upsert_plant_query(&mut queries, plant, &now);
        self.runtime
            .transaction(queries)
            .await
            .context("upsert plant")?;
        Ok(())
    }

    async fn push_kr_rps_asset_projection_queries(
        &self,
        queries: &mut Vec<(String, Vec<hiqlite::Param>)>,
        plant: &Plant,
        registry_account_organization_id: Option<&str>,
        business_no: Option<&str>,
        generation: &[GenerationRecord],
        rates: &[StoredRateRecord],
        rec: &[RecRecord],
        now: &str,
    ) -> Result<KrRpsProjectionContext> {
        let asset_id = self
            .resolve_kr_rps_asset_id_for_projection(&plant.facility_code)
            .await?;
        let profile_id = stable_scoped_id("asset_profile:kr_rps", &plant.facility_code);
        let registry_id = "registry-kr-rps".to_owned();
        let existing_registry_account_id = self
            .existing_kr_rps_registry_account_id_for_facility(&plant.facility_code)
            .await?;
        let new_registry_account = match (
            existing_registry_account_id.as_ref(),
            registry_account_organization_id,
            business_no,
        ) {
            (None, Some(account_organization_id), Some(account_scope)) => {
                let registry_account_id = stable_scoped_id(
                    "registry_account:kr_rps",
                    &format!("{account_organization_id}:{account_scope}"),
                );
                Some((
                    registry_account_id,
                    account_organization_id.to_owned(),
                    account_scope.to_owned(),
                ))
            }
            _ => None,
        };
        let registry_account_id = existing_registry_account_id.clone().or_else(|| {
            new_registry_account
                .as_ref()
                .map(|account| account.0.clone())
        });
        let registry_project_id = stable_scoped_id("registry_project:kr_rps", &plant.facility_code);
        let data_source_id =
            stable_scoped_id("data_source:kr_rps:generation", &plant.facility_code);
        let rate_source_id = stable_scoped_id("data_source:kr_rps:rate", &plant.facility_code);
        let profile_metadata = serde_json::json!({
            "facilityCode": plant.facility_code,
            "contractNo": plant.contract_no,
            "recStartDate": plant.rec_start_date,
            "confirmIssued": plant.confirm_issued,
            "legacyPlantId": plant.id,
        })
        .to_string();
        let registry_project_metadata = serde_json::json!({
            "facilityCode": plant.facility_code,
            "contractNo": plant.contract_no,
        })
        .to_string();

        queries.push((
            "INSERT INTO assets (
                    id, owner_organization_id, created_by_organization_id, name, country_code,
                    address, capacity_kw, timezone, created_at, updated_at
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                 ON CONFLICT(id) DO UPDATE SET
                    name = excluded.name,
                    country_code = excluded.country_code,
                    address = excluded.address,
                    capacity_kw = excluded.capacity_kw,
                    timezone = excluded.timezone,
                    updated_at = excluded.updated_at"
                .to_owned(),
            params!(
                asset_id.clone(),
                plant.owner_organization_id.clone(),
                plant.created_by_organization_id.clone(),
                plant.name.clone(),
                "KR".to_owned(),
                plant.address.clone(),
                plant.installed_kw,
                "Asia/Seoul".to_owned(),
                now.to_owned(),
                now.to_owned()
            ),
        ));

        queries.push(("INSERT INTO asset_profiles (
                    id, asset_id, country_code, provider, profile_type, external_id, metadata_json, created_at, updated_at
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                 ON CONFLICT(country_code, provider, profile_type, external_id) DO UPDATE SET
                    country_code = excluded.country_code,
                    provider = excluded.provider,
                    metadata_json = excluded.metadata_json,
                    updated_at = excluded.updated_at".to_owned(),
                params!(
                    profile_id,
                    asset_id.clone(),
                    "KR".to_owned(),
                    "korea_energy_rps".to_owned(),
                    "kr_rps".to_owned(),
                    plant.facility_code.clone(),
                    profile_metadata,
                    now.to_owned(),
                    now.to_owned()
                ),
            ));

        queries.push(("INSERT INTO registries (
                    id, registry_type, country_code, provider, name, metadata_json, created_at, updated_at
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                 ON CONFLICT(id) DO UPDATE SET
                    registry_type = excluded.registry_type,
                    country_code = excluded.country_code,
                    provider = excluded.provider,
                    name = excluded.name,
                    metadata_json = excluded.metadata_json,
                    updated_at = excluded.updated_at".to_owned(),
                params!(
                    registry_id.clone(),
                    "rec".to_owned(),
                    Some("KR".to_owned()),
                    "korea_energy_rps".to_owned(),
                    "Korea RPS".to_owned(),
                    "{}".to_owned(),
                    now.to_owned(),
                    now.to_owned()
                ),
            ));

        if let Some((registry_account_id, account_organization_id, account_scope)) =
            &new_registry_account
        {
            queries.push((
                "INSERT INTO registry_accounts (
                    id, registry_id, organization_id, external_account_id, display_name, status,
                    metadata_json, created_at, updated_at
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                 ON CONFLICT(registry_id, organization_id, external_account_id) DO UPDATE SET
                    display_name = excluded.display_name,
                    status = excluded.status,
                    metadata_json = excluded.metadata_json,
                    updated_at = excluded.updated_at"
                    .to_owned(),
                params!(
                    registry_account_id.clone(),
                    registry_id.clone(),
                    account_organization_id.clone(),
                    account_scope.to_owned(),
                    Some(account_organization_id.clone()),
                    "active".to_owned(),
                    "{}".to_owned(),
                    now.to_owned(),
                    now.to_owned()
                ),
            ));
        }

        if let Some(registry_account_id) = &registry_account_id {
            queries.push((
                "INSERT OR IGNORE INTO credential_bindings (
                        credential_id, registry_account_id, created_at
                     )
                     SELECT id, $1, $2
                     FROM auth_credentials
                     WHERE organization_id = (
                           SELECT organization_id
                           FROM registry_accounts
                           WHERE id = $3
                       )
                       AND provider = 'korea_energy_rps'
                       AND credential_type = 'certificate'
                       AND active = 1"
                    .to_owned(),
                params!(
                    registry_account_id.clone(),
                    now.to_owned(),
                    registry_account_id.clone()
                ),
            ));
        }

        queries.push(("INSERT INTO registry_projects (
                    id, registry_id, asset_id, registry_account_id, external_project_id,
                    project_type, status, metadata_json, created_at, updated_at
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                 ON CONFLICT(registry_id, external_project_id) DO UPDATE SET
                    asset_id = excluded.asset_id,
                    registry_account_id = COALESCE(registry_projects.registry_account_id, excluded.registry_account_id),
                    project_type = excluded.project_type,
                    status = excluded.status,
                    metadata_json = excluded.metadata_json,
                    updated_at = excluded.updated_at".to_owned(),
                params!(
                    registry_project_id.clone(),
                    registry_id.clone(),
                    asset_id.clone(),
                    registry_account_id.clone(),
                    plant.facility_code.clone(),
                    "generation_facility".to_owned(),
                    "active".to_owned(),
                    registry_project_metadata,
                    now.to_owned(),
                    now.to_owned()
                ),
            ));

        queries.push((
            "INSERT INTO data_sources (
                    id, asset_id, registry_id, registry_project_id, source_type, provider,
                    external_source_id, unit, metadata_json, created_at, updated_at
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
                 ON CONFLICT(id) DO UPDATE SET
                    asset_id = excluded.asset_id,
                    registry_id = excluded.registry_id,
                    registry_project_id = excluded.registry_project_id,
                    source_type = excluded.source_type,
                    provider = excluded.provider,
                    external_source_id = excluded.external_source_id,
                    unit = excluded.unit,
                    metadata_json = excluded.metadata_json,
                    updated_at = excluded.updated_at"
                .to_owned(),
            params!(
                data_source_id.clone(),
                asset_id.clone(),
                Some(registry_id.clone()),
                Some(registry_project_id.clone()),
                "registry".to_owned(),
                "korea_energy_rps".to_owned(),
                Some(plant.facility_code.clone()),
                "kWh".to_owned(),
                serde_json::json!({
                    "adapter": "kr_rps",
                    "dataKind": "generation",
                    "valueMetricType": KR_RPS_GENERATION_METRIC_GROSS_GENERATION_KWH,
                    "valueSourceField": "GELEC_QTY",
                })
                .to_string(),
                now.to_owned(),
                now.to_owned()
            ),
        ));

        queries.push((
            "INSERT INTO data_sources (
                    id, asset_id, registry_id, registry_project_id, source_type, provider,
                    external_source_id, unit, metadata_json, created_at, updated_at
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
                 ON CONFLICT(id) DO UPDATE SET
                    asset_id = excluded.asset_id,
                    registry_id = excluded.registry_id,
                    registry_project_id = excluded.registry_project_id,
                    source_type = excluded.source_type,
                    provider = excluded.provider,
                    external_source_id = excluded.external_source_id,
                    unit = excluded.unit,
                    metadata_json = excluded.metadata_json,
                    updated_at = excluded.updated_at"
                .to_owned(),
            params!(
                rate_source_id.clone(),
                asset_id.clone(),
                Some(registry_id.clone()),
                Some(registry_project_id.clone()),
                "registry".to_owned(),
                "korea_energy_rps".to_owned(),
                Some(plant.facility_code.clone()),
                "KRW/kWh".to_owned(),
                serde_json::json!({ "dataKind": "rate" }).to_string(),
                now.to_owned(),
                now.to_owned()
            ),
        ));

        for record in generation {
            let period_start = month_start_date(&record.month)?;
            let period_end = next_month_start_date(&record.month)?;
            let generation_id = stable_scoped_id(
                "generation_data:kr_rps",
                &format!(
                    "{}:{}:{}:{}",
                    plant.facility_code,
                    KR_RPS_GENERATION_METRIC_GROSS_GENERATION_KWH,
                    period_start,
                    period_end
                ),
            );
            queries.push((
                "INSERT INTO generation_data (
                        id, asset_id, source_id, period_start, period_end, metric_type, value,
                        unit, quality, metadata_json, recorded_at, updated_at
                     ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
                     ON CONFLICT(asset_id, source_id, metric_type, period_start, period_end) DO UPDATE SET
                        value = excluded.value,
                        unit = excluded.unit,
                        quality = excluded.quality,
                        metadata_json = excluded.metadata_json,
                        recorded_at = excluded.recorded_at,
                        updated_at = excluded.updated_at"
                    .to_owned(),
                params!(
                    generation_id,
                    asset_id.clone(),
                    data_source_id.clone(),
                    period_start,
                    period_end,
                    KR_RPS_GENERATION_METRIC_GROSS_GENERATION_KWH.to_owned(),
                    record.generation_kwh,
                    "kWh".to_owned(),
                    "settled".to_owned(),
                    serde_json::json!({
                        "month": record.month,
                        "adapter": "kr_rps",
                        "valueMetricType": KR_RPS_GENERATION_METRIC_GROSS_GENERATION_KWH,
                        "valueSourceField": "GELEC_QTY",
                        "selfConsumptionKwh": record.self_consumption_kwh,
                        "selfConsumptionSourceField": "HOUSE_CNSM_QTY",
                        "supplyKwh": record.supply_kwh,
                        "supplySourceField": "SPLY_ELCP_QTY",
                        "applicationDeadline": record.application_deadline,
                    })
                    .to_string(),
                    now.to_owned(),
                    now.to_owned()
                ),
            ));
        }

        for record in rates {
            let period_start = month_start_date(&record.record.month)?;
            let period_end = next_month_start_date(&record.record.month)?;
            let quality = if record.published {
                "published"
            } else {
                "unpublished"
            };
            let metrics = [
                ("self_rate", record.record.self_rate),
                ("national_average", record.record.national_average),
                ("nearby_average", record.record.nearby_average),
                ("nearby_highest", record.record.nearby_highest),
            ];
            for (metric_type, value) in metrics {
                let Some(value) = value else {
                    continue;
                };
                let rate_id = stable_scoped_id(
                    "rate_data:kr_rps",
                    &format!(
                        "{}:{metric_type}:{}:{}",
                        plant.facility_code, period_start, period_end
                    ),
                );
                queries.push((
                    "INSERT INTO rate_data (
                        id, asset_id, source_id, period_start, period_end, metric_type, value,
                        unit, currency, quality, metadata_json, recorded_at, updated_at
                     ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
                     ON CONFLICT(asset_id, source_id, metric_type, period_start, period_end)
                     DO UPDATE SET
                        value = excluded.value,
                        unit = excluded.unit,
                        currency = excluded.currency,
                        quality = excluded.quality,
                        metadata_json = excluded.metadata_json,
                        recorded_at = excluded.recorded_at,
                        updated_at = excluded.updated_at"
                        .to_owned(),
                    params!(
                        rate_id,
                        asset_id.clone(),
                        rate_source_id.clone(),
                        period_start.clone(),
                        period_end.clone(),
                        metric_type.to_owned(),
                        value,
                        "KRW/kWh".to_owned(),
                        Some("KRW".to_owned()),
                        quality.to_owned(),
                        serde_json::json!({
                            "month": record.record.month,
                            "published": record.published,
                            "sourceControlFlag": record.source_control_flag,
                        })
                        .to_string(),
                        now.to_owned(),
                        now.to_owned()
                    ),
                ));
            }
        }

        for record in rec {
            let serial_number = format!("{}:{}", plant.facility_code, record.application_no);
            let certificate_id = stable_scoped_id("energy_certificate:kr_rps", &serial_number);
            queries.push(("INSERT INTO energy_certificates (
                        id, certificate_type, registry_id, registry_project_id, asset_id,
                        serial_number, vintage_start, vintage_end, quantity_mwh, owner_account_id,
                        status, issued_at, retired_at, metadata_json, updated_at
                     ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
                     ON CONFLICT(registry_id, serial_number) DO UPDATE SET
                        registry_project_id = excluded.registry_project_id,
                        asset_id = excluded.asset_id,
                        vintage_start = excluded.vintage_start,
                        vintage_end = excluded.vintage_end,
                        quantity_mwh = excluded.quantity_mwh,
                        owner_account_id = COALESCE(energy_certificates.owner_account_id, excluded.owner_account_id),
                        status = excluded.status,
                        issued_at = excluded.issued_at,
                        retired_at = excluded.retired_at,
                        metadata_json = excluded.metadata_json,
                        updated_at = excluded.updated_at".to_owned(),
                    params!(
                        certificate_id,
                        "REC".to_owned(),
                        registry_id.clone(),
                        Some(registry_project_id.clone()),
                        asset_id.clone(),
                        serial_number,
                        Some(month_start_date(&record.month)?),
                        Some(next_month_start_date(&record.month)?),
                        record.supply_kwh as f64 / 1000.0,
                        registry_account_id.clone(),
                        record
                            .status
                            .clone()
                            .unwrap_or_else(|| "unknown".to_owned()),
                        Some(record.issued_date.clone()),
                        Option::<String>::None,
                        serde_json::json!({
                            "month": record.month,
                            "applicationNo": record.application_no,
                            "recQuantity": record.rec_quantity,
                            "feeKrw": record.fee_krw,
                            "expiryDate": record.expiry_date,
                            "virtualAccount": record.virtual_account,
                            "bankName": record.bank_name,
                        })
                        .to_string(),
                        now.to_owned()
                    ),
                ));
        }

        Ok(KrRpsProjectionContext {
            asset_id,
            data_source_id,
            rate_source_id,
            registry_project_id,
        })
    }

    pub async fn upsert_kr_rps_asset_projection(
        &self,
        plant: &Plant,
        registry_account_organization_id: Option<&str>,
        business_no: Option<&str>,
        generation: &[GenerationRecord],
        rec: &[RecRecord],
    ) -> Result<String> {
        let now = now_string();
        let mut queries = Vec::with_capacity(8 + generation.len() + rec.len());
        let context = self
            .push_kr_rps_asset_projection_queries(
                &mut queries,
                plant,
                registry_account_organization_id,
                business_no,
                generation,
                &[],
                rec,
                &now,
            )
            .await?;
        self.runtime
            .transaction(queries)
            .await
            .context("upsert KR RPS asset projection transaction")?;
        Ok(context.asset_id)
    }

    pub async fn materialize_kr_rps_sync(
        &self,
        input: KrRpsSyncMaterialization<'_>,
    ) -> Result<String> {
        let now = now_string();
        let mut queries = Vec::with_capacity(
            16 + input.generation.len()
                + input.rates.len()
                + input.rec.len()
                + input.documents.len(),
        );
        Self::push_upsert_plant_query(&mut queries, input.plant, &now);
        let projection = self
            .push_kr_rps_asset_projection_queries(
                &mut queries,
                input.plant,
                input.registry_account_organization_id,
                input.business_no,
                input.generation,
                input.rates,
                input.rec,
                &now,
            )
            .await?;
        Self::push_legacy_record_upsert_queries(
            &mut queries,
            &input.plant.id,
            input.generation,
            input.rates,
            input.rec,
            &now,
        );
        if matches!(input.mode, SyncMode::Full) {
            self.push_kr_rps_projection_prune_queries(
                &mut queries,
                &input.plant.facility_code,
                &projection,
                input.generation,
                input.rates,
                input.rec,
            )
            .await?;
            self.push_legacy_record_prune_queries(
                &mut queries,
                &input.plant.id,
                input.generation,
                input.rates,
                input.rec,
            )
            .await?;
            self.push_document_replace_queries(
                &mut queries,
                &input.plant.id,
                input.documents,
                &now,
            )
            .await?;
        }
        Self::push_update_sync_run_target_query(&mut queries, input.run_id, Some(&input.plant.id));
        Self::push_upsert_sync_run_item_query(
            &mut queries,
            input.run_id,
            Some(&projection.asset_id),
            &input.plant.facility_code,
            "completed",
            input.generation.len(),
            input.rates.len(),
            input.rec.len(),
            input.documents.len(),
            None,
            &now,
        );
        Self::push_finish_sync_run_query(
            &mut queries,
            input.run_id,
            "completed",
            input.duration_ms,
            None,
            &now,
        );
        self.runtime
            .transaction(queries)
            .await
            .context("materialize KR RPS sync transaction")?;
        Ok(projection.asset_id)
    }

    fn push_update_sync_run_target_query(
        queries: &mut Vec<(String, Vec<hiqlite::Param>)>,
        run_id: &str,
        plant_id: Option<&str>,
    ) {
        queries.push((
            "UPDATE sync_runs SET plant_id = $1 WHERE id = $2".to_owned(),
            params!(plant_id.map(ToOwned::to_owned), run_id.to_owned()),
        ));
    }

    fn push_finish_sync_run_query(
        queries: &mut Vec<(String, Vec<hiqlite::Param>)>,
        run_id: &str,
        status: &str,
        duration_ms: i64,
        error_message: Option<&str>,
        now: &str,
    ) {
        queries.push((
            "UPDATE sync_runs
             SET status = $1, finished_at = $2, duration_ms = $3, error_message = $4
             WHERE id = $5"
                .to_owned(),
            params!(
                status.to_owned(),
                now.to_owned(),
                duration_ms,
                error_message.map(ToOwned::to_owned),
                run_id.to_owned()
            ),
        ));
    }

    fn push_upsert_sync_run_item_query(
        queries: &mut Vec<(String, Vec<hiqlite::Param>)>,
        sync_run_id: &str,
        asset_id: Option<&str>,
        facility_code: &str,
        status: &str,
        generation_count: usize,
        rate_count: usize,
        rec_count: usize,
        document_count: usize,
        error_message: Option<&str>,
        now: &str,
    ) {
        let id = stable_scoped_id("sync_run_item", &format!("{sync_run_id}:{facility_code}"));
        let registry_project_id =
            asset_id.map(|_| stable_scoped_id("registry_project:kr_rps", facility_code));
        let data_source_id =
            asset_id.map(|_| stable_scoped_id("data_source:kr_rps:generation", facility_code));
        queries.push((
            "INSERT INTO sync_run_items (
                id, sync_run_id, asset_id, registry_project_id, data_source_id,
                facility_code, status, generation_count, rate_count, rec_count,
                document_count, error_message, write_version, created_at, updated_at
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
             ON CONFLICT(sync_run_id, facility_code) DO UPDATE SET
                asset_id = excluded.asset_id,
                registry_project_id = excluded.registry_project_id,
                data_source_id = excluded.data_source_id,
                status = excluded.status,
                generation_count = excluded.generation_count,
                rate_count = excluded.rate_count,
                rec_count = excluded.rec_count,
                document_count = excluded.document_count,
                error_message = excluded.error_message,
                write_version = excluded.write_version,
                updated_at = excluded.updated_at"
                .to_owned(),
            params!(
                id,
                sync_run_id.to_owned(),
                asset_id.map(ToOwned::to_owned),
                registry_project_id,
                data_source_id,
                facility_code.to_owned(),
                status.to_owned(),
                generation_count as i64,
                rate_count as i64,
                rec_count as i64,
                document_count as i64,
                error_message.map(ToOwned::to_owned),
                now.to_owned(),
                now.to_owned(),
                now.to_owned()
            ),
        ));
    }

    fn push_legacy_record_upsert_queries(
        queries: &mut Vec<(String, Vec<hiqlite::Param>)>,
        plant_id: &str,
        generation: &[GenerationRecord],
        rates: &[StoredRateRecord],
        rec: &[RecRecord],
        now: &str,
    ) {
        for record in generation {
            queries.push(("INSERT INTO generation_records (plant_id, month, generation_kwh, self_consumption_kwh, supply_kwh, application_deadline, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)
                 ON CONFLICT(plant_id, month) DO UPDATE SET
                    generation_kwh = excluded.generation_kwh,
                    self_consumption_kwh = excluded.self_consumption_kwh,
                    supply_kwh = excluded.supply_kwh,
                    application_deadline = excluded.application_deadline,
                    updated_at = excluded.updated_at".to_owned(),
                params!(
                    plant_id.to_owned(),
                    record.month.clone(),
                    record.generation_kwh,
                    record.self_consumption_kwh,
                    record.supply_kwh,
                    record.application_deadline.clone(),
                    now.to_owned()
                ),
            ));
        }
        for record in rates {
            queries.push(("INSERT INTO rate_records (plant_id, month, self_rate, national_average, nearby_average, nearby_highest, published, source_control_flag, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                 ON CONFLICT(plant_id, month) DO UPDATE SET
                    self_rate = excluded.self_rate,
                    national_average = excluded.national_average,
                    nearby_average = excluded.nearby_average,
                    nearby_highest = excluded.nearby_highest,
                    published = excluded.published,
                    source_control_flag = excluded.source_control_flag,
                    updated_at = excluded.updated_at".to_owned(),
                params!(
                    plant_id.to_owned(),
                    record.record.month.clone(),
                    record.record.self_rate,
                    record.record.national_average,
                    record.record.nearby_average,
                    record.record.nearby_highest,
                    if record.published { 1_i64 } else { 0_i64 },
                    record.source_control_flag.clone(),
                    now.to_owned()
                ),
            ));
        }
        for record in rec {
            queries.push(("INSERT INTO rec_records (plant_id, application_no, month, supply_kwh, rec_quantity, fee_krw, issued_date, expiry_date, virtual_account, bank_name, status, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
                 ON CONFLICT(plant_id, application_no) DO UPDATE SET
                    month = excluded.month,
                    supply_kwh = excluded.supply_kwh,
                    rec_quantity = excluded.rec_quantity,
                    fee_krw = excluded.fee_krw,
                    issued_date = excluded.issued_date,
                    expiry_date = excluded.expiry_date,
                    virtual_account = excluded.virtual_account,
                    bank_name = excluded.bank_name,
                    status = excluded.status,
                    updated_at = excluded.updated_at".to_owned(),
                params!(
                    plant_id.to_owned(),
                    record.application_no.clone(),
                    record.month.clone(),
                    record.supply_kwh,
                    record.rec_quantity,
                    record.fee_krw,
                    record.issued_date.clone(),
                    record.expiry_date.clone(),
                    record.virtual_account.clone(),
                    record.bank_name.clone(),
                    record.status.clone(),
                    now.to_owned()
                ),
            ));
        }
    }

    async fn push_kr_rps_projection_prune_queries(
        &self,
        queries: &mut Vec<(String, Vec<hiqlite::Param>)>,
        facility_code: &str,
        projection: &KrRpsProjectionContext,
        generation: &[GenerationRecord],
        rates: &[StoredRateRecord],
        rec: &[RecRecord],
    ) -> Result<()> {
        let expected_generation_pairs = generation
            .iter()
            .map(|record| {
                let period_start = month_start_date(&record.month)?;
                let period_end = next_month_start_date(&record.month)?;
                Ok::<_, anyhow::Error>((
                    KR_RPS_GENERATION_METRIC_GROSS_GENERATION_KWH.to_owned(),
                    period_start,
                    period_end,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let mut generation_sql =
            "DELETE FROM generation_data WHERE asset_id = $1 AND source_id = $2".to_owned();
        let mut generation_params = params!(
            projection.asset_id.clone(),
            projection.data_source_id.clone()
        );
        if !expected_generation_pairs.is_empty() {
            generation_sql.push_str(" AND NOT (");
            let mut clauses = Vec::with_capacity(expected_generation_pairs.len());
            let mut param_index = 3;
            for (metric_type, period_start, period_end) in expected_generation_pairs {
                clauses.push(format!(
                    "(metric_type = ${} AND period_start = ${} AND period_end = ${})",
                    param_index,
                    param_index + 1,
                    param_index + 2
                ));
                generation_params.push(metric_type.into());
                generation_params.push(period_start.into());
                generation_params.push(period_end.into());
                param_index += 3;
            }
            generation_sql.push_str(&clauses.join(" OR "));
            generation_sql.push(')');
        }
        queries.push((generation_sql, generation_params));

        let mut expected_rate_pairs = Vec::new();
        for record in rates {
            let period_start = month_start_date(&record.record.month)?;
            let period_end = next_month_start_date(&record.record.month)?;
            for (metric_type, value) in [
                ("self_rate", record.record.self_rate),
                ("national_average", record.record.national_average),
                ("nearby_average", record.record.nearby_average),
                ("nearby_highest", record.record.nearby_highest),
            ] {
                if value.is_some() {
                    expected_rate_pairs.push((
                        metric_type.to_owned(),
                        period_start.clone(),
                        period_end.clone(),
                    ));
                }
            }
        }
        let mut rate_sql =
            "DELETE FROM rate_data WHERE asset_id = $1 AND source_id = $2".to_owned();
        let mut rate_params = params!(
            projection.asset_id.clone(),
            projection.rate_source_id.clone()
        );
        if !expected_rate_pairs.is_empty() {
            rate_sql.push_str(" AND NOT (");
            let mut clauses = Vec::with_capacity(expected_rate_pairs.len());
            let mut param_index = 3;
            for (metric_type, period_start, period_end) in expected_rate_pairs {
                clauses.push(format!(
                    "(metric_type = ${} AND period_start = ${} AND period_end = ${})",
                    param_index,
                    param_index + 1,
                    param_index + 2
                ));
                rate_params.push(metric_type.into());
                rate_params.push(period_start.into());
                rate_params.push(period_end.into());
                param_index += 3;
            }
            rate_sql.push_str(&clauses.join(" OR "));
            rate_sql.push(')');
        }
        queries.push((rate_sql, rate_params));

        let expected_serials: Vec<String> = rec
            .iter()
            .map(|record| format!("{}:{}", facility_code, record.application_no))
            .collect();
        let mut certificate_sql = "DELETE FROM energy_certificates
             WHERE asset_id = $1
               AND registry_id = 'registry-kr-rps'
               AND (registry_project_id = $2 OR registry_project_id IS NULL)"
            .to_owned();
        let mut certificate_params = params!(
            projection.asset_id.clone(),
            projection.registry_project_id.clone()
        );
        if !expected_serials.is_empty() {
            certificate_sql.push_str(&format!(
                " AND serial_number NOT IN ({})",
                Self::numbered_placeholders(3, expected_serials.len())
            ));
            for serial in expected_serials {
                certificate_params.push(serial.into());
            }
        }
        queries.push((certificate_sql, certificate_params));
        Ok(())
    }

    async fn push_legacy_record_prune_queries(
        &self,
        queries: &mut Vec<(String, Vec<hiqlite::Param>)>,
        plant_id: &str,
        generation: &[GenerationRecord],
        rates: &[StoredRateRecord],
        rec: &[RecRecord],
    ) -> Result<()> {
        let expected_generation_months: Vec<String> = generation
            .iter()
            .map(|record| record.month.clone())
            .collect();
        let expected_rate_months: Vec<String> = rates
            .iter()
            .map(|record| record.record.month.clone())
            .collect();
        let expected_rec_applications: Vec<String> = rec
            .iter()
            .map(|record| record.application_no.clone())
            .collect();

        Self::push_delete_not_in_query(
            queries,
            "generation_records",
            "month",
            plant_id,
            expected_generation_months,
        );
        Self::push_delete_not_in_query(
            queries,
            "rate_records",
            "month",
            plant_id,
            expected_rate_months,
        );
        Self::push_delete_not_in_query(
            queries,
            "rec_records",
            "application_no",
            plant_id,
            expected_rec_applications,
        );
        Ok(())
    }

    async fn push_document_replace_queries(
        &self,
        queries: &mut Vec<(String, Vec<hiqlite::Param>)>,
        plant_id: &str,
        docs: &[PersistedDocument],
        now: &str,
    ) -> Result<()> {
        for doc in docs {
            queries.push(("INSERT INTO documents (id, plant_id, category, attachment_code, file_class, file_sec_name, file_name, required, source_sha256, output_sha256, object_key, content_type, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
                 ON CONFLICT(id) DO UPDATE SET
                    plant_id = excluded.plant_id,
                    category = excluded.category,
                    attachment_code = excluded.attachment_code,
                    file_class = excluded.file_class,
                    file_sec_name = excluded.file_sec_name,
                    file_name = excluded.file_name,
                    required = excluded.required,
                    source_sha256 = excluded.source_sha256,
                    output_sha256 = excluded.output_sha256,
                    object_key = excluded.object_key,
                    content_type = excluded.content_type,
                    updated_at = excluded.updated_at".to_owned(),
                params!(
                    doc.id.clone(),
                    plant_id.to_owned(),
                    doc.category.to_storage(),
                    doc.attachment_code.clone(),
                    doc.file_class.clone(),
                    doc.file_sec_name.clone(),
                    doc.file_name.clone(),
                    if doc.required { 1_i64 } else { 0_i64 },
                    doc.source_sha256.clone(),
                    doc.output_sha256.clone(),
                    doc.object_key.clone(),
                    doc.content_type.clone(),
                    now.to_owned()
                ),
            ));
        }
        let keep_ids: Vec<String> = docs.iter().map(|doc| doc.id.clone()).collect();
        Self::push_delete_not_in_query(queries, "documents", "id", plant_id, keep_ids);
        Ok(())
    }

    fn numbered_placeholders(start: usize, count: usize) -> String {
        (start..start + count)
            .map(|idx| format!("${idx}"))
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn push_delete_not_in_query(
        queries: &mut Vec<(String, Vec<hiqlite::Param>)>,
        table: &'static str,
        column: &'static str,
        plant_id: &str,
        keep_values: Vec<String>,
    ) {
        let mut sql = format!("DELETE FROM {table} WHERE plant_id = $1");
        let mut params = params!(plant_id.to_owned());
        if !keep_values.is_empty() {
            sql.push_str(&format!(
                " AND {column} NOT IN ({})",
                Self::numbered_placeholders(2, keep_values.len())
            ));
            for value in keep_values {
                params.push(value.into());
            }
        }
        queries.push((sql, params));
    }

    pub async fn replace_kr_rps_asset_projection(
        &self,
        plant: &Plant,
        registry_account_organization_id: Option<&str>,
        business_no: Option<&str>,
        generation: &[GenerationRecord],
        rec: &[RecRecord],
    ) -> Result<String> {
        let data_source_id =
            stable_scoped_id("data_source:kr_rps:generation", &plant.facility_code);
        let registry_project_id = stable_scoped_id("registry_project:kr_rps", &plant.facility_code);

        let asset_id = self
            .upsert_kr_rps_asset_projection(
                plant,
                registry_account_organization_id,
                business_no,
                generation,
                rec,
            )
            .await?;
        self.prune_kr_rps_asset_projection_after_full(
            &plant.facility_code,
            &asset_id,
            &data_source_id,
            &registry_project_id,
            generation,
            rec,
        )
        .await?;
        Ok(asset_id)
    }

    async fn prune_kr_rps_asset_projection_after_full(
        &self,
        facility_code: &str,
        asset_id: &str,
        data_source_id: &str,
        registry_project_id: &str,
        generation: &[GenerationRecord],
        rec: &[RecRecord],
    ) -> Result<()> {
        let expected_generation_pairs = generation
            .iter()
            .map(|record| {
                let period_start = month_start_date(&record.month)?;
                let period_end = next_month_start_date(&record.month)?;
                Ok::<_, anyhow::Error>((
                    KR_RPS_GENERATION_METRIC_GROSS_GENERATION_KWH.to_owned(),
                    period_start,
                    period_end,
                ))
            })
            .collect::<Result<BTreeSet<_>>>()?;
        let generation_rows: Vec<MetricPeriodRow> = self
            .query_rows_with(
                self.default_read_consistency_with_override(Some(ReadConsistency::Strong)),
                "SELECT metric_type, period_start, period_end
                 FROM generation_data
                 WHERE asset_id = $1 AND source_id = $2",
                params!(asset_id.to_owned(), data_source_id.to_owned()),
                "KR RPS generation data full prune candidates",
            )
            .await?;
        for row in generation_rows {
            if expected_generation_pairs.contains(&(
                row.metric_type.clone(),
                row.period_start.clone(),
                row.period_end.clone(),
            )) {
                continue;
            }
            self.runtime
                .execute(
                    "DELETE FROM generation_data
                     WHERE asset_id = $1 AND source_id = $2 AND metric_type = $3 AND period_start = $4 AND period_end = $5",
                    params!(
                        asset_id.to_owned(),
                        data_source_id.to_owned(),
                        row.metric_type,
                        row.period_start,
                        row.period_end
                    ),
                )
                .await
                .context("prune stale KR RPS generation data")?;
        }

        let expected_serials = rec
            .iter()
            .map(|record| format!("{}:{}", facility_code, record.application_no))
            .collect::<BTreeSet<_>>();
        let certificate_rows: Vec<TextValueRow> = self
            .query_rows_with(
                self.default_read_consistency_with_override(Some(ReadConsistency::Strong)),
                "SELECT serial_number AS value
                 FROM energy_certificates
                 WHERE asset_id = $1
                   AND registry_id = 'registry-kr-rps'
                   AND (registry_project_id = $2 OR registry_project_id IS NULL)",
                params!(asset_id.to_owned(), registry_project_id.to_owned()),
                "KR RPS energy certificate full prune candidates",
            )
            .await?;
        for row in certificate_rows {
            if expected_serials.contains(row.value.as_str()) {
                continue;
            }
            self.runtime
                .execute(
                    "DELETE FROM energy_certificates
                     WHERE asset_id = $1
                       AND registry_id = 'registry-kr-rps'
                       AND serial_number = $2",
                    params!(asset_id.to_owned(), row.value),
                )
                .await
                .context("prune stale KR RPS energy certificate")?;
        }
        Ok(())
    }

    async fn resolve_kr_rps_asset_id_for_projection(&self, facility_code: &str) -> Result<String> {
        let profile_asset_id = match self
            .get_kr_rps_asset_id_by_facility_code_with_consistency(
                facility_code,
                Some(ReadConsistency::Strong),
            )
            .await
        {
            Ok(asset_id) => Ok(Some(asset_id)),
            Err(err)
                if matches!(
                    err.downcast_ref::<DatabaseLookupError>(),
                    Some(DatabaseLookupError::PlantNotFound)
                ) =>
            {
                Ok(None)
            }
            Err(err) => Err(err),
        }?;
        let project_asset_id = self
            .existing_kr_rps_registry_project_asset_id_for_facility(facility_code)
            .await?;
        match (profile_asset_id, project_asset_id) {
            (Some(profile_asset_id), Some(project_asset_id))
                if profile_asset_id != project_asset_id =>
            {
                Err(anyhow!(
                    "KR RPS asset mapping conflict for facility {}: profile asset {}, registry project asset {}",
                    facility_code,
                    profile_asset_id,
                    project_asset_id
                ))
            }
            (Some(asset_id), _) | (_, Some(asset_id)) => Ok(asset_id),
            (None, None) => Ok(stable_scoped_id(
                "asset",
                &format!("kr_rps:{facility_code}"),
            )),
        }
    }

    async fn existing_kr_rps_registry_project_asset_id_for_facility(
        &self,
        facility_code: &str,
    ) -> Result<Option<String>> {
        let mut rows: Vec<TextValueRow> = self
            .query_rows_with(
                ReadConsistency::Strong,
                "SELECT asset_id AS value
                 FROM registry_projects
                 WHERE registry_id = 'registry-kr-rps'
                   AND external_project_id = $1
                 LIMIT 1",
                params!(facility_code.to_owned()),
                "existing KR RPS registry project asset for facility",
            )
            .await?;
        Ok(rows.pop().map(|row| row.value))
    }

    async fn existing_kr_rps_registry_account_id_for_facility(
        &self,
        facility_code: &str,
    ) -> Result<Option<String>> {
        #[derive(Debug)]
        struct RegistryAccountIdRow {
            registry_account_id: String,
        }

        impl From<&mut Row<'_>> for RegistryAccountIdRow {
            fn from(row: &mut Row<'_>) -> Self {
                Self {
                    registry_account_id: row.get("registry_account_id"),
                }
            }
        }

        let mut rows: Vec<RegistryAccountIdRow> = self
            .query_rows_with(
                ReadConsistency::Strong,
                "SELECT registry_account_id
                 FROM registry_projects
                 WHERE registry_id = 'registry-kr-rps'
                   AND external_project_id = $1
                   AND registry_account_id IS NOT NULL
                 LIMIT 1",
                params!(facility_code.to_owned()),
                "existing KR RPS registry account for facility",
            )
            .await?;
        Ok(rows.pop().map(|row| row.registry_account_id))
    }

    pub async fn replace_full_records(
        &self,
        plant_id: &str,
        generation: &[GenerationRecord],
        rates: &[StoredRateRecord],
        rec: &[RecRecord],
    ) -> Result<()> {
        self.upsert_generation_records(plant_id, generation).await?;
        self.upsert_rate_records(plant_id, rates).await?;
        self.upsert_rec_records(plant_id, rec).await?;
        self.prune_full_records_after_replace(plant_id, generation, rates, rec)
            .await?;
        Ok(())
    }

    async fn prune_full_records_after_replace(
        &self,
        plant_id: &str,
        generation: &[GenerationRecord],
        rates: &[StoredRateRecord],
        rec: &[RecRecord],
    ) -> Result<()> {
        let expected_generation_months = generation
            .iter()
            .map(|record| record.month.clone())
            .collect::<BTreeSet<_>>();
        let expected_rate_months = rates
            .iter()
            .map(|record| record.record.month.clone())
            .collect::<BTreeSet<_>>();
        let expected_rec_applications = rec
            .iter()
            .map(|record| record.application_no.clone())
            .collect::<BTreeSet<_>>();

        let generation_rows: Vec<TextValueRow> = self
            .query_rows_with(
                self.default_read_consistency_with_override(Some(ReadConsistency::Strong)),
                "SELECT month AS value FROM generation_records WHERE plant_id = $1",
                params!(plant_id.to_owned()),
                "legacy generation full prune candidates",
            )
            .await?;
        let rate_rows: Vec<TextValueRow> = self
            .query_rows_with(
                self.default_read_consistency_with_override(Some(ReadConsistency::Strong)),
                "SELECT month AS value FROM rate_records WHERE plant_id = $1",
                params!(plant_id.to_owned()),
                "legacy rate full prune candidates",
            )
            .await?;
        let rec_rows: Vec<TextValueRow> = self
            .query_rows_with(
                self.default_read_consistency_with_override(Some(ReadConsistency::Strong)),
                "SELECT application_no AS value FROM rec_records WHERE plant_id = $1",
                params!(plant_id.to_owned()),
                "legacy REC full prune candidates",
            )
            .await?;

        for row in generation_rows {
            if expected_generation_months.contains(row.value.as_str()) {
                continue;
            }
            self.runtime
                .execute(
                    "DELETE FROM generation_records WHERE plant_id = $1 AND month = $2",
                    params!(plant_id.to_owned(), row.value),
                )
                .await
                .context("prune stale generation record")?;
        }
        for row in rate_rows {
            if expected_rate_months.contains(row.value.as_str()) {
                continue;
            }
            self.runtime
                .execute(
                    "DELETE FROM rate_records WHERE plant_id = $1 AND month = $2",
                    params!(plant_id.to_owned(), row.value),
                )
                .await
                .context("prune stale rate record")?;
        }
        for row in rec_rows {
            if expected_rec_applications.contains(row.value.as_str()) {
                continue;
            }
            self.runtime
                .execute(
                    "DELETE FROM rec_records WHERE plant_id = $1 AND application_no = $2",
                    params!(plant_id.to_owned(), row.value),
                )
                .await
                .context("prune stale REC record")?;
        }
        Ok(())
    }

    pub async fn upsert_generation_records(
        &self,
        plant_id: &str,
        records: &[GenerationRecord],
    ) -> Result<()> {
        let updated_at = now_string();
        for record in records {
            self.runtime
                .execute(
                    "INSERT INTO generation_records (plant_id, month, generation_kwh, self_consumption_kwh, supply_kwh, application_deadline, updated_at)
                     VALUES ($1, $2, $3, $4, $5, $6, $7)
                     ON CONFLICT(plant_id, month) DO UPDATE SET
                        generation_kwh = excluded.generation_kwh,
                        self_consumption_kwh = excluded.self_consumption_kwh,
                        supply_kwh = excluded.supply_kwh,
                        application_deadline = excluded.application_deadline,
                        updated_at = excluded.updated_at",
                    params!(
                        plant_id.to_owned(),
                        record.month.clone(),
                        record.generation_kwh,
                        record.self_consumption_kwh,
                        record.supply_kwh,
                        record.application_deadline.clone(),
                        updated_at.clone()
                    ),
                )
                .await
                .with_context(|| format!("upsert generation {}", record.month))?;
        }
        Ok(())
    }

    pub async fn upsert_rate_records(
        &self,
        plant_id: &str,
        records: &[StoredRateRecord],
    ) -> Result<()> {
        let updated_at = now_string();
        for record in records {
            self.runtime
                .execute(
                    "INSERT INTO rate_records (plant_id, month, self_rate, national_average, nearby_average, nearby_highest, published, source_control_flag, updated_at)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                     ON CONFLICT(plant_id, month) DO UPDATE SET
                        self_rate = excluded.self_rate,
                        national_average = excluded.national_average,
                        nearby_average = excluded.nearby_average,
                        nearby_highest = excluded.nearby_highest,
                        published = excluded.published,
                        source_control_flag = excluded.source_control_flag,
                        updated_at = excluded.updated_at",
                    params!(
                        plant_id.to_owned(),
                        record.record.month.clone(),
                        record.record.self_rate,
                        record.record.national_average,
                        record.record.nearby_average,
                        record.record.nearby_highest,
                        if record.published { 1_i64 } else { 0_i64 },
                        record.source_control_flag.clone(),
                        updated_at.clone()
                    ),
                )
                .await
                .with_context(|| format!("upsert rate {}", record.record.month))?;
        }
        Ok(())
    }

    pub async fn upsert_rec_records(&self, plant_id: &str, records: &[RecRecord]) -> Result<()> {
        let updated_at = now_string();
        for record in records {
            self.runtime
                .execute(
                    "INSERT INTO rec_records (plant_id, application_no, month, supply_kwh, rec_quantity, fee_krw, issued_date, expiry_date, virtual_account, bank_name, status, updated_at)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
                     ON CONFLICT(plant_id, application_no) DO UPDATE SET
                        month = excluded.month,
                        supply_kwh = excluded.supply_kwh,
                        rec_quantity = excluded.rec_quantity,
                        fee_krw = excluded.fee_krw,
                        issued_date = excluded.issued_date,
                        expiry_date = excluded.expiry_date,
                        virtual_account = excluded.virtual_account,
                        bank_name = excluded.bank_name,
                        status = excluded.status,
                        updated_at = excluded.updated_at",
                    params!(
                        plant_id.to_owned(),
                        record.application_no.clone(),
                        record.month.clone(),
                        record.supply_kwh,
                        record.rec_quantity,
                        record.fee_krw,
                        record.issued_date.clone(),
                        record.expiry_date.clone(),
                        record.virtual_account.clone(),
                        record.bank_name.clone(),
                        record.status.clone(),
                        updated_at.clone()
                    ),
                )
                .await
                .with_context(|| format!("upsert rec {}", record.application_no))?;
        }
        Ok(())
    }

    pub async fn list_document_objects(&self, plant_id: &str) -> Result<Vec<String>> {
        let rows: Vec<DocumentRow> = self
            .query_consistent_rows(
                "SELECT id, plant_id, category, file_class, file_sec_name, file_name, required, source_sha256, output_sha256, object_key, content_type
                 FROM documents WHERE plant_id = $1",
                params!(plant_id.to_owned()),
                "document objects",
            )
            .await
            .context("list document rows")?;
        Ok(rows.into_iter().map(|row| row.object_key).collect())
    }

    pub async fn replace_documents(
        &self,
        plant_id: &str,
        docs: &[PersistedDocument],
    ) -> Result<()> {
        let updated_at = now_string();
        for doc in docs {
            self.runtime
                .execute(
                    "INSERT INTO documents (id, plant_id, category, attachment_code, file_class, file_sec_name, file_name, required, source_sha256, output_sha256, object_key, content_type, updated_at)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
                     ON CONFLICT(id) DO UPDATE SET
                        plant_id = excluded.plant_id,
                        category = excluded.category,
                        attachment_code = excluded.attachment_code,
                        file_class = excluded.file_class,
                        file_sec_name = excluded.file_sec_name,
                        file_name = excluded.file_name,
                        required = excluded.required,
                        source_sha256 = excluded.source_sha256,
                        output_sha256 = excluded.output_sha256,
                        object_key = excluded.object_key,
                        content_type = excluded.content_type,
                        updated_at = excluded.updated_at",
                    params!(
                        doc.id.clone(),
                        plant_id.to_owned(),
                        doc.category.to_storage(),
                        doc.attachment_code.clone(),
                        doc.file_class.clone(),
                        doc.file_sec_name.clone(),
                        doc.file_name.clone(),
                        if doc.required { 1_i64 } else { 0_i64 },
                        doc.source_sha256.clone(),
                        doc.output_sha256.clone(),
                        doc.object_key.clone(),
                        doc.content_type.clone(),
                        updated_at.clone()
                    ),
                )
                .await
                .with_context(|| format!("insert document {}", doc.file_name))?;
        }
        let keep_ids = docs
            .iter()
            .map(|doc| doc.id.clone())
            .collect::<BTreeSet<_>>();
        let rows: Vec<TextValueRow> = self
            .query_rows_with(
                self.default_read_consistency_with_override(Some(ReadConsistency::Strong)),
                "SELECT id AS value FROM documents WHERE plant_id = $1",
                params!(plant_id.to_owned()),
                "document full prune candidates",
            )
            .await?;
        for row in rows {
            if keep_ids.contains(row.value.as_str()) {
                continue;
            }
            self.runtime
                .execute(
                    "DELETE FROM documents WHERE plant_id = $1 AND id = $2",
                    params!(plant_id.to_owned(), row.value),
                )
                .await
                .context("prune stale document row")?;
        }
        Ok(())
    }

    pub async fn list_visible_plants(
        &self,
        user_id: &str,
        organization_id: &str,
    ) -> Result<Vec<Plant>> {
        self.list_visible_plants_with_consistency(user_id, organization_id, None)
            .await
    }

    pub async fn list_visible_plants_with_consistency(
        &self,
        user_id: &str,
        organization_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Vec<Plant>> {
        let rows: Vec<PlantRow> = self
            .query_rows_with(
                self.default_read_consistency_with_override(consistency),
                "SELECT p.id, p.facility_code, p.owner_organization_id, p.created_by_organization_id,
                        p.name, p.contract_no, p.address, p.installed_kw, p.rec_start_date,
                        p.confirm_issued
                 FROM effective_user_plant_visibility v
                 JOIN plants p ON p.id = v.plant_id
                 WHERE v.user_id = $1
                   AND v.organization_id = $2
                 UNION
                 SELECT p.id, p.facility_code, p.owner_organization_id, p.created_by_organization_id,
                        p.name, p.contract_no, p.address, p.installed_kw, p.rec_start_date,
                        p.confirm_issued
                 FROM effective_user_asset_visibility v
                 JOIN asset_profiles ap
                   ON ap.asset_id = v.asset_id
                  AND ap.country_code = 'KR'
                  AND ap.provider = 'korea_energy_rps'
                  AND ap.profile_type = 'kr_rps'
                 JOIN plants p ON p.facility_code = ap.external_id
                 WHERE v.user_id = $1
                   AND v.organization_id = $2
                 ORDER BY facility_code ASC",
                params!(user_id.to_owned(), organization_id.to_owned()),
                "visible plants",
            )
            .await
            .context("list visible plants")?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    pub async fn has_user_plant_action(
        &self,
        user_id: &str,
        organization_id: &str,
        plant_id: &str,
        action_id: &str,
    ) -> Result<bool> {
        self.has_user_plant_action_with_consistency(
            user_id,
            organization_id,
            plant_id,
            action_id,
            None,
        )
        .await
    }

    pub async fn has_user_plant_action_with_consistency(
        &self,
        user_id: &str,
        organization_id: &str,
        plant_id: &str,
        action_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<bool> {
        Ok(self
            .query_count(
                self.default_read_consistency_with_override(consistency),
                "SELECT COUNT(*) as count
                 FROM effective_user_plant_actions
                 WHERE user_id = $1
                   AND organization_id = $2
                   AND plant_id = $3
                   AND action_id = $4",
                params!(
                    user_id.to_owned(),
                    organization_id.to_owned(),
                    plant_id.to_owned(),
                    action_id.to_owned()
                ),
                "effective user plant action",
            )
            .await?
            > 0)
    }

    pub async fn has_user_asset_action_with_consistency(
        &self,
        user_id: &str,
        organization_id: &str,
        asset_id: &str,
        action_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<bool> {
        Ok(self
            .query_count(
                self.default_read_consistency_with_override(consistency),
                "SELECT COUNT(*) as count
                 FROM effective_user_asset_actions
                 WHERE user_id = $1
                   AND organization_id = $2
                   AND asset_id = $3
                   AND action_id = $4",
                params!(
                    user_id.to_owned(),
                    organization_id.to_owned(),
                    asset_id.to_owned(),
                    action_id.to_owned()
                ),
                "effective user asset action",
            )
            .await?
            > 0)
    }

    pub async fn has_usable_kr_rps_certificate_credential_with_consistency(
        &self,
        organization_id: &str,
        facility_code: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<bool> {
        Ok(self
            .query_count(
                self.default_read_consistency_with_override(consistency),
                "SELECT COUNT(*) as count
                 FROM auth_credentials ac
                 WHERE ac.organization_id = $1
                   AND ac.provider = 'korea_energy_rps'
                   AND ac.credential_type = 'certificate'
                   AND ac.active = 1
                   AND (
                        EXISTS (
                            SELECT 1
                            FROM registry_projects rp
                            JOIN assets a ON a.id = rp.asset_id
                            WHERE rp.registry_id = 'registry-kr-rps'
                              AND rp.external_project_id = $2
                              AND rp.registry_account_id IS NULL
                              AND a.owner_organization_id = $1
                        )
                        OR EXISTS (
                            SELECT 1
                            FROM registry_projects rp
                            JOIN registry_accounts ra ON ra.id = rp.registry_account_id
                            JOIN credential_bindings cb ON cb.registry_account_id = ra.id
                            WHERE rp.registry_id = 'registry-kr-rps'
                              AND rp.external_project_id = $2
                              AND ra.organization_id = ac.organization_id
                              AND ra.organization_id = $1
                              AND cb.credential_id = ac.id
                        )
                   )",
                params!(organization_id.to_owned(), facility_code.to_owned()),
                "usable KR RPS certificate credential",
            )
            .await?
            > 0)
    }

    pub async fn get_kr_rps_asset_id_by_facility_code_with_consistency(
        &self,
        facility_code: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<String> {
        #[derive(Debug)]
        struct AssetIdRow {
            asset_id: String,
        }

        impl From<&mut Row<'_>> for AssetIdRow {
            fn from(row: &mut Row<'_>) -> Self {
                Self {
                    asset_id: row.get("asset_id"),
                }
            }
        }

        let mut rows: Vec<AssetIdRow> = self
            .query_rows_with(
                self.default_read_consistency_with_override(consistency),
                "SELECT asset_id
                 FROM asset_profiles
                 WHERE country_code = 'KR'
                   AND profile_type = 'kr_rps'
                   AND provider = 'korea_energy_rps'
                   AND external_id = $1
                 LIMIT 1",
                params!(facility_code.to_owned()),
                "KR RPS asset profile",
            )
            .await
            .context("query KR RPS asset profile")?;
        rows.pop()
            .map(|row| row.asset_id)
            .ok_or_else(|| anyhow::Error::new(DatabaseLookupError::PlantNotFound))
    }

    pub async fn kr_rps_projection_counts(
        &self,
        facility_code: &str,
    ) -> Result<KrRpsProjectionCounts> {
        let asset_id = self
            .get_kr_rps_asset_id_by_facility_code_with_consistency(facility_code, None)
            .await?;
        let asset_profiles = self
            .query_count(
                self.default_read_consistency_with_override(None),
                "SELECT COUNT(*) as count
                 FROM asset_profiles
                 WHERE asset_id = $1
                   AND profile_type = 'kr_rps'
                   AND provider = 'korea_energy_rps'
                   AND external_id = $2",
                params!(asset_id.clone(), facility_code.to_owned()),
                "KR RPS asset profile count",
            )
            .await?;
        let registry_projects = self
            .query_count(
                self.default_read_consistency_with_override(None),
                "SELECT COUNT(*) as count
                 FROM registry_projects
                 WHERE asset_id = $1 AND external_project_id = $2",
                params!(asset_id.clone(), facility_code.to_owned()),
                "KR RPS registry project count",
            )
            .await?;
        let data_sources = self
            .query_count(
                self.default_read_consistency_with_override(None),
                "SELECT COUNT(*) as count
                 FROM data_sources
                 WHERE asset_id = $1 AND provider = 'korea_energy_rps'",
                params!(asset_id.clone()),
                "KR RPS data source count",
            )
            .await?;
        let generation_data = self
            .query_count(
                self.default_read_consistency_with_override(None),
                "SELECT COUNT(*) as count FROM generation_data WHERE asset_id = $1",
                params!(asset_id.clone()),
                "KR RPS generation data count",
            )
            .await?;
        let rate_data = self
            .query_count(
                self.default_read_consistency_with_override(None),
                "SELECT COUNT(*) as count FROM rate_data WHERE asset_id = $1",
                params!(asset_id.clone()),
                "KR RPS rate data count",
            )
            .await?;
        let energy_certificates = self
            .query_count(
                self.default_read_consistency_with_override(None),
                "SELECT COUNT(*) as count FROM energy_certificates WHERE asset_id = $1",
                params!(asset_id),
                "KR RPS energy certificate count",
            )
            .await?;
        Ok(KrRpsProjectionCounts {
            asset_profiles,
            registry_projects,
            data_sources,
            generation_data,
            rate_data,
            energy_certificates,
        })
    }

    pub async fn get_kr_rps_registry_project_binding(
        &self,
        facility_code: &str,
    ) -> Result<KrRpsRegistryProjectBinding> {
        #[derive(Debug)]
        struct BindingRow {
            asset_owner_organization_id: String,
            registry_account_organization_id: Option<String>,
            registry_account_external_account_id: Option<String>,
        }

        impl From<&mut Row<'_>> for BindingRow {
            fn from(row: &mut Row<'_>) -> Self {
                Self {
                    asset_owner_organization_id: row.get("asset_owner_organization_id"),
                    registry_account_organization_id: row.get("registry_account_organization_id"),
                    registry_account_external_account_id: row
                        .get("registry_account_external_account_id"),
                }
            }
        }

        let mut rows: Vec<BindingRow> = self
            .query_rows_with(
                self.default_read_consistency_with_override(Some(ReadConsistency::Strong)),
                "SELECT a.owner_organization_id AS asset_owner_organization_id,
                        ra.organization_id AS registry_account_organization_id,
                        ra.external_account_id AS registry_account_external_account_id
                 FROM registry_projects rp
                 JOIN assets a ON a.id = rp.asset_id
                 LEFT JOIN registry_accounts ra ON ra.id = rp.registry_account_id
                 WHERE rp.registry_id = 'registry-kr-rps'
                   AND rp.external_project_id = $1
                 LIMIT 1",
                params!(facility_code.to_owned()),
                "KR RPS registry project binding",
            )
            .await?;
        rows.pop()
            .map(|row| KrRpsRegistryProjectBinding {
                asset_owner_organization_id: row.asset_owner_organization_id,
                registry_account_organization_id: row.registry_account_organization_id,
                registry_account_external_account_id: row.registry_account_external_account_id,
            })
            .ok_or_else(|| anyhow::Error::new(DatabaseLookupError::PlantNotFound))
    }

    pub async fn get_latest_sync_run_audit_for_facility(
        &self,
        facility_code: &str,
    ) -> Result<SyncRunAudit> {
        #[derive(Debug)]
        struct AuditRow {
            run_status: String,
            item_status: Option<String>,
            run_error_message: Option<String>,
            item_error_message: Option<String>,
        }

        impl From<&mut Row<'_>> for AuditRow {
            fn from(row: &mut Row<'_>) -> Self {
                Self {
                    run_status: row.get("run_status"),
                    item_status: row.get("item_status"),
                    run_error_message: row.get("run_error_message"),
                    item_error_message: row.get("item_error_message"),
                }
            }
        }

        let mut rows: Vec<AuditRow> = self
            .query_rows_with(
                self.default_read_consistency_with_override(Some(ReadConsistency::Strong)),
                "SELECT sr.status AS run_status,
	                        sri.status AS item_status,
	                        sr.error_message AS run_error_message,
	                        sri.error_message AS item_error_message
	                 FROM sync_runs sr
	                 LEFT JOIN plants p
	                   ON p.id = sr.plant_id
	                 LEFT JOIN sync_run_items sri
	                   ON sri.sync_run_id = sr.id
	                  AND sri.facility_code = $1
	                 WHERE p.facility_code = $1 OR sri.facility_code = $1
	                 ORDER BY sr.started_at DESC, sr.id DESC
	                 LIMIT 1",
                params!(facility_code.to_owned()),
                "latest sync run audit for facility",
            )
            .await?;
        rows.pop()
            .map(|row| SyncRunAudit {
                run_status: row.run_status,
                item_status: row.item_status,
                run_error_message: row.run_error_message,
                item_error_message: row.item_error_message,
            })
            .ok_or_else(|| anyhow!("sync run audit not found for facility {facility_code}"))
    }

    pub async fn list_user_plant_action_facility_codes_with_consistency(
        &self,
        user_id: &str,
        organization_id: &str,
        action_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Vec<String>> {
        #[derive(Debug)]
        struct FacilityCodeRow {
            facility_code: String,
        }

        impl From<&mut Row<'_>> for FacilityCodeRow {
            fn from(row: &mut Row<'_>) -> Self {
                Self {
                    facility_code: row.get("facility_code"),
                }
            }
        }

        let read_consistency = self.default_read_consistency_with_override(consistency);
        let rows: Vec<FacilityCodeRow> = self
            .query_rows_with(
                read_consistency.clone(),
                "SELECT DISTINCT p.facility_code
                 FROM effective_user_plant_visibility v
                 JOIN effective_user_plant_actions a
                   ON a.user_id = v.user_id
                  AND a.organization_id = v.organization_id
                  AND a.plant_id = v.plant_id
                 JOIN plants p ON p.id = v.plant_id
                 WHERE v.user_id = $1
                   AND v.organization_id = $2
                   AND a.action_id = $3
                 ORDER BY p.facility_code ASC",
                params!(
                    user_id.to_owned(),
                    organization_id.to_owned(),
                    action_id.to_owned()
                ),
                "effective user plant action facility codes",
            )
            .await
            .context("list user plant action facility codes")?;
        let mut facility_codes: BTreeSet<String> =
            rows.into_iter().map(|row| row.facility_code).collect();

        if let Some(asset_action_id) = kr_rps_plant_action_to_asset_action_id(action_id) {
            let asset_rows: Vec<FacilityCodeRow> = self
                .query_rows_with(
                    read_consistency,
                    "SELECT DISTINCT p.facility_code
                     FROM effective_user_asset_visibility v
                     JOIN effective_user_asset_actions a
                       ON a.user_id = v.user_id
                      AND a.organization_id = v.organization_id
                      AND a.asset_id = v.asset_id
                     JOIN asset_profiles ap
                       ON ap.asset_id = v.asset_id
                      AND ap.country_code = 'KR'
                      AND ap.provider = 'korea_energy_rps'
                      AND ap.profile_type = 'kr_rps'
                     JOIN plants p ON p.facility_code = ap.external_id
                     WHERE v.user_id = $1
                       AND v.organization_id = $2
                       AND a.action_id = $3
                     ORDER BY p.facility_code ASC",
                    params!(
                        user_id.to_owned(),
                        organization_id.to_owned(),
                        asset_action_id.to_owned()
                    ),
                    "effective user asset action facility codes",
                )
                .await
                .context("list user asset action facility codes")?;
            facility_codes.extend(asset_rows.into_iter().map(|row| row.facility_code));
        }

        Ok(facility_codes.into_iter().collect())
    }

    pub async fn has_user_organization_action(
        &self,
        user_id: &str,
        organization_id: &str,
        target_organization_id: &str,
        action_id: &str,
    ) -> Result<bool> {
        self.has_user_organization_action_with_consistency(
            user_id,
            organization_id,
            target_organization_id,
            action_id,
            None,
        )
        .await
    }

    pub async fn has_user_organization_action_with_consistency(
        &self,
        user_id: &str,
        organization_id: &str,
        target_organization_id: &str,
        action_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<bool> {
        let consistency = self.validation_read_consistency_with_override(consistency);
        let has_membership = self
            .exists_row_with(
                consistency.clone(),
                "SELECT COUNT(*) as count
                 FROM organization_memberships
                 WHERE user_id = $1
                   AND organization_id = $2
                   AND status = 'active'",
                params!(user_id.to_owned(), organization_id.to_owned()),
                "active organization membership",
            )
            .await?;
        if !has_membership {
            return Ok(false);
        }

        let direct_user = self
            .exists_row_with(
                consistency.clone(),
                "SELECT COUNT(*) as count
                 FROM role_bindings rb
                 JOIN role_actions ra ON ra.role_id = rb.role_id
                 WHERE rb.subject_type = 'user'
                   AND rb.subject_id = $1
                   AND rb.subject_organization_id = $2
                   AND rb.resource_type = 'organization'
                   AND rb.resource_id = $3
                   AND rb.state = 'active'
                   AND ra.action_id = $4",
                params!(
                    user_id.to_owned(),
                    organization_id.to_owned(),
                    target_organization_id.to_owned(),
                    action_id.to_owned()
                ),
                "direct user organization action",
            )
            .await?;
        if direct_user {
            return Ok(true);
        }

        self.exists_row_with(
            consistency,
            "SELECT COUNT(*) as count
             FROM role_bindings rb
             JOIN role_actions ra ON ra.role_id = rb.role_id
             JOIN group_memberships gm ON gm.group_id = rb.subject_id
             JOIN organization_memberships om ON om.id = gm.organization_membership_id
             WHERE rb.subject_type = 'group'
               AND rb.subject_organization_id = $1
               AND rb.resource_type = 'organization'
               AND rb.resource_id = $2
               AND rb.state = 'active'
               AND ra.action_id = $3
               AND om.user_id = $4
               AND om.organization_id = $1
               AND om.status = 'active'",
            params!(
                organization_id.to_owned(),
                target_organization_id.to_owned(),
                action_id.to_owned(),
                user_id.to_owned()
            ),
            "group organization action",
        )
        .await
    }

    pub async fn find_active_agreement_id(
        &self,
        grantor_organization_id: &str,
        grantee_organization_id: &str,
    ) -> Result<Option<String>> {
        self.find_active_agreement_id_with_consistency(
            grantor_organization_id,
            grantee_organization_id,
            None,
        )
        .await
    }

    pub async fn find_active_agreement_id_with_consistency(
        &self,
        grantor_organization_id: &str,
        grantee_organization_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Option<String>> {
        let now = Utc::now().to_rfc3339();
        let rows: Vec<AgreementIdRow> = self
            .query_rows_with(
                self.validation_read_consistency_with_override(consistency),
                "SELECT id
                 FROM agreements
                 WHERE grantor_organization_id = $1
                   AND grantee_organization_id = $2
                   AND status = 'active'
                   AND (starts_at IS NULL OR starts_at <= $3)
                   AND (ends_at IS NULL OR ends_at >= $3)
                 ORDER BY created_at DESC, id DESC
                 LIMIT 1",
                params!(
                    grantor_organization_id.to_owned(),
                    grantee_organization_id.to_owned(),
                    now
                ),
                "active agreement id",
            )
            .await?;
        Ok(rows.into_iter().next().map(|row| row.id))
    }

    pub async fn latest_generation_month(&self, plant_id: &str) -> Result<Option<String>> {
        let rows: Vec<GenerationRow> = self
            .query_consistent_rows(
                "SELECT month, generation_kwh, self_consumption_kwh, supply_kwh, application_deadline
                 FROM generation_records WHERE plant_id = $1 ORDER BY month DESC LIMIT 1",
                params!(plant_id.to_owned()),
                "latest generation month",
            )
            .await
            .context("query latest generation month")?;
        Ok(rows.into_iter().next().map(|row| row.month))
    }

    pub async fn get_plant_by_facility_code(&self, facility_code: &str) -> Result<Plant> {
        self.get_plant_by_facility_code_with_consistency(facility_code, None)
            .await
    }

    pub async fn get_plant_by_facility_code_with_consistency(
        &self,
        facility_code: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Plant> {
        let mut plants: Vec<PlantRow> = self
            .query_rows_with(
                self.default_read_consistency_with_override(consistency),
                "SELECT id, facility_code, owner_organization_id, created_by_organization_id, name, contract_no, address, installed_kw, rec_start_date, confirm_issued
                 FROM plants WHERE facility_code = $1 LIMIT 1",
                params!(facility_code.to_owned()),
                "plant",
            )
            .await
            .context("query plant")?;
        let plant_row = plants
            .pop()
            .ok_or_else(|| anyhow::Error::new(DatabaseLookupError::PlantNotFound))?;
        Ok(plant_row.into())
    }

    pub async fn get_plant_by_id_with_consistency(
        &self,
        plant_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Plant> {
        let mut plants: Vec<PlantRow> = self
            .query_rows_with(
                self.default_read_consistency_with_override(consistency),
                "SELECT id, facility_code, owner_organization_id, created_by_organization_id, name, contract_no, address, installed_kw, rec_start_date, confirm_issued
                 FROM plants WHERE id = $1 LIMIT 1",
                params!(plant_id.to_owned()),
                "plant",
            )
            .await
            .context("query plant")?;
        let plant_row = plants
            .pop()
            .ok_or_else(|| anyhow::Error::new(DatabaseLookupError::PlantNotFound))?;
        Ok(plant_row.into())
    }

    pub async fn get_plant_summary(&self, facility_code: &str) -> Result<PlantSummary> {
        self.get_plant_summary_with_consistency(facility_code, None)
            .await
    }

    pub async fn get_plant_summary_with_consistency(
        &self,
        facility_code: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<PlantSummary> {
        let plant = self
            .get_plant_by_facility_code_with_consistency(facility_code, consistency.clone())
            .await?;
        self.get_plant_summary_by_id_with_consistency(&plant.id, consistency)
            .await
    }

    pub async fn get_plant_summary_by_id(&self, plant_id: &str) -> Result<PlantSummary> {
        self.get_plant_summary_by_id_with_consistency(plant_id, None)
            .await
    }

    pub async fn get_plant_summary_by_id_with_consistency(
        &self,
        plant_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<PlantSummary> {
        let read_consistency = self.default_read_consistency_with_override(consistency);
        let mut plants: Vec<PlantRow> = self
            .query_rows_with(
                read_consistency.clone(),
                "SELECT id, facility_code, owner_organization_id, created_by_organization_id, name, contract_no, address, installed_kw, rec_start_date, confirm_issued
                 FROM plants WHERE id = $1 LIMIT 1",
                params!(plant_id.to_owned()),
                "plant",
            )
            .await
            .context("query plant")?;
        let plant_row = plants
            .pop()
            .ok_or_else(|| anyhow::Error::new(DatabaseLookupError::PlantNotFound))?;
        let resolved_plant_id = plant_row.id.clone();

        let mut runs: Vec<SyncRunRow> = self
            .query_rows_with(
                read_consistency.clone(),
                "SELECT id, organization_id, plant_id, mode, status, started_at, finished_at, duration_ms
                 FROM sync_runs WHERE plant_id = $1 AND status = 'completed'
                 ORDER BY started_at DESC LIMIT 1",
                params!(resolved_plant_id.clone()),
                "latest sync run",
            )
            .await
            .context("query latest sync run")?;
        let last_sync = runs.pop().map(Into::into);

        let generation = self
            .query_row_with::<CountSumRow, _>(
                read_consistency.clone(),
                "SELECT COUNT(*) as count, CAST(COALESCE(SUM(generation_kwh), 0) AS REAL) as total
                 FROM generation_records WHERE plant_id = $1",
                params!(resolved_plant_id.clone()),
                "generation aggregate",
            )
            .await
            .context("aggregate generation")?;
        let rates = self
            .query_row_with::<CountRow, _>(
                read_consistency.clone(),
                "SELECT COUNT(*) as count FROM rate_records WHERE plant_id = $1 AND published = 1",
                params!(resolved_plant_id.clone()),
                "rates aggregate",
            )
            .await
            .context("aggregate rates")?;
        let rec = self
            .query_row_with::<CountSumIntRow, _>(
                read_consistency.clone(),
                "SELECT COUNT(*) as count, COALESCE(SUM(rec_quantity), 0) as total
                 FROM rec_records WHERE plant_id = $1",
                params!(resolved_plant_id.clone()),
                "rec aggregate",
            )
            .await
            .context("aggregate rec")?;
        let documents = self
            .query_row_with::<CountRow, _>(
                read_consistency.clone(),
                "SELECT COUNT(*) as count FROM documents WHERE plant_id = $1",
                params!(resolved_plant_id.clone()),
                "document aggregate",
            )
            .await
            .context("aggregate documents")?;
        let category_rows: Vec<CategoryCountRow> = self
            .query_rows_with(
                read_consistency,
                "SELECT category, COUNT(*) as count FROM documents WHERE plant_id = $1 GROUP BY category ORDER BY category ASC",
                params!(resolved_plant_id),
                "document category aggregate",
            )
            .await
            .context("aggregate document categories")?;

        Ok(PlantSummary {
            plant: plant_row.into(),
            last_sync,
            generation_count: generation.count,
            generation_total_kwh: generation.total.unwrap_or_default(),
            rates_count: rates.count,
            rec_count: rec.count,
            rec_total: rec.total.unwrap_or_default(),
            document_count: documents.count,
            document_categories: category_rows
                .into_iter()
                .filter_map(|row| {
                    <DocumentCategory as DocumentCategoryExt>::from_storage(&row.category)
                        .map(|cat| (cat, row.count))
                })
                .collect(),
        })
    }

    pub async fn list_generation(&self, plant_id: &str) -> Result<Vec<GenerationRecord>> {
        self.list_generation_with_consistency(plant_id, None).await
    }

    pub async fn list_generation_with_consistency(
        &self,
        plant_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Vec<GenerationRecord>> {
        let rows: Vec<GenerationRow> = self
            .query_rows_with(
                self.default_read_consistency_with_override(consistency),
                "SELECT month, generation_kwh, self_consumption_kwh, supply_kwh, application_deadline
                 FROM generation_records WHERE plant_id = $1 ORDER BY month ASC",
                params!(plant_id.to_owned()),
                "generation list",
            )
            .await
            .context("list generation")?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    pub async fn list_rates(&self, plant_id: &str) -> Result<Vec<RateRecord>> {
        self.list_rates_with_consistency(plant_id, None).await
    }

    pub async fn list_rates_with_consistency(
        &self,
        plant_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Vec<RateRecord>> {
        let rows: Vec<RateRow> = self
            .query_rows_with(
                self.default_read_consistency_with_override(consistency),
                "SELECT month, self_rate, national_average, nearby_average, nearby_highest
                 FROM rate_records WHERE plant_id = $1 AND published = 1 ORDER BY month ASC",
                params!(plant_id.to_owned()),
                "rates list",
            )
            .await
            .context("list rates")?;
        Ok(rows
            .into_iter()
            .map(|row| RateRecord {
                month: row.month,
                self_rate: row.self_rate,
                national_average: row.national_average,
                nearby_average: row.nearby_average,
                nearby_highest: row.nearby_highest,
            })
            .collect())
    }

    pub async fn list_rec(&self, plant_id: &str) -> Result<Vec<RecRecord>> {
        self.list_rec_with_consistency(plant_id, None).await
    }

    pub async fn list_rec_with_consistency(
        &self,
        plant_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Vec<RecRecord>> {
        let rows: Vec<RecRow> = self
            .query_rows_with(
                self.default_read_consistency_with_override(consistency),
                "SELECT month, application_no, supply_kwh, rec_quantity, fee_krw, issued_date, expiry_date, virtual_account, bank_name, status
                 FROM rec_records WHERE plant_id = $1 ORDER BY month ASC, application_no ASC",
                params!(plant_id.to_owned()),
                "rec list",
            )
            .await
            .context("list rec")?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    pub async fn list_documents(&self, plant_id: &str) -> Result<Vec<StoredDocument>> {
        self.list_documents_with_consistency(plant_id, None).await
    }

    pub async fn list_documents_with_consistency(
        &self,
        plant_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Vec<StoredDocument>> {
        let rows: Vec<DocumentRow> = self
            .query_rows_with(
                self.default_read_consistency_with_override(consistency),
                "SELECT id, plant_id, category, file_class, file_sec_name, file_name, required, source_sha256, output_sha256, object_key, content_type
                 FROM documents WHERE plant_id = $1 ORDER BY category ASC, file_name ASC",
                params!(plant_id.to_owned()),
                "document list",
            )
            .await
            .context("list documents")?;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                <DocumentCategory as DocumentCategoryExt>::from_storage(&row.category).map(
                    |category| StoredDocument {
                        plant_id: row.plant_id,
                        artifact: DocumentArtifact {
                            id: row.id,
                            category,
                            file_class: row.file_class,
                            file_sec_name: row.file_sec_name,
                            file_name: row.file_name,
                            required: row.required == 1,
                            source_sha256: row.source_sha256,
                            output_sha256: row.output_sha256,
                            download_url: None,
                        },
                        object_key: row.object_key,
                        content_type: row.content_type,
                    },
                )
            })
            .collect())
    }

    pub async fn get_document(&self, id: &str) -> Result<StoredDocument> {
        self.get_document_with_consistency(id, None).await
    }

    pub async fn get_document_with_consistency(
        &self,
        id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<StoredDocument> {
        let rows: Vec<DocumentRow> = self
            .query_rows_with(
                self.default_read_consistency_with_override(consistency),
                "SELECT id, plant_id, category, file_class, file_sec_name, file_name, required, source_sha256, output_sha256, object_key, content_type
                 FROM documents WHERE id = $1 LIMIT 1",
                params!(id.to_owned()),
                "document",
            )
            .await
            .context("get document")?;
        let mut rows = rows.into_iter();
        let row = rows.next().ok_or_else(|| anyhow!("document not found"))?;
        let category = <DocumentCategory as DocumentCategoryExt>::from_storage(&row.category)
            .ok_or_else(|| anyhow!("invalid document category {}", row.category))?;
        Ok(StoredDocument {
            plant_id: row.plant_id,
            artifact: DocumentArtifact {
                id: row.id,
                category,
                file_class: row.file_class,
                file_sec_name: row.file_sec_name,
                file_name: row.file_name,
                required: row.required == 1,
                source_sha256: row.source_sha256,
                output_sha256: row.output_sha256,
                download_url: None,
            },
            object_key: row.object_key,
            content_type: row.content_type,
        })
    }

    pub async fn list_backup_local(&self) -> Result<Vec<BackupEntry>> {
        let items = self
            .runtime
            .backup_list_local()
            .await
            .context("list local backups")?;
        Ok(items
            .into_iter()
            .map(|item| BackupEntry {
                name: item.name,
                last_modified: item.last_modified,
                size: item.size,
            })
            .collect())
    }

    pub async fn list_backup_s3(&self) -> Result<Vec<BackupEntry>> {
        let items = self
            .runtime
            .backup_list_s3()
            .await
            .context("list s3 backups")?;
        Ok(items
            .into_iter()
            .map(|item| BackupEntry {
                name: item.name,
                last_modified: item.last_modified,
                size: item.size,
            })
            .collect())
    }

    pub async fn trigger_backup(&self) -> Result<()> {
        self.runtime
            .backup()
            .await
            .context("trigger hiqlite backup")
    }

    pub async fn save_role_binding(&self, binding: &RoleBinding) -> Result<()> {
        validate_role_binding(binding)?;
        self.validate_role_binding_refs(binding).await?;
        self.runtime
            .execute(
                "INSERT INTO role_bindings (
                    id, subject_type, subject_id, subject_organization_id, resource_type, resource_id,
                    role_id, agreement_id, state, created_by_user_id, created_at, updated_at
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
                 ON CONFLICT(id) DO UPDATE SET
                    subject_type = excluded.subject_type,
                    subject_id = excluded.subject_id,
                    subject_organization_id = excluded.subject_organization_id,
                    resource_type = excluded.resource_type,
                    resource_id = excluded.resource_id,
                    role_id = excluded.role_id,
                    agreement_id = excluded.agreement_id,
                    state = excluded.state,
                    updated_at = excluded.updated_at",
                params!(
                    binding.id.clone(),
                    role_binding_subject_type_str(&binding.subject_type),
                    binding.subject_id.clone(),
                    binding.subject_organization_id.clone(),
                    role_binding_resource_type_str(&binding.resource_type),
                    binding.resource_id.clone(),
                    binding.role_id.clone(),
                    binding.agreement_id.clone(),
                    role_binding_state_str(&binding.state),
                    binding.created_by_user_id.clone(),
                    binding.created_at.clone(),
                    binding.updated_at.clone()
                ),
            )
            .await
            .context("save role binding")?;
        self.rebuild_organization_expansion().await?;
        Ok(())
    }

    async fn rebuild_organization_expansion(&self) -> Result<()> {
        let consistency = self.validation_read_consistency();
        let seed_rows: Vec<OrganizationPlantActionSeedRow> = self
            .query_rows_with(
                consistency.clone(),
                "SELECT rb.subject_id AS organization_id,
                        rb.resource_id AS plant_id,
                        ra.action_id AS action_id
                 FROM role_bindings rb
                 JOIN role_actions ra ON ra.role_id = rb.role_id
                 LEFT JOIN agreements a ON a.id = rb.agreement_id
                 WHERE rb.subject_type = 'organization'
                   AND rb.resource_type = 'plant'
                   AND rb.state = 'active'
                   AND (
                        rb.agreement_id IS NULL
                        OR (
                            a.status = 'active'
                            AND (a.starts_at IS NULL OR a.starts_at <= $1)
                            AND (a.ends_at IS NULL OR a.ends_at >= $1)
                        )
                   )
                 ORDER BY rb.subject_id ASC, rb.resource_id ASC, ra.action_id ASC",
                vec![now_string().into()],
                "organization plant action seeds",
            )
            .await?;
        let membership_rows: Vec<OrganizationMembershipSeedRow> = self
            .query_rows_with(
                consistency.clone(),
                "SELECT id, organization_id, user_id
                 FROM organization_memberships
                 WHERE status = 'active'
                 ORDER BY organization_id ASC, user_id ASC, id ASC",
                vec![],
                "active organization memberships for user expansion",
            )
            .await?;
        let group_membership_rows: Vec<GroupMembershipSeedRow> = self
            .query_rows_with(
                consistency.clone(),
                "SELECT group_id, organization_membership_id
                 FROM group_memberships
                 ORDER BY group_id ASC, organization_membership_id ASC",
                vec![],
                "group memberships for user expansion",
            )
            .await?;
        let group_seed_rows: Vec<GroupPlantActionSeedRow> = self
            .query_rows_with(
                consistency.clone(),
                "SELECT rb.subject_id AS group_id,
                        rb.subject_organization_id AS organization_id,
                        rb.resource_id AS plant_id,
                        ra.action_id AS action_id
                 FROM role_bindings rb
                 JOIN role_actions ra ON ra.role_id = rb.role_id
                 WHERE rb.subject_type = 'group'
                   AND rb.resource_type = 'plant'
                   AND rb.state = 'active'
                 ORDER BY rb.subject_id ASC, rb.resource_id ASC, ra.action_id ASC",
                vec![],
                "group plant action seeds",
            )
            .await?;
        let user_seed_rows: Vec<UserPlantActionSeedRow> = self
            .query_rows_with(
                consistency,
                "SELECT rb.subject_id AS user_id,
                        rb.subject_organization_id AS organization_id,
                        rb.resource_id AS plant_id,
                        ra.action_id AS action_id
                 FROM role_bindings rb
                 JOIN role_actions ra ON ra.role_id = rb.role_id
                 WHERE rb.subject_type = 'user'
                   AND rb.resource_type = 'plant'
                   AND rb.state = 'active'
                 ORDER BY rb.subject_id ASC, rb.resource_id ASC, ra.action_id ASC",
                vec![],
                "user plant action seeds",
            )
            .await?;
        let asset_seed_rows: Vec<OrganizationPlantActionSeedRow> = self
            .query_rows_with(
                self.validation_read_consistency(),
                "SELECT rb.subject_id AS organization_id,
                        rb.resource_id AS plant_id,
                        ra.action_id AS action_id
                 FROM role_bindings rb
                 JOIN role_actions ra ON ra.role_id = rb.role_id
                 LEFT JOIN agreements a ON a.id = rb.agreement_id
                 WHERE rb.subject_type = 'organization'
                   AND rb.resource_type = 'asset'
                   AND rb.state = 'active'
                   AND (
                        rb.agreement_id IS NULL
                        OR (
                            a.status = 'active'
                            AND (a.starts_at IS NULL OR a.starts_at <= $1)
                            AND (a.ends_at IS NULL OR a.ends_at >= $1)
                        )
                   )
                 ORDER BY rb.subject_id ASC, rb.resource_id ASC, ra.action_id ASC",
                vec![now_string().into()],
                "organization asset action seeds",
            )
            .await?;
        let group_asset_seed_rows: Vec<GroupPlantActionSeedRow> = self
            .query_rows_with(
                self.validation_read_consistency(),
                "SELECT rb.subject_id AS group_id,
                        rb.subject_organization_id AS organization_id,
                        rb.resource_id AS plant_id,
                        ra.action_id AS action_id
                 FROM role_bindings rb
                 JOIN role_actions ra ON ra.role_id = rb.role_id
                 WHERE rb.subject_type = 'group'
                   AND rb.resource_type = 'asset'
                   AND rb.state = 'active'
                 ORDER BY rb.subject_id ASC, rb.resource_id ASC, ra.action_id ASC",
                vec![],
                "group asset action seeds",
            )
            .await?;
        let user_asset_seed_rows: Vec<UserPlantActionSeedRow> = self
            .query_rows_with(
                self.validation_read_consistency(),
                "SELECT rb.subject_id AS user_id,
                        rb.subject_organization_id AS organization_id,
                        rb.resource_id AS plant_id,
                        ra.action_id AS action_id
                 FROM role_bindings rb
                 JOIN role_actions ra ON ra.role_id = rb.role_id
                 WHERE rb.subject_type = 'user'
                   AND rb.resource_type = 'asset'
                   AND rb.state = 'active'
                 ORDER BY rb.subject_id ASC, rb.resource_id ASC, ra.action_id ASC",
                vec![],
                "user asset action seeds",
            )
            .await?;
        let mut org_action_rows: HashSet<(String, String, String)> = HashSet::new();
        for seed in &seed_rows {
            org_action_rows.insert((
                seed.organization_id.clone(),
                seed.plant_id.clone(),
                seed.action_id.clone(),
            ));
        }

        let membership_by_id: HashMap<String, (String, String)> = membership_rows
            .iter()
            .map(|row| {
                (
                    row.id.clone(),
                    (row.user_id.clone(), row.organization_id.clone()),
                )
            })
            .collect();
        let active_membership_pairs: HashSet<(String, String)> = membership_rows
            .iter()
            .map(|row| (row.user_id.clone(), row.organization_id.clone()))
            .collect();
        let mut users_by_group: HashMap<String, Vec<(String, String)>> = HashMap::new();
        for group_membership in &group_membership_rows {
            if let Some((user_id, organization_id)) =
                membership_by_id.get(&group_membership.organization_membership_id)
            {
                users_by_group
                    .entry(group_membership.group_id.clone())
                    .or_default()
                    .push((user_id.clone(), organization_id.clone()));
            }
        }

        let mut user_action_rows: HashSet<(String, String, String, String)> = HashSet::new();
        for seed in &group_seed_rows {
            if let Some(users) = users_by_group.get(&seed.group_id) {
                for (user_id, organization_id) in users {
                    if organization_id != &seed.organization_id {
                        continue;
                    }
                    if org_action_rows.contains(&(
                        seed.organization_id.clone(),
                        seed.plant_id.clone(),
                        seed.action_id.clone(),
                    )) {
                        user_action_rows.insert((
                            user_id.clone(),
                            seed.organization_id.clone(),
                            seed.plant_id.clone(),
                            seed.action_id.clone(),
                        ));
                    }
                }
            }
        }
        for seed in &user_seed_rows {
            if !active_membership_pairs
                .contains(&(seed.user_id.clone(), seed.organization_id.clone()))
            {
                continue;
            }
            if org_action_rows.contains(&(
                seed.organization_id.clone(),
                seed.plant_id.clone(),
                seed.action_id.clone(),
            )) {
                user_action_rows.insert((
                    seed.user_id.clone(),
                    seed.organization_id.clone(),
                    seed.plant_id.clone(),
                    seed.action_id.clone(),
                ));
            }
        }
        let visibility_rows: HashSet<(String, String, String)> = user_action_rows
            .iter()
            .filter(|(_, _, _, action_id)| action_id == "plant.profile.read")
            .map(|(user_id, organization_id, plant_id, _)| {
                (user_id.clone(), organization_id.clone(), plant_id.clone())
            })
            .collect();
        let mut asset_org_action_rows: HashSet<(String, String, String)> = HashSet::new();
        for seed in &asset_seed_rows {
            asset_org_action_rows.insert((
                seed.organization_id.clone(),
                seed.plant_id.clone(),
                seed.action_id.clone(),
            ));
        }
        let mut asset_user_action_rows: HashSet<(String, String, String, String)> = HashSet::new();
        for seed in &group_asset_seed_rows {
            if let Some(users) = users_by_group.get(&seed.group_id) {
                for (user_id, organization_id) in users {
                    if organization_id != &seed.organization_id {
                        continue;
                    }
                    if asset_org_action_rows.contains(&(
                        seed.organization_id.clone(),
                        seed.plant_id.clone(),
                        seed.action_id.clone(),
                    )) {
                        asset_user_action_rows.insert((
                            user_id.clone(),
                            seed.organization_id.clone(),
                            seed.plant_id.clone(),
                            seed.action_id.clone(),
                        ));
                    }
                }
            }
        }
        for seed in &user_asset_seed_rows {
            if !active_membership_pairs
                .contains(&(seed.user_id.clone(), seed.organization_id.clone()))
            {
                continue;
            }
            if asset_org_action_rows.contains(&(
                seed.organization_id.clone(),
                seed.plant_id.clone(),
                seed.action_id.clone(),
            )) {
                asset_user_action_rows.insert((
                    seed.user_id.clone(),
                    seed.organization_id.clone(),
                    seed.plant_id.clone(),
                    seed.action_id.clone(),
                ));
            }
        }
        let asset_visibility_rows: HashSet<(String, String, String)> = asset_user_action_rows
            .iter()
            .filter(|(_, _, _, action_id)| action_id == "asset.profile.read")
            .map(|(user_id, organization_id, asset_id, _)| {
                (user_id.clone(), organization_id.clone(), asset_id.clone())
            })
            .collect();

        // HashSet iteration is non-deterministic; keep transaction input ordered so rebuilds are
        // reproducible in tests/debugging while using the cheaper unstable sort because duplicates
        // have already been eliminated by the sets above.
        let mut org_action_rows: Vec<_> = org_action_rows.into_iter().collect();
        org_action_rows.sort_unstable();

        let mut user_action_rows: Vec<_> = user_action_rows.into_iter().collect();
        user_action_rows.sort_unstable();

        let mut visibility_rows: Vec<_> = visibility_rows.into_iter().collect();
        visibility_rows.sort_unstable();
        let mut asset_org_action_rows: Vec<_> = asset_org_action_rows.into_iter().collect();
        asset_org_action_rows.sort_unstable();

        let mut asset_user_action_rows: Vec<_> = asset_user_action_rows.into_iter().collect();
        asset_user_action_rows.sort_unstable();

        let mut asset_visibility_rows: Vec<_> = asset_visibility_rows.into_iter().collect();
        asset_visibility_rows.sort_unstable();

        let updated_at = Utc::now().to_rfc3339();
        let mut queries: Vec<(String, Vec<hiqlite::Param>)> = Vec::with_capacity(
            6 + org_action_rows.len()
                + user_action_rows.len()
                + visibility_rows.len()
                + asset_org_action_rows.len()
                + asset_user_action_rows.len()
                + asset_visibility_rows.len(),
        );
        queries.push((
            "DELETE FROM effective_organization_plant_actions".to_owned(),
            vec![],
        ));
        queries.push((
            "DELETE FROM effective_user_plant_actions".to_owned(),
            vec![],
        ));
        queries.push((
            "DELETE FROM effective_user_plant_visibility".to_owned(),
            vec![],
        ));
        queries.push((
            "DELETE FROM effective_organization_asset_actions".to_owned(),
            vec![],
        ));
        queries.push((
            "DELETE FROM effective_user_asset_actions".to_owned(),
            vec![],
        ));
        queries.push((
            "DELETE FROM effective_user_asset_visibility".to_owned(),
            vec![],
        ));

        for (organization_id, plant_id, action_id) in org_action_rows {
            queries.push((
                "INSERT INTO effective_organization_plant_actions (
                    organization_id, plant_id, action_id, updated_at
                 ) VALUES ($1, $2, $3, $4)"
                    .to_owned(),
                params!(organization_id, plant_id, action_id, updated_at.clone()),
            ));
        }

        for (user_id, organization_id, plant_id, action_id) in user_action_rows {
            queries.push((
                "INSERT INTO effective_user_plant_actions (
                    user_id, organization_id, plant_id, action_id, updated_at
                 ) VALUES ($1, $2, $3, $4, $5)"
                    .to_owned(),
                params!(
                    user_id,
                    organization_id,
                    plant_id,
                    action_id,
                    updated_at.clone()
                ),
            ));
        }

        for (user_id, organization_id, plant_id) in visibility_rows {
            queries.push((
                "INSERT INTO effective_user_plant_visibility (
                    user_id, organization_id, plant_id, updated_at
                 ) VALUES ($1, $2, $3, $4)"
                    .to_owned(),
                params!(user_id, organization_id, plant_id, updated_at.clone()),
            ));
        }
        for (organization_id, asset_id, action_id) in asset_org_action_rows {
            queries.push((
                "INSERT INTO effective_organization_asset_actions (
                    organization_id, asset_id, action_id, updated_at
                 ) VALUES ($1, $2, $3, $4)"
                    .to_owned(),
                params!(organization_id, asset_id, action_id, updated_at.clone()),
            ));
        }

        for (user_id, organization_id, asset_id, action_id) in asset_user_action_rows {
            queries.push((
                "INSERT INTO effective_user_asset_actions (
                    user_id, organization_id, asset_id, action_id, updated_at
                 ) VALUES ($1, $2, $3, $4, $5)"
                    .to_owned(),
                params!(
                    user_id,
                    organization_id,
                    asset_id,
                    action_id,
                    updated_at.clone()
                ),
            ));
        }

        for (user_id, organization_id, asset_id) in asset_visibility_rows {
            queries.push((
                "INSERT INTO effective_user_asset_visibility (
                    user_id, organization_id, asset_id, updated_at
                 ) VALUES ($1, $2, $3, $4)"
                    .to_owned(),
                params!(user_id, organization_id, asset_id, updated_at.clone()),
            ));
        }

        self.runtime
            .transaction(queries)
            .await
            .context("rebuild organization expansion transaction")?;

        Ok(())
    }

    pub async fn list_role_bindings_for_subject(
        &self,
        subject_type: RoleBindingSubjectType,
        subject_id: &str,
        subject_organization_id: Option<&str>,
    ) -> Result<Vec<RoleBinding>> {
        let mut sql = String::from(
            "SELECT id, subject_type, subject_id, subject_organization_id, resource_type, resource_id,
                    role_id, agreement_id, state, created_by_user_id, created_at, updated_at
             FROM role_bindings
             WHERE subject_type = $1 AND subject_id = $2",
        );
        let mut sql_params = params!(
            role_binding_subject_type_str(&subject_type),
            subject_id.to_owned()
        );
        if let Some(subject_organization_id) = subject_organization_id {
            sql.push_str(" AND subject_organization_id = $3");
            sql_params.push(subject_organization_id.to_owned().into());
        }
        sql.push_str(" ORDER BY id ASC");

        let rows: Vec<RoleBindingRow> = self
            .query_default_rows(sql, sql_params, "role bindings for subject")
            .await?;
        rows.into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>>>()
    }

    pub async fn list_role_bindings_for_resource(
        &self,
        resource_type: RoleBindingResourceType,
        resource_id: &str,
    ) -> Result<Vec<RoleBinding>> {
        let rows: Vec<RoleBindingRow> = self
            .query_default_rows(
                "SELECT id, subject_type, subject_id, subject_organization_id, resource_type, resource_id,
                        role_id, agreement_id, state, created_by_user_id, created_at, updated_at
                 FROM role_bindings
                 WHERE resource_type = $1 AND resource_id = $2
                 ORDER BY id ASC",
                params!(
                    role_binding_resource_type_str(&resource_type),
                    resource_id.to_owned()
                ),
                "role bindings for resource",
            )
            .await?;
        rows.into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>>>()
    }

    async fn validate_role_binding_refs(&self, binding: &RoleBinding) -> Result<()> {
        let consistency = self.validation_read_consistency();
        let validation_time = Utc::now().to_rfc3339();

        if !self
            .exists_row_with(
                consistency.clone(),
                "SELECT COUNT(*) as count FROM organizations WHERE id = $1",
                params!(binding.subject_organization_id.clone()),
                "subject organization existence",
            )
            .await?
        {
            return Err(anyhow!(
                "subject organization {} does not exist",
                binding.subject_organization_id
            ));
        }

        let role_rows = self
            .query_rows_with::<RoleOrgRow, _>(
                consistency.clone(),
                "SELECT organization_id FROM roles WHERE id = $1",
                params!(binding.role_id.clone()),
                "role existence and ownership",
            )
            .await?;
        let role_org = role_rows
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("role {} does not exist", binding.role_id))?
            .organization_id;
        if let Some(role_org) = role_org.as_deref() {
            if role_org != binding.subject_organization_id {
                return Err(anyhow!(
                    "role {} belongs to organization {}, not {}",
                    binding.role_id,
                    role_org,
                    binding.subject_organization_id
                ));
            }
        }

        if !self
            .exists_row_with(
                consistency.clone(),
                "SELECT COUNT(*) as count FROM users WHERE id = $1",
                params!(binding.created_by_user_id.clone()),
                "creator user existence",
            )
            .await?
        {
            return Err(anyhow!(
                "creator user {} does not exist",
                binding.created_by_user_id
            ));
        }

        let plant_owner = if matches!(binding.resource_type, RoleBindingResourceType::Plant) {
            let plant_owner_rows = self
                .query_rows_with::<PlantOwnerRow, _>(
                    consistency.clone(),
                    "SELECT owner_organization_id FROM plants WHERE id = $1",
                    params!(binding.resource_id.clone()),
                    "plant owner for role binding validation",
                )
                .await?;
            Some(
                plant_owner_rows
                    .into_iter()
                    .next()
                    .ok_or_else(|| {
                        anyhow!("plant resource {} does not exist", binding.resource_id)
                    })?
                    .owner_organization_id,
            )
        } else {
            None
        };
        let asset_owner = if matches!(binding.resource_type, RoleBindingResourceType::Asset) {
            let asset_owner_rows = self
                .query_rows_with::<PlantOwnerRow, _>(
                    consistency.clone(),
                    "SELECT owner_organization_id FROM assets WHERE id = $1",
                    params!(binding.resource_id.clone()),
                    "asset owner for role binding validation",
                )
                .await?;
            Some(
                asset_owner_rows
                    .into_iter()
                    .next()
                    .ok_or_else(|| {
                        anyhow!("asset resource {} does not exist", binding.resource_id)
                    })?
                    .owner_organization_id,
            )
        } else {
            None
        };
        let facility_owner = asset_owner.as_ref().or(plant_owner.as_ref());

        if matches!(binding.subject_type, RoleBindingSubjectType::Organization)
            && matches!(
                binding.resource_type,
                RoleBindingResourceType::Asset | RoleBindingResourceType::Plant
            )
        {
            let subject_is_owner = facility_owner
                .map(|owner| owner.as_str() == binding.subject_id.as_str())
                .unwrap_or(false);
            if subject_is_owner {
                if binding.agreement_id.is_some() {
                    return Err(anyhow!(
                        "owner organization facility binding must not carry agreement references"
                    ));
                }
            } else if binding.agreement_id.is_none() {
                return Err(anyhow!(
                    "organization-to-facility role binding must reference an agreement unless the subject organization owns the facility"
                ));
            }
        }

        if let Some(agreement_id) = &binding.agreement_id {
            let agreement_exists = self
                .query_count(
                    consistency.clone(),
                    "SELECT COUNT(*) as count FROM agreements WHERE id = $1",
                    params!(agreement_id.clone()),
                    "agreement existence",
                )
                .await?;
            if agreement_exists == 0 {
                return Err(anyhow!("agreement {} does not exist", agreement_id));
            }
            let agreement_active = self
                .query_count(
                    consistency.clone(),
                    "SELECT COUNT(*) as count
                     FROM agreements
                     WHERE id = $1
                       AND status = 'active'
                       AND (starts_at IS NULL OR starts_at <= $2)
                       AND (ends_at IS NULL OR ends_at >= $2)",
                    params!(agreement_id.clone(), validation_time.clone()),
                    "agreement activity window",
                )
                .await?;
            if agreement_active == 0 {
                return Err(anyhow!(
                    "agreement {} is not active for validation timestamp {}",
                    agreement_id,
                    validation_time
                ));
            }
            if matches!(binding.subject_type, RoleBindingSubjectType::Organization)
                && matches!(binding.resource_type, RoleBindingResourceType::Plant)
            {
                let agreement_matches = self
                    .query_count(
                        consistency.clone(),
                        "SELECT COUNT(*) as count
                         FROM agreements a, plants p
                         WHERE a.id = $1
                           AND p.id = $2
                           AND a.grantee_organization_id = $3
                           AND a.grantor_organization_id = p.owner_organization_id",
                        params!(
                            agreement_id.clone(),
                            binding.resource_id.clone(),
                            binding.subject_id.clone()
                        ),
                        "agreement plant relationship match",
                    )
                    .await?;
                if agreement_matches == 0 {
                    return Err(anyhow!(
                        "agreement {} does not match plant {} and organization {}",
                        agreement_id,
                        binding.resource_id,
                        binding.subject_id
                    ));
                }
            }
            if matches!(binding.subject_type, RoleBindingSubjectType::Organization)
                && matches!(binding.resource_type, RoleBindingResourceType::Asset)
            {
                let agreement_matches = self
                    .query_count(
                        consistency.clone(),
                        "SELECT COUNT(*) as count
                         FROM agreements a, assets asset
                         WHERE a.id = $1
                           AND asset.id = $2
                           AND a.grantee_organization_id = $3
                           AND a.grantor_organization_id = asset.owner_organization_id",
                        params!(
                            agreement_id.clone(),
                            binding.resource_id.clone(),
                            binding.subject_id.clone()
                        ),
                        "agreement asset relationship match",
                    )
                    .await?;
                if agreement_matches == 0 {
                    return Err(anyhow!(
                        "agreement {} does not match asset {} and organization {}",
                        agreement_id,
                        binding.resource_id,
                        binding.subject_id
                    ));
                }
            }
        }

        match binding.subject_type {
            RoleBindingSubjectType::Organization => {
                if !self
                    .exists_row_with(
                        consistency.clone(),
                        "SELECT COUNT(*) as count FROM organizations WHERE id = $1",
                        params!(binding.subject_id.clone()),
                        "organization subject existence",
                    )
                    .await?
                {
                    return Err(anyhow!(
                        "organization subject {} does not exist",
                        binding.subject_id
                    ));
                }
            }
            RoleBindingSubjectType::Group => {
                let group_matches = self
                    .query_count(
                        consistency.clone(),
                        "SELECT COUNT(*) as count FROM groups WHERE id = $1 AND organization_id = $2",
                        params!(
                            binding.subject_id.clone(),
                            binding.subject_organization_id.clone()
                        ),
                        "group subject organization match",
                    )
                    .await?;
                if group_matches == 0 {
                    return Err(anyhow!(
                        "group subject {} does not belong to organization {}",
                        binding.subject_id,
                        binding.subject_organization_id
                    ));
                }
            }
            RoleBindingSubjectType::User => {
                let membership_matches = self
                    .query_count(
                        consistency.clone(),
                        "SELECT COUNT(*) as count FROM organization_memberships WHERE user_id = $1 AND organization_id = $2",
                        params!(
                            binding.subject_id.clone(),
                            binding.subject_organization_id.clone()
                        ),
                        "user subject organization match",
                    )
                    .await?;
                if membership_matches == 0 {
                    return Err(anyhow!(
                        "user subject {} does not belong to organization {}",
                        binding.subject_id,
                        binding.subject_organization_id
                    ));
                }
            }
        }

        match binding.resource_type {
            RoleBindingResourceType::Organization => {
                if !self
                    .exists_row_with(
                        consistency.clone(),
                        "SELECT COUNT(*) as count FROM organizations WHERE id = $1",
                        params!(binding.resource_id.clone()),
                        "organization resource existence",
                    )
                    .await?
                {
                    return Err(anyhow!(
                        "organization resource {} does not exist",
                        binding.resource_id
                    ));
                }
            }
            RoleBindingResourceType::Asset => {
                if asset_owner.is_none() {
                    return Err(anyhow!(
                        "asset resource {} does not exist",
                        binding.resource_id
                    ));
                }
            }
            RoleBindingResourceType::Plant => {
                if plant_owner.is_none() {
                    return Err(anyhow!(
                        "plant resource {} does not exist",
                        binding.resource_id
                    ));
                }
            }
        }

        Ok(())
    }
}

fn validation_read_consistency_for_mode(
    runtime_mode: &crate::config::RuntimeMode,
) -> ReadConsistency {
    match runtime_mode {
        crate::config::RuntimeMode::Embedded => ReadConsistency::Local,
        crate::config::RuntimeMode::Remote => ReadConsistency::Strong,
        crate::config::RuntimeMode::Split => ReadConsistency::Strong,
    }
}

#[cfg(test)]
mod tests {
    use super::{should_refresh_certificate_last_used_at, validation_read_consistency_for_mode};
    use crate::config::{ReadConsistency, RuntimeMode};

    #[test]
    fn validation_reads_remain_strong_in_split_mode() {
        assert_eq!(
            validation_read_consistency_for_mode(&RuntimeMode::Split),
            ReadConsistency::Strong
        );
        assert_eq!(
            validation_read_consistency_for_mode(&RuntimeMode::Remote),
            ReadConsistency::Strong
        );
        assert_eq!(
            validation_read_consistency_for_mode(&RuntimeMode::Embedded),
            ReadConsistency::Local
        );
    }

    #[test]
    fn invalid_last_used_at_refreshes_instead_of_failing() {
        let should_refresh = should_refresh_certificate_last_used_at(Some("not-a-timestamp"))
            .expect("invalid timestamp should not fail");
        assert!(should_refresh);
    }
}

#[derive(Debug, Clone)]
pub struct StoredRateRecord {
    pub record: RateRecord,
    pub published: bool,
    pub source_control_flag: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PersistedDocument {
    pub id: String,
    pub category: DocumentCategory,
    pub attachment_code: String,
    pub file_class: String,
    pub file_sec_name: String,
    pub file_name: String,
    pub required: bool,
    pub source_sha256: Option<String>,
    pub output_sha256: Option<String>,
    pub object_key: String,
    pub content_type: String,
}

fn sync_mode_str(mode: &SyncMode) -> &'static str {
    match mode {
        SyncMode::Delta => "delta",
        SyncMode::Full => "full",
    }
}

fn now_string() -> String {
    DateTime::<Utc>::from(std::time::SystemTime::now()).to_rfc3339()
}

fn stable_scoped_id(scope: &str, value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(scope.as_bytes());
    hasher.update(b":");
    hasher.update(value.as_bytes());
    format!(
        "{}-{}",
        scope.replace(':', "-"),
        hex::encode(&hasher.finalize()[..8])
    )
}

fn kr_rps_plant_action_to_asset_action_id(action_id: &str) -> Option<&'static str> {
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

fn month_start_date(month: &str) -> Result<String> {
    let date = NaiveDate::parse_from_str(&format!("{month}-01"), "%Y-%m-%d")
        .with_context(|| format!("parse month {month}"))?;
    Ok(date.format("%Y-%m-%d").to_string())
}

fn next_month_start_date(month: &str) -> Result<String> {
    let date = NaiveDate::parse_from_str(&format!("{month}-01"), "%Y-%m-%d")
        .with_context(|| format!("parse month {month}"))?;
    let (year, month) = if date.month() == 12 {
        (date.year() + 1, 1)
    } else {
        (date.year(), date.month() + 1)
    };
    let next = NaiveDate::from_ymd_opt(year, month, 1)
        .ok_or_else(|| anyhow!("invalid next month for {}", date.format("%Y-%m-%d")))?;
    Ok(next.format("%Y-%m-%d").to_string())
}

fn should_refresh_certificate_last_used_at(last_used_at: Option<&str>) -> Result<bool> {
    let Some(last_used_at) = last_used_at else {
        return Ok(true);
    };
    let parsed = match DateTime::parse_from_rfc3339(last_used_at) {
        Ok(parsed) => parsed.with_timezone(&Utc),
        Err(err) => {
            warn!(
                last_used_at,
                error = %err,
                "failed to parse certificate last_used_at; refreshing timestamp"
            );
            return Ok(true);
        }
    };
    Ok(Utc::now().signed_duration_since(parsed)
        >= ChronoDuration::seconds(CERTIFICATE_LAST_USED_AT_WRITE_INTERVAL_SECS))
}

impl From<&mut Row<'_>> for StoredCertificateRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            id: row.get("id"),
            party_id: row.get("party_id"),
            business_no: row.get("business_no"),
            common_name: row.get("common_name"),
            encrypted_blob: row.get("encrypted_blob"),
            wrapped_dek_b64: row.get("wrapped_dek"),
            kms_key_id: row.get("kms_key_id"),
            active: row.get("active"),
            last_used_at: row.get("last_used_at"),
            created_at: row.get("created_at"),
        }
    }
}

impl TryFrom<StoredCertificateRow> for StoredCertificate {
    type Error = anyhow::Error;

    fn try_from(row: StoredCertificateRow) -> Result<Self> {
        Ok(Self {
            id: row.id,
            party_id: row.party_id,
            business_no: row.business_no,
            common_name: row.common_name,
            encrypted_blob: row.encrypted_blob,
            wrapped_dek: STANDARD
                .decode(row.wrapped_dek_b64)
                .context("decode wrapped_dek from base64")?,
            kms_key_id: row.kms_key_id,
            active: row.active,
            last_used_at: row.last_used_at,
            created_at: row.created_at,
        })
    }
}

impl From<&mut Row<'_>> for CertGatewayChallengeRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            id: row.get("id"),
            organization_id: row.get("organization_id"),
            mode: row.get("mode"),
            nonce: row.get("nonce"),
            payload_hash: row.get("payload_hash"),
            facility_code: row.get("facility_code"),
            expires_at: row.get("expires_at"),
            consumed_at: row.get("consumed_at"),
            created_by_user_id: row.get("created_by_user_id"),
            created_at: row.get("created_at"),
        }
    }
}

impl From<CertGatewayChallengeRow> for CertGatewayChallenge {
    fn from(row: CertGatewayChallengeRow) -> Self {
        Self {
            id: row.id,
            organization_id: row.organization_id,
            mode: row.mode,
            nonce: row.nonce,
            payload_hash: row.payload_hash,
            facility_code: row.facility_code,
            expires_at: row.expires_at,
            consumed_at: row.consumed_at,
            created_by_user_id: row.created_by_user_id,
            created_at: row.created_at,
        }
    }
}

impl From<&mut Row<'_>> for CrawlTaskTokenRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            id: row.get("id"),
            organization_id: row.get("organization_id"),
            certificate_id: row.get("certificate_id"),
            token_hash: row.get("token_hash"),
            scope_json: row.get("scope_json"),
            expires_at: row.get("expires_at"),
            used_at: row.get("used_at"),
            created_by_user_id: row.get("created_by_user_id"),
            created_at: row.get("created_at"),
        }
    }
}

impl From<CrawlTaskTokenRow> for CrawlTaskToken {
    fn from(row: CrawlTaskTokenRow) -> Self {
        Self {
            id: row.id,
            organization_id: row.organization_id,
            certificate_id: row.certificate_id,
            token_hash: row.token_hash,
            scope_json: row.scope_json,
            expires_at: row.expires_at,
            used_at: row.used_at,
            created_by_user_id: row.created_by_user_id,
            created_at: row.created_at,
        }
    }
}

impl From<&mut Row<'_>> for CountRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            count: row.get("count"),
        }
    }
}

impl From<&mut Row<'_>> for CountSumRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            count: row.get("count"),
            total: row.get("total"),
        }
    }
}

impl From<&mut Row<'_>> for CountSumIntRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            count: row.get("count"),
            total: row.get("total"),
        }
    }
}

impl From<&mut Row<'_>> for UserIdentityJoinRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            identity_id: row.get("identity_id"),
            user_id: row.get("user_id"),
        }
    }
}

impl From<&mut Row<'_>> for UserLookupRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            id: row.get("id"),
            email: row.get("email"),
            display_name: row.get("display_name"),
            active: row.get("active"),
        }
    }
}

impl From<&mut Row<'_>> for AuthIdentityCacheRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            provider: row.get("provider"),
            external_user_id: row.get("external_user_id"),
            email: row.get("email"),
            display_name: row.get("display_name"),
            active: row.get("active"),
            expires_at: row.get("expires_at"),
        }
    }
}

impl From<&mut Row<'_>> for PlantRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            id: row.get("id"),
            facility_code: row.get("facility_code"),
            owner_organization_id: row.get("owner_organization_id"),
            created_by_organization_id: row.get("created_by_organization_id"),
            name: row.get("name"),
            contract_no: row.get("contract_no"),
            address: row.get("address"),
            installed_kw: row.get("installed_kw"),
            rec_start_date: row.get("rec_start_date"),
            confirm_issued: row.get("confirm_issued"),
        }
    }
}

impl From<&mut Row<'_>> for SyncRunRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            id: row.get("id"),
            organization_id: row.get("organization_id"),
            plant_id: row.get("plant_id"),
            mode: row.get("mode"),
            status: row.get("status"),
            started_at: row.get("started_at"),
            finished_at: row.get("finished_at"),
            duration_ms: row.get("duration_ms"),
        }
    }
}

impl From<&mut Row<'_>> for RoleBindingRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            id: row.get("id"),
            subject_type: row.get("subject_type"),
            subject_id: row.get("subject_id"),
            subject_organization_id: row.get("subject_organization_id"),
            resource_type: row.get("resource_type"),
            resource_id: row.get("resource_id"),
            role_id: row.get("role_id"),
            agreement_id: row.get("agreement_id"),
            state: row.get("state"),
            created_by_user_id: row.get("created_by_user_id"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
        }
    }
}

impl From<&mut Row<'_>> for RoleOrgRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            organization_id: row.get("organization_id"),
        }
    }
}

impl From<&mut Row<'_>> for PartyIDRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            party_id: row.get("party_id"),
        }
    }
}

impl From<&mut Row<'_>> for OrganizationRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            id: row.get("id"),
            party_id: row.get("party_id"),
            business_no: row.get("business_no"),
            name: row.get("name"),
            inactivated_at: row.get("inactivated_at"),
        }
    }
}

impl From<&mut Row<'_>> for PartyRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            id: row.get("id"),
            business_no: row.get("business_no"),
            legal_name: row.get("legal_name"),
            active: row.get("active"),
        }
    }
}

impl From<&mut Row<'_>> for AgreementDocumentRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            id: row.get("id"),
            agreement_id: row.get("agreement_id"),
            document_type: row.get("document_type"),
            file_name: row.get("file_name"),
            object_key: row.get("object_key"),
            content_type: row.get("content_type"),
            document_no: row.get("document_no"),
            uploaded_at: row.get("uploaded_at"),
        }
    }
}

impl From<&mut Row<'_>> for ActionRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            id: row.get("id"),
            code: row.get("code"),
            name: row.get("name"),
            description: row.get("description"),
            risk_level: row.get("risk_level"),
            visibility_granting: row.get("visibility_granting"),
            grantable_to_clients: row.get("grantable_to_clients"),
            consent_label: row.get("consent_label"),
            consent_description: row.get("consent_description"),
        }
    }
}

impl From<&mut Row<'_>> for ClientRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            id: row.get("id"),
            organization_id: row.get("organization_id"),
            slug: row.get("slug"),
            name: row.get("name"),
            description: row.get("description"),
            client_type: row.get("client_type"),
            app_type: row.get("app_type"),
            first_party: row.get("first_party"),
            consent_policy: row.get("consent_policy"),
            active: row.get("active"),
            created_by_user_id: row.get("created_by_user_id"),
        }
    }
}

impl From<&mut Row<'_>> for ClientGrantLookupRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self { id: row.get("id") }
    }
}

impl From<&mut Row<'_>> for ActionLookupRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self { id: row.get("id") }
    }
}

impl From<&mut Row<'_>> for OrganizationPlantActionSeedRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            organization_id: row.get("organization_id"),
            plant_id: row.get("plant_id"),
            action_id: row.get("action_id"),
        }
    }
}

impl From<&mut Row<'_>> for OrganizationMembershipSeedRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            id: row.get("id"),
            organization_id: row.get("organization_id"),
            user_id: row.get("user_id"),
        }
    }
}

impl From<&mut Row<'_>> for GroupMembershipSeedRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            group_id: row.get("group_id"),
            organization_membership_id: row.get("organization_membership_id"),
        }
    }
}

impl From<&mut Row<'_>> for GroupPlantActionSeedRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            group_id: row.get("group_id"),
            organization_id: row.get("organization_id"),
            plant_id: row.get("plant_id"),
            action_id: row.get("action_id"),
        }
    }
}

impl From<&mut Row<'_>> for UserPlantActionSeedRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            user_id: row.get("user_id"),
            organization_id: row.get("organization_id"),
            plant_id: row.get("plant_id"),
            action_id: row.get("action_id"),
        }
    }
}

impl From<&mut Row<'_>> for GenerationRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            month: row.get("month"),
            generation_kwh: row.get("generation_kwh"),
            self_consumption_kwh: row.get("self_consumption_kwh"),
            supply_kwh: row.get("supply_kwh"),
            application_deadline: row.get("application_deadline"),
        }
    }
}

impl From<&mut Row<'_>> for RateRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            month: row.get("month"),
            self_rate: row.get("self_rate"),
            national_average: row.get("national_average"),
            nearby_average: row.get("nearby_average"),
            nearby_highest: row.get("nearby_highest"),
        }
    }
}

impl From<&mut Row<'_>> for RecRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            month: row.get("month"),
            application_no: row.get("application_no"),
            supply_kwh: row.get("supply_kwh"),
            rec_quantity: row.get("rec_quantity"),
            fee_krw: row.get("fee_krw"),
            issued_date: row.get("issued_date"),
            expiry_date: row.get("expiry_date"),
            virtual_account: row.get("virtual_account"),
            bank_name: row.get("bank_name"),
            status: row.get("status"),
        }
    }
}

impl From<&mut Row<'_>> for DocumentRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            id: row.get("id"),
            plant_id: row.get("plant_id"),
            category: row.get("category"),
            file_class: row.get("file_class"),
            file_sec_name: row.get("file_sec_name"),
            file_name: row.get("file_name"),
            required: row.get("required"),
            source_sha256: row.get("source_sha256"),
            output_sha256: row.get("output_sha256"),
            object_key: row.get("object_key"),
            content_type: row.get("content_type"),
        }
    }
}

impl From<&mut Row<'_>> for CategoryCountRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            category: row.get("category"),
            count: row.get("count"),
        }
    }
}

impl From<PlantRow> for Plant {
    fn from(value: PlantRow) -> Self {
        Self {
            id: value.id,
            facility_code: value.facility_code,
            name: value.name,
            owner_organization_id: value.owner_organization_id,
            created_by_organization_id: value.created_by_organization_id,
            contract_no: value.contract_no,
            address: value.address,
            installed_kw: value.installed_kw,
            rec_start_date: value.rec_start_date,
            confirm_issued: value.confirm_issued,
        }
    }
}

impl From<OrganizationRow> for Organization {
    fn from(value: OrganizationRow) -> Self {
        Self {
            id: value.id,
            party_id: value.party_id,
            business_no: value.business_no,
            name: value.name,
            inactivated_at: value.inactivated_at,
        }
    }
}

impl From<PartyRow> for Party {
    fn from(value: PartyRow) -> Self {
        Self {
            id: value.id,
            business_no: value.business_no,
            legal_name: value.legal_name,
            active: value.active == 1,
        }
    }
}

impl TryFrom<AgreementDocumentRow> for AgreementDocument {
    type Error = anyhow::Error;

    fn try_from(value: AgreementDocumentRow) -> Result<Self> {
        Ok(Self {
            id: value.id,
            agreement_id: value.agreement_id,
            document_type: agreement_document_type_from_str(&value.document_type)?,
            file_name: value.file_name,
            object_key: value.object_key,
            content_type: value.content_type,
            document_no: value.document_no,
            uploaded_at: value.uploaded_at,
        })
    }
}

impl TryFrom<ActionRow> for Action {
    type Error = anyhow::Error;

    fn try_from(value: ActionRow) -> Result<Self> {
        Ok(Self {
            id: value.id,
            code: value.code,
            name: value.name,
            description: value.description,
            risk_level: action_risk_level_from_str(&value.risk_level)?,
            visibility_granting: value.visibility_granting == 1,
            grantable_to_clients: value.grantable_to_clients == 1,
            consent_label: value.consent_label,
            consent_description: value.consent_description,
        })
    }
}

impl TryFrom<ClientRow> for Client {
    type Error = anyhow::Error;

    fn try_from(value: ClientRow) -> Result<Self> {
        Ok(Self {
            id: value.id,
            organization_id: value.organization_id,
            slug: value.slug,
            name: value.name,
            description: value.description,
            client_type: client_type_from_str(&value.client_type)?,
            app_type: client_app_type_from_str(&value.app_type)?,
            first_party: value.first_party == 1,
            consent_policy: client_consent_policy_from_str(&value.consent_policy)?,
            active: value.active == 1,
            created_by_user_id: value.created_by_user_id,
        })
    }
}

impl From<SyncRunRow> for SyncRun {
    fn from(value: SyncRunRow) -> Self {
        Self {
            id: value.id,
            organization_id: value.organization_id,
            plant_id: value.plant_id,
            mode: match value.mode.as_str() {
                "delta" => SyncMode::Delta,
                _ => SyncMode::Full,
            },
            status: value.status,
            started_at: Some(value.started_at),
            finished_at: value.finished_at,
            duration_ms: value.duration_ms,
        }
    }
}

impl TryFrom<RoleBindingRow> for RoleBinding {
    type Error = anyhow::Error;

    fn try_from(value: RoleBindingRow) -> Result<Self> {
        Ok(Self {
            id: value.id,
            subject_type: role_binding_subject_type_from_str(&value.subject_type)?,
            subject_id: value.subject_id,
            subject_organization_id: value.subject_organization_id,
            resource_type: role_binding_resource_type_from_str(&value.resource_type)?,
            resource_id: value.resource_id,
            role_id: value.role_id,
            agreement_id: value.agreement_id,
            state: role_binding_state_from_str(&value.state)?,
            created_by_user_id: value.created_by_user_id,
            created_at: value.created_at,
            updated_at: value.updated_at,
        })
    }
}

impl From<GenerationRow> for GenerationRecord {
    fn from(value: GenerationRow) -> Self {
        Self {
            month: value.month,
            generation_kwh: value.generation_kwh,
            self_consumption_kwh: value.self_consumption_kwh,
            supply_kwh: value.supply_kwh,
            application_deadline: value.application_deadline,
        }
    }
}

impl From<RecRow> for RecRecord {
    fn from(value: RecRow) -> Self {
        Self {
            month: value.month,
            application_no: value.application_no,
            supply_kwh: value.supply_kwh,
            rec_quantity: value.rec_quantity,
            fee_krw: value.fee_krw,
            issued_date: value.issued_date,
            expiry_date: value.expiry_date,
            virtual_account: value.virtual_account,
            bank_name: value.bank_name,
            status: value.status,
        }
    }
}

fn validate_role_binding(binding: &RoleBinding) -> Result<()> {
    if binding.subject_organization_id.trim().is_empty() {
        return Err(anyhow!(
            "role binding subject organization id must not be empty"
        ));
    }
    if binding.resource_id.trim().is_empty() {
        return Err(anyhow!("role binding resource id must not be empty"));
    }
    if matches!(binding.subject_type, RoleBindingSubjectType::Organization)
        && binding.subject_id != binding.subject_organization_id
    {
        return Err(anyhow!(
            "organization role binding must use the same subject id and subject organization id"
        ));
    }
    if !matches!(binding.subject_type, RoleBindingSubjectType::Organization)
        && binding.agreement_id.is_some()
    {
        return Err(anyhow!(
            "group/user role bindings must not carry agreement references"
        ));
    }
    if matches!(binding.resource_type, RoleBindingResourceType::Organization)
        && binding.resource_id != binding.subject_organization_id
    {
        return Err(anyhow!(
            "organization-scoped role binding must target the subject organization"
        ));
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct AgreementIdRow {
    id: String,
}

impl From<&mut Row<'_>> for AgreementIdRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            id: row.get::<String>("id"),
        }
    }
}

#[derive(Debug, Clone)]
struct PlantOwnerRow {
    owner_organization_id: String,
}

impl From<&mut Row<'_>> for PlantOwnerRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            owner_organization_id: row.get::<String>("owner_organization_id"),
        }
    }
}

fn role_binding_subject_type_str(subject_type: &RoleBindingSubjectType) -> &'static str {
    match subject_type {
        RoleBindingSubjectType::Organization => "organization",
        RoleBindingSubjectType::Group => "group",
        RoleBindingSubjectType::User => "user",
    }
}

fn role_binding_subject_type_from_str(value: &str) -> Result<RoleBindingSubjectType> {
    match value {
        "organization" => Ok(RoleBindingSubjectType::Organization),
        "group" => Ok(RoleBindingSubjectType::Group),
        "user" => Ok(RoleBindingSubjectType::User),
        other => Err(anyhow!("invalid role binding subject type {other}")),
    }
}

fn role_binding_resource_type_str(resource_type: &RoleBindingResourceType) -> &'static str {
    match resource_type {
        RoleBindingResourceType::Organization => "organization",
        RoleBindingResourceType::Asset => "asset",
        RoleBindingResourceType::Plant => "plant",
    }
}

fn role_binding_resource_type_from_str(value: &str) -> Result<RoleBindingResourceType> {
    match value {
        "organization" => Ok(RoleBindingResourceType::Organization),
        "asset" => Ok(RoleBindingResourceType::Asset),
        "plant" => Ok(RoleBindingResourceType::Plant),
        other => Err(anyhow!("invalid role binding resource type {other}")),
    }
}

fn organization_party_relation_type_str_legal() -> &'static str {
    "legal"
}

fn role_binding_state_str(state: &RoleBindingState) -> &'static str {
    match state {
        RoleBindingState::Active => "active",
        RoleBindingState::Disabled => "disabled",
    }
}

fn role_binding_state_from_str(value: &str) -> Result<RoleBindingState> {
    match value {
        "active" => Ok(RoleBindingState::Active),
        "disabled" => Ok(RoleBindingState::Disabled),
        other => Err(anyhow!("invalid role binding state {other}")),
    }
}

fn agreement_type_str(agreement_type: &domain::AgreementType) -> &'static str {
    match agreement_type {
        domain::AgreementType::Delegation => "delegation",
        domain::AgreementType::Epc => "epc",
        domain::AgreementType::Om => "om",
        domain::AgreementType::Distributor => "distributor",
        domain::AgreementType::Dealer => "dealer",
    }
}

fn agreement_status_str(status: &domain::AgreementStatus) -> &'static str {
    match status {
        domain::AgreementStatus::Draft => "draft",
        domain::AgreementStatus::Active => "active",
        domain::AgreementStatus::Expired => "expired",
        domain::AgreementStatus::Terminated => "terminated",
    }
}

fn agreement_document_type_str(document_type: &AgreementDocumentType) -> &'static str {
    match document_type {
        AgreementDocumentType::MasterContract => "master_contract",
        AgreementDocumentType::Appendix => "appendix",
        AgreementDocumentType::Amendment => "amendment",
        AgreementDocumentType::AuthorizationLetter => "authorization_letter",
        AgreementDocumentType::InvoiceBasis => "invoice_basis",
        AgreementDocumentType::Other => "other",
    }
}

fn client_type_str(client_type: &ClientType) -> &'static str {
    match client_type {
        ClientType::Confidential => "confidential",
        ClientType::Public => "public",
    }
}

fn client_app_type_str(app_type: &ClientAppType) -> &'static str {
    match app_type {
        ClientAppType::Web => "web",
        ClientAppType::Mobile => "mobile",
        ClientAppType::Server => "server",
        ClientAppType::Internal => "internal",
    }
}

fn client_consent_policy_str(policy: &ClientConsentPolicy) -> &'static str {
    match policy {
        ClientConsentPolicy::Explicit => "explicit",
        ClientConsentPolicy::AutoApprove => "auto_approve",
    }
}

fn client_grant_status_str(status: &ClientGrantStatus) -> &'static str {
    match status {
        ClientGrantStatus::Active => "active",
        ClientGrantStatus::Revoked => "revoked",
        ClientGrantStatus::Expired => "expired",
    }
}

fn agreement_document_type_from_str(value: &str) -> Result<AgreementDocumentType> {
    match value {
        "master_contract" => Ok(AgreementDocumentType::MasterContract),
        "appendix" => Ok(AgreementDocumentType::Appendix),
        "amendment" => Ok(AgreementDocumentType::Amendment),
        "authorization_letter" => Ok(AgreementDocumentType::AuthorizationLetter),
        "invoice_basis" => Ok(AgreementDocumentType::InvoiceBasis),
        "other" => Ok(AgreementDocumentType::Other),
        other => Err(anyhow!("invalid agreement document type {other}")),
    }
}

fn action_risk_level_from_str(value: &str) -> Result<ActionRiskLevel> {
    match value {
        "low" => Ok(ActionRiskLevel::Low),
        "medium" => Ok(ActionRiskLevel::Medium),
        "high" => Ok(ActionRiskLevel::High),
        "critical" => Ok(ActionRiskLevel::Critical),
        other => Err(anyhow!("invalid action risk level {other}")),
    }
}

fn client_type_from_str(value: &str) -> Result<ClientType> {
    match value {
        "confidential" => Ok(ClientType::Confidential),
        "public" => Ok(ClientType::Public),
        other => Err(anyhow!("invalid client type {other}")),
    }
}

fn client_app_type_from_str(value: &str) -> Result<ClientAppType> {
    match value {
        "web" => Ok(ClientAppType::Web),
        "mobile" => Ok(ClientAppType::Mobile),
        "server" => Ok(ClientAppType::Server),
        "internal" => Ok(ClientAppType::Internal),
        other => Err(anyhow!("invalid client app type {other}")),
    }
}

fn client_consent_policy_from_str(value: &str) -> Result<ClientConsentPolicy> {
    match value {
        "explicit" => Ok(ClientConsentPolicy::Explicit),
        "auto_approve" => Ok(ClientConsentPolicy::AutoApprove),
        other => Err(anyhow!("invalid client consent policy {other}")),
    }
}

trait DocumentCategoryExt {
    fn to_storage(&self) -> &'static str;
    fn from_storage(value: &str) -> Option<DocumentCategory>;
}

impl DocumentCategoryExt for DocumentCategory {
    fn to_storage(&self) -> &'static str {
        match self {
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

    fn from_storage(value: &str) -> Option<DocumentCategory> {
        match value {
            "plant_info" => Some(DocumentCategory::PlantInfo),
            "permit" => Some(DocumentCategory::Permit),
            "site_land" => Some(DocumentCategory::SiteLand),
            "drawings" => Some(DocumentCategory::Drawings),
            "photos" => Some(DocumentCategory::Photos),
            "equipment" => Some(DocumentCategory::Equipment),
            "finance" => Some(DocumentCategory::Finance),
            "development" => Some(DocumentCategory::Development),
            "other" => Some(DocumentCategory::Other),
            _ => None,
        }
    }
}

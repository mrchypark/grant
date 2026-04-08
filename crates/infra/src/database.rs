use crate::{
    config::ReadConsistency,
    runtime::{HiqliteRuntime, Readiness},
};
use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use domain::{
    Agreement, DocumentArtifact, DocumentCategory, GenerationRecord, HealthStatus, Organization,
    Plant, RateRecord, RecRecord, Role, RoleBinding, RoleBindingResourceType, RoleBindingState,
    RoleBindingSubjectType, SyncMode, SyncRun, User,
};
use hiqlite::Row;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet, VecDeque};
use uuid::Uuid;

macro_rules! params {
    ($($value:expr),* $(,)?) => {
        vec![$(::hiqlite::Param::from($value)),*]
    };
}

#[derive(Clone)]
pub struct Database {
    runtime: HiqliteRuntime,
    bootstrap_user_id: String,
    bootstrap_user_name: String,
}

#[derive(Debug, Clone)]
pub struct StoredCertificate {
    pub id: String,
    pub organization_id: String,
    pub business_no: Option<String>,
    pub common_name: String,
    pub encrypted_blob: String,
    pub active: i64,
    pub last_used_at: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct StoredDocument {
    pub plant_id: String,
    pub artifact: DocumentArtifact,
    pub object_key: String,
    pub content_type: String,
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
struct OrganizationIdRow {
    id: String,
}

#[derive(Debug, Clone)]
struct OrganizationRow {
    id: String,
    business_no: Option<String>,
    name: String,
    legal_entity_id: Option<String>,
    active: i64,
}

#[derive(Debug, Clone)]
struct OrganizationEdgeRow {
    parent_organization_id: String,
    child_organization_id: String,
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
                CREATE TABLE IF NOT EXISTS organizations (
                    id TEXT PRIMARY KEY,
                    business_no TEXT UNIQUE,
                    name TEXT NOT NULL,
                    legal_entity_id TEXT,
                    active INTEGER NOT NULL DEFAULT 1,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS organization_edges (
                    parent_organization_id TEXT NOT NULL,
                    child_organization_id TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    PRIMARY KEY (parent_organization_id, child_organization_id)
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
                CREATE TABLE IF NOT EXISTS actions (
                    id TEXT PRIMARY KEY,
                    code TEXT NOT NULL UNIQUE,
                    name TEXT NOT NULL,
                    description TEXT,
                    risk_level TEXT NOT NULL,
                    visibility_granting INTEGER NOT NULL DEFAULT 0,
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
                CREATE TABLE IF NOT EXISTS role_rules (
                    id TEXT PRIMARY KEY,
                    role_id TEXT NOT NULL,
                    effect TEXT NOT NULL,
                    action_pattern TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
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
                CREATE TABLE IF NOT EXISTS organization_closure (
                    ancestor_organization_id TEXT NOT NULL,
                    descendant_organization_id TEXT NOT NULL,
                    depth INTEGER NOT NULL,
                    PRIMARY KEY (ancestor_organization_id, descendant_organization_id)
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
                CREATE INDEX IF NOT EXISTS idx_organization_memberships_org_user
                    ON organization_memberships (organization_id, user_id);
                CREATE UNIQUE INDEX IF NOT EXISTS idx_user_identities_provider_external
                    ON user_identities (provider, external_user_id);
                CREATE INDEX IF NOT EXISTS idx_auth_identity_cache_expires_at
                    ON auth_identity_cache (expires_at);
                CREATE INDEX IF NOT EXISTS idx_group_memberships_group_member
                    ON group_memberships (group_id, organization_membership_id);
                CREATE INDEX IF NOT EXISTS idx_agreements_grantor_grantee
                    ON agreements (grantor_organization_id, grantee_organization_id, status);
                CREATE INDEX IF NOT EXISTS idx_actions_code
                    ON actions (code);
                CREATE INDEX IF NOT EXISTS idx_roles_org_code
                    ON roles (organization_id, code);
                CREATE INDEX IF NOT EXISTS idx_role_rules_role
                    ON role_rules (role_id);
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
                CREATE INDEX IF NOT EXISTS idx_organization_closure_descendant
                    ON organization_closure (descendant_organization_id, ancestor_organization_id);
                CREATE INDEX IF NOT EXISTS idx_effective_organization_plant_actions_org_plant
                    ON effective_organization_plant_actions (organization_id, plant_id, action_id);
                CREATE INDEX IF NOT EXISTS idx_effective_user_plant_actions_user_org
                    ON effective_user_plant_actions (user_id, organization_id, plant_id, action_id);
                CREATE INDEX IF NOT EXISTS idx_effective_user_plant_visibility_user_org
                    ON effective_user_plant_visibility (user_id, organization_id, plant_id);
                CREATE TABLE IF NOT EXISTS certificates (
                    id TEXT PRIMARY KEY,
                    organization_id TEXT NOT NULL,
                    common_name TEXT NOT NULL,
                    encrypted_blob TEXT NOT NULL,
                    active INTEGER NOT NULL DEFAULT 1,
                    last_used_at TEXT,
                    created_at TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_certificates_organization_active
                    ON certificates (organization_id, active, created_at DESC);
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
                CREATE INDEX IF NOT EXISTS idx_sync_runs_plant_started
                    ON sync_runs (plant_id, started_at DESC);
                CREATE INDEX IF NOT EXISTS idx_plants_owner_facility
                    ON plants (owner_organization_id, facility_code);
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
                "INSERT INTO organizations (id, business_no, name, legal_entity_id, active, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)
                 ON CONFLICT(id) DO UPDATE SET
                    business_no = excluded.business_no,
                    name = excluded.name,
                    active = excluded.active,
                    updated_at = excluded.updated_at",
                params!(
                    self.bootstrap_user_id.clone(),
                    Some(self.bootstrap_user_id.clone()),
                    self.bootstrap_user_name.clone(),
                    Option::<String>::None,
                    1_i64,
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
        for (code, name, risk_level, visibility_granting) in [
            ("organization.plant.create", "Create Plant", "medium", 0_i64),
            (
                "organization.plant.create_for_owner",
                "Create Plant For Owner",
                "high",
                0_i64,
            ),
            (
                "organization.member.manage",
                "Manage Organization Members",
                "high",
                0_i64,
            ),
            (
                "organization.group.manage",
                "Manage Organization Groups",
                "high",
                0_i64,
            ),
            (
                "organization.role.manage",
                "Manage Organization Roles",
                "high",
                0_i64,
            ),
            (
                "organization.permission.manage",
                "Manage Organization Permissions",
                "critical",
                0_i64,
            ),
            ("plant.profile.read", "Read Plant Profile", "low", 1_i64),
            (
                "plant.profile.write",
                "Write Plant Profile",
                "medium",
                0_i64,
            ),
            ("plant.telemetry.read", "Read Plant Telemetry", "low", 0_i64),
            ("plant.alert.read", "Read Plant Alerts", "low", 0_i64),
            ("plant.document.read", "Read Plant Documents", "low", 0_i64),
            (
                "plant.document.write",
                "Write Plant Documents",
                "medium",
                0_i64,
            ),
            ("plant.report.read", "Read Plant Reports", "low", 0_i64),
            ("plant.report.write", "Write Plant Reports", "medium", 0_i64),
            ("plant.rec.read", "Read Plant REC", "low", 0_i64),
            ("plant.rate.read", "Read Plant Rates", "low", 0_i64),
            ("plant.sync.execute", "Execute Plant Sync", "high", 0_i64),
            ("plant.device.read", "Read Plant Device", "low", 0_i64),
            (
                "plant.device.control",
                "Control Plant Device",
                "critical",
                0_i64,
            ),
            (
                "plant.certificate.use",
                "Use Plant Certificate",
                "critical",
                0_i64,
            ),
            (
                "plant.permission.manage",
                "Manage Plant Permissions",
                "critical",
                0_i64,
            ),
            (
                "plant.ownership.transfer",
                "Transfer Plant Ownership",
                "critical",
                0_i64,
            ),
        ] {
            self.runtime
                .execute(
                    "INSERT INTO actions (id, code, name, description, risk_level, visibility_granting, created_at, updated_at)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                     ON CONFLICT(code) DO UPDATE SET
                        name = excluded.name,
                        risk_level = excluded.risk_level,
                        visibility_granting = excluded.visibility_granting,
                        updated_at = excluded.updated_at",
                    params!(
                        code.to_owned(),
                        code.to_owned(),
                        name.to_owned(),
                        Option::<String>::None,
                        risk_level.to_owned(),
                        visibility_granting,
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
        _common_name: &str,
    ) -> Result<Organization> {
        let now = now_string();
        self.runtime
            .execute(
                "INSERT INTO organizations (id, business_no, name, legal_entity_id, active, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)
                 ON CONFLICT(id) DO UPDATE SET
                    business_no = excluded.business_no,
                    name = excluded.name,
                    active = excluded.active,
                    updated_at = excluded.updated_at",
                params!(
                    organization_id.to_owned(),
                    Some(organization_id.to_owned()),
                    display_name.to_owned(),
                    Option::<String>::None,
                    1_i64,
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
        Ok(Organization {
            id: organization_id.to_owned(),
            business_no: Some(organization_id.to_owned()),
            name: display_name.to_owned(),
            legal_entity_id: None,
            active: true,
        })
    }

    pub async fn upsert_organization_identity(
        &self,
        organization_id: &str,
        business_no: Option<&str>,
        display_name: &str,
    ) -> Result<Organization> {
        let now = now_string();
        self.runtime
            .execute(
                "INSERT INTO organizations (id, business_no, name, legal_entity_id, active, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)
                 ON CONFLICT(id) DO UPDATE SET
                    business_no = excluded.business_no,
                    name = excluded.name,
                    active = excluded.active,
                    updated_at = excluded.updated_at",
                params!(
                    organization_id.to_owned(),
                    business_no.map(ToOwned::to_owned),
                    display_name.to_owned(),
                    Option::<String>::None,
                    1_i64,
                    now.clone(),
                    now.clone()
                ),
            )
            .await
            .context("upsert organization identity")?;
        Ok(Organization {
            id: organization_id.to_owned(),
            business_no: business_no.map(ToOwned::to_owned),
            name: display_name.to_owned(),
            legal_entity_id: None,
            active: true,
        })
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
        let _ = external_user_id;
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
                 FROM organization_memberships
                 WHERE user_id = $1 AND organization_id = $2 AND status = 'active'",
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
                "SELECT o.id, o.business_no, o.name, o.legal_entity_id, o.active
                 FROM organizations o
                 JOIN organization_memberships om ON om.organization_id = o.id
                 WHERE om.user_id = $1
                   AND om.status = 'active'
                   AND o.active = 1
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

    pub async fn save_certificate(
        &self,
        organization_id: &str,
        business_no: Option<&str>,
        common_name: &str,
        encrypted_blob: &str,
    ) -> Result<StoredCertificate> {
        self.upsert_organization_identity(organization_id, business_no, common_name)
            .await?;

        self.runtime
            .execute(
                "UPDATE certificates SET active = 0 WHERE organization_id = $1",
                params!(organization_id.to_owned()),
            )
            .await
            .context("deactivate prior certificates")?;

        let id = Uuid::new_v4().simple().to_string();
        let now = now_string();
        self.runtime
            .execute(
                "INSERT INTO certificates (id, organization_id, common_name, encrypted_blob, active, last_used_at, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
                params!(
                    id.clone(),
                    organization_id.to_owned(),
                    common_name.to_owned(),
                    encrypted_blob.to_owned(),
                    1_i64,
                    Option::<String>::None,
                    now
                ),
            )
            .await
            .context("insert certificate")?;
        self.load_active_certificate(organization_id).await
    }

    pub async fn get_organization(&self, organization_id: &str) -> Result<Organization> {
        let rows: Vec<OrganizationRow> = self
            .query_default_rows(
                "SELECT id, business_no, name, legal_entity_id, active
                 FROM organizations
                 WHERE id = $1
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

    pub async fn load_active_certificate(
        &self,
        organization_id: &str,
    ) -> Result<StoredCertificate> {
        let rows: Vec<StoredCertificate> = self
            .query_consistent_rows(
                "SELECT c.id, c.organization_id, o.business_no, c.common_name, c.encrypted_blob, c.active, c.last_used_at, c.created_at
                 FROM certificates c
                 LEFT JOIN organizations o ON o.id = c.organization_id
                 WHERE c.organization_id = $1
                 ORDER BY c.active DESC, c.created_at DESC
                 LIMIT 1",
                params!(organization_id.to_owned()),
                "active certificate",
            )
            .await
            .context("query active certificate")?;
        let cert = rows
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("certificate not found"))?;
        self.runtime
            .execute(
                "UPDATE certificates SET last_used_at = $1 WHERE id = $2",
                params!(now_string(), cert.id.clone()),
            )
            .await
            .context("update certificate last_used_at")?;
        Ok(cert)
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
        self.runtime
            .execute(
                "UPDATE sync_runs
                 SET status = $1, finished_at = $2, duration_ms = $3, error_message = $4
                 WHERE id = $5",
                params!(
                    status.to_owned(),
                    now_string(),
                    duration_ms,
                    error_message.map(ToOwned::to_owned),
                    run_id.to_owned()
                ),
            )
            .await
            .context("finish sync run")?;
        Ok(())
    }

    pub async fn upsert_plant(&self, plant: &Plant) -> Result<()> {
        let now = now_string();
        self.runtime
            .execute(
                "INSERT INTO plants (id, facility_code, owner_organization_id, created_by_organization_id, name, contract_no, address, installed_kw, rec_start_date, confirm_issued, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
                 ON CONFLICT(id) DO UPDATE SET
                    facility_code = excluded.facility_code,
                    owner_organization_id = excluded.owner_organization_id,
                    created_by_organization_id = excluded.created_by_organization_id,
                    name = excluded.name,
                    contract_no = excluded.contract_no,
                    address = excluded.address,
                    installed_kw = excluded.installed_kw,
                    rec_start_date = excluded.rec_start_date,
                    confirm_issued = excluded.confirm_issued,
                    updated_at = excluded.updated_at",
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
                    now.clone(),
                    now
                ),
            )
            .await
            .context("upsert plant")?;
        Ok(())
    }

    pub async fn replace_full_records(
        &self,
        plant_id: &str,
        generation: &[GenerationRecord],
        rates: &[StoredRateRecord],
        rec: &[RecRecord],
    ) -> Result<()> {
        self.runtime
            .execute(
                "DELETE FROM generation_records WHERE plant_id = $1",
                params!(plant_id.to_owned()),
            )
            .await
            .context("clear generation records")?;
        self.runtime
            .execute(
                "DELETE FROM rate_records WHERE plant_id = $1",
                params!(plant_id.to_owned()),
            )
            .await
            .context("clear rate records")?;
        self.runtime
            .execute(
                "DELETE FROM rec_records WHERE plant_id = $1",
                params!(plant_id.to_owned()),
            )
            .await
            .context("clear rec records")?;
        self.upsert_generation_records(plant_id, generation).await?;
        self.upsert_rate_records(plant_id, rates).await?;
        self.upsert_rec_records(plant_id, rec).await?;
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
                "SELECT id, category, file_class, file_sec_name, file_name, required, source_sha256, output_sha256, object_key, content_type
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
        self.runtime
            .execute(
                "DELETE FROM documents WHERE plant_id = $1",
                params!(plant_id.to_owned()),
            )
            .await
            .context("clear documents")?;
        let updated_at = now_string();
        for doc in docs {
            self.runtime
                .execute(
                    "INSERT INTO documents (id, plant_id, category, attachment_code, file_class, file_sec_name, file_name, required, source_sha256, output_sha256, object_key, content_type, updated_at)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
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
                 ORDER BY p.facility_code ASC",
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
        let plant_row = plants.pop().ok_or_else(|| anyhow!("plant not found"))?;
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
        let plant_row = plants.pop().ok_or_else(|| anyhow!("plant not found"))?;
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
        let org_rows: Vec<OrganizationIdRow> = self
            .query_rows_with(
                consistency.clone(),
                "SELECT id FROM organizations ORDER BY id ASC",
                vec![],
                "organization ids for closure rebuild",
            )
            .await?;
        let edge_rows: Vec<OrganizationEdgeRow> = self
            .query_rows_with(
                consistency.clone(),
                "SELECT parent_organization_id, child_organization_id
                 FROM organization_edges
                 ORDER BY parent_organization_id ASC, child_organization_id ASC",
                vec![],
                "organization edges for closure rebuild",
            )
            .await?;
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

        let mut children_by_parent: HashMap<String, Vec<String>> = HashMap::new();
        for edge in &edge_rows {
            children_by_parent
                .entry(edge.parent_organization_id.clone())
                .or_default()
                .push(edge.child_organization_id.clone());
        }

        let mut closure_rows: Vec<(String, String, i64)> = Vec::new();
        for org in &org_rows {
            let mut queue = VecDeque::from([(org.id.clone(), 0_i64)]);
            let mut seen: HashSet<String> = HashSet::new();
            while let Some((descendant, depth)) = queue.pop_front() {
                if !seen.insert(descendant.clone()) {
                    continue;
                }
                closure_rows.push((org.id.clone(), descendant.clone(), depth));
                if let Some(children) = children_by_parent.get(&descendant) {
                    for child in children {
                        queue.push_back((child.clone(), depth + 1));
                    }
                }
            }
        }

        let mut ancestors_by_descendant: HashMap<String, Vec<String>> = HashMap::new();
        for (ancestor, descendant, _) in &closure_rows {
            ancestors_by_descendant
                .entry(descendant.clone())
                .or_default()
                .push(ancestor.clone());
        }

        let mut org_action_rows: HashSet<(String, String, String)> = HashSet::new();
        for seed in &seed_rows {
            if let Some(ancestors) = ancestors_by_descendant.get(&seed.organization_id) {
                for ancestor in ancestors {
                    org_action_rows.insert((
                        ancestor.clone(),
                        seed.plant_id.clone(),
                        seed.action_id.clone(),
                    ));
                }
            }
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

        let mut closure_rows = closure_rows;
        closure_rows.sort();

        let mut org_action_rows: Vec<_> = org_action_rows.into_iter().collect();
        org_action_rows.sort();

        let mut user_action_rows: Vec<_> = user_action_rows.into_iter().collect();
        user_action_rows.sort();

        let mut visibility_rows: Vec<_> = visibility_rows.into_iter().collect();
        visibility_rows.sort();

        let updated_at = Utc::now().to_rfc3339();
        let mut queries: Vec<(&'static str, Vec<hiqlite::Param>)> = Vec::with_capacity(
            4 + closure_rows.len()
                + org_action_rows.len()
                + user_action_rows.len()
                + visibility_rows.len(),
        );
        queries.push(("DELETE FROM organization_closure", vec![]));
        queries.push(("DELETE FROM effective_organization_plant_actions", vec![]));
        queries.push(("DELETE FROM effective_user_plant_actions", vec![]));
        queries.push(("DELETE FROM effective_user_plant_visibility", vec![]));

        for (ancestor, descendant, depth) in closure_rows {
            queries.push((
                "INSERT INTO organization_closure (
                    ancestor_organization_id, descendant_organization_id, depth
                 ) VALUES ($1, $2, $3)",
                params!(ancestor, descendant, depth),
            ));
        }

        for (organization_id, plant_id, action_id) in org_action_rows {
            queries.push((
                "INSERT INTO effective_organization_plant_actions (
                    organization_id, plant_id, action_id, updated_at
                 ) VALUES ($1, $2, $3, $4)",
                params!(organization_id, plant_id, action_id, updated_at.clone()),
            ));
        }

        for (user_id, organization_id, plant_id, action_id) in user_action_rows {
            queries.push((
                "INSERT INTO effective_user_plant_actions (
                    user_id, organization_id, plant_id, action_id, updated_at
                 ) VALUES ($1, $2, $3, $4, $5)",
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
                 ) VALUES ($1, $2, $3, $4)",
                params!(user_id, organization_id, plant_id, updated_at.clone()),
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

        if matches!(binding.subject_type, RoleBindingSubjectType::Organization)
            && matches!(binding.resource_type, RoleBindingResourceType::Plant)
        {
            let subject_is_owner = plant_owner
                .as_deref()
                .map(|owner| owner == binding.subject_id)
                .unwrap_or(false);
            if subject_is_owner {
                if binding.agreement_id.is_some() {
                    return Err(anyhow!(
                        "owner organization plant binding must not carry agreement references"
                    ));
                }
            } else if binding.agreement_id.is_none() {
                return Err(anyhow!(
                    "organization-to-plant role binding must reference an agreement unless the subject organization owns the plant"
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
    use super::validation_read_consistency_for_mode;
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

impl From<&mut Row<'_>> for StoredCertificate {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            id: row.get("id"),
            organization_id: row.get("organization_id"),
            business_no: row.get("business_no"),
            common_name: row.get("common_name"),
            encrypted_blob: row.get("encrypted_blob"),
            active: row.get("active"),
            last_used_at: row.get("last_used_at"),
            created_at: row.get("created_at"),
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

impl From<&mut Row<'_>> for OrganizationIdRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self { id: row.get("id") }
    }
}

impl From<&mut Row<'_>> for OrganizationRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            id: row.get("id"),
            business_no: row.get("business_no"),
            name: row.get("name"),
            legal_entity_id: row.get("legal_entity_id"),
            active: row.get("active"),
        }
    }
}

impl From<&mut Row<'_>> for OrganizationEdgeRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            parent_organization_id: row.get("parent_organization_id"),
            child_organization_id: row.get("child_organization_id"),
        }
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
            business_no: value.business_no,
            name: value.name,
            legal_entity_id: value.legal_entity_id,
            active: value.active == 1,
        }
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
        RoleBindingResourceType::Plant => "plant",
    }
}

fn role_binding_resource_type_from_str(value: &str) -> Result<RoleBindingResourceType> {
    match value {
        "organization" => Ok(RoleBindingResourceType::Organization),
        "plant" => Ok(RoleBindingResourceType::Plant),
        other => Err(anyhow!("invalid role binding resource type {other}")),
    }
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

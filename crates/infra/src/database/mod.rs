use crate::{
    config::ReadConsistency,
    runtime::{HiqliteRuntime, Readiness},
};
use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use domain::{
    Action, Agreement, Application, ApplicationAction, ApplicationGrant, ApplicationGrantAction,
    ApplicationGrantStatus, HealthStatus, Organization, Role, RoleBinding, RoleBindingState,
    RoleBindingSubjectType, ServiceProvider, User,
};
use hiqlite::Row;
use std::borrow::Cow;
use std::collections::HashSet;
use uuid::Uuid;

mod codec;
mod rows;

use codec::*;
use rows::*;

macro_rules! params {
    ($($value:expr),* $(,)?) => {
        vec![$(::hiqlite::Param::from($value)),*]
    };
}

const MAX_AUTHZ_CHECKS_PER_QUERY: usize = 100;

#[derive(Clone)]
pub struct Database {
    runtime: HiqliteRuntime,
    bootstrap_user_id: String,
    bootstrap_user_name: String,
}

#[derive(Debug, Clone)]
pub struct BackupEntry {
    pub name: String,
    pub last_modified: i64,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Copy)]
pub struct BindIdentityInput<'a> {
    pub user_id: &'a str,
    pub provider: &'a str,
    pub external_user_id: &'a str,
    pub email: Option<&'a str>,
    pub display_name: &'a str,
    pub active: bool,
    pub allow_rebind: bool,
}

#[derive(Debug, Clone)]
pub struct UserActionCheck<'a> {
    pub user_id: &'a str,
    pub organization_id: &'a str,
    pub provider_id: &'a str,
    pub resource_type: &'a str,
    pub resource_id: &'a str,
    pub action_id: &'a str,
    pub consistency: Option<ReadConsistency>,
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
        self.drop_removed_grant_tables().await?;
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
                CREATE TABLE IF NOT EXISTS service_providers (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    description TEXT,
                    active INTEGER NOT NULL DEFAULT 1,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS actions (
                    id TEXT PRIMARY KEY,
                    provider_id TEXT NOT NULL DEFAULT 'grant',
                    code TEXT NOT NULL,
                    name TEXT NOT NULL,
                    description TEXT,
                    risk_level TEXT NOT NULL,
                    visibility_granting INTEGER NOT NULL DEFAULT 0,
                    grantable_to_applications INTEGER NOT NULL DEFAULT 0,
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
                    provider_id TEXT NOT NULL DEFAULT 'grant',
                    resource_type TEXT NOT NULL,
                    resource_id TEXT NOT NULL,
                    role_id TEXT NOT NULL,
                    agreement_id TEXT,
                    state TEXT NOT NULL,
                    created_by_user_id TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS applications (
                    id TEXT PRIMARY KEY,
                    organization_id TEXT NOT NULL,
                    slug TEXT NOT NULL UNIQUE,
                    name TEXT NOT NULL,
                    description TEXT,
                    application_type TEXT NOT NULL,
                    app_type TEXT NOT NULL,
                    first_party INTEGER NOT NULL DEFAULT 0,
                    consent_policy TEXT NOT NULL,
                    active INTEGER NOT NULL DEFAULT 1,
                    created_by_user_id TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS application_actions (
                    application_id TEXT NOT NULL,
                    action_id TEXT NOT NULL,
                    PRIMARY KEY (application_id, action_id)
                );
                CREATE TABLE IF NOT EXISTS application_grants (
                    id TEXT PRIMARY KEY,
                    user_id TEXT NOT NULL,
                    application_id TEXT NOT NULL,
                    organization_id TEXT NOT NULL,
                    status TEXT NOT NULL,
                    consented_at TEXT NOT NULL,
                    revoked_at TEXT,
                    revoked_by_user_id TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS application_grant_actions (
                    application_grant_id TEXT NOT NULL,
                    action_id TEXT NOT NULL,
                    PRIMARY KEY (application_grant_id, action_id)
                );
                "#
                .to_owned(),
            )
            .await
            .context("create schema")?;
        self.add_column_if_missing("actions", "provider_id", "TEXT NOT NULL DEFAULT 'grant'")
            .await?;
        self.add_column_if_missing(
            "role_bindings",
            "provider_id",
            "TEXT NOT NULL DEFAULT 'grant'",
        )
        .await?;
        self.migrate_actions_provider_scoped_uniqueness().await?;
        self.migrate_legacy_account_organization_authorization()
            .await?;
        self.ensure_indexes().await?;
        self.drop_removed_grant_tables().await?;
        self.delete_removed_grant_authorization_rows().await?;
        Ok(())
    }

    async fn ensure_indexes(&self) -> Result<()> {
        self.runtime
            .batch(
                r#"
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
                CREATE INDEX IF NOT EXISTS idx_service_providers_active
                    ON service_providers (active, id);
                CREATE INDEX IF NOT EXISTS idx_actions_code
                    ON actions (code);
                DROP INDEX IF EXISTS idx_actions_provider;
                CREATE UNIQUE INDEX IF NOT EXISTS idx_actions_provider_code_unique
                    ON actions (provider_id, code);
                CREATE UNIQUE INDEX IF NOT EXISTS idx_roles_global_code
                    ON roles (code)
                    WHERE organization_id IS NULL;
                CREATE INDEX IF NOT EXISTS idx_role_bindings_subject_resource_state
                    ON role_bindings (subject_type, subject_id, provider_id, resource_type, resource_id, state);
                DROP INDEX IF EXISTS idx_role_bindings_logical_unique;
                CREATE UNIQUE INDEX IF NOT EXISTS idx_role_bindings_logical_unique
                    ON role_bindings (
                        subject_type,
                        subject_id,
                        subject_organization_id,
                        provider_id,
                        resource_type,
                        resource_id,
                        role_id
                    );
                CREATE INDEX IF NOT EXISTS idx_application_grants_user_application
                    ON application_grants (user_id, application_id, organization_id, status);
                CREATE UNIQUE INDEX IF NOT EXISTS idx_application_grants_active_unique
                    ON application_grants (user_id, application_id, organization_id)
                    WHERE status = 'active';
                "#
                .to_owned(),
            )
            .await
            .context("create indexes")?;
        Ok(())
    }

    async fn migrate_actions_provider_scoped_uniqueness(&self) -> Result<()> {
        let rows: Vec<TableSqlRow> = self
            .query_default_rows(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'actions' LIMIT 1",
                vec![],
                "actions table sql",
            )
            .await?;
        let table_sql = rows
            .into_iter()
            .next()
            .and_then(|row| row.sql)
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !table_sql.contains("code text not null unique") {
            return Ok(());
        }

        self.runtime
            .batch(
                r#"
                DROP INDEX IF EXISTS idx_actions_code;
                DROP INDEX IF EXISTS idx_actions_provider;
                CREATE TABLE actions_provider_scoped_migration (
                    id TEXT PRIMARY KEY,
                    provider_id TEXT NOT NULL DEFAULT 'grant',
                    code TEXT NOT NULL,
                    name TEXT NOT NULL,
                    description TEXT,
                    risk_level TEXT NOT NULL,
                    visibility_granting INTEGER NOT NULL DEFAULT 0,
                    grantable_to_applications INTEGER NOT NULL DEFAULT 0,
                    consent_label TEXT,
                    consent_description TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                INSERT INTO actions_provider_scoped_migration (
                    id, provider_id, code, name, description, risk_level, visibility_granting,
                    grantable_to_applications, consent_label, consent_description, created_at, updated_at
                )
                SELECT
                    id, provider_id, code, name, description, risk_level, visibility_granting,
                    grantable_to_applications, consent_label, consent_description, created_at, updated_at
                FROM actions;
                DROP TABLE actions;
                ALTER TABLE actions_provider_scoped_migration RENAME TO actions;
                "#
                .to_owned(),
            )
            .await
            .context("migrate actions provider-scoped uniqueness")?;
        Ok(())
    }

    async fn add_column_if_missing(
        &self,
        table_name: &str,
        column_name: &str,
        column_definition: &str,
    ) -> Result<()> {
        let escaped_table = table_name.replace('\'', "''");
        let columns: Vec<ColumnNameRow> = self
            .query_default_rows(
                format!("SELECT name FROM pragma_table_info('{escaped_table}')"),
                vec![],
                "table column lookup",
            )
            .await?;
        if columns.iter().any(|column| column.name == column_name) {
            return Ok(());
        }
        self.runtime
            .execute(
                format!("ALTER TABLE {table_name} ADD COLUMN {column_name} {column_definition}"),
                vec![],
            )
            .await
            .with_context(|| format!("add {table_name}.{column_name}"))?;
        Ok(())
    }

    async fn drop_removed_grant_tables(&self) -> Result<()> {
        self.runtime
            .batch(
                r#"
                DROP TABLE IF EXISTS agreement_documents;
                DROP TABLE IF EXISTS assets;
                DROP TABLE IF EXISTS asset_profiles;
                DROP TABLE IF EXISTS registries;
                DROP TABLE IF EXISTS registry_accounts;
                DROP TABLE IF EXISTS registry_projects;
                DROP TABLE IF EXISTS data_sources;
                DROP TABLE IF EXISTS generation_data;
                DROP TABLE IF EXISTS rate_data;
                DROP TABLE IF EXISTS rate_data_old_real_value;
                DROP TABLE IF EXISTS energy_certificates;
                DROP TABLE IF EXISTS auth_credentials;
                DROP TABLE IF EXISTS certificates;
                DROP TABLE IF EXISTS cert_gateway_challenges;
                DROP TABLE IF EXISTS crawl_task_tokens;
                DROP TABLE IF EXISTS credential_bindings;
                DROP TABLE IF EXISTS country_capabilities;
                DROP TABLE IF EXISTS sync_runs;
                DROP TABLE IF EXISTS sync_run_items;
                DROP TABLE IF EXISTS effective_organization_plant_actions;
                DROP TABLE IF EXISTS effective_user_plant_actions;
                DROP TABLE IF EXISTS effective_user_plant_visibility;
                DROP TABLE IF EXISTS effective_organization_asset_actions;
                DROP TABLE IF EXISTS effective_user_asset_actions;
                DROP TABLE IF EXISTS effective_user_asset_visibility;
                DROP TABLE IF EXISTS plants;
                DROP TABLE IF EXISTS generation_records;
                DROP TABLE IF EXISTS rate_records;
                DROP TABLE IF EXISTS rec_records;
                DROP TABLE IF EXISTS documents;
                "#
                .to_owned(),
            )
            .await
            .context("drop removed grant tables")?;
        Ok(())
    }

    async fn delete_removed_grant_authorization_rows(&self) -> Result<()> {
        self.runtime
            .batch(
                r#"
                DELETE FROM application_grant_actions
                WHERE action_id IN (
                    SELECT id FROM actions
                    WHERE provider_id IN ('grant', 'account')
                      AND (
                          code LIKE 'plant.%'
                          OR code LIKE 'asset.%'
                          OR code LIKE 'document.%'
                          OR code LIKE 'cert-data.%'
                          OR code LIKE 'cert_data.%'
                          OR code LIKE 'organization.plant.%'
                          OR code = 'organization.credential.use'
                      )
                );
                DELETE FROM application_actions
                WHERE action_id IN (
                    SELECT id FROM actions
                    WHERE provider_id IN ('grant', 'account')
                      AND (
                          code LIKE 'plant.%'
                          OR code LIKE 'asset.%'
                          OR code LIKE 'document.%'
                          OR code LIKE 'cert-data.%'
                          OR code LIKE 'cert_data.%'
                          OR code LIKE 'organization.plant.%'
                          OR code = 'organization.credential.use'
                      )
                );
                DELETE FROM role_actions
                WHERE action_id IN (
                    SELECT id FROM actions
                    WHERE provider_id IN ('grant', 'account')
                      AND (
                          code LIKE 'plant.%'
                          OR code LIKE 'asset.%'
                          OR code LIKE 'document.%'
                          OR code LIKE 'cert-data.%'
                          OR code LIKE 'cert_data.%'
                          OR code LIKE 'organization.plant.%'
                          OR code = 'organization.credential.use'
                      )
                );
                DELETE FROM role_bindings
                WHERE provider_id IN ('grant', 'account')
                  AND resource_type <> 'organization';
                DELETE FROM actions
                WHERE provider_id IN ('grant', 'account')
                  AND (
                      code LIKE 'plant.%'
                      OR code LIKE 'asset.%'
                      OR code LIKE 'document.%'
                      OR code LIKE 'cert-data.%'
                      OR code LIKE 'cert_data.%'
                      OR code LIKE 'organization.plant.%'
                      OR code = 'organization.credential.use'
                  );
                "#
                .to_owned(),
            )
            .await
            .context("delete removed grant authorization rows")?;
        Ok(())
    }

    async fn migrate_legacy_account_organization_authorization(&self) -> Result<()> {
        self.runtime
            .batch(
                r#"
                UPDATE OR IGNORE role_actions
                SET action_id = (
                    SELECT grant_action.id
                    FROM actions AS account_action
                    JOIN actions AS grant_action
                      ON grant_action.provider_id = 'grant'
                     AND grant_action.code = account_action.code
                    WHERE account_action.id = role_actions.action_id
                      AND account_action.provider_id = 'account'
                      AND account_action.code LIKE 'organization.%'
                    LIMIT 1
                )
                WHERE EXISTS (
                    SELECT 1
                    FROM actions AS account_action
                    JOIN actions AS grant_action
                      ON grant_action.provider_id = 'grant'
                     AND grant_action.code = account_action.code
                    WHERE account_action.id = role_actions.action_id
                      AND account_action.provider_id = 'account'
                      AND account_action.code LIKE 'organization.%'
                );

                UPDATE OR IGNORE application_actions
                SET action_id = (
                    SELECT grant_action.id
                    FROM actions AS account_action
                    JOIN actions AS grant_action
                      ON grant_action.provider_id = 'grant'
                     AND grant_action.code = account_action.code
                    WHERE account_action.id = application_actions.action_id
                      AND account_action.provider_id = 'account'
                      AND account_action.code LIKE 'organization.%'
                    LIMIT 1
                )
                WHERE EXISTS (
                    SELECT 1
                    FROM actions AS account_action
                    JOIN actions AS grant_action
                      ON grant_action.provider_id = 'grant'
                     AND grant_action.code = account_action.code
                    WHERE account_action.id = application_actions.action_id
                      AND account_action.provider_id = 'account'
                      AND account_action.code LIKE 'organization.%'
                );

                UPDATE OR IGNORE application_grant_actions
                SET action_id = (
                    SELECT grant_action.id
                    FROM actions AS account_action
                    JOIN actions AS grant_action
                      ON grant_action.provider_id = 'grant'
                     AND grant_action.code = account_action.code
                    WHERE account_action.id = application_grant_actions.action_id
                      AND account_action.provider_id = 'account'
                      AND account_action.code LIKE 'organization.%'
                    LIMIT 1
                )
                WHERE EXISTS (
                    SELECT 1
                    FROM actions AS account_action
                    JOIN actions AS grant_action
                      ON grant_action.provider_id = 'grant'
                     AND grant_action.code = account_action.code
                    WHERE account_action.id = application_grant_actions.action_id
                      AND account_action.provider_id = 'account'
                      AND account_action.code LIKE 'organization.%'
                );

                DELETE FROM role_actions
                WHERE action_id IN (
                    SELECT account_action.id
                    FROM actions AS account_action
                    JOIN actions AS grant_action
                      ON grant_action.provider_id = 'grant'
                     AND grant_action.code = account_action.code
                    WHERE account_action.provider_id = 'account'
                      AND account_action.code LIKE 'organization.%'
                );

                DELETE FROM application_actions
                WHERE action_id IN (
                    SELECT account_action.id
                    FROM actions AS account_action
                    JOIN actions AS grant_action
                      ON grant_action.provider_id = 'grant'
                     AND grant_action.code = account_action.code
                    WHERE account_action.provider_id = 'account'
                      AND account_action.code LIKE 'organization.%'
                );

                DELETE FROM application_grant_actions
                WHERE action_id IN (
                    SELECT account_action.id
                    FROM actions AS account_action
                    JOIN actions AS grant_action
                      ON grant_action.provider_id = 'grant'
                     AND grant_action.code = account_action.code
                    WHERE account_action.provider_id = 'account'
                      AND account_action.code LIKE 'organization.%'
                );

                DELETE FROM actions
                WHERE provider_id = 'account'
                  AND code LIKE 'organization.%'
                  AND EXISTS (
                      SELECT 1
                      FROM actions AS grant_action
                      WHERE grant_action.provider_id = 'grant'
                        AND grant_action.code = actions.code
                  );

                UPDATE actions
                SET provider_id = 'grant'
                WHERE provider_id = 'account'
                  AND code LIKE 'organization.%';

                UPDATE role_bindings AS grant_binding
                SET
                    state = CASE
                        WHEN EXISTS (
                            SELECT 1
                            FROM role_bindings AS account_binding
                            WHERE account_binding.subject_type = grant_binding.subject_type
                              AND account_binding.subject_id = grant_binding.subject_id
                              AND account_binding.subject_organization_id = grant_binding.subject_organization_id
                              AND account_binding.provider_id = 'account'
                              AND account_binding.resource_type = grant_binding.resource_type
                              AND account_binding.resource_id = grant_binding.resource_id
                              AND account_binding.role_id = grant_binding.role_id
                              AND account_binding.state = 'active'
                        )
                        THEN 'active'
                        ELSE grant_binding.state
                    END,
                    agreement_id = CASE
                        WHEN EXISTS (
                            SELECT 1
                            FROM role_bindings AS account_binding
                            WHERE account_binding.subject_type = grant_binding.subject_type
                              AND account_binding.subject_id = grant_binding.subject_id
                              AND account_binding.subject_organization_id = grant_binding.subject_organization_id
                              AND account_binding.provider_id = 'account'
                              AND account_binding.resource_type = grant_binding.resource_type
                              AND account_binding.resource_id = grant_binding.resource_id
                              AND account_binding.role_id = grant_binding.role_id
                              AND account_binding.state = 'active'
                        )
                        THEN (
                            SELECT account_binding.agreement_id
                            FROM role_bindings AS account_binding
                            WHERE account_binding.subject_type = grant_binding.subject_type
                              AND account_binding.subject_id = grant_binding.subject_id
                              AND account_binding.subject_organization_id = grant_binding.subject_organization_id
                              AND account_binding.provider_id = 'account'
                              AND account_binding.resource_type = grant_binding.resource_type
                              AND account_binding.resource_id = grant_binding.resource_id
                              AND account_binding.role_id = grant_binding.role_id
                              AND account_binding.state = 'active'
                            ORDER BY account_binding.updated_at DESC, account_binding.id ASC
                            LIMIT 1
                        )
                        ELSE grant_binding.agreement_id
                    END,
                    updated_at = CASE
                        WHEN EXISTS (
                            SELECT 1
                            FROM role_bindings AS account_binding
                            WHERE account_binding.subject_type = grant_binding.subject_type
                              AND account_binding.subject_id = grant_binding.subject_id
                              AND account_binding.subject_organization_id = grant_binding.subject_organization_id
                              AND account_binding.provider_id = 'account'
                              AND account_binding.resource_type = grant_binding.resource_type
                              AND account_binding.resource_id = grant_binding.resource_id
                              AND account_binding.role_id = grant_binding.role_id
                              AND account_binding.updated_at > grant_binding.updated_at
                        )
                        THEN (
                            SELECT account_binding.updated_at
                            FROM role_bindings AS account_binding
                            WHERE account_binding.subject_type = grant_binding.subject_type
                              AND account_binding.subject_id = grant_binding.subject_id
                              AND account_binding.subject_organization_id = grant_binding.subject_organization_id
                              AND account_binding.provider_id = 'account'
                              AND account_binding.resource_type = grant_binding.resource_type
                              AND account_binding.resource_id = grant_binding.resource_id
                              AND account_binding.role_id = grant_binding.role_id
                            ORDER BY account_binding.updated_at DESC, account_binding.id ASC
                            LIMIT 1
                        )
                        ELSE grant_binding.updated_at
                    END
                WHERE grant_binding.provider_id = 'grant'
                  AND grant_binding.resource_type = 'organization'
                  AND EXISTS (
                      SELECT 1
                      FROM role_bindings AS account_binding
                      WHERE account_binding.subject_type = grant_binding.subject_type
                        AND account_binding.subject_id = grant_binding.subject_id
                        AND account_binding.subject_organization_id = grant_binding.subject_organization_id
                        AND account_binding.provider_id = 'account'
                        AND account_binding.resource_type = grant_binding.resource_type
                        AND account_binding.resource_id = grant_binding.resource_id
                        AND account_binding.role_id = grant_binding.role_id
                  );

                DELETE FROM role_bindings
                WHERE provider_id = 'account'
                  AND resource_type = 'organization'
                  AND EXISTS (
                      SELECT 1
                      FROM role_bindings grant_binding
                      WHERE grant_binding.subject_type = role_bindings.subject_type
                        AND grant_binding.subject_id = role_bindings.subject_id
                        AND grant_binding.subject_organization_id = role_bindings.subject_organization_id
                        AND grant_binding.provider_id = 'grant'
                        AND grant_binding.resource_type = role_bindings.resource_type
                        AND grant_binding.resource_id = role_bindings.resource_id
                        AND grant_binding.role_id = role_bindings.role_id
                  );

                UPDATE role_bindings
                SET provider_id = 'grant'
                WHERE provider_id = 'account'
                  AND resource_type = 'organization';
                "#
                .to_owned(),
            )
            .await
            .context("migrate legacy account organization authorization")?;
        Ok(())
    }

    pub async fn ensure_schema(&self) -> Result<()> {
        self.ensure_schema_only().await?;
        self.ensure_bootstrap_data().await
    }

    pub async fn ensure_bootstrap_data(&self) -> Result<()> {
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
            "organization.application.manage",
            "organization.profile.read",
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
            provider_id: "grant".to_owned(),
            resource_type: "organization".to_owned(),
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
        self.upsert_service_provider(&ServiceProvider {
            id: "grant".to_owned(),
            name: "Grant".to_owned(),
            description: Some("Grant organization authorization service".to_owned()),
            active: true,
        })
        .await?;
        for (code, name, risk_level, visibility_granting, grantable_to_applications) in [
            (
                "organization.application.manage",
                "Manage Organization Applications",
                "high",
                0_i64,
                0_i64,
            ),
            (
                "organization.profile.read",
                "Read Organization Profile",
                "low",
                1_i64,
                1_i64,
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
        ] {
            self.runtime
                .execute(
                    "INSERT INTO actions (
                        id, provider_id, code, name, description, risk_level, visibility_granting,
                        grantable_to_applications, consent_label, consent_description, created_at, updated_at
                     )
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
                     ON CONFLICT(provider_id, code) DO UPDATE SET
                        name = excluded.name,
                        risk_level = excluded.risk_level,
                        visibility_granting = excluded.visibility_granting,
                        grantable_to_applications = excluded.grantable_to_applications,
                        consent_label = excluded.consent_label,
                        consent_description = excluded.consent_description,
                        updated_at = excluded.updated_at",
                    params!(
                        code.to_owned(),
                        "grant".to_owned(),
                        code.to_owned(),
                        name.to_owned(),
                        Option::<String>::None,
                        risk_level.to_owned(),
                        visibility_granting,
                        grantable_to_applications,
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

    pub async fn bind_identity_to_user(&self, input: BindIdentityInput<'_>) -> Result<User> {
        let now = now_string();
        let existing: Option<UserIdentityJoinRow> = self
            .runtime
            .query_rows(
                self.validation_read_consistency(),
                "SELECT ui.id as identity_id,
                        ui.user_id as user_id
                 FROM user_identities ui
                 WHERE ui.provider = $1 AND ui.external_user_id = $2",
                params!(input.provider.to_owned(), input.external_user_id.to_owned()),
            )
            .await
            .context("query user identity for binding")?
            .into_iter()
            .next();

        if !input.allow_rebind
            && let Some(existing) = existing.as_ref()
            && existing.user_id != input.user_id
        {
            anyhow::bail!(
                "identity {}:{} is already bound to another user {}",
                input.provider,
                input.external_user_id,
                existing.user_id
            );
        }

        let target_user = self
            .find_user_by_id(input.user_id, None)
            .await?
            .ok_or_else(|| anyhow::anyhow!("target user {} does not exist", input.user_id))?;

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
                    input
                        .email
                        .map(ToOwned::to_owned)
                        .or(target_user.email.clone()),
                    input.display_name.to_owned(),
                    if input.active { 1_i64 } else { 0_i64 },
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
                    input.user_id.to_owned(),
                    input.provider.to_owned(),
                    input.external_user_id.to_owned(),
                    input.email.map(ToOwned::to_owned),
                    Some(input.display_name.to_owned()),
                    Some(now.clone()),
                    now.clone(),
                    now.clone()
                ),
            )
            .await
            .context("bind identity to user")?;

        Ok(User {
            id: input.user_id.to_owned(),
            email: input.email.map(ToOwned::to_owned).or(target_user.email),
            display_name: input.display_name.to_owned(),
            active: input.active,
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

    pub async fn upsert_service_provider(&self, provider: &ServiceProvider) -> Result<()> {
        let now = now_string();
        self.runtime
            .execute(
                "INSERT INTO service_providers (id, name, description, active, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT(id) DO UPDATE SET
                    name = excluded.name,
                    description = excluded.description,
                    active = excluded.active,
                    updated_at = excluded.updated_at",
                params!(
                    provider.id.clone(),
                    provider.name.clone(),
                    provider.description.clone(),
                    if provider.active { 1_i64 } else { 0_i64 },
                    now.clone(),
                    now
                ),
            )
            .await
            .context("upsert service provider")?;
        Ok(())
    }

    pub async fn list_service_providers_with_consistency(
        &self,
        consistency: Option<ReadConsistency>,
    ) -> Result<Vec<ServiceProvider>> {
        let rows: Vec<ServiceProviderRow> = self
            .query_rows_with(
                self.validation_read_consistency_with_override(consistency),
                "SELECT id, name, description, active
                 FROM service_providers
                 ORDER BY name ASC, id ASC",
                vec![],
                "service providers",
            )
            .await?;
        Ok(rows.into_iter().map(ServiceProvider::from).collect())
    }

    pub async fn upsert_action(&self, action: &Action) -> Result<()> {
        if !self
            .exists_row_with(
                ReadConsistency::Strong,
                "SELECT COUNT(*) as count FROM service_providers WHERE id = $1",
                params!(action.provider_id.clone()),
                "action provider existence",
            )
            .await?
        {
            return Err(anyhow!(
                "service provider {} does not exist",
                action.provider_id
            ));
        }
        let now = now_string();
        self.runtime
            .execute(
                "INSERT INTO actions (
                    id, provider_id, code, name, description, risk_level, visibility_granting,
                    grantable_to_applications, consent_label, consent_description, created_at, updated_at
                 )
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
                 ON CONFLICT(provider_id, code) DO UPDATE SET
                    name = excluded.name,
                    description = excluded.description,
                    risk_level = excluded.risk_level,
                    visibility_granting = excluded.visibility_granting,
                    grantable_to_applications = excluded.grantable_to_applications,
                    consent_label = excluded.consent_label,
                    consent_description = excluded.consent_description,
                    updated_at = excluded.updated_at",
                params!(
                    action.id.clone(),
                    action.provider_id.clone(),
                    action.code.clone(),
                    action.name.clone(),
                    action.description.clone(),
                    action_risk_level_str(&action.risk_level),
                    if action.visibility_granting { 1_i64 } else { 0_i64 },
                    if action.grantable_to_applications {
                        1_i64
                    } else {
                        0_i64
                    },
                    action.consent_label.clone(),
                    action.consent_description.clone(),
                    now.clone(),
                    now
                ),
            )
            .await
            .context("upsert action")?;
        Ok(())
    }

    pub async fn replace_provider_actions(
        &self,
        provider_id: &str,
        actions: &[Action],
    ) -> Result<Vec<Action>> {
        if !self
            .exists_row_with(
                ReadConsistency::Strong,
                "SELECT COUNT(*) as count FROM service_providers WHERE id = $1",
                params!(provider_id.to_owned()),
                "provider existence for action replace",
            )
            .await?
        {
            return Err(anyhow!("service provider {provider_id} does not exist"));
        }
        validate_provider_action_manifest(provider_id, actions)?;
        let keep_ids = actions
            .iter()
            .map(|action| action.id.clone())
            .collect::<HashSet<_>>();
        let existing_actions = self
            .list_actions_for_provider_with_consistency(provider_id, Some(ReadConsistency::Strong))
            .await?;
        for existing in existing_actions {
            if keep_ids.contains(&existing.id) {
                continue;
            }
            self.runtime
                .transaction([
                    (
                        Cow::Borrowed("DELETE FROM application_grant_actions WHERE action_id = $1"),
                        params!(existing.id.clone()),
                    ),
                    (
                        Cow::Borrowed("DELETE FROM application_actions WHERE action_id = $1"),
                        params!(existing.id.clone()),
                    ),
                    (
                        Cow::Borrowed("DELETE FROM role_actions WHERE action_id = $1"),
                        params!(existing.id.clone()),
                    ),
                    (
                        Cow::Borrowed("DELETE FROM actions WHERE id = $1"),
                        params!(existing.id.clone()),
                    ),
                ])
                .await
                .with_context(|| format!("remove stale provider action {}", existing.id))?;
        }
        for action in actions {
            self.upsert_action(action).await?;
        }
        self.list_actions_for_provider_with_consistency(provider_id, Some(ReadConsistency::Strong))
            .await
    }

    pub async fn list_actions_for_provider_with_consistency(
        &self,
        provider_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Vec<Action>> {
        let rows: Vec<ActionRow> = self
            .query_rows_with(
                self.validation_read_consistency_with_override(consistency),
                "SELECT id, provider_id, code, name, description, risk_level, visibility_granting,
                        grantable_to_applications, consent_label, consent_description
                 FROM actions
                 WHERE provider_id = $1
                 ORDER BY code ASC",
                params!(provider_id.to_owned()),
                "provider actions",
            )
            .await?;
        rows.into_iter().map(TryInto::try_into).collect()
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

    pub async fn insert_role(&self, role: &Role) -> Result<()> {
        let now = now_string();
        self.runtime
            .execute(
                "INSERT INTO roles (id, organization_id, code, name, description, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
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
            .context("insert role")?;
        Ok(())
    }

    pub async fn get_role_with_consistency(
        &self,
        role_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Role> {
        let rows: Vec<RoleRow> = self
            .query_rows_with(
                self.validation_read_consistency_with_override(consistency),
                "SELECT id, organization_id, code, name, description
                 FROM roles
                 WHERE id = $1
                 LIMIT 1",
                params!(role_id.to_owned()),
                "role by id",
            )
            .await?;
        rows.into_iter()
            .next()
            .ok_or_else(|| anyhow!("role not found: {role_id}"))
            .map(Role::from)
    }

    pub async fn list_roles_for_organization_with_consistency(
        &self,
        organization_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Vec<Role>> {
        let rows: Vec<RoleRow> = self
            .query_rows_with(
                self.validation_read_consistency_with_override(consistency),
                "SELECT id, organization_id, code, name, description
                 FROM roles
                 WHERE organization_id IS NULL OR organization_id = $1
                 ORDER BY organization_id IS NOT NULL DESC, name ASC, id ASC",
                params!(organization_id.to_owned()),
                "roles by organization",
            )
            .await?;
        Ok(rows.into_iter().map(Role::from).collect())
    }

    pub async fn replace_role_actions(
        &self,
        role_id: &str,
        action_ids: &[String],
    ) -> Result<Vec<Action>> {
        self.get_role_with_consistency(role_id, Some(ReadConsistency::Strong))
            .await?;
        let mut deduped = Vec::new();
        let mut seen = HashSet::new();
        for action_id in action_ids {
            let trimmed = action_id.trim();
            if trimmed.is_empty() {
                return Err(anyhow!("role action ids must not be empty"));
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
            let rows: Vec<ActionLookupRow> = self
                .query_rows_with(
                    ReadConsistency::Strong,
                    format!("SELECT id FROM actions WHERE id IN ({placeholders})"),
                    deduped
                        .iter()
                        .cloned()
                        .map(hiqlite::Param::from)
                        .collect::<Vec<_>>(),
                    "role action existence set",
                )
                .await?;
            let action_ids = rows.into_iter().map(|row| row.id).collect::<HashSet<_>>();
            if let Some(missing) = deduped
                .iter()
                .find(|action_id| !action_ids.contains(*action_id))
            {
                return Err(anyhow!("action {missing} does not exist"));
            }
        }
        let mut queries = Vec::with_capacity(1 + deduped.len());
        queries.push((
            Cow::Borrowed("DELETE FROM role_actions WHERE role_id = $1"),
            params!(role_id.to_owned()),
        ));
        for action_id in deduped {
            queries.push((
                Cow::Borrowed(
                    "INSERT INTO role_actions (role_id, action_id)
                     VALUES ($1, $2)
                     ON CONFLICT(role_id, action_id) DO NOTHING",
                ),
                params!(role_id.to_owned(), action_id.clone()),
            ));
        }
        self.runtime
            .transaction(queries)
            .await
            .context("replace role actions transaction")?;
        self.list_role_actions_with_consistency(role_id, Some(ReadConsistency::Strong))
            .await
    }

    pub async fn list_role_actions_with_consistency(
        &self,
        role_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Vec<Action>> {
        let rows: Vec<ActionRow> = self
            .query_rows_with(
                self.validation_read_consistency_with_override(consistency),
                "SELECT a.id, a.provider_id, a.code, a.name, a.description, a.risk_level,
                        a.visibility_granting, a.grantable_to_applications, a.consent_label,
                        a.consent_description
                 FROM role_actions ra
                 JOIN actions a ON a.id = ra.action_id
                 WHERE ra.role_id = $1
                 ORDER BY a.code ASC",
                params!(role_id.to_owned()),
                "role actions",
            )
            .await?;
        rows.into_iter().map(TryInto::try_into).collect()
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

    async fn validate_application_references(&self, application: &Application) -> Result<()> {
        if !self
            .exists_row_with(
                ReadConsistency::Strong,
                "SELECT COUNT(*) as count FROM organizations WHERE id = $1",
                params!(application.organization_id.clone()),
                "application organization existence",
            )
            .await?
        {
            return Err(anyhow!(
                "application organization {} does not exist",
                application.organization_id
            ));
        }
        if !self
            .exists_row_with(
                ReadConsistency::Strong,
                "SELECT COUNT(*) as count FROM users WHERE id = $1",
                params!(application.created_by_user_id.clone()),
                "application creator existence",
            )
            .await?
        {
            return Err(anyhow!(
                "application creator user {} does not exist",
                application.created_by_user_id
            ));
        }
        Ok(())
    }

    pub async fn insert_application(&self, application: &Application) -> Result<()> {
        self.validate_application_references(application).await?;
        let now = now_string();
        self.runtime
            .execute(
                "INSERT INTO applications (
                    id, organization_id, slug, name, description, application_type, app_type,
                    first_party, consent_policy, active, created_by_user_id, created_at, updated_at
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
                params!(
                    application.id.clone(),
                    application.organization_id.clone(),
                    application.slug.clone(),
                    application.name.clone(),
                    application.description.clone(),
                    application_type_str(&application.application_type),
                    application_app_type_str(&application.app_type),
                    if application.first_party {
                        1_i64
                    } else {
                        0_i64
                    },
                    application_consent_policy_str(&application.consent_policy),
                    if application.active { 1_i64 } else { 0_i64 },
                    application.created_by_user_id.clone(),
                    now.clone(),
                    now
                ),
            )
            .await
            .context("insert application")?;
        Ok(())
    }

    pub async fn update_application(
        &self,
        application: &Application,
        slug_changed: bool,
    ) -> Result<()> {
        self.validate_application_references(application).await?;
        let conflict_count = self
            .query_count(
                ReadConsistency::Strong,
                "SELECT COUNT(*) as count
                 FROM applications
                 WHERE slug = $1
                   AND id <> $2",
                params!(application.slug.clone(), application.id.clone()),
                "application slug conflict",
            )
            .await?;
        if conflict_count > 0 {
            return Err(anyhow!(
                "application slug already exists: {}",
                application.slug
            ));
        }
        let now = now_string();
        let stmt = if slug_changed {
            "UPDATE applications
             SET organization_id = $2,
                 slug = $3,
                 name = $4,
                 description = $5,
                 application_type = $6,
                 app_type = $7,
                 first_party = $8,
                 consent_policy = $9,
                 active = $10,
                 created_by_user_id = $11,
                 updated_at = $12
             WHERE id = $1
               AND NOT EXISTS (
                   SELECT 1 FROM applications existing
                   WHERE existing.slug = $13 AND existing.id != $14
               )"
        } else {
            "UPDATE applications
             SET organization_id = $2,
                 slug = $3,
                 name = $4,
                 description = $5,
                 application_type = $6,
                 app_type = $7,
                 first_party = $8,
                 consent_policy = $9,
                 active = $10,
                 created_by_user_id = $11,
                 updated_at = $12
             WHERE id = $1"
        };
        let mut update_params = params!(
            application.id.clone(),
            application.organization_id.clone(),
            application.slug.clone(),
            application.name.clone(),
            application.description.clone(),
            application_type_str(&application.application_type),
            application_app_type_str(&application.app_type),
            if application.first_party {
                1_i64
            } else {
                0_i64
            },
            application_consent_policy_str(&application.consent_policy),
            if application.active { 1_i64 } else { 0_i64 },
            application.created_by_user_id.clone(),
            now
        );
        if slug_changed {
            update_params.push(hiqlite::Param::from(application.slug.clone()));
            update_params.push(hiqlite::Param::from(application.id.clone()));
        }
        let updated = self
            .runtime
            .execute(stmt, update_params)
            .await
            .context("update application")?;
        if updated == 0 {
            if self
                .find_application_by_id_with_consistency(
                    &application.id,
                    Some(ReadConsistency::Strong),
                )
                .await?
                .is_none()
            {
                return Err(anyhow!("application not found: {}", application.id));
            }
            if slug_changed {
                return Err(anyhow!(
                    "application slug already exists: {}",
                    application.slug
                ));
            }
        }
        Ok(())
    }

    pub async fn upsert_application(&self, application: &Application) -> Result<()> {
        self.validate_application_references(application).await?;
        let now = now_string();
        self.runtime
            .execute(
                "INSERT INTO applications (
                    id, organization_id, slug, name, description, application_type, app_type,
                    first_party, consent_policy, active, created_by_user_id, created_at, updated_at
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
                 ON CONFLICT(id) DO UPDATE SET
                    organization_id = excluded.organization_id,
                    slug = excluded.slug,
                    name = excluded.name,
                    description = excluded.description,
                    application_type = excluded.application_type,
                    app_type = excluded.app_type,
                    first_party = excluded.first_party,
                    consent_policy = excluded.consent_policy,
                    active = excluded.active,
                    created_by_user_id = excluded.created_by_user_id,
                    updated_at = excluded.updated_at",
                params!(
                    application.id.clone(),
                    application.organization_id.clone(),
                    application.slug.clone(),
                    application.name.clone(),
                    application.description.clone(),
                    application_type_str(&application.application_type),
                    application_app_type_str(&application.app_type),
                    if application.first_party {
                        1_i64
                    } else {
                        0_i64
                    },
                    application_consent_policy_str(&application.consent_policy),
                    if application.active { 1_i64 } else { 0_i64 },
                    application.created_by_user_id.clone(),
                    now.clone(),
                    now
                ),
            )
            .await
            .context("upsert application")?;
        Ok(())
    }

    pub async fn list_applications_with_consistency(
        &self,
        organization_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Vec<Application>> {
        let rows: Vec<ApplicationRow> = self
            .query_rows_with(
                self.validation_read_consistency_with_override(consistency),
                "SELECT id, organization_id, slug, name, description, application_type, app_type,
                        first_party, consent_policy, active, created_by_user_id
                 FROM applications
                 WHERE organization_id = $1
                 ORDER BY name ASC, slug ASC, id ASC",
                params!(organization_id.to_owned()),
                "applications by organization",
            )
            .await?;
        rows.into_iter().map(TryInto::try_into).collect()
    }

    pub async fn find_application_by_id_with_consistency(
        &self,
        application_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Option<Application>> {
        let rows: Vec<ApplicationRow> = self
            .query_rows_with(
                self.validation_read_consistency_with_override(consistency),
                "SELECT id, organization_id, slug, name, description, application_type, app_type,
                        first_party, consent_policy, active, created_by_user_id
                 FROM applications
                 WHERE id = $1
                 LIMIT 1",
                params!(application_id.to_owned()),
                "application by id optional",
            )
            .await?;
        rows.into_iter().next().map(TryInto::try_into).transpose()
    }

    pub async fn get_application_with_consistency(
        &self,
        application_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Application> {
        let application = self
            .find_application_by_id_with_consistency(application_id, consistency)
            .await?
            .ok_or_else(|| anyhow!("application not found: {application_id}"))?;
        Ok(application)
    }

    pub async fn find_application_by_slug_with_consistency(
        &self,
        slug: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Option<Application>> {
        let rows: Vec<ApplicationRow> = self
            .query_rows_with(
                self.validation_read_consistency_with_override(consistency),
                "SELECT id, organization_id, slug, name, description, application_type, app_type,
                        first_party, consent_policy, active, created_by_user_id
                 FROM applications
                 WHERE slug = $1
                 LIMIT 1",
                params!(slug.to_owned()),
                "application by slug optional",
            )
            .await?;
        rows.into_iter().next().map(TryInto::try_into).transpose()
    }

    pub async fn upsert_application_action(
        &self,
        application_action: &ApplicationAction,
    ) -> Result<()> {
        if !self
            .exists_row_with(
                ReadConsistency::Strong,
                "SELECT COUNT(*) as count FROM applications WHERE id = $1",
                params!(application_action.application_id.clone()),
                "application existence for action",
            )
            .await?
        {
            return Err(anyhow!(
                "application {} does not exist",
                application_action.application_id
            ));
        }
        let grantable_count = self
            .query_count(
                ReadConsistency::Strong,
                "SELECT COUNT(*) as count
                 FROM actions
                 WHERE id = $1
                   AND grantable_to_applications = 1",
                params!(application_action.action_id.clone()),
                "grantable application action",
            )
            .await?;
        if grantable_count == 0 {
            return Err(anyhow!(
                "action {} is not grantable to applications",
                application_action.action_id
            ));
        }
        self.runtime
            .execute(
                "INSERT INTO application_actions (application_id, action_id)
                 VALUES ($1, $2)
                 ON CONFLICT(application_id, action_id) DO NOTHING",
                params!(
                    application_action.application_id.clone(),
                    application_action.action_id.clone()
                ),
            )
            .await
            .context("upsert application action")?;
        Ok(())
    }

    pub async fn replace_application_actions(
        &self,
        application_id: &str,
        action_ids: &[String],
    ) -> Result<Vec<Action>> {
        if !self
            .exists_row_with(
                ReadConsistency::Strong,
                "SELECT COUNT(*) as count FROM applications WHERE id = $1",
                params!(application_id.to_owned()),
                "application existence for action replace",
            )
            .await?
        {
            return Err(anyhow!("application {application_id} does not exist"));
        }
        let mut deduped = Vec::new();
        let mut seen = HashSet::new();
        for action_id in action_ids {
            let trimmed = action_id.trim();
            if trimmed.is_empty() {
                return Err(anyhow!("application action ids must not be empty"));
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
                         WHERE grantable_to_applications = 1
                           AND id IN ({placeholders})"
                    ),
                    deduped
                        .iter()
                        .cloned()
                        .map(hiqlite::Param::from)
                        .collect::<Vec<_>>(),
                    "grantable application action replace set",
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
                return Err(anyhow!("action {missing} is not grantable to applications"));
            }
        }

        let mut queries = Vec::with_capacity(1 + deduped.len());
        queries.push((
            Cow::Borrowed("DELETE FROM application_actions WHERE application_id = $1"),
            params!(application_id.to_owned()),
        ));
        for action_id in deduped {
            queries.push((
                Cow::Borrowed(
                    "INSERT INTO application_actions (application_id, action_id)
                     VALUES ($1, $2)
                     ON CONFLICT(application_id, action_id) DO NOTHING",
                ),
                params!(application_id.to_owned(), action_id.clone()),
            ));
        }

        self.runtime
            .transaction(queries)
            .await
            .context("replace application actions transaction")?;
        self.list_application_actions_with_consistency(
            application_id,
            Some(ReadConsistency::Strong),
        )
        .await
    }

    pub async fn list_application_actions_with_consistency(
        &self,
        application_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Vec<Action>> {
        let rows: Vec<ActionRow> = self
            .query_rows_with(
                self.validation_read_consistency_with_override(consistency),
                "SELECT a.id, a.provider_id, a.code, a.name, a.description, a.risk_level,
                        a.visibility_granting, a.grantable_to_applications, a.consent_label,
                        a.consent_description
                 FROM application_actions ca
                 JOIN actions a ON a.id = ca.action_id
                 WHERE ca.application_id = $1
                 ORDER BY a.code ASC",
                params!(application_id.to_owned()),
                "application actions",
            )
            .await?;
        rows.into_iter().map(TryInto::try_into).collect()
    }

    pub async fn upsert_application_grant(
        &self,
        application_grant: &ApplicationGrant,
    ) -> Result<String> {
        self.validate_application_grant_refs(application_grant)
            .await?;
        if application_grant.status == ApplicationGrantStatus::Active {
            let existing: Vec<ApplicationGrantLookupRow> = self
                .query_consistent_rows(
                    "SELECT id, created_at
                     FROM application_grants
                     WHERE user_id = $1
                       AND application_id = $2
                       AND organization_id = $3
                       AND status = 'active'
                     LIMIT 1",
                    params!(
                        application_grant.user_id.clone(),
                        application_grant.application_id.clone(),
                        application_grant.organization_id.clone()
                    ),
                    "active application grant lookup",
                )
                .await?;
            if let Some(existing) = existing.into_iter().next() {
                self.runtime
                    .transaction([
                        (
                            Cow::Borrowed(
                                "UPDATE application_grants
                                 SET user_id = $1,
                                     application_id = $2,
                                     organization_id = $3,
                                     status = $4,
                                     consented_at = $5,
                                     revoked_at = $6,
                                     revoked_by_user_id = $7,
                                     updated_at = $8
                                 WHERE id = $9",
                            ),
                            params!(
                                application_grant.user_id.clone(),
                                application_grant.application_id.clone(),
                                application_grant.organization_id.clone(),
                                application_grant_status_str(&application_grant.status),
                                application_grant.consented_at.clone(),
                                application_grant.revoked_at.clone(),
                                application_grant.revoked_by_user_id.clone(),
                                application_grant.updated_at.clone(),
                                existing.id.clone()
                            ),
                        ),
                        (
                            Cow::Borrowed(
                                "DELETE FROM application_grant_actions WHERE application_grant_id = $1",
                            ),
                            params!(existing.id.clone()),
                        ),
                    ])
                    .await
                    .context("reset existing active application grant transaction")?;
                return Ok(existing.id);
            }
        }

        self.runtime
            .execute(
                "INSERT INTO application_grants (
                    id, user_id, application_id, organization_id, status, consented_at,
                    revoked_at, revoked_by_user_id, created_at, updated_at
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                 ON CONFLICT(id) DO UPDATE SET
                    user_id = excluded.user_id,
                    application_id = excluded.application_id,
                    organization_id = excluded.organization_id,
                    status = excluded.status,
                    consented_at = excluded.consented_at,
                    revoked_at = excluded.revoked_at,
                    revoked_by_user_id = excluded.revoked_by_user_id,
                    created_at = excluded.created_at,
                    updated_at = excluded.updated_at",
                params!(
                    application_grant.id.clone(),
                    application_grant.user_id.clone(),
                    application_grant.application_id.clone(),
                    application_grant.organization_id.clone(),
                    application_grant_status_str(&application_grant.status),
                    application_grant.consented_at.clone(),
                    application_grant.revoked_at.clone(),
                    application_grant.revoked_by_user_id.clone(),
                    application_grant.created_at.clone(),
                    application_grant.updated_at.clone()
                ),
            )
            .await
            .context("upsert application grant")?;
        Ok(application_grant.id.clone())
    }

    pub async fn insert_application_grant(
        &self,
        application_grant: &ApplicationGrant,
    ) -> Result<String> {
        self.validate_application_grant_refs(application_grant)
            .await?;
        self.runtime
            .execute(
                "INSERT INTO application_grants (
                    id, user_id, application_id, organization_id, status, consented_at,
                    revoked_at, revoked_by_user_id, created_at, updated_at
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
                params!(
                    application_grant.id.clone(),
                    application_grant.user_id.clone(),
                    application_grant.application_id.clone(),
                    application_grant.organization_id.clone(),
                    application_grant_status_str(&application_grant.status),
                    application_grant.consented_at.clone(),
                    application_grant.revoked_at.clone(),
                    application_grant.revoked_by_user_id.clone(),
                    application_grant.created_at.clone(),
                    application_grant.updated_at.clone()
                ),
            )
            .await
            .context("insert application grant")?;
        Ok(application_grant.id.clone())
    }

    async fn validate_application_grant_refs(
        &self,
        application_grant: &ApplicationGrant,
    ) -> Result<()> {
        if !self
            .exists_row_with(
                ReadConsistency::Strong,
                "SELECT COUNT(*) as count FROM users WHERE id = $1",
                params!(application_grant.user_id.clone()),
                "application grant user existence",
            )
            .await?
        {
            return Err(anyhow!(
                "application grant user {} does not exist",
                application_grant.user_id
            ));
        }
        if !self
            .exists_row_with(
                ReadConsistency::Strong,
                "SELECT COUNT(*) as count FROM applications WHERE id = $1",
                params!(application_grant.application_id.clone()),
                "application grant application existence",
            )
            .await?
        {
            return Err(anyhow!(
                "application grant application {} does not exist",
                application_grant.application_id
            ));
        }
        if !self
            .exists_row_with(
                ReadConsistency::Strong,
                "SELECT COUNT(*) as count FROM organizations WHERE id = $1",
                params!(application_grant.organization_id.clone()),
                "application grant organization existence",
            )
            .await?
        {
            return Err(anyhow!(
                "application grant organization {} does not exist",
                application_grant.organization_id
            ));
        }
        Ok(())
    }

    pub async fn list_application_grants_for_organization_with_consistency(
        &self,
        organization_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Vec<ApplicationGrant>> {
        let rows: Vec<ApplicationGrantRow> = self
            .query_rows_with(
                self.validation_read_consistency_with_override(consistency),
                "SELECT id, user_id, application_id, organization_id, status, consented_at,
                        revoked_at, revoked_by_user_id, created_at, updated_at
                 FROM application_grants
                 WHERE organization_id = $1
                 ORDER BY updated_at DESC, id ASC",
                params!(organization_id.to_owned()),
                "application grants by organization",
            )
            .await?;
        rows.into_iter().map(TryInto::try_into).collect()
    }

    pub async fn get_application_grant_with_consistency(
        &self,
        grant_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<ApplicationGrant> {
        let rows: Vec<ApplicationGrantRow> = self
            .query_rows_with(
                self.validation_read_consistency_with_override(consistency),
                "SELECT id, user_id, application_id, organization_id, status, consented_at,
                        revoked_at, revoked_by_user_id, created_at, updated_at
                 FROM application_grants
                 WHERE id = $1
                 LIMIT 1",
                params!(grant_id.to_owned()),
                "application grant by id",
            )
            .await?;
        rows.into_iter()
            .next()
            .ok_or_else(|| anyhow!("application grant not found: {grant_id}"))?
            .try_into()
    }

    pub async fn replace_application_grant_actions(
        &self,
        grant_id: &str,
        action_ids: &[String],
    ) -> Result<Vec<Action>> {
        let grant = self
            .get_application_grant_with_consistency(grant_id, Some(ReadConsistency::Strong))
            .await?;
        let deduped = self
            .validate_application_grant_action_subset(&grant.application_id, action_ids)
            .await?;
        self.replace_application_grant_actions_checked(grant_id, &deduped)
            .await
    }

    pub async fn validate_application_grant_action_subset(
        &self,
        application_id: &str,
        action_ids: &[String],
    ) -> Result<Vec<String>> {
        let mut deduped = Vec::new();
        let mut seen = HashSet::new();
        for action_id in action_ids {
            let trimmed = action_id.trim();
            if trimmed.is_empty() {
                return Err(anyhow!("application grant action ids must not be empty"));
            }
            if seen.insert(trimmed.to_owned()) {
                deduped.push(trimmed.to_owned());
            }
        }
        if !deduped.is_empty() {
            let placeholders = (2..=(deduped.len() + 1))
                .map(|index| format!("${index}"))
                .collect::<Vec<_>>()
                .join(", ");
            let allowed_rows: Vec<ActionLookupRow> = self
                .query_rows_with(
                    ReadConsistency::Strong,
                    format!(
                        "SELECT ca.action_id AS id
                         FROM application_actions ca
                         WHERE ca.application_id = $1
                           AND ca.action_id IN ({placeholders})"
                    ),
                    std::iter::once(hiqlite::Param::from(application_id.to_owned()))
                        .chain(deduped.iter().cloned().map(hiqlite::Param::from))
                        .collect::<Vec<_>>(),
                    "application grant action replace subset",
                )
                .await?;
            let allowed_ids = allowed_rows
                .into_iter()
                .map(|row| row.id)
                .collect::<HashSet<_>>();
            if let Some(missing) = deduped
                .iter()
                .find(|action_id| !allowed_ids.contains(*action_id))
            {
                return Err(anyhow!(
                    "application grant action {missing} must be included in application_actions for the parent application"
                ));
            }
        }
        Ok(deduped)
    }

    async fn replace_application_grant_actions_checked(
        &self,
        grant_id: &str,
        deduped: &[String],
    ) -> Result<Vec<Action>> {
        let mut queries = Vec::with_capacity(1 + deduped.len());
        queries.push((
            Cow::Borrowed("DELETE FROM application_grant_actions WHERE application_grant_id = $1"),
            params!(grant_id.to_owned()),
        ));
        for action_id in deduped {
            queries.push((
                Cow::Borrowed(
                    "INSERT INTO application_grant_actions (application_grant_id, action_id)
                     VALUES ($1, $2)
                     ON CONFLICT(application_grant_id, action_id) DO NOTHING",
                ),
                params!(grant_id.to_owned(), action_id.clone()),
            ));
        }
        self.runtime
            .transaction(queries)
            .await
            .context("replace application grant actions transaction")?;
        self.list_application_grant_actions_with_consistency(
            grant_id,
            Some(ReadConsistency::Strong),
        )
        .await
    }

    pub async fn list_application_grant_actions_with_consistency(
        &self,
        grant_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Vec<Action>> {
        let rows: Vec<ActionRow> = self
            .query_rows_with(
                self.validation_read_consistency_with_override(consistency),
                "SELECT a.id, a.provider_id, a.code, a.name, a.description, a.risk_level,
                        a.visibility_granting, a.grantable_to_applications, a.consent_label,
                        a.consent_description
                 FROM application_grant_actions cga
                 JOIN actions a ON a.id = cga.action_id
                 WHERE cga.application_grant_id = $1
                 ORDER BY a.code ASC",
                params!(grant_id.to_owned()),
                "application grant actions",
            )
            .await?;
        rows.into_iter().map(TryInto::try_into).collect()
    }

    pub async fn revoke_application_grant(
        &self,
        grant_id: &str,
        revoked_by_user_id: &str,
    ) -> Result<ApplicationGrant> {
        let now = now_string();
        let updated = self
            .runtime
            .execute(
                "UPDATE application_grants
                 SET status = 'revoked',
                     revoked_at = $1,
                     revoked_by_user_id = $2,
                     updated_at = $3
                 WHERE id = $4",
                params!(
                    now.clone(),
                    revoked_by_user_id.to_owned(),
                    now,
                    grant_id.to_owned()
                ),
            )
            .await
            .context("revoke application grant")?;
        if updated == 0 {
            return Err(anyhow!("application grant not found: {grant_id}"));
        }
        self.get_application_grant_with_consistency(grant_id, Some(ReadConsistency::Strong))
            .await
    }

    pub async fn upsert_application_grant_action(
        &self,
        application_grant_action: &ApplicationGrantAction,
    ) -> Result<()> {
        let allowed_count = self
            .query_count(
                self.runtime.default_read_consistency(),
                "SELECT COUNT(*) as count
                 FROM application_grants cg
                 JOIN application_actions ca
                   ON ca.application_id = cg.application_id
                 WHERE cg.id = $1
                   AND ca.action_id = $2",
                params!(
                    application_grant_action.application_grant_id.clone(),
                    application_grant_action.action_id.clone()
                ),
                "application grant action subset",
            )
            .await?;
        if allowed_count == 0 {
            return Err(anyhow!(
                "application grant action must be included in application_actions for the parent application"
            ));
        }
        self.runtime
            .execute(
                "INSERT INTO application_grant_actions (application_grant_id, action_id)
                 VALUES ($1, $2)
                 ON CONFLICT(application_grant_id, action_id) DO NOTHING",
                params!(
                    application_grant_action.application_grant_id.clone(),
                    application_grant_action.action_id.clone()
                ),
            )
            .await
            .context("upsert application grant action")?;
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
        Ok(())
    }

    pub async fn list_agreements_for_organization_with_consistency(
        &self,
        organization_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Vec<Agreement>> {
        let rows: Vec<AgreementRow> = self
            .query_rows_with(
                self.validation_read_consistency_with_override(consistency),
                "SELECT id, agreement_type, grantor_organization_id, grantee_organization_id,
                        status, title, reference_no, starts_at, ends_at
                 FROM agreements
                 WHERE grantor_organization_id = $1 OR grantee_organization_id = $1
                 ORDER BY title ASC, id ASC",
                params!(organization_id.to_owned()),
                "agreements by organization",
            )
            .await?;
        rows.into_iter().map(TryInto::try_into).collect()
    }

    pub async fn get_agreement_with_consistency(
        &self,
        agreement_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Agreement> {
        self.find_agreement_by_id_with_consistency(agreement_id, consistency)
            .await?
            .ok_or_else(|| anyhow!("agreement not found: {agreement_id}"))
    }

    pub async fn find_agreement_by_id_with_consistency(
        &self,
        agreement_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Option<Agreement>> {
        let rows: Vec<AgreementRow> = self
            .query_rows_with(
                self.validation_read_consistency_with_override(consistency),
                "SELECT id, agreement_type, grantor_organization_id, grantee_organization_id,
                        status, title, reference_no, starts_at, ends_at
                 FROM agreements
                 WHERE id = $1
                 LIMIT 1",
                params!(agreement_id.to_owned()),
                "agreement by id",
            )
            .await?;
        rows.into_iter().next().map(TryInto::try_into).transpose()
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

    pub async fn has_user_organization_action_with_consistency(
        &self,
        user_id: &str,
        organization_id: &str,
        target_organization_id: &str,
        action_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<bool> {
        self.has_user_action(UserActionCheck {
            user_id,
            organization_id,
            provider_id: "grant",
            resource_type: "organization",
            resource_id: target_organization_id,
            action_id,
            consistency,
        })
        .await
    }

    pub async fn has_user_actions(&self, checks: &[UserActionCheck<'_>]) -> Result<Vec<bool>> {
        let mut results = vec![false; checks.len()];
        for (chunk_number, chunk) in checks.chunks(MAX_AUTHZ_CHECKS_PER_QUERY).enumerate() {
            let Some(first) = chunk.first() else {
                continue;
            };
            let consistency =
                self.default_read_consistency_with_override(first.consistency.clone());
            let validation_time = Utc::now().to_rfc3339();
            let mut query_params = Vec::with_capacity(chunk.len() * 8);
            let values = chunk
                .iter()
                .enumerate()
                .map(|(chunk_index, check)| {
                    let idx = chunk_number * MAX_AUTHZ_CHECKS_PER_QUERY + chunk_index;
                    let base = query_params.len() + 1;
                    query_params.push(hiqlite::Param::from(idx as i64));
                    query_params.push(hiqlite::Param::from(check.user_id.to_owned()));
                    query_params.push(hiqlite::Param::from(check.organization_id.to_owned()));
                    query_params.push(hiqlite::Param::from(check.provider_id.to_owned()));
                    query_params.push(hiqlite::Param::from(check.resource_type.to_owned()));
                    query_params.push(hiqlite::Param::from(check.resource_id.to_owned()));
                    query_params.push(hiqlite::Param::from(check.action_id.to_owned()));
                    query_params.push(hiqlite::Param::from(validation_time.clone()));
                    format!(
                        "(${base}, ${}, ${}, ${}, ${}, ${}, ${}, ${})",
                        base + 1,
                        base + 2,
                        base + 3,
                        base + 4,
                        base + 5,
                        base + 6,
                        base + 7
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            let rows: Vec<UserActionDecisionRow> = self
                .query_rows_with(
                    consistency,
                    format!(
                        r#"
                        WITH requested (
                            idx, user_id, organization_id, provider_id, resource_type,
                            resource_id, action_id, validation_time
                        ) AS (
                            VALUES {values}
                        ),
                        allowed AS (
                            SELECT DISTINCT req.idx
                            FROM requested req
                            JOIN role_bindings rb
                              ON rb.subject_type = 'organization'
                             AND rb.subject_id = req.organization_id
                             AND rb.subject_organization_id = req.organization_id
                             AND rb.provider_id = req.provider_id
                             AND rb.resource_type = req.resource_type
                             AND rb.resource_id = req.resource_id
                             AND rb.state = 'active'
                            JOIN role_actions ra
                              ON ra.role_id = rb.role_id
                             AND ra.action_id = req.action_id
                            JOIN actions act
                              ON act.id = ra.action_id
                             AND act.provider_id = rb.provider_id
                            JOIN organization_memberships om
                              ON om.organization_id = rb.subject_id
                             AND om.user_id = req.user_id
                             AND om.status = 'active'
                            LEFT JOIN agreements a ON a.id = rb.agreement_id
                            WHERE rb.agreement_id IS NULL OR (
                                a.status = 'active'
                                AND (a.starts_at IS NULL OR a.starts_at <= req.validation_time)
                                AND (a.ends_at IS NULL OR a.ends_at >= req.validation_time)
                            )

                            UNION

                            SELECT DISTINCT req.idx
                            FROM requested req
                            JOIN role_bindings rb
                              ON rb.subject_type = 'user'
                             AND rb.subject_id = req.user_id
                             AND rb.subject_organization_id = req.organization_id
                             AND rb.provider_id = req.provider_id
                             AND rb.resource_type = req.resource_type
                             AND rb.resource_id = req.resource_id
                             AND rb.state = 'active'
                            JOIN role_actions ra
                              ON ra.role_id = rb.role_id
                             AND ra.action_id = req.action_id
                            JOIN actions act
                              ON act.id = ra.action_id
                             AND act.provider_id = rb.provider_id
                            JOIN organization_memberships om
                              ON om.organization_id = rb.subject_organization_id
                             AND om.user_id = rb.subject_id
                             AND om.status = 'active'

                            UNION

                            SELECT DISTINCT req.idx
                            FROM requested req
                            JOIN role_bindings rb
                              ON rb.subject_type = 'group'
                             AND rb.subject_organization_id = req.organization_id
                             AND rb.provider_id = req.provider_id
                             AND rb.resource_type = req.resource_type
                             AND rb.resource_id = req.resource_id
                             AND rb.state = 'active'
                            JOIN role_actions ra
                              ON ra.role_id = rb.role_id
                             AND ra.action_id = req.action_id
                            JOIN actions act
                              ON act.id = ra.action_id
                             AND act.provider_id = rb.provider_id
                            JOIN group_memberships gm ON gm.group_id = rb.subject_id
                            JOIN organization_memberships om
                              ON om.id = gm.organization_membership_id
                             AND om.user_id = req.user_id
                             AND om.organization_id = req.organization_id
                             AND om.status = 'active'
                        )
                        SELECT req.idx,
                               CASE WHEN allowed.idx IS NULL THEN 0 ELSE 1 END AS allowed
                        FROM requested req
                        LEFT JOIN allowed ON allowed.idx = req.idx
                        ORDER BY req.idx ASC
                        "#
                    ),
                    query_params,
                    "bulk user organization actions",
                )
                .await?;
            for row in rows {
                if let Some(result) = results.get_mut(row.idx as usize) {
                    *result = row.allowed != 0;
                }
            }
        }
        Ok(results)
    }

    pub async fn has_user_action(&self, check: UserActionCheck<'_>) -> Result<bool> {
        Ok(self
            .has_user_actions(&[check])
            .await?
            .into_iter()
            .next()
            .unwrap_or(false))
    }

    pub async fn save_role_binding(&self, binding: &RoleBinding) -> Result<()> {
        validate_role_binding(binding)?;
        self.validate_role_binding_refs(binding).await?;
        self.runtime
            .execute(
                "INSERT INTO role_bindings (
	                    id, subject_type, subject_id, subject_organization_id, provider_id, resource_type,
	                    resource_id, role_id, agreement_id, state, created_by_user_id, created_at, updated_at
	                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
	                 ON CONFLICT(id) DO UPDATE SET
	                    subject_type = excluded.subject_type,
	                    subject_id = excluded.subject_id,
	                    subject_organization_id = excluded.subject_organization_id,
	                    provider_id = excluded.provider_id,
	                    resource_type = excluded.resource_type,
	                    resource_id = excluded.resource_id,
	                    role_id = excluded.role_id,
	                    agreement_id = excluded.agreement_id,
	                    state = excluded.state,
	                    updated_at = excluded.updated_at
	                 ON CONFLICT(subject_type, subject_id, subject_organization_id, provider_id, resource_type, resource_id, role_id)
	                 DO UPDATE SET
	                    agreement_id = excluded.agreement_id,
	                    state = excluded.state,
	                    updated_at = excluded.updated_at",
                params!(
                    binding.id.clone(),
                    role_binding_subject_type_str(&binding.subject_type),
                    binding.subject_id.clone(),
                    binding.subject_organization_id.clone(),
                    binding.provider_id.clone(),
                    binding.resource_type.clone(),
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
        Ok(())
    }

    pub async fn insert_role_binding(&self, binding: &RoleBinding) -> Result<()> {
        validate_role_binding(binding)?;
        self.validate_role_binding_refs(binding).await?;
        self.runtime
            .execute(
                "INSERT INTO role_bindings (
                    id, subject_type, subject_id, subject_organization_id, provider_id, resource_type,
                    resource_id, role_id, agreement_id, state, created_by_user_id, created_at, updated_at
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
                params!(
                    binding.id.clone(),
                    role_binding_subject_type_str(&binding.subject_type),
                    binding.subject_id.clone(),
                    binding.subject_organization_id.clone(),
                    binding.provider_id.clone(),
                    binding.resource_type.clone(),
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
            .context("insert role binding")?;
        Ok(())
    }

    pub async fn list_role_bindings_for_subject(
        &self,
        subject_type: RoleBindingSubjectType,
        subject_id: &str,
        subject_organization_id: Option<&str>,
    ) -> Result<Vec<RoleBinding>> {
        let mut sql = String::from(
            "SELECT id, subject_type, subject_id, subject_organization_id, provider_id, resource_type,
                    resource_id, role_id, agreement_id, state, created_by_user_id, created_at, updated_at
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
        provider_id: &str,
        resource_type: &str,
        resource_id: &str,
    ) -> Result<Vec<RoleBinding>> {
        let rows: Vec<RoleBindingRow> = self
            .query_default_rows(
                "SELECT id, subject_type, subject_id, subject_organization_id, provider_id, resource_type,
                        resource_id, role_id, agreement_id, state, created_by_user_id, created_at, updated_at
                 FROM role_bindings
                 WHERE provider_id = $1 AND resource_type = $2 AND resource_id = $3
                 ORDER BY id ASC",
                params!(
                    provider_id.to_owned(),
                    resource_type.to_owned(),
                    resource_id.to_owned()
                ),
                "role bindings for resource",
            )
            .await?;
        rows.into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>>>()
    }

    pub async fn list_role_bindings_for_organization_with_consistency(
        &self,
        organization_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Vec<RoleBinding>> {
        let rows: Vec<RoleBindingRow> = self
            .query_rows_with(
                self.validation_read_consistency_with_override(consistency),
                "SELECT id, subject_type, subject_id, subject_organization_id, provider_id, resource_type,
                        resource_id, role_id, agreement_id, state, created_by_user_id, created_at, updated_at
                 FROM role_bindings
                 WHERE subject_organization_id = $1
                    OR (provider_id = 'grant' AND resource_type = 'organization' AND resource_id = $1)
                 ORDER BY id ASC",
                params!(organization_id.to_owned()),
                "role bindings by organization",
            )
            .await?;
        rows.into_iter().map(TryInto::try_into).collect()
    }

    pub async fn get_role_binding_with_consistency(
        &self,
        binding_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<RoleBinding> {
        let rows: Vec<RoleBindingRow> = self
            .query_rows_with(
                self.validation_read_consistency_with_override(consistency),
                "SELECT id, subject_type, subject_id, subject_organization_id, provider_id, resource_type,
                        resource_id, role_id, agreement_id, state, created_by_user_id, created_at, updated_at
                 FROM role_bindings
                 WHERE id = $1
                 LIMIT 1",
                params!(binding_id.to_owned()),
                "role binding by id",
            )
            .await?;
        rows.into_iter()
            .next()
            .ok_or_else(|| anyhow!("role binding not found: {binding_id}"))?
            .try_into()
    }

    pub async fn delete_role_binding(&self, binding_id: &str) -> Result<()> {
        let deleted = self
            .runtime
            .execute(
                "DELETE FROM role_bindings WHERE id = $1",
                params!(binding_id.to_owned()),
            )
            .await
            .context("delete role binding")?;
        if deleted == 0 {
            return Err(anyhow!("role binding not found: {binding_id}"));
        }
        Ok(())
    }

    async fn validate_role_binding_refs(&self, binding: &RoleBinding) -> Result<()> {
        let consistency = self.validation_read_consistency();
        let validation_time = Utc::now().to_rfc3339();

        if !self
            .exists_row_with(
                consistency.clone(),
                "SELECT COUNT(*) as count FROM service_providers WHERE id = $1",
                params!(binding.provider_id.clone()),
                "binding provider existence",
            )
            .await?
        {
            return Err(anyhow!(
                "service provider {} does not exist",
                binding.provider_id
            ));
        }

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
        match role_org.as_deref() {
            Some(role_org) if role_org != binding.subject_organization_id => {
                return Err(anyhow!(
                    "role {} belongs to organization {}, not {}",
                    binding.role_id,
                    role_org,
                    binding.subject_organization_id
                ));
            }
            _ => {}
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
            if binding.provider_id == "grant"
                && binding.resource_type == "organization"
                && binding.resource_id != binding.subject_organization_id
            {
                let agreement_matches = self
                    .query_count(
                        consistency.clone(),
                        "SELECT COUNT(*) as count
                         FROM agreements
                         WHERE id = $1
                           AND grantor_organization_id = $2
                           AND grantee_organization_id = $3",
                        params!(
                            agreement_id.clone(),
                            binding.resource_id.clone(),
                            binding.subject_organization_id.clone()
                        ),
                        "organization agreement grantor grantee match",
                    )
                    .await?;
                if agreement_matches == 0 {
                    return Err(anyhow!(
                        "agreement {} does not grant organization {} access to organization {}",
                        agreement_id,
                        binding.subject_organization_id,
                        binding.resource_id
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

        if binding.provider_id == "grant"
            && binding.resource_type == "organization"
            && !self
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

fn now_string() -> String {
    DateTime::<Utc>::from(std::time::SystemTime::now()).to_rfc3339()
}

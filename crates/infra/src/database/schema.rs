use super::core::now_string;
use super::*;

impl Database {
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
                    rauthy_client_id TEXT,
                    secret_rotated_at TEXT,
                    created_by_user_id TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS application_redirect_uris (
                    application_id TEXT NOT NULL,
                    redirect_uri TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    PRIMARY KEY (application_id, redirect_uri)
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
        self.add_column_if_missing("applications", "rauthy_client_id", "TEXT")
            .await?;
        self.add_column_if_missing("applications", "secret_rotated_at", "TEXT")
            .await?;
        self.runtime
            .batch(
                "UPDATE applications
                 SET rauthy_client_id = id
                 WHERE rauthy_client_id IS NULL OR TRIM(rauthy_client_id) = '';"
                    .to_owned(),
            )
            .await
            .context("backfill application Rauthy client ids")?;
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
}

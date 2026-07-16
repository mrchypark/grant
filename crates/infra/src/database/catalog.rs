use super::core::now_string;
use super::*;

#[derive(Debug)]
struct FieldViewActionCapabilityRow {
    provider_id: String,
    action_id: String,
}

impl From<&mut Row<'_>> for FieldViewActionCapabilityRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            provider_id: row.get("provider_id"),
            action_id: row.get("action_id"),
        }
    }
}

impl Database {
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
                        Cow::Borrowed("DELETE FROM role_action_field_views WHERE action_id = $1"),
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

    pub async fn is_field_view_action_with_consistency(
        &self,
        provider_id: &str,
        action_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<bool> {
        self.exists_row_with(
            self.validation_read_consistency_with_override(consistency),
            "SELECT COUNT(*) AS count
             FROM actions AS a
             JOIN service_providers AS sp ON sp.id = a.provider_id
             WHERE a.id = $1
               AND a.provider_id = $2
               AND a.visibility_granting = 1
               AND sp.active = 1",
            params!(action_id.to_owned(), provider_id.to_owned()),
            "field view action capability",
        )
        .await
    }

    pub async fn field_view_actions_with_consistency(
        &self,
        actions: &HashSet<(String, String)>,
        consistency: Option<ReadConsistency>,
    ) -> Result<HashSet<(String, String)>> {
        let consistency = self.validation_read_consistency_with_override(consistency);
        let actions = actions.iter().collect::<Vec<_>>();
        let mut supported = HashSet::with_capacity(actions.len());
        // Each tuple binds two parameters. Keep comfortably below SQLite's conservative
        // 999-variable limit while allowing the public batch maximum.
        for chunk in actions.chunks(400) {
            let mut params = Vec::with_capacity(chunk.len() * 2);
            let mut values = Vec::with_capacity(chunk.len());
            for (index, (provider_id, action_id)) in chunk.iter().enumerate() {
                let base = index * 2 + 1;
                values.push(format!("(${base}, ${})", base + 1));
                params.push(hiqlite::Param::from((*provider_id).clone()));
                params.push(hiqlite::Param::from((*action_id).clone()));
            }
            let rows: Vec<FieldViewActionCapabilityRow> = self
                .query_rows_with(
                    consistency.clone(),
                    format!(
                        "WITH requested(provider_id, action_id) AS (VALUES {})
                         SELECT a.provider_id, a.id AS action_id
                         FROM requested AS req
                         JOIN actions AS a
                           ON a.provider_id = req.provider_id
                          AND a.id = req.action_id
                          AND a.visibility_granting = 1
                         JOIN service_providers AS sp
                           ON sp.id = a.provider_id
                          AND sp.active = 1",
                        values.join(", ")
                    ),
                    params,
                    "field view action capabilities",
                )
                .await?;
            supported.extend(rows.into_iter().map(|row| (row.provider_id, row.action_id)));
        }
        Ok(supported)
    }
}

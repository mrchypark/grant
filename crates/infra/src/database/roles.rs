use super::core::now_string;
use super::*;

impl Database {
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
        let mut queries = Vec::with_capacity(2 + deduped.len());
        if deduped.is_empty() {
            queries.push((
                Cow::Borrowed("DELETE FROM role_action_field_views WHERE role_id = $1"),
                params!(role_id.to_owned()),
            ));
            queries.push((
                Cow::Borrowed("DELETE FROM role_actions WHERE role_id = $1"),
                params!(role_id.to_owned()),
            ));
        } else {
            let placeholders = (2..(2 + deduped.len()))
                .map(|index| format!("${index}"))
                .collect::<Vec<_>>()
                .join(", ");
            let delete_params = std::iter::once(hiqlite::Param::from(role_id.to_owned()))
                .chain(deduped.iter().cloned().map(hiqlite::Param::from))
                .collect::<Vec<_>>();
            queries.push((
                Cow::Owned(format!(
                    "DELETE FROM role_action_field_views
                     WHERE role_id = $1 AND action_id NOT IN ({placeholders})"
                )),
                delete_params.clone(),
            ));
            queries.push((
                Cow::Owned(format!(
                    "DELETE FROM role_actions
                     WHERE role_id = $1 AND action_id NOT IN ({placeholders})"
                )),
                delete_params,
            ));
        }
        for action_id in &deduped {
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
}

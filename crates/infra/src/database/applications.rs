use super::core::now_string;
use super::*;

impl Database {
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
}

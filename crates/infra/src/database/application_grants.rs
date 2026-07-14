use super::core::now_string;
use super::*;

impl Database {
    pub async fn has_application_consent_for_action(
        &self,
        user_id: &str,
        organization_id: &str,
        application_id: &str,
        action_id: &str,
    ) -> Result<bool> {
        let trusted = self
            .exists_row_with(
                ReadConsistency::Strong,
                "SELECT COUNT(*) AS count
                 FROM applications app
                 JOIN application_actions aa ON aa.application_id = app.id
                 WHERE app.id = $1
                   AND app.organization_id = $2
                   AND app.active = 1
                   AND app.provisioning_status = 'ready'
                   AND app.first_party = 1
                   AND app.consent_policy = 'auto_approve'
                   AND aa.action_id = $3",
                params!(
                    application_id.to_owned(),
                    organization_id.to_owned(),
                    action_id.to_owned()
                ),
                "trusted application action decision",
            )
            .await?;
        if trusted {
            return Ok(true);
        }
        self.exists_row_with(
            ReadConsistency::Strong,
            "SELECT COUNT(*) AS count
             FROM application_grants ag
             JOIN applications app
               ON app.id = ag.application_id
              AND app.organization_id = ag.organization_id
              AND app.active = 1
              AND app.provisioning_status = 'ready'
             JOIN application_grant_actions aga
               ON aga.application_grant_id = ag.id
             JOIN application_actions aa
               ON aa.application_id = ag.application_id
              AND aa.action_id = aga.action_id
             WHERE ag.user_id = $1
               AND ag.organization_id = $2
               AND ag.application_id = $3
               AND ag.status = 'active'
               AND aga.action_id = $4",
            params!(
                user_id.to_owned(),
                organization_id.to_owned(),
                application_id.to_owned(),
                action_id.to_owned()
            ),
            "explicit application consent action decision",
        )
        .await
    }

    pub async fn find_active_application_consent_with_consistency(
        &self,
        user_id: &str,
        application_id: &str,
        organization_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Option<ApplicationGrant>> {
        let rows: Vec<ApplicationGrantRow> = self
            .query_rows_with(
                self.validation_read_consistency_with_override(consistency),
                "SELECT id, user_id, application_id, organization_id, status, consented_at,
                        revoked_at, revoked_by_user_id, created_at, updated_at
                 FROM application_grants
                 WHERE user_id = $1
                   AND application_id = $2
                   AND organization_id = $3
                   AND status = 'active'
                 LIMIT 1",
                params!(
                    user_id.to_owned(),
                    application_id.to_owned(),
                    organization_id.to_owned()
                ),
                "active application consent",
            )
            .await?;
        rows.into_iter().next().map(TryInto::try_into).transpose()
    }

    pub async fn replace_active_application_consent(
        &self,
        application_grant: &ApplicationGrant,
        action_ids: &[String],
    ) -> Result<ApplicationGrant> {
        self.validate_application_grant_refs(application_grant)
            .await?;
        let deduped = self
            .validate_application_grant_action_subset(&application_grant.application_id, action_ids)
            .await?;
        let expected_action_count = deduped.len() as i64;
        let assertion_id = Uuid::new_v4().simple().to_string();
        let mut queries = Vec::with_capacity(9);
        queries.push((
            Cow::Borrowed(
                "INSERT INTO application_grants (
                    id, user_id, application_id, organization_id, status, consented_at,
                    revoked_at, revoked_by_user_id, created_at, updated_at
                 )
                 SELECT $1, $2, $3, $4, 'active', $5, NULL, NULL, $6, $7
                 FROM applications app
                 JOIN organization_memberships om
                   ON om.organization_id = app.organization_id
                  AND om.user_id = $2
                  AND om.status = 'active'
                 JOIN users u ON u.id = om.user_id AND u.active = 1
                 WHERE app.id = $3
                   AND app.organization_id = $4
                   AND app.active = 1
                   AND app.first_party = 0
                   AND app.consent_policy = 'explicit'
                 ON CONFLICT(user_id, application_id, organization_id)
                 WHERE status = 'active'
                 DO UPDATE SET
                    status = 'active',
                    consented_at = excluded.consented_at,
                    revoked_at = NULL,
                    revoked_by_user_id = NULL,
                    updated_at = excluded.updated_at",
            ),
            params!(
                application_grant.id.clone(),
                application_grant.user_id.clone(),
                application_grant.application_id.clone(),
                application_grant.organization_id.clone(),
                application_grant.consented_at.clone(),
                application_grant.created_at.clone(),
                application_grant.updated_at.clone()
            ),
        ));
        queries.push((
            Cow::Borrowed(
                "DELETE FROM application_grant_actions
                 WHERE application_grant_id = (
                     SELECT id FROM application_grants
                     WHERE user_id = $1
                       AND application_id = $2
                       AND organization_id = $3
                       AND status = 'active'
                 )",
            ),
            params!(
                application_grant.user_id.clone(),
                application_grant.application_id.clone(),
                application_grant.organization_id.clone()
            ),
        ));

        for action_ids in deduped.chunks(250) {
            let values = (1..=action_ids.len())
                .map(|index| format!("(${index})"))
                .collect::<Vec<_>>()
                .join(", ");
            let user_param = action_ids.len() + 1;
            let app_param = action_ids.len() + 2;
            let org_param = action_ids.len() + 3;
            let mut insert_params = action_ids
                .iter()
                .cloned()
                .map(hiqlite::Param::from)
                .collect::<Vec<_>>();
            insert_params.push(hiqlite::Param::from(application_grant.user_id.clone()));
            insert_params.push(hiqlite::Param::from(
                application_grant.application_id.clone(),
            ));
            insert_params.push(hiqlite::Param::from(
                application_grant.organization_id.clone(),
            ));
            queries.push((
                Cow::Owned(format!(
                    "WITH requested(action_id) AS (VALUES {values})
                     INSERT INTO application_grant_actions (application_grant_id, action_id)
                     SELECT ag.id, requested.action_id
                     FROM requested
                     JOIN application_grants ag
                       ON ag.user_id = ${user_param}
                      AND ag.application_id = ${app_param}
                      AND ag.organization_id = ${org_param}
                      AND ag.status = 'active'
                     JOIN application_actions aa
                       ON aa.application_id = ag.application_id
                      AND aa.action_id = requested.action_id"
                )),
                insert_params,
            ));
        }
        queries.push((
            Cow::Borrowed(
                "INSERT INTO application_consent_assertions (id, valid)
                 SELECT $1,
                        CASE WHEN EXISTS (
                            SELECT 1
                            FROM applications app
                            JOIN organization_memberships om
                              ON om.organization_id = app.organization_id
                             AND om.user_id = $2
                             AND om.status = 'active'
                            JOIN users u ON u.id = om.user_id AND u.active = 1
                            JOIN application_grants ag
                              ON ag.user_id = $2
                             AND ag.application_id = app.id
                             AND ag.organization_id = app.organization_id
                             AND ag.status = 'active'
                            WHERE app.id = $3
                              AND app.organization_id = $4
                              AND app.active = 1
                              AND app.first_party = 0
                              AND app.consent_policy = 'explicit'
                              AND (
                                  SELECT COUNT(*)
                                  FROM application_grant_actions aga
                                  WHERE aga.application_grant_id = ag.id
                              ) = $5
                        ) THEN 1 ELSE 0 END",
            ),
            params!(
                assertion_id.clone(),
                application_grant.user_id.clone(),
                application_grant.application_id.clone(),
                application_grant.organization_id.clone(),
                expected_action_count
            ),
        ));
        queries.push((
            Cow::Borrowed("DELETE FROM application_consent_assertions WHERE id = $1"),
            params!(assertion_id),
        ));
        let transaction_result = self.runtime.transaction(queries).await;
        if let Err(err) = transaction_result {
            if err.chain().any(|cause| {
                cause
                    .to_string()
                    .contains("application_consent_assertion_valid")
            }) {
                return Err(ApplicationConsentWriteError::PreconditionsChanged.into());
            }
            return Err(err).context("replace active application consent transaction");
        }
        self.find_active_application_consent_with_consistency(
            &application_grant.user_id,
            &application_grant.application_id,
            &application_grant.organization_id,
            Some(ReadConsistency::Strong),
        )
        .await?
        .ok_or_else(|| anyhow!("active application consent missing after replacement"))
    }

    pub async fn revoke_active_application_consent(
        &self,
        user_id: &str,
        application_id: &str,
        organization_id: &str,
    ) -> Result<()> {
        let now = now_string();
        self.runtime
            .execute(
                "UPDATE application_grants
                 SET status = 'revoked', revoked_at = $1, revoked_by_user_id = $2, updated_at = $3
                 WHERE user_id = $2
                   AND application_id = $4
                   AND organization_id = $5
                   AND status = 'active'",
                params!(
                    now.clone(),
                    user_id.to_owned(),
                    now,
                    application_id.to_owned(),
                    organization_id.to_owned()
                ),
            )
            .await
            .context("revoke active application consent")?;
        Ok(())
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
                 JOIN application_grants cg ON cg.id = cga.application_grant_id
                 JOIN application_actions ca
                   ON ca.application_id = cg.application_id
                  AND ca.action_id = cga.action_id
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
}

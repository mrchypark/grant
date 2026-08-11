use super::core::now_string;
use super::*;
use domain::ApplicationConsentPolicy;

struct ApplicationSecurityMutationStateRow {
    first_party: i64,
    consent_policy: String,
    lease_matches: i64,
}

impl From<&mut Row<'_>> for ApplicationSecurityMutationStateRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            first_party: row.get("first_party"),
            consent_policy: row.get("consent_policy"),
            lease_matches: row.get("lease_matches"),
        }
    }
}

impl Database {
    async fn validate_application_security_mutation_lease(
        &self,
        application_id: &str,
        lease_token: &str,
        valid_at: Option<&str>,
    ) -> Result<()> {
        let (query, query_params) = if let Some(valid_at) = valid_at {
            (
                "SELECT first_party, consent_policy,
                        CASE WHEN security_mutation_token = ?2
                                   AND security_mutation_expires_at > ?3
                             THEN 1 ELSE 0 END AS lease_matches
                 FROM applications WHERE id = ?1 LIMIT 1",
                params!(
                    application_id.to_owned(),
                    lease_token.to_owned(),
                    valid_at.to_owned()
                ),
            )
        } else {
            (
                "SELECT first_party, consent_policy,
                        CASE WHEN security_mutation_token = ?2
                             THEN 1 ELSE 0 END AS lease_matches
                 FROM applications WHERE id = ?1 LIMIT 1",
                params!(application_id.to_owned(), lease_token.to_owned()),
            )
        };
        let state = self
            .query_rows_with::<ApplicationSecurityMutationStateRow, _>(
                ReadConsistency::Strong,
                query,
                query_params,
                "application security mutation state",
            )
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| ApplicationSecurityMutationError::ApplicationNotFound {
                application_id: application_id.to_owned(),
            })?;
        if state.first_party == 1
            && state.consent_policy
                == application_consent_policy_str(&ApplicationConsentPolicy::AutoApprove)
        {
            return Err(ApplicationSecurityMutationError::TrustedApplication {
                application_id: application_id.to_owned(),
            }
            .into());
        }

        if state.lease_matches != 1 {
            return Err(ApplicationSecurityMutationError::LeaseExpiredOrInvalid {
                application_id: application_id.to_owned(),
            }
            .into());
        }
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
                    first_party, consent_policy, active, provisioning_status, rauthy_client_id,
                    created_by_user_id, created_at, updated_at
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)",
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
                    application_provisioning_status_str(&application.provisioning_status),
                    application.rauthy_client_id.clone(),
                    application.created_by_user_id.clone(),
                    now.clone(),
                    now
                ),
            )
            .await
            .context("insert application")?;
        Ok(())
    }

    pub async fn insert_application_with_optional_redirect_uri(
        &self,
        application: &Application,
        redirect_uri: Option<&str>,
    ) -> Result<()> {
        self.validate_application_references(application).await?;
        let now = now_string();
        let mut queries = vec![(
            Cow::Borrowed(
                "INSERT INTO applications (
                    id, organization_id, slug, name, description, application_type, app_type,
                    first_party, consent_policy, active, provisioning_status, rauthy_client_id,
                    created_by_user_id, created_at, updated_at
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)",
            ),
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
                application_provisioning_status_str(&application.provisioning_status),
                application.rauthy_client_id.clone(),
                application.created_by_user_id.clone(),
                now.clone(),
                now.clone()
            ),
        )];
        if let Some(redirect_uri) = redirect_uri {
            queries.push((
                Cow::Borrowed(
                    "INSERT INTO application_redirect_uris (application_id, redirect_uri, created_at)
                     VALUES ($1, $2, $3)",
                ),
                params!(application.id.clone(), redirect_uri.to_owned(), now),
            ));
        }
        self.runtime
            .transaction(queries)
            .await
            .context("insert application redirect URI manifest transaction")?;
        Ok(())
    }

    pub async fn update_application_with_security_lease(
        &self,
        application: &Application,
        slug_changed: bool,
        lease_token: &str,
    ) -> Result<()> {
        let preflight_now = now_string();
        self.validate_application_security_mutation_lease(
            &application.id,
            lease_token,
            Some(&preflight_now),
        )
        .await?;
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
        let mutation_now = now_string();
        let stmt = if slug_changed {
            "UPDATE applications
             SET organization_id = ?2,
                 slug = ?3,
                 name = ?4,
                 description = ?5,
                 application_type = ?6,
                 app_type = ?7,
                 active = ?8,
                 created_by_user_id = ?9,
                 updated_at = ?10
             WHERE id = ?1
               AND NOT (first_party = 1 AND consent_policy = 'auto_approve')
               AND security_mutation_token = ?11
               AND security_mutation_expires_at > ?10
               AND NOT EXISTS (
                   SELECT 1 FROM applications existing
                   WHERE existing.slug = ?12 AND existing.id != ?13
               )"
        } else {
            "UPDATE applications
             SET organization_id = ?2,
                 slug = ?3,
                 name = ?4,
                 description = ?5,
                 application_type = ?6,
                 app_type = ?7,
                 active = ?8,
                 created_by_user_id = ?9,
                 updated_at = ?10
             WHERE id = ?1
               AND NOT (first_party = 1 AND consent_policy = 'auto_approve')
               AND security_mutation_token = ?11
               AND security_mutation_expires_at > ?10"
        };
        let mut update_params = params!(
            application.id.clone(),
            application.organization_id.clone(),
            application.slug.clone(),
            application.name.clone(),
            application.description.clone(),
            application_type_str(&application.application_type),
            application_app_type_str(&application.app_type),
            if application.active { 1_i64 } else { 0_i64 },
            application.created_by_user_id.clone(),
            mutation_now.clone()
        );
        update_params.push(hiqlite::Param::from(lease_token.to_owned()));
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
            self.validate_application_security_mutation_lease(
                &application.id,
                lease_token,
                Some(&mutation_now),
            )
            .await?;
            if slug_changed {
                return Err(anyhow!(
                    "application slug already exists: {}",
                    application.slug
                ));
            }
            return Err(anyhow!(
                "application update made no change: {}",
                application.id
            ));
        }
        Ok(())
    }

    pub async fn set_application_trust(
        &self,
        organization_id: &str,
        application_id: &str,
        trusted: bool,
    ) -> Result<Application> {
        let now = now_string();
        let assertion_id = Uuid::new_v4().simple().to_string();
        let transaction_result = self
            .runtime
            .transaction([
                (
                    Cow::Borrowed(
                        "INSERT INTO application_consent_assertions (id, valid)
                         SELECT $1,
                                CASE WHEN EXISTS (
                                    SELECT 1 FROM applications
                                    WHERE id = $2
                                      AND organization_id = $3
                                      AND (
                                          security_mutation_token IS NULL
                                          OR security_mutation_expires_at IS NULL
                                          OR security_mutation_expires_at <= $4
                                      )
                                ) THEN 1 ELSE 0 END",
                    ),
                    params!(
                        assertion_id.clone(),
                        application_id.to_owned(),
                        organization_id.to_owned(),
                        now.clone()
                    ),
                ),
                (
                    Cow::Borrowed(
                        "UPDATE applications
                 SET first_party = $1,
                     consent_policy = $2,
                     updated_at = $3,
                     security_mutation_token = NULL,
                     security_mutation_expires_at = NULL
                 WHERE id = $4 AND organization_id = $5",
                    ),
                    params!(
                        if trusted { 1_i64 } else { 0_i64 },
                        if trusted { "auto_approve" } else { "explicit" },
                        now.clone(),
                        application_id.to_owned(),
                        organization_id.to_owned()
                    ),
                ),
                (
                    Cow::Borrowed(
                        "UPDATE application_grants
                         SET status = 'revoked',
                             revoked_at = $1,
                             revoked_by_user_id = NULL,
                             updated_at = $1
                         WHERE application_id = $2
                           AND organization_id = $3
                           AND status = 'active'
                           AND $4 = 1",
                    ),
                    params!(
                        now,
                        application_id.to_owned(),
                        organization_id.to_owned(),
                        if trusted { 1_i64 } else { 0_i64 }
                    ),
                ),
                (
                    Cow::Borrowed("DELETE FROM application_consent_assertions WHERE id = $1"),
                    params!(assertion_id),
                ),
            ])
            .await;
        if let Err(err) = transaction_result {
            if !err.chain().any(|cause| {
                cause
                    .to_string()
                    .contains("application_consent_assertion_valid")
            }) {
                return Err(err).context("set application trust transaction");
            }
            if self
                .exists_row_with(
                    ReadConsistency::Strong,
                    "SELECT COUNT(*) AS count FROM applications
                     WHERE id = $1 AND organization_id = $2",
                    params!(application_id.to_owned(), organization_id.to_owned()),
                    "application trust target existence",
                )
                .await?
            {
                return Err(ApplicationTrustMutationError::SecurityMutationInProgress {
                    application_id: application_id.to_owned(),
                }
                .into());
            }
            return Err(anyhow!(
                "application not found: {application_id} in organization {organization_id}"
            ));
        }
        self.get_application_with_consistency(application_id, Some(ReadConsistency::Strong))
            .await
    }

    pub async fn try_acquire_application_security_mutation(
        &self,
        application_id: &str,
        organization_id: &str,
        lease_token: &str,
        acquired_at: &str,
        expires_at: &str,
    ) -> Result<bool> {
        let updated = self
            .runtime
            .execute(
                "UPDATE applications
                 SET security_mutation_token = $1,
                     security_mutation_expires_at = $2
                 WHERE id = $3
                   AND organization_id = $4
                   AND NOT (first_party = 1 AND consent_policy = 'auto_approve')
                   AND (
                       security_mutation_token IS NULL
                       OR security_mutation_expires_at IS NULL
                       OR security_mutation_expires_at <= $5
                   )",
                params!(
                    lease_token.to_owned(),
                    expires_at.to_owned(),
                    application_id.to_owned(),
                    organization_id.to_owned(),
                    acquired_at.to_owned()
                ),
            )
            .await
            .context("acquire application security mutation lease")?;
        Ok(updated == 1)
    }

    pub async fn release_application_security_mutation(
        &self,
        application_id: &str,
        lease_token: &str,
    ) -> Result<()> {
        let released = self
            .runtime
            .execute(
                "UPDATE applications
                 SET security_mutation_token = NULL,
                     security_mutation_expires_at = NULL
                 WHERE id = $1 AND security_mutation_token = $2",
                params!(application_id.to_owned(), lease_token.to_owned()),
            )
            .await
            .context("release application security mutation lease")?;
        if released == 0 {
            self.validate_application_security_mutation_lease(application_id, lease_token, None)
                .await?;
            return Err(anyhow!(
                "application security mutation lease release made no change: {application_id}"
            ));
        }
        Ok(())
    }

    async fn acquire_direct_application_security_mutation(
        &self,
        application_id: &str,
    ) -> Result<String> {
        let application = self
            .get_application_with_consistency(application_id, Some(ReadConsistency::Strong))
            .await?;
        let token = Uuid::new_v4().simple().to_string();
        let acquired_at = Utc::now();
        let expires_at = acquired_at + chrono::Duration::minutes(10);
        if !self
            .try_acquire_application_security_mutation(
                application_id,
                &application.organization_id,
                &token,
                &acquired_at.to_rfc3339(),
                &expires_at.to_rfc3339(),
            )
            .await?
        {
            return Err(anyhow!(
                "application security mutation already in progress or application is trusted: {application_id}"
            ));
        }
        Ok(token)
    }

    async fn with_direct_application_security_mutation<T, F, Fut>(
        &self,
        application_id: &str,
        operation: F,
    ) -> Result<T>
    where
        F: FnOnce(String) -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        let lease_token = self
            .acquire_direct_application_security_mutation(application_id)
            .await?;
        let result = operation(lease_token.clone()).await;
        let _ = self
            .release_application_security_mutation(application_id, &lease_token)
            .await;
        result
    }

    pub async fn upsert_application(&self, application: &Application) -> Result<()> {
        self.validate_application_references(application).await?;
        let now = now_string();
        self.runtime
            .execute(
                "INSERT INTO applications (
                    id, organization_id, slug, name, description, application_type, app_type,
                    first_party, consent_policy, active, provisioning_status, rauthy_client_id,
                    created_by_user_id, created_at, updated_at
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
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
                    provisioning_status = excluded.provisioning_status,
                    rauthy_client_id = excluded.rauthy_client_id,
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
                    application_provisioning_status_str(&application.provisioning_status),
                    application.rauthy_client_id.clone(),
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
                        first_party, consent_policy, active, provisioning_status, rauthy_client_id,
                        created_by_user_id
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
                        first_party, consent_policy, active, provisioning_status, rauthy_client_id,
                        created_by_user_id
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
                        first_party, consent_policy, active, provisioning_status, rauthy_client_id,
                        created_by_user_id
                 FROM applications
                 WHERE slug = $1
                 LIMIT 1",
                params!(slug.to_owned()),
                "application by slug optional",
            )
            .await?;
        rows.into_iter().next().map(TryInto::try_into).transpose()
    }

    pub async fn find_active_application_by_client_for_organization(
        &self,
        rauthy_client_id: &str,
        organization_id: &str,
    ) -> Result<Option<Application>> {
        let rows: Vec<ApplicationRow> = self
            .query_rows_with(
                ReadConsistency::Strong,
                "SELECT id, organization_id, slug, name, description, application_type, app_type,
                        first_party, consent_policy, active, provisioning_status, rauthy_client_id,
                        created_by_user_id
                 FROM applications
                 WHERE rauthy_client_id = $1
                   AND organization_id = $2
                   AND active = 1
                   AND provisioning_status = 'ready'
                 LIMIT 2",
                params!(rauthy_client_id.to_owned(), organization_id.to_owned()),
                "active application by Rauthy client and organization",
            )
            .await?;
        if rows.len() > 1 {
            return Err(anyhow!(
                "multiple applications map to Rauthy client {rauthy_client_id}"
            ));
        }
        rows.into_iter().next().map(TryInto::try_into).transpose()
    }

    pub async fn mark_application_provisioning_ready(
        &self,
        application_id: &str,
        organization_id: &str,
    ) -> Result<Application> {
        let updated = self
            .runtime
            .execute(
                "UPDATE applications
                 SET provisioning_status = 'ready', updated_at = $1
                 WHERE id = $2
                   AND organization_id = $3
                   AND provisioning_status = 'unprovisioned'",
                params!(
                    now_string(),
                    application_id.to_owned(),
                    organization_id.to_owned()
                ),
            )
            .await
            .context("mark application provisioning ready")?;
        if updated == 0 {
            let current = self
                .get_application_with_consistency(application_id, Some(ReadConsistency::Strong))
                .await?;
            if current.organization_id != organization_id {
                return Err(anyhow!(
                    "application not found: {application_id} in organization {organization_id}"
                ));
            }
            if current.provisioning_status == ApplicationProvisioningStatus::Ready {
                return Ok(current);
            }
            return Err(anyhow!(
                "application provisioning state changed concurrently"
            ));
        }
        self.get_application_with_consistency(application_id, Some(ReadConsistency::Strong))
            .await
    }

    pub async fn insert_application_redirect_uri(
        &self,
        redirect_uri: &ApplicationRedirectUri,
    ) -> Result<()> {
        self.with_direct_application_security_mutation(
            &redirect_uri.application_id,
            |lease_token| async move {
                self.insert_application_redirect_uri_with_security_lease(redirect_uri, &lease_token)
                    .await
            },
        )
        .await
    }

    pub async fn insert_application_redirect_uri_with_security_lease(
        &self,
        redirect_uri: &ApplicationRedirectUri,
        lease_token: &str,
    ) -> Result<()> {
        let mutation_now = now_string();
        let inserted = self
            .runtime
            .execute(
                "INSERT INTO application_redirect_uris (application_id, redirect_uri, created_at)
                 SELECT $1, $2, $3
                 FROM applications app
                 WHERE app.id = $1
                   AND NOT (app.first_party = 1 AND app.consent_policy = 'auto_approve')
                   AND app.security_mutation_token = $4
                   AND app.security_mutation_expires_at > $5
                 ON CONFLICT(application_id, redirect_uri) DO NOTHING",
                params!(
                    redirect_uri.application_id.clone(),
                    redirect_uri.redirect_uri.clone(),
                    redirect_uri.created_at.clone(),
                    lease_token.to_owned(),
                    mutation_now.clone()
                ),
            )
            .await
            .context("insert application redirect uri")?;
        if inserted == 0 {
            self.validate_application_security_mutation_lease(
                &redirect_uri.application_id,
                lease_token,
                Some(&mutation_now),
            )
            .await?;
            if self
                .exists_row_with(
                    ReadConsistency::Strong,
                    "SELECT COUNT(*) AS count FROM application_redirect_uris
                     WHERE application_id = $1 AND redirect_uri = $2",
                    params!(
                        redirect_uri.application_id.clone(),
                        redirect_uri.redirect_uri.clone()
                    ),
                    "inserted application redirect uri",
                )
                .await?
            {
                return Ok(());
            }
            return Err(anyhow!(
                "application redirect URI insert made no change: {}",
                redirect_uri.application_id
            ));
        }
        Ok(())
    }

    pub async fn list_application_redirect_uris_with_consistency(
        &self,
        application_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Vec<ApplicationRedirectUri>> {
        let rows: Vec<ApplicationRedirectUriRow> = self
            .query_rows_with(
                self.validation_read_consistency_with_override(consistency),
                "SELECT application_id, redirect_uri, created_at
                 FROM application_redirect_uris
                 WHERE application_id = $1
                 ORDER BY redirect_uri ASC",
                params!(application_id.to_owned()),
                "application redirect uris",
            )
            .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    pub async fn delete_application_redirect_uri(
        &self,
        application_id: &str,
        redirect_uri: &str,
    ) -> Result<()> {
        self.with_direct_application_security_mutation(application_id, |lease_token| async move {
            self.delete_application_redirect_uri_with_security_lease(
                application_id,
                redirect_uri,
                &lease_token,
            )
            .await
        })
        .await
    }

    pub async fn delete_application_redirect_uri_with_security_lease(
        &self,
        application_id: &str,
        redirect_uri: &str,
        lease_token: &str,
    ) -> Result<()> {
        let mutation_now = now_string();
        let deleted = self
            .runtime
            .execute(
                "DELETE FROM application_redirect_uris
                 WHERE application_id = $1
                   AND redirect_uri = $2
                   AND EXISTS (
                       SELECT 1 FROM applications app
                         WHERE app.id = application_redirect_uris.application_id
                         AND NOT (app.first_party = 1 AND app.consent_policy = 'auto_approve')
                         AND app.security_mutation_token = $3
                         AND app.security_mutation_expires_at > $4
                   )",
                params!(
                    application_id.to_owned(),
                    redirect_uri.to_owned(),
                    lease_token.to_owned(),
                    mutation_now.clone()
                ),
            )
            .await
            .context("delete application redirect uri")?;
        if deleted == 0 {
            self.validate_application_security_mutation_lease(
                application_id,
                lease_token,
                Some(&mutation_now),
            )
            .await?;
        }
        Ok(())
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
        self.with_direct_application_security_mutation(application_id, |lease_token| async move {
            self.replace_application_actions_with_security_lease(
                application_id,
                action_ids,
                &lease_token,
            )
            .await
        })
        .await
    }

    pub async fn replace_application_actions_with_security_lease(
        &self,
        application_id: &str,
        action_ids: &[String],
        lease_token: &str,
    ) -> Result<Vec<Action>> {
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

        let assertion_id = Uuid::new_v4().simple().to_string();
        let mutation_now = now_string();
        let mut queries = Vec::with_capacity(4 + deduped.len());
        queries.push((
            Cow::Borrowed(
                "INSERT INTO application_consent_assertions (id, valid)
                 SELECT $1,
                        CASE WHEN EXISTS (
                            SELECT 1 FROM applications app
                            WHERE app.id = $2
                              AND NOT (app.first_party = 1 AND app.consent_policy = 'auto_approve')
                              AND app.security_mutation_token = $3
                              AND app.security_mutation_expires_at > $4
                        ) THEN 1 ELSE 0 END",
            ),
            params!(
                assertion_id.clone(),
                application_id.to_owned(),
                lease_token.to_owned(),
                mutation_now.clone()
            ),
        ));
        queries.push((
            Cow::Borrowed(
                "DELETE FROM application_actions
                 WHERE application_id = $1
                   AND EXISTS (
                       SELECT 1 FROM applications app
                       WHERE app.id = application_actions.application_id
                         AND NOT (app.first_party = 1 AND app.consent_policy = 'auto_approve')
                   )",
            ),
            params!(application_id.to_owned()),
        ));
        for action_id in deduped {
            queries.push((
                Cow::Borrowed(
                    "INSERT INTO application_actions (application_id, action_id)
                     SELECT $1, $2 FROM applications app
                     WHERE app.id = $1
                       AND NOT (app.first_party = 1 AND app.consent_policy = 'auto_approve')
                     ON CONFLICT(application_id, action_id) DO NOTHING",
                ),
                params!(application_id.to_owned(), action_id.clone()),
            ));
        }
        queries.push((
            Cow::Borrowed(
                "DELETE FROM application_grant_actions
                 WHERE application_grant_id IN (
                     SELECT ag.id FROM application_grants ag
                     JOIN applications app ON app.id = ag.application_id
                     WHERE ag.application_id = $1
                       AND NOT (app.first_party = 1 AND app.consent_policy = 'auto_approve')
                 )
                   AND NOT EXISTS (
                       SELECT 1 FROM application_actions ca
                       JOIN application_grants cg
                         ON cg.application_id = ca.application_id
                       WHERE cg.id = application_grant_actions.application_grant_id
                         AND ca.action_id = application_grant_actions.action_id
                   )",
            ),
            params!(application_id.to_owned()),
        ));
        queries.push((
            Cow::Borrowed("DELETE FROM application_consent_assertions WHERE id = $1"),
            params!(assertion_id),
        ));

        if let Err(err) = self.runtime.transaction(queries).await {
            if err.chain().any(|cause| {
                cause
                    .to_string()
                    .contains("application_consent_assertion_valid")
            }) {
                self.validate_application_security_mutation_lease(
                    application_id,
                    lease_token,
                    Some(&mutation_now),
                )
                .await?;
            }
            return Err(err).context("replace application actions transaction");
        }
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

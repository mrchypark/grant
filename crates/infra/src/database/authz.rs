use super::*;

impl Database {
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
            application_id: None,
            consistency,
        })
        .await
    }

    pub async fn has_user_actions(&self, checks: &[UserActionCheck<'_>]) -> Result<Vec<bool>> {
        Ok(self
            .evaluate_user_actions(checks)
            .await?
            .into_iter()
            .map(|decision| decision.allowed)
            .collect())
    }

    pub async fn evaluate_user_actions(
        &self,
        checks: &[UserActionCheck<'_>],
    ) -> Result<Vec<UserActionDecision>> {
        self.evaluate_user_actions_with_inactive_organization_policy(checks, false)
            .await
    }

    pub async fn has_user_organization_reactivation_action_with_consistency(
        &self,
        user_id: &str,
        organization_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<bool> {
        let check = UserActionCheck {
            user_id,
            organization_id,
            provider_id: "grant",
            resource_type: "organization",
            resource_id: organization_id,
            action_id: "organization.permission.manage",
            application_id: None,
            consistency,
        };
        Ok(self
            .evaluate_user_actions_with_inactive_organization_policy(&[check], true)
            .await?
            .into_iter()
            .next()
            .is_some_and(|decision| decision.allowed))
    }

    async fn evaluate_user_actions_with_inactive_organization_policy(
        &self,
        checks: &[UserActionCheck<'_>],
        allow_inactive_self_reactivation: bool,
    ) -> Result<Vec<UserActionDecision>> {
        let denied = UserActionDecision {
            allowed: false,
            rbac_allowed: false,
            trusted_application: false,
        };
        let mut results = vec![denied; checks.len()];
        for (chunk_number, chunk) in checks.chunks(MAX_AUTHZ_CHECKS_PER_QUERY).enumerate() {
            let chunk_start = chunk_number
                .checked_mul(MAX_AUTHZ_CHECKS_PER_QUERY)
                .ok_or_else(|| anyhow!("authorization chunk offset overflow"))?;
            let chunk_end = chunk_start
                .checked_add(chunk.len())
                .ok_or_else(|| anyhow!("authorization chunk end overflow"))?;
            let Some(first) = chunk.first() else {
                continue;
            };
            let consistency =
                self.default_read_consistency_with_override(first.consistency.clone());
            let validation_time = Utc::now().to_rfc3339();
            let mut query_params = Vec::with_capacity(chunk.len() * 10);
            let values = chunk
                .iter()
                .enumerate()
                .map(|(chunk_index, check)| -> Result<String> {
                    let absolute_index = chunk_start
                        .checked_add(chunk_index)
                        .ok_or_else(|| anyhow!("authorization result index overflow"))?;
                    let query_index = i64::try_from(absolute_index)
                        .context("authorization result index exceeds SQLite INTEGER")?;
                    let base = query_params.len() + 1;
                    query_params.push(hiqlite::Param::from(query_index));
                    query_params.push(hiqlite::Param::from(check.user_id.to_owned()));
                    query_params.push(hiqlite::Param::from(check.organization_id.to_owned()));
                    query_params.push(hiqlite::Param::from(check.provider_id.to_owned()));
                    query_params.push(hiqlite::Param::from(check.resource_type.to_owned()));
                    query_params.push(hiqlite::Param::from(check.resource_id.to_owned()));
                    query_params.push(hiqlite::Param::from(check.action_id.to_owned()));
                    query_params.push(hiqlite::Param::from(
                        check.application_id.map(ToOwned::to_owned),
                    ));
                    query_params.push(hiqlite::Param::from(validation_time.clone()));
                    query_params.push(hiqlite::Param::from(if allow_inactive_self_reactivation {
                        1_i64
                    } else {
                        0_i64
                    }));
                    Ok(format!(
                        "(${base}, ${}, ${}, ${}, ${}, ${}, ${}, ${}, ${}, ${})",
                        base + 1,
                        base + 2,
                        base + 3,
                        base + 4,
                        base + 5,
                        base + 6,
                        base + 7,
                        base + 8,
                        base + 9
                    ))
                })
                .collect::<Result<Vec<_>>>()?
                .join(", ");
            let rows: Vec<UserActionDecisionRow> = self
                .query_rows_with(
                    consistency,
                    format!(
                        r#"
                        WITH requested (
                            idx, user_id, organization_id, provider_id, resource_type,
                            resource_id, action_id, application_id, validation_time,
                            allow_inactive_self_reactivation
                        ) AS (
                            VALUES {values}
                        ),
                        eligible_requested AS (
                            SELECT req.*
                            FROM requested req
                            JOIN users u
                              ON u.id = req.user_id
                             AND u.active = 1
                            JOIN service_providers sp
                              ON sp.id = req.provider_id
                             AND sp.active = 1
                            JOIN organizations subject_org
                              ON subject_org.id = req.organization_id
                            WHERE (
                                subject_org.inactivated_at IS NULL
                                OR (
                                    req.allow_inactive_self_reactivation = 1
                                    AND req.provider_id = 'grant'
                                    AND req.resource_type = 'organization'
                                    AND req.resource_id = req.organization_id
                                    AND req.action_id = 'organization.permission.manage'
                                )
                            )
                            AND (
                                req.provider_id <> 'grant'
                                OR req.resource_type <> 'organization'
                                OR EXISTS (
                                    SELECT 1
                                    FROM organizations target_org
                                    WHERE target_org.id = req.resource_id
                                      AND target_org.inactivated_at IS NULL
                                )
                                OR (
                                    req.allow_inactive_self_reactivation = 1
                                    AND req.provider_id = 'grant'
                                    AND req.resource_type = 'organization'
                                    AND req.resource_id = req.organization_id
                                    AND req.action_id = 'organization.permission.manage'
                                )
                            )
                        ),
                        rbac_allowed AS (
                            SELECT DISTINCT req.idx
                            FROM eligible_requested req
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
                            FROM eligible_requested req
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
                            FROM eligible_requested req
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
                        ),
                        trusted_application_allowed AS (
                            SELECT DISTINCT req.idx
                            FROM eligible_requested req
                            JOIN applications app
                             ON app.id = req.application_id
                             AND app.organization_id = req.organization_id
                             AND app.active = 1
                             AND app.provisioning_status = 'ready'
                             AND app.first_party = 1
                             AND app.consent_policy = 'auto_approve'
                            JOIN application_actions aa
                              ON aa.application_id = app.id
                             AND aa.action_id = req.action_id
                            WHERE req.application_id IS NOT NULL
                        ),
                        explicit_consent_allowed AS (
                            SELECT DISTINCT req.idx
                            FROM eligible_requested req
                            JOIN application_grants ag
                              ON ag.user_id = req.user_id
                             AND ag.organization_id = req.organization_id
                             AND ag.application_id = req.application_id
                             AND ag.status = 'active'
                            JOIN applications app
                              ON app.id = ag.application_id
                             AND app.organization_id = ag.organization_id
                             AND app.active = 1
                             AND app.provisioning_status = 'ready'
                            JOIN application_grant_actions aga
                              ON aga.application_grant_id = ag.id
                             AND aga.action_id = req.action_id
                            JOIN application_actions aa
                              ON aa.application_id = ag.application_id
                             AND aa.action_id = aga.action_id
                            WHERE req.application_id IS NOT NULL
                        )
                        SELECT req.idx,
                               CASE WHEN rbac.idx IS NULL THEN 0 ELSE 1 END AS rbac_allowed,
                               CASE
                                   WHEN rbac.idx IS NULL THEN 0
                                   WHEN req.application_id IS NULL THEN 1
                                   WHEN trusted.idx IS NOT NULL OR consent.idx IS NOT NULL THEN 1
                                   ELSE 0
                               END AS allowed,
                               CASE WHEN trusted.idx IS NULL THEN 0 ELSE 1 END AS trusted_application
                        FROM requested req
                        LEFT JOIN rbac_allowed rbac ON rbac.idx = req.idx
                        LEFT JOIN trusted_application_allowed trusted ON trusted.idx = req.idx
                        LEFT JOIN explicit_consent_allowed consent ON consent.idx = req.idx
                        ORDER BY req.idx ASC
                        "#
                    ),
                    query_params,
                    "bulk user organization actions",
                )
                .await?;
            for row in rows {
                let result_index = usize::try_from(row.idx)
                    .context("authorization query returned a negative result index")?;
                if !(chunk_start..chunk_end).contains(&result_index) {
                    return Err(anyhow!(
                        "authorization query returned result index {result_index} outside chunk range {chunk_start}..{chunk_end}"
                    ));
                }
                let result = results.get_mut(result_index).ok_or_else(|| {
                    anyhow!(
                        "authorization query returned result index {result_index} outside result length {}",
                        checks.len()
                    )
                })?;
                *result = UserActionDecision {
                    allowed: row.allowed != 0,
                    rbac_allowed: row.rbac_allowed != 0,
                    trusted_application: row.trusted_application != 0,
                };
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
        let written = self
            .runtime
            .execute(
                "INSERT INTO role_bindings (
		                    id, subject_type, subject_id, subject_organization_id, provider_id, resource_type,
		                    resource_id, role_id, agreement_id, state, created_by_user_id, created_at, updated_at
		                 )
                         SELECT $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13
                         WHERE EXISTS (
                             SELECT 1 FROM service_providers
                             WHERE id = $5 AND active = 1
                         )
                           AND EXISTS (
                             SELECT 1 FROM organizations
                             WHERE id = $4 AND inactivated_at IS NULL
                         )
                           AND (
                             $2 <> 'group'
                             OR EXISTS (
                                 SELECT 1 FROM groups
                                 WHERE id = $3 AND organization_id = $4
                             )
                           )
                           AND (
                             $5 <> 'grant'
                             OR $6 <> 'organization'
                             OR EXISTS (
                                 SELECT 1 FROM organizations
                                 WHERE id = $7 AND inactivated_at IS NULL
                             )
                           )
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
        if written == 0 {
            return Err(anyhow!(
                "role binding active provider, organization, resource, or group reference changed before commit"
            ));
        }
        Ok(())
    }

    pub async fn insert_role_binding(&self, binding: &RoleBinding) -> Result<()> {
        validate_role_binding(binding)?;
        self.validate_role_binding_refs(binding).await?;
        let inserted = self
            .runtime
            .execute(
                "INSERT INTO role_bindings (
                    id, subject_type, subject_id, subject_organization_id, provider_id, resource_type,
                    resource_id, role_id, agreement_id, state, created_by_user_id, created_at, updated_at
                 )
                 SELECT $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13
                 WHERE EXISTS (
                     SELECT 1 FROM service_providers
                     WHERE id = $5 AND active = 1
                 )
                   AND EXISTS (
                     SELECT 1 FROM organizations
                     WHERE id = $4 AND inactivated_at IS NULL
                 )
                   AND (
                     $2 <> 'group'
                     OR EXISTS (
                         SELECT 1 FROM groups
                         WHERE id = $3 AND organization_id = $4
                     )
                   )
                   AND (
                     $5 <> 'grant'
                     OR $6 <> 'organization'
                     OR EXISTS (
                         SELECT 1 FROM organizations
                         WHERE id = $7 AND inactivated_at IS NULL
                     )
                   )",
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
        if inserted == 0 {
            return Err(anyhow!(
                "role binding active provider, organization, resource, or group reference changed before commit"
            ));
        }
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
                "SELECT COUNT(*) as count FROM service_providers WHERE id = $1 AND active = 1",
                params!(binding.provider_id.clone()),
                "binding active provider",
            )
            .await?
        {
            return Err(anyhow!(
                "service provider {} is inactive",
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
        if !self
            .exists_row_with(
                consistency.clone(),
                "SELECT COUNT(*) as count FROM organizations WHERE id = $1 AND inactivated_at IS NULL",
                params!(binding.subject_organization_id.clone()),
                "active subject organization",
            )
            .await?
        {
            return Err(anyhow!(
                "subject organization {} is inactive",
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
                        "SELECT COUNT(*) as count
                         FROM groups g
                         JOIN organizations o ON o.id = g.organization_id
                         WHERE g.id = $1
                           AND g.organization_id = $2
                           AND o.inactivated_at IS NULL",
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

        if binding.provider_id == "grant" && binding.resource_type == "organization" {
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
            if !self
                .exists_row_with(
                    consistency,
                    "SELECT COUNT(*) as count FROM organizations WHERE id = $1 AND inactivated_at IS NULL",
                    params!(binding.resource_id.clone()),
                    "active organization resource",
                )
                .await?
            {
                return Err(anyhow!(
                    "organization resource {} is inactive",
                    binding.resource_id
                ));
            }
        }

        Ok(())
    }
}

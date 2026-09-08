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
                               CASE WHEN trusted.idx IS NULL THEN 0 ELSE 1 END AS trusted_application,
                               CASE WHEN trusted.idx IS NOT NULL OR consent.idx IS NOT NULL THEN 1 ELSE 0 END AS application_allowed
                        FROM requested req
                        LEFT JOIN rbac_allowed rbac ON rbac.idx = req.idx
                        LEFT JOIN trusted_application_allowed trusted ON trusted.idx = req.idx
                        LEFT JOIN explicit_consent_allowed consent ON consent.idx = req.idx
                        ORDER BY req.idx ASC

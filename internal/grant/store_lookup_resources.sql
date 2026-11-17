WITH req (
    user_id, organization_id, provider_id, resource_type,
    action_id, application_id, validation_time, cursor_id
) AS (
    VALUES (?, ?, ?, ?, ?, ?, ?, ?)
),
{agreement_bounds},
eligible AS (
    SELECT req.*
    FROM req
    JOIN users u
      ON u.id = req.user_id
     AND u.active = 1
    JOIN service_providers sp
      ON sp.id = req.provider_id
     AND sp.active = 1
    JOIN organizations subject_org
      ON subject_org.id = req.organization_id
     AND subject_org.inactivated_at IS NULL
    WHERE req.application_id = ''
       OR EXISTS (
           SELECT 1
           FROM applications app
           JOIN application_actions aa
             ON aa.application_id = app.id
            AND aa.action_id = req.action_id
           WHERE app.id = req.application_id
             AND app.organization_id = req.organization_id
             AND app.active = 1
             AND app.provisioning_status = 'ready'
             AND (
                 (app.first_party = 1 AND app.consent_policy = 'auto_approve')
                 OR EXISTS (
                     SELECT 1
                     FROM application_grants ag
                     JOIN application_grant_actions aga
                       ON aga.application_grant_id = ag.id
                      AND aga.action_id = req.action_id
                     WHERE ag.user_id = req.user_id
                       AND ag.organization_id = req.organization_id
                       AND ag.application_id = app.id
                       AND ag.status = 'active'
                 )
             )
       )
),
candidates AS (
    SELECT rb.resource_id
    FROM eligible req
    JOIN role_bindings rb
      ON rb.subject_type = 'organization'
     AND rb.subject_id = req.organization_id
     AND rb.subject_organization_id = req.organization_id
     AND rb.provider_id = req.provider_id
     AND rb.resource_type = req.resource_type
     AND rb.state = 'active'
     AND (req.cursor_id = '' OR rb.resource_id > req.cursor_id)
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
    LEFT JOIN valid_agreements a ON a.id = rb.agreement_id
    WHERE (
        rb.role_id NOT IN ('role-system-bootstrap-owner', 'role-system-management-owner')
        OR om.membership_role = 'owner'
    )
    AND (rb.agreement_id IS NULL OR (
        a.id IS NOT NULL
        AND (a.starts_at IS NULL OR a.starts_at <= req.validation_time)
        AND (a.ends_at IS NULL OR a.ends_at >= req.validation_time)
    ))

    UNION

    SELECT rb.resource_id
    FROM eligible req
    JOIN role_bindings rb
      ON rb.subject_type = 'user'
     AND rb.subject_id = req.user_id
     AND rb.subject_organization_id = req.organization_id
     AND rb.provider_id = req.provider_id
     AND rb.resource_type = req.resource_type
     AND rb.state = 'active'
     AND (req.cursor_id = '' OR rb.resource_id > req.cursor_id)
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
    LEFT JOIN valid_agreements a ON a.id = rb.agreement_id
    WHERE (
        rb.role_id NOT IN ('role-system-bootstrap-owner', 'role-system-management-owner')
        OR om.membership_role = 'owner'
    )
    AND (rb.agreement_id IS NULL OR (
        a.id IS NOT NULL
        AND (a.starts_at IS NULL OR a.starts_at <= req.validation_time)
        AND (a.ends_at IS NULL OR a.ends_at >= req.validation_time)
    ))

    UNION

    SELECT rb.resource_id
    FROM eligible req
    JOIN role_bindings rb
      ON rb.subject_type = 'group'
     AND rb.subject_organization_id = req.organization_id
     AND rb.provider_id = req.provider_id
     AND rb.resource_type = req.resource_type
     AND rb.state = 'active'
     AND (req.cursor_id = '' OR rb.resource_id > req.cursor_id)
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
    LEFT JOIN valid_agreements a ON a.id = rb.agreement_id
    WHERE (
        rb.role_id NOT IN ('role-system-bootstrap-owner', 'role-system-management-owner')
        OR EXISTS (
            SELECT 1
            FROM organization_memberships owner_om
            WHERE owner_om.organization_id = req.organization_id
              AND owner_om.user_id = req.user_id
              AND owner_om.status = 'active'
              AND owner_om.membership_role = 'owner'
        )
    )
    AND (rb.agreement_id IS NULL OR (
        a.id IS NOT NULL
        AND (a.starts_at IS NULL OR a.starts_at <= req.validation_time)
        AND (a.ends_at IS NULL OR a.ends_at >= req.validation_time)
    ))

    UNION

    SELECT req.organization_id AS resource_id
    FROM eligible req
    JOIN organization_memberships om
      ON om.organization_id = req.organization_id
     AND om.user_id = req.user_id
     AND om.status = 'active'
     AND om.membership_role = 'owner'
    JOIN role_actions ra
      ON ra.role_id = CASE req.provider_id
          WHEN 'grant' THEN 'role-system-bootstrap-owner'
          WHEN 'grant-management' THEN 'role-system-management-owner'
      END
     AND ra.action_id = req.action_id
    JOIN actions act
      ON act.id = ra.action_id
     AND act.provider_id = req.provider_id
    WHERE req.resource_type = 'organization'
      AND req.provider_id IN ('grant', 'grant-management')
      AND (req.cursor_id = '' OR req.organization_id > req.cursor_id)
)
SELECT DISTINCT c.resource_id
FROM candidates c
CROSS JOIN eligible req
WHERE req.provider_id <> 'grant'
   OR req.resource_type <> 'organization'
   OR EXISTS (
       SELECT 1
       FROM organizations target_org
       WHERE target_org.id = c.resource_id
         AND target_org.inactivated_at IS NULL
   )
ORDER BY c.resource_id ASC
LIMIT ?

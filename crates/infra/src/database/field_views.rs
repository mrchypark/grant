use super::core::now_string;
use super::*;

#[derive(Debug)]
struct FieldViewRow {
    field_pointer: String,
    view_mode: String,
    updated_at: String,
}

impl From<&mut Row<'_>> for FieldViewRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            field_pointer: row.get("field_pointer"),
            view_mode: row.get("view_mode"),
            updated_at: row.get("updated_at"),
        }
    }
}

#[derive(Debug)]
struct FieldViewAuditRow {
    request_hash: String,
    role_id: String,
    action_id: String,
    resource_type: String,
    schema_key: String,
    after_json: String,
}

impl From<&mut Row<'_>> for FieldViewAuditRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            request_hash: row.get("request_hash"),
            role_id: row.get("role_id"),
            action_id: row.get("action_id"),
            resource_type: row.get("resource_type"),
            schema_key: row.get("schema_key"),
            after_json: row.get("after_json"),
        }
    }
}

#[derive(Debug)]
struct TimestampRow {
    updated_at: Option<String>,
}

impl From<&mut Row<'_>> for TimestampRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            updated_at: row.get("updated_at"),
        }
    }
}

impl Database {
    pub async fn list_role_action_field_views_with_consistency(
        &self,
        role_id: &str,
        action_id: &str,
        resource_type: &str,
        schema_key: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<RoleActionFieldViewPolicy> {
        let consistency = self.validation_read_consistency_with_override(consistency);
        let rows: Vec<FieldViewRow> = self
            .query_rows_with(
                consistency.clone(),
                "SELECT field_pointer, view_mode, updated_at
                 FROM role_action_field_views
                 WHERE role_id = $1
                   AND action_id = $2
                   AND resource_type = $3
                   AND schema_key = $4
                 ORDER BY field_pointer ASC",
                params!(
                    role_id.to_owned(),
                    action_id.to_owned(),
                    resource_type.to_owned(),
                    schema_key.to_owned()
                ),
                "role action field views",
            )
            .await?;
        let mut field_views = BTreeMap::new();
        let mut updated_at = None;
        for row in rows {
            let mode =
                domain::FieldViewMode::from_db_str(&row.view_mode).map_err(anyhow::Error::msg)?;
            let field_pointer = normalize_stored_field_view_pointer(&row.field_pointer)?;
            if field_views.insert(field_pointer.clone(), mode).is_some() {
                return Err(anyhow!(
                    "duplicate stored field view pointer {}",
                    field_pointer
                ));
            }
            updated_at = match updated_at {
                Some(current) if current >= row.updated_at => Some(current),
                _ => Some(row.updated_at),
            };
        }
        if updated_at.is_none() {
            let timestamp: TimestampRow = self
                .query_row_with(
                    consistency,
                    "SELECT MAX(created_at) AS updated_at
                     FROM field_view_policy_audit_outbox
                     WHERE role_id = $1
                       AND action_id = $2
                       AND resource_type = $3
                       AND schema_key = $4",
                    params!(
                        role_id.to_owned(),
                        action_id.to_owned(),
                        resource_type.to_owned(),
                        schema_key.to_owned()
                    ),
                    "role action field view update timestamp",
                )
                .await?;
            updated_at = timestamp.updated_at;
        }
        Ok(RoleActionFieldViewPolicy {
            role_id: role_id.to_owned(),
            action_id: action_id.to_owned(),
            resource_type: resource_type.to_owned(),
            schema_key: schema_key.to_owned(),
            field_views,
            updated_at,
        })
    }

    pub async fn replace_role_action_field_views(
        &self,
        input: ReplaceRoleActionFieldViewPolicy,
    ) -> Result<ReplaceRoleActionFieldViewPolicyOutcome> {
        if let Some(outcome) = self
            .field_view_idempotency_outcome(&input.idempotency_key, &input.request_hash)
            .await?
        {
            return Ok(outcome);
        }

        let now = now_string();
        let event_id = Uuid::new_v4().simple().to_string();
        let after = RoleActionFieldViewPolicy {
            role_id: input.role_id.clone(),
            action_id: input.action_id.clone(),
            resource_type: input.resource_type.clone(),
            schema_key: input.schema_key.clone(),
            field_views: input.field_views.clone(),
            updated_at: Some(now.clone()),
        };
        let after_json = serde_json::to_string(&SerializableFieldViewPolicy::from(&after))
            .context("serialize field view policy audit result")?;
        let mut queries = Vec::with_capacity(2 + input.field_views.len());
        queries.push((
            Cow::Borrowed(
                "INSERT INTO field_view_policy_audit_outbox (
                    event_id, idempotency_key, request_hash, organization_id,
                    actor_user_id, role_id, action_id, resource_type, schema_key,
                    before_json, after_json, created_at
                 )
                 SELECT $1, $2, $3, $4, $5, $6, $7, $8, $9,
                        json_object(
                            'roleId', $6,
                            'actionId', $7,
                            'resourceType', $8,
                            'schemaKey', $9,
                            'fieldViews', json(COALESCE((
                                SELECT json_group_object(field_pointer, view_mode)
                                FROM (
                                    SELECT field_pointer, view_mode
                                    FROM role_action_field_views
                                    WHERE role_id = $6
                                      AND action_id = $7
                                      AND resource_type = $8
                                      AND schema_key = $9
                                    ORDER BY field_pointer ASC
                                )
                            ), '{}')),
                            'updatedAt', COALESCE(
                                (SELECT MAX(updated_at)
                                 FROM role_action_field_views
                                 WHERE role_id = $6
                                   AND action_id = $7
                                   AND resource_type = $8
                                   AND schema_key = $9),
                                (SELECT MAX(created_at)
                                 FROM field_view_policy_audit_outbox
                                 WHERE role_id = $6
                                   AND action_id = $7
                                   AND resource_type = $8
                                   AND schema_key = $9)
                            )
                        ),
                        $10, $11
                 FROM roles AS r
                 JOIN role_actions AS ra
                   ON ra.role_id = r.id AND ra.action_id = $7
                 JOIN actions AS a
                   ON a.id = ra.action_id
                 JOIN service_providers AS sp
                   ON sp.id = a.provider_id
                 WHERE r.id = $6
                   AND r.organization_id = $4
                   AND a.visibility_granting = 1
                   AND sp.active = 1
                 ON CONFLICT(idempotency_key) DO NOTHING",
            ),
            params!(
                event_id.clone(),
                input.idempotency_key.clone(),
                input.request_hash.clone(),
                input.organization_id.clone(),
                input.actor_user_id.clone(),
                input.role_id.clone(),
                input.action_id.clone(),
                input.resource_type.clone(),
                input.schema_key.clone(),
                after_json,
                now.clone()
            ),
        ));
        queries.push((
            Cow::Borrowed(
                "DELETE FROM role_action_field_views
                 WHERE role_id = $1
                   AND action_id = $2
                   AND resource_type = $3
                   AND schema_key = $4
                   AND EXISTS (
                       SELECT 1
                       FROM field_view_policy_audit_outbox
                       WHERE event_id = $5
                         AND idempotency_key = $6
                         AND request_hash = $7
                   )",
            ),
            params!(
                input.role_id.clone(),
                input.action_id.clone(),
                input.resource_type.clone(),
                input.schema_key.clone(),
                event_id.clone(),
                input.idempotency_key.clone(),
                input.request_hash.clone()
            ),
        ));
        for (field_pointer, view_mode) in &input.field_views {
            let view_mode = view_mode.as_positive_db_str().ok_or_else(|| {
                anyhow!("field view database input must contain only positive modes")
            })?;
            queries.push((
                Cow::Borrowed(
                    "INSERT INTO role_action_field_views (
                        role_id, action_id, resource_type, schema_key,
                        field_pointer, view_mode, created_at, updated_at
                     )
                     SELECT $1, $2, $3, $4, $5, $6, $7, $7
                     WHERE EXISTS (
                         SELECT 1
                         FROM field_view_policy_audit_outbox
                         WHERE event_id = $8
                           AND idempotency_key = $9
                           AND request_hash = $10
                     )",
                ),
                params!(
                    input.role_id.clone(),
                    input.action_id.clone(),
                    input.resource_type.clone(),
                    input.schema_key.clone(),
                    field_pointer.clone(),
                    view_mode.to_owned(),
                    now.clone(),
                    event_id.clone(),
                    input.idempotency_key.clone(),
                    input.request_hash.clone()
                ),
            ));
        }
        self.runtime
            .transaction(queries)
            .await
            .context("replace role action field views transaction")?;

        self.field_view_idempotency_outcome(&input.idempotency_key, &input.request_hash)
            .await?
            .ok_or_else(|| {
                anyhow!(
                    "role action {} does not exist for role {} in organization {}",
                    input.action_id,
                    input.role_id,
                    input.organization_id
                )
            })
    }

    pub(crate) async fn field_view_idempotency_outcome(
        &self,
        idempotency_key: &str,
        request_hash: &str,
    ) -> Result<Option<ReplaceRoleActionFieldViewPolicyOutcome>> {
        let rows: Vec<FieldViewAuditRow> = self
            .query_rows_with(
                ReadConsistency::Strong,
                "SELECT request_hash, role_id, action_id, resource_type, schema_key, after_json
                 FROM field_view_policy_audit_outbox
                 WHERE idempotency_key = $1
                 LIMIT 1",
                params!(idempotency_key.to_owned()),
                "field view idempotency result",
            )
            .await?;
        let Some(row) = rows.into_iter().next() else {
            return Ok(None);
        };
        if row.request_hash != request_hash {
            return Ok(Some(
                ReplaceRoleActionFieldViewPolicyOutcome::IdempotencyConflict,
            ));
        }
        let stored: SerializableFieldViewPolicy = serde_json::from_str(&row.after_json)
            .context("deserialize stored field view policy result")?;
        if stored.role_id != row.role_id
            || stored.action_id != row.action_id
            || stored.resource_type != row.resource_type
            || stored.schema_key != row.schema_key
        {
            return Err(anyhow!(
                "stored field view policy replay scope does not match its audit row"
            ));
        }
        Ok(Some(ReplaceRoleActionFieldViewPolicyOutcome::Stored(
            stored.try_into()?,
        )))
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SerializableFieldViewPolicy {
    role_id: String,
    action_id: String,
    resource_type: String,
    schema_key: String,
    field_views: BTreeMap<String, domain::FieldViewMode>,
    updated_at: Option<String>,
}

impl From<&RoleActionFieldViewPolicy> for SerializableFieldViewPolicy {
    fn from(value: &RoleActionFieldViewPolicy) -> Self {
        Self {
            role_id: value.role_id.clone(),
            action_id: value.action_id.clone(),
            resource_type: value.resource_type.clone(),
            schema_key: value.schema_key.clone(),
            field_views: value.field_views.clone(),
            updated_at: value.updated_at.clone(),
        }
    }
}

impl TryFrom<SerializableFieldViewPolicy> for RoleActionFieldViewPolicy {
    type Error = anyhow::Error;

    fn try_from(value: SerializableFieldViewPolicy) -> Result<Self> {
        validate_stored_exact_identifier("actionId", &value.action_id, None)?;
        validate_stored_exact_identifier(
            "resourceType",
            &value.resource_type,
            Some(domain::MAX_FIELD_VIEW_RESOURCE_TYPE_BYTES),
        )?;
        validate_stored_exact_identifier(
            "schemaKey",
            &value.schema_key,
            Some(domain::MAX_FIELD_VIEW_SCHEMA_KEY_BYTES),
        )?;
        let field_views = normalize_stored_field_views(value.field_views)?;
        Ok(Self {
            role_id: value.role_id,
            action_id: value.action_id,
            resource_type: value.resource_type,
            schema_key: value.schema_key,
            field_views,
            updated_at: value.updated_at,
        })
    }
}

fn normalize_stored_field_views(
    field_views: BTreeMap<String, domain::FieldViewMode>,
) -> Result<BTreeMap<String, domain::FieldViewMode>> {
    let mut normalized = BTreeMap::new();
    for (field_pointer, mode) in field_views {
        if mode.as_positive_db_str().is_none() {
            return Err(anyhow!(
                "stored field view policy contains non-positive mode"
            ));
        }
        let field_pointer = normalize_stored_field_view_pointer(&field_pointer)?;
        if normalized.insert(field_pointer.clone(), mode).is_some() {
            return Err(anyhow!(
                "duplicate stored field view pointer {field_pointer}"
            ));
        }
    }
    Ok(normalized)
}

fn normalize_stored_field_view_pointer(field_pointer: &str) -> Result<String> {
    let field_pointer = domain::normalize_field_view_pointer(field_pointer)
        .map_err(|error| anyhow!("invalid stored field view pointer: {error}"))?;
    if field_pointer.len() > domain::MAX_FIELD_VIEW_POINTER_BYTES {
        return Err(anyhow!(
            "stored field view pointer must be at most {} bytes",
            domain::MAX_FIELD_VIEW_POINTER_BYTES
        ));
    }
    Ok(field_pointer)
}

fn validate_stored_exact_identifier(
    field_name: &str,
    value: &str,
    max_bytes: Option<usize>,
) -> Result<()> {
    if value.is_empty() || value.trim() != value {
        return Err(anyhow!("invalid stored field view policy {field_name}"));
    }
    if max_bytes.is_some_and(|max_bytes| value.len() > max_bytes) {
        return Err(anyhow!(
            "stored field view policy {field_name} exceeds its byte limit"
        ));
    }
    Ok(())
}

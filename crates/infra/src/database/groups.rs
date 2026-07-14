use super::core::now_string;
use super::*;

impl Database {
    pub async fn list_groups_with_consistency(
        &self,
        organization_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Vec<Group>> {
        let rows: Vec<GroupRow> = self
            .query_rows_with(
                self.validation_read_consistency_with_override(consistency),
                "SELECT id, organization_id, name, description
                 FROM groups
                 WHERE organization_id = $1
                 ORDER BY name ASC, id ASC",
                params!(organization_id.to_owned()),
                "groups by organization",
            )
            .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    pub async fn find_group_with_consistency(
        &self,
        group_id: &str,
        organization_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Option<Group>> {
        let rows: Vec<GroupRow> = self
            .query_rows_with(
                self.validation_read_consistency_with_override(consistency),
                "SELECT id, organization_id, name, description
                 FROM groups
                 WHERE id = $1 AND organization_id = $2
                 LIMIT 1",
                params!(group_id.to_owned(), organization_id.to_owned()),
                "group by id and organization",
            )
            .await?;
        Ok(rows.into_iter().next().map(Into::into))
    }

    pub async fn insert_group(&self, group: &Group) -> Result<()> {
        if !self
            .exists_row_with(
                ReadConsistency::Strong,
                "SELECT COUNT(*) AS count FROM organizations WHERE id = $1",
                params!(group.organization_id.clone()),
                "group organization existence",
            )
            .await?
        {
            return Err(anyhow!("organization not found: {}", group.organization_id));
        }
        let now = now_string();
        self.runtime
            .execute(
                "INSERT INTO groups (id, organization_id, name, description, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6)",
                params!(
                    group.id.clone(),
                    group.organization_id.clone(),
                    group.name.clone(),
                    group.description.clone(),
                    now.clone(),
                    now
                ),
            )
            .await
            .context("insert group")?;
        Ok(())
    }

    pub async fn update_group(&self, group: &Group) -> Result<()> {
        let updated = self
            .runtime
            .execute(
                "UPDATE groups
                 SET name = $1, description = $2, updated_at = $3
                 WHERE id = $4 AND organization_id = $5",
                params!(
                    group.name.clone(),
                    group.description.clone(),
                    now_string(),
                    group.id.clone(),
                    group.organization_id.clone()
                ),
            )
            .await
            .context("update group")?;
        if updated == 0 {
            return Err(anyhow!("group not found: {}", group.id));
        }
        Ok(())
    }

    /// Deletes the group and its memberships only when no role binding references it.
    /// Returns `false` when a binding prevented deletion.
    pub async fn delete_group_if_unbound(
        &self,
        group_id: &str,
        organization_id: &str,
    ) -> Result<bool> {
        let no_binding = "NOT EXISTS (
            SELECT 1 FROM role_bindings rb
            WHERE rb.subject_type = 'group'
              AND rb.subject_id = $1
        )";
        let applied = self
            .runtime
            .transaction([
                (
                    Cow::Owned(format!(
                        "DELETE FROM group_memberships
                         WHERE group_id = $1
                           AND EXISTS (
                               SELECT 1 FROM groups g
                               WHERE g.id = group_memberships.group_id
                                 AND g.organization_id = $2
                           )
                           AND {no_binding}"
                    )),
                    params!(group_id.to_owned(), organization_id.to_owned()),
                ),
                (
                    Cow::Owned(format!(
                        "DELETE FROM groups
                         WHERE id = $1 AND organization_id = $2 AND {no_binding}"
                    )),
                    params!(group_id.to_owned(), organization_id.to_owned()),
                ),
            ])
            .await
            .context("delete unbound group transaction")?;
        let [_, deleted_groups] = applied.as_slice() else {
            return Err(anyhow!(
                "delete unbound group transaction returned invalid results"
            ));
        };
        Ok(*deleted_groups == 1)
    }

    pub async fn list_group_members_with_consistency(
        &self,
        group_id: &str,
        organization_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Vec<GroupMember>> {
        let rows: Vec<GroupMemberRow> = self
            .query_rows_with(
                self.validation_read_consistency_with_override(consistency),
                "SELECT gm.group_id, gm.organization_membership_id, om.user_id
                 FROM groups g
                 JOIN group_memberships gm ON gm.group_id = g.id
                 JOIN organization_memberships om
                   ON om.id = gm.organization_membership_id
                  AND om.organization_id = g.organization_id
                 WHERE g.id = $1 AND g.organization_id = $2
                 ORDER BY om.user_id ASC, om.id ASC",
                params!(group_id.to_owned(), organization_id.to_owned()),
                "group members",
            )
            .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    pub async fn add_group_member_if_active(
        &self,
        group_id: &str,
        organization_id: &str,
        user_id: &str,
    ) -> Result<Option<GroupMember>> {
        self.runtime
            .execute(
                "INSERT INTO group_memberships (group_id, organization_membership_id)
                 SELECT g.id, om.id
                 FROM groups g
                 JOIN organization_memberships om ON om.organization_id = g.organization_id
                 JOIN users u ON u.id = om.user_id
                 WHERE g.id = $1
                   AND g.organization_id = $2
                   AND om.user_id = $3
                   AND om.status = 'active'
                   AND u.active = 1
                 ON CONFLICT(group_id, organization_membership_id) DO NOTHING",
                params!(
                    group_id.to_owned(),
                    organization_id.to_owned(),
                    user_id.to_owned()
                ),
            )
            .await
            .context("add active group member")?;
        let rows: Vec<GroupMemberRow> = self
            .query_rows_with(
                ReadConsistency::Strong,
                "SELECT gm.group_id, gm.organization_membership_id, om.user_id
                 FROM group_memberships gm
                 JOIN organization_memberships om ON om.id = gm.organization_membership_id
                 WHERE gm.group_id = $1
                   AND om.organization_id = $2
                   AND om.user_id = $3
                 LIMIT 1",
                params!(
                    group_id.to_owned(),
                    organization_id.to_owned(),
                    user_id.to_owned()
                ),
                "added group member",
            )
            .await?;
        Ok(rows.into_iter().next().map(Into::into))
    }

    pub async fn remove_group_member(
        &self,
        group_id: &str,
        organization_id: &str,
        user_id: &str,
    ) -> Result<()> {
        self.runtime
            .execute(
                "DELETE FROM group_memberships
                 WHERE group_id = $1
                   AND organization_membership_id IN (
                       SELECT id FROM organization_memberships
                       WHERE organization_id = $2 AND user_id = $3
                   )",
                params!(
                    group_id.to_owned(),
                    organization_id.to_owned(),
                    user_id.to_owned()
                ),
            )
            .await
            .context("remove group member")?;
        Ok(())
    }
}

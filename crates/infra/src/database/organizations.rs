use super::core::now_string;
use super::*;

impl Database {
    pub async fn get_organization(&self, organization_id: &str) -> Result<Organization> {
        let rows: Vec<OrganizationRow> = self
            .query_default_rows(
                "SELECT o.id, opl.party_id, o.business_no, o.name, o.inactivated_at
                 FROM organizations o
                 LEFT JOIN organization_party_links opl
                   ON opl.organization_id = o.id
                  AND opl.relation_type = 'legal'
                 WHERE o.id = $1
                 LIMIT 1",
                params!(organization_id.to_owned()),
                "organization",
            )
            .await
            .context("query organization")?;
        let row = rows
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("organization not found"))?;
        Ok(row.into())
    }

    pub async fn get_organization_business_no(
        &self,
        organization_id: &str,
    ) -> Result<Option<String>> {
        let rows: Vec<OrganizationRow> = self
            .query_default_rows(
                "SELECT o.id, opl.party_id, COALESCE(p.business_no, o.business_no) AS business_no, o.name, o.inactivated_at
                 FROM organizations o
                 LEFT JOIN organization_party_links opl
                   ON opl.organization_id = o.id
                  AND opl.relation_type = 'legal'
                 LEFT JOIN parties p ON p.id = opl.party_id
                 WHERE o.id = $1
                 LIMIT 1",
                params!(organization_id.to_owned()),
                "organization business number",
            )
            .await
            .context("query organization business number")?;
        Ok(rows.into_iter().next().and_then(|row| row.business_no))
    }

    pub async fn deactivate_organization(&self, organization_id: &str) -> Result<Organization> {
        let now = now_string();
        self.runtime
            .execute(
                "UPDATE organizations
                 SET inactivated_at = $1, updated_at = $2
                 WHERE id = $3",
                params!(now.clone(), now, organization_id.to_owned()),
            )
            .await
            .context("deactivate organization")?;
        self.get_organization(organization_id).await
    }

    pub async fn reactivate_organization(&self, organization_id: &str) -> Result<Organization> {
        self.runtime
            .execute(
                "UPDATE organizations
                 SET inactivated_at = $1, updated_at = $2
                 WHERE id = $3",
                params!(
                    Option::<String>::None,
                    now_string(),
                    organization_id.to_owned()
                ),
            )
            .await
            .context("reactivate organization")?;
        self.get_organization(organization_id).await
    }
}

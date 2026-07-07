use super::core::now_string;
use super::*;

impl Database {
    pub async fn upsert_agreement(&self, agreement: &Agreement) -> Result<()> {
        let now = now_string();
        self.runtime
            .execute(
                "INSERT INTO agreements (
                    id, agreement_type, grantor_organization_id, grantee_organization_id,
                    status, title, reference_no, starts_at, ends_at, created_at, updated_at
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
                 ON CONFLICT(id) DO UPDATE SET
                    agreement_type = excluded.agreement_type,
                    grantor_organization_id = excluded.grantor_organization_id,
                    grantee_organization_id = excluded.grantee_organization_id,
                    status = excluded.status,
                    title = excluded.title,
                    reference_no = excluded.reference_no,
                    starts_at = excluded.starts_at,
                    ends_at = excluded.ends_at,
                    updated_at = excluded.updated_at",
                params!(
                    agreement.id.clone(),
                    agreement_type_str(&agreement.agreement_type),
                    agreement.grantor_organization_id.clone(),
                    agreement.grantee_organization_id.clone(),
                    agreement_status_str(&agreement.status),
                    agreement.title.clone(),
                    agreement.reference_no.clone(),
                    agreement.starts_at.clone(),
                    agreement.ends_at.clone(),
                    now.clone(),
                    now
                ),
            )
            .await
            .context("upsert agreement")?;
        Ok(())
    }

    pub async fn list_agreements_for_organization_with_consistency(
        &self,
        organization_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Vec<Agreement>> {
        let rows: Vec<AgreementRow> = self
            .query_rows_with(
                self.validation_read_consistency_with_override(consistency),
                "SELECT id, agreement_type, grantor_organization_id, grantee_organization_id,
                        status, title, reference_no, starts_at, ends_at
                 FROM agreements
                 WHERE grantor_organization_id = $1 OR grantee_organization_id = $1
                 ORDER BY title ASC, id ASC",
                params!(organization_id.to_owned()),
                "agreements by organization",
            )
            .await?;
        rows.into_iter().map(TryInto::try_into).collect()
    }

    pub async fn get_agreement_with_consistency(
        &self,
        agreement_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Agreement> {
        self.find_agreement_by_id_with_consistency(agreement_id, consistency)
            .await?
            .ok_or_else(|| anyhow!("agreement not found: {agreement_id}"))
    }

    pub async fn find_agreement_by_id_with_consistency(
        &self,
        agreement_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Option<Agreement>> {
        let rows: Vec<AgreementRow> = self
            .query_rows_with(
                self.validation_read_consistency_with_override(consistency),
                "SELECT id, agreement_type, grantor_organization_id, grantee_organization_id,
                        status, title, reference_no, starts_at, ends_at
                 FROM agreements
                 WHERE id = $1
                 LIMIT 1",
                params!(agreement_id.to_owned()),
                "agreement by id",
            )
            .await?;
        rows.into_iter().next().map(TryInto::try_into).transpose()
    }
}

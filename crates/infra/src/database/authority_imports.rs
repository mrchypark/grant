use super::core::now_string;
use super::*;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

const PATCH_DATA_PROVIDER_ID: &str = "patch-data";
const PATCH_DATA_PLANT_READ_ACTION_ID: &str = "patch-data.plant.read";
const PATCH_DATA_PLANT_READER_ROLE_ID: &str = "role-patch-data-plant-reader";

#[derive(Debug)]
struct ImportReceiptRow {
    generation: String,
    ledger_digest: String,
    foundation_digest: String,
}

#[derive(Debug)]
struct ImportedHierarchyRow {
    parent_organization_id: String,
    child_organization_id: String,
    source_generation: String,
}

impl From<&mut Row<'_>> for ImportedHierarchyRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            parent_organization_id: row.get("parent_organization_id"),
            child_organization_id: row.get("child_organization_id"),
            source_generation: row.get("source_generation"),
        }
    }
}

#[derive(Debug)]
struct ImportCatalogStateRow {
    valid: i64,
}

impl From<&mut Row<'_>> for ImportCatalogStateRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            valid: row.get("valid"),
        }
    }
}

impl From<&mut Row<'_>> for ImportReceiptRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            generation: row.get("generation"),
            ledger_digest: row.get("ledger_digest"),
            foundation_digest: row.get("foundation_digest"),
        }
    }
}

impl Database {
    /// Records one exact rule-ledger generation and replaces the imported
    /// organization hierarchy snapshot atomically. This intentionally does
    /// not maintain a parallel user/permission authority: canonical Grant
    /// memberships and role bindings are populated by the next, ledger-shaped
    /// mapper once its exact source fields are frozen.
    pub async fn import_patch_data_authority_foundation(
        &self,
        command: PatchDataAuthorityImport,
    ) -> Result<PatchDataAuthorityImportResult> {
        validate_patch_data_authority_import(&command)?;
        let foundation_digest = patch_data_foundation_digest(&command.hierarchy_edges);
        let existing: Vec<ImportReceiptRow> = self
            .query_rows_with(
                ReadConsistency::Strong,
                "SELECT generation, ledger_digest, foundation_digest FROM patch_data_authority_imports WHERE provider_id=$1",
                params!(PATCH_DATA_PROVIDER_ID.to_owned()),
                "Patch Data authority import receipt",
            )
            .await?;
        if let Some(receipt) = existing.first()
            && receipt.generation == command.generation
            && receipt.ledger_digest == command.ledger_digest
            && receipt.foundation_digest == foundation_digest
        {
            self.verify_patch_data_authority_foundation(receipt).await?;
            return Ok(PatchDataAuthorityImportResult {
                applied: false,
                generation: receipt.generation.clone(),
                ledger_digest: receipt.ledger_digest.clone(),
            });
        }
        let previous_matches = match (
            existing.first(),
            command.expected_previous_generation.as_deref(),
        ) {
            (None, None) => true,
            (Some(receipt), Some(expected)) => receipt.generation == expected,
            _ => false,
        };
        if !previous_matches {
            anyhow::bail!("Patch Data authority import generation conflict");
        }

        let now = now_string();
        let assertion_id = Uuid::new_v4().simple().to_string();
        let catalog_assertion_id = Uuid::new_v4().simple().to_string();
        let receipt_assertion = match command.expected_previous_generation.as_deref() {
            None => (
                Cow::Borrowed(
                    "INSERT INTO authority_import_assertions (id,valid)
                     SELECT $1,CASE WHEN NOT EXISTS (SELECT 1 FROM patch_data_authority_imports WHERE provider_id=$2) THEN 1 ELSE 0 END",
                ),
                params!(assertion_id.clone(), PATCH_DATA_PROVIDER_ID.to_owned()),
            ),
            Some(expected) => (
                Cow::Borrowed(
                    "INSERT INTO authority_import_assertions (id,valid)
                     SELECT $1,CASE WHEN EXISTS (SELECT 1 FROM patch_data_authority_imports WHERE provider_id=$2 AND generation=$3) THEN 1 ELSE 0 END",
                ),
                params!(
                    assertion_id.clone(),
                    PATCH_DATA_PROVIDER_ID.to_owned(),
                    expected.to_owned()
                ),
            ),
        };
        let mut queries: Vec<(Cow<'static, str>, Vec<hiqlite::Param>)> = vec![
            receipt_assertion,
            (
                Cow::Borrowed(
                    "INSERT INTO authority_import_assertions (id,valid)
                     SELECT $1,CASE WHEN
                       NOT EXISTS (SELECT 1 FROM actions WHERE id=$2 AND (provider_id<>$3 OR code<>$2))
                       AND NOT EXISTS (SELECT 1 FROM actions WHERE provider_id=$3 AND code=$2 AND id<>$2)
                       AND NOT EXISTS (SELECT 1 FROM roles WHERE id=$4 AND (organization_id IS NOT NULL OR code<>$4))
                       AND NOT EXISTS (SELECT 1 FROM roles WHERE organization_id IS NULL AND code=$4 AND id<>$4)
                     THEN 1 ELSE 0 END",
                ),
                params!(
                    catalog_assertion_id.clone(),
                    PATCH_DATA_PLANT_READ_ACTION_ID.to_owned(),
                    PATCH_DATA_PROVIDER_ID.to_owned(),
                    PATCH_DATA_PLANT_READER_ROLE_ID.to_owned()
                ),
            ),
            (
                Cow::Borrowed(
                    "INSERT INTO patch_data_authority_imports (provider_id,generation,ledger_digest,foundation_digest,applied_at) VALUES ($1,$2,$3,$4,$5)
                     ON CONFLICT(provider_id) DO UPDATE SET generation=excluded.generation,ledger_digest=excluded.ledger_digest,foundation_digest=excluded.foundation_digest,applied_at=excluded.applied_at",
                ),
                params!(
                    PATCH_DATA_PROVIDER_ID.to_owned(),
                    command.generation.clone(),
                    command.ledger_digest.clone(),
                    foundation_digest,
                    now.clone()
                ),
            ),
            (
                Cow::Borrowed(
                    "DELETE FROM organization_hierarchy_edges WHERE source_provider=$1",
                ),
                params!(PATCH_DATA_PROVIDER_ID.to_owned()),
            ),
            (
                Cow::Borrowed(
                    "INSERT INTO service_providers (id,name,description,active,created_at,updated_at)
                     VALUES ($1,'Patch Data','Patch Data resources imported into Grant authority',1,$2,$2)
                     ON CONFLICT(id) DO UPDATE SET name=excluded.name,description=excluded.description,active=1,updated_at=excluded.updated_at",
                ),
                params!(PATCH_DATA_PROVIDER_ID.to_owned(), now.clone()),
            ),
            (
                Cow::Borrowed(
                    "INSERT INTO actions (id,provider_id,code,name,description,risk_level,visibility_granting,grantable_to_applications,consent_label,consent_description,created_at,updated_at)
                     VALUES ($1,$2,$1,'Read Patch Data plant','Read one exact Patch Data plant resource','low',0,1,NULL,NULL,$3,$3)
                     ON CONFLICT(id) DO UPDATE SET name=excluded.name,description=excluded.description,risk_level='low',visibility_granting=0,grantable_to_applications=1,updated_at=excluded.updated_at",
                ),
                params!(
                    PATCH_DATA_PLANT_READ_ACTION_ID.to_owned(),
                    PATCH_DATA_PROVIDER_ID.to_owned(),
                    now.clone()
                ),
            ),
            (
                Cow::Borrowed(
                    "INSERT INTO roles (id,organization_id,code,name,description,created_at,updated_at)
                     VALUES ($1,NULL,$1,'Patch Data plant reader','Imported Patch Data plant read permission',$2,$2)
                     ON CONFLICT(id) DO UPDATE SET name=excluded.name,description=excluded.description,updated_at=excluded.updated_at",
                ),
                params!(PATCH_DATA_PLANT_READER_ROLE_ID.to_owned(), now.clone()),
            ),
            (
                Cow::Borrowed(
                    "INSERT INTO role_actions (role_id,action_id) VALUES ($1,$2) ON CONFLICT(role_id,action_id) DO NOTHING",
                ),
                params!(
                    PATCH_DATA_PLANT_READER_ROLE_ID.to_owned(),
                    PATCH_DATA_PLANT_READ_ACTION_ID.to_owned()
                ),
            ),
        ];
        for edge in &command.hierarchy_edges {
            queries.push((
                Cow::Borrowed(
                    "INSERT INTO organization_hierarchy_edges (source_provider,parent_organization_id,child_organization_id,source_generation,created_at)
                     VALUES ($1,$2,$3,$4,$5)",
                ),
                params!(
                    PATCH_DATA_PROVIDER_ID.to_owned(),
                    edge.parent_organization_id.clone(),
                    edge.child_organization_id.clone(),
                    command.generation.clone(),
                    now.clone()
                ),
            ));
        }
        queries.push((
            Cow::Borrowed("DELETE FROM authority_import_assertions WHERE id IN ($1,$2)"),
            params!(assertion_id, catalog_assertion_id),
        ));
        self.runtime
            .transaction(queries)
            .await
            .context("import Patch Data authority foundation transaction")?;
        Ok(PatchDataAuthorityImportResult {
            applied: true,
            generation: command.generation,
            ledger_digest: command.ledger_digest,
        })
    }

    async fn verify_patch_data_authority_foundation(
        &self,
        receipt: &ImportReceiptRow,
    ) -> Result<()> {
        let rows: Vec<ImportedHierarchyRow> = self
            .query_rows_with(
                ReadConsistency::Strong,
                "SELECT parent_organization_id,child_organization_id,source_generation
                 FROM organization_hierarchy_edges WHERE source_provider=$1
                 ORDER BY parent_organization_id,child_organization_id",
                params!(PATCH_DATA_PROVIDER_ID.to_owned()),
                "Patch Data imported hierarchy",
            )
            .await?;
        if rows
            .iter()
            .any(|row| row.source_generation != receipt.generation)
        {
            anyhow::bail!("Patch Data authority import state drift");
        }
        let edges = rows
            .into_iter()
            .map(|row| PatchDataHierarchyEdge {
                parent_organization_id: row.parent_organization_id,
                child_organization_id: row.child_organization_id,
            })
            .collect::<Vec<_>>();
        if patch_data_foundation_digest(&edges) != receipt.foundation_digest {
            anyhow::bail!("Patch Data authority import state drift");
        }
        let catalog: Vec<ImportCatalogStateRow> = self
            .query_rows_with(
                ReadConsistency::Strong,
                "SELECT CASE WHEN
                   EXISTS (SELECT 1 FROM service_providers WHERE id=$1 AND name='Patch Data' AND description='Patch Data resources imported into Grant authority' AND active=1)
                   AND EXISTS (SELECT 1 FROM actions WHERE id=$2 AND provider_id=$1 AND code=$2 AND name='Read Patch Data plant' AND description='Read one exact Patch Data plant resource' AND risk_level='low' AND visibility_granting=0 AND grantable_to_applications=1 AND consent_label IS NULL AND consent_description IS NULL)
                   AND EXISTS (SELECT 1 FROM roles WHERE id=$3 AND organization_id IS NULL AND code=$3 AND name='Patch Data plant reader' AND description='Imported Patch Data plant read permission')
                   AND EXISTS (SELECT 1 FROM role_actions WHERE role_id=$3 AND action_id=$2)
                 THEN 1 ELSE 0 END AS valid",
                params!(
                    PATCH_DATA_PROVIDER_ID.to_owned(),
                    PATCH_DATA_PLANT_READ_ACTION_ID.to_owned(),
                    PATCH_DATA_PLANT_READER_ROLE_ID.to_owned()
                ),
                "Patch Data imported catalog",
            )
            .await?;
        if catalog.first().is_none_or(|row| row.valid != 1) {
            anyhow::bail!("Patch Data authority import state drift");
        }
        Ok(())
    }
}

fn patch_data_foundation_digest(edges: &[PatchDataHierarchyEdge]) -> String {
    let mut digest = Sha256::new();
    digest.update(b"grant.patch-data-authority-foundation.v1");
    for edge in edges {
        for value in [
            edge.parent_organization_id.as_bytes(),
            edge.child_organization_id.as_bytes(),
        ] {
            digest.update((value.len() as u64).to_be_bytes());
            digest.update(value);
        }
    }
    hex::encode(digest.finalize())
}

fn validate_patch_data_authority_import(command: &PatchDataAuthorityImport) -> Result<()> {
    validate_import_token("generation", &command.generation, 255)?;
    if command.ledger_digest.len() != 64
        || !command
            .ledger_digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        anyhow::bail!("ledgerDigest must be 64 lowercase hexadecimal characters");
    }
    if let Some(previous) = &command.expected_previous_generation {
        validate_import_token("expectedPreviousGeneration", previous, 255)?;
        if previous == &command.generation {
            anyhow::bail!("new generation must differ from expected previous generation");
        }
    }
    let mut previous_edge: Option<(&str, &str)> = None;
    let mut children = HashSet::new();
    let mut parents = HashMap::new();
    for edge in &command.hierarchy_edges {
        validate_import_token("parentOrganizationId", &edge.parent_organization_id, 255)?;
        validate_import_token("childOrganizationId", &edge.child_organization_id, 255)?;
        if edge.parent_organization_id == edge.child_organization_id {
            anyhow::bail!("organization hierarchy cannot contain a self edge");
        }
        let key = (
            edge.parent_organization_id.as_str(),
            edge.child_organization_id.as_str(),
        );
        if previous_edge.is_some_and(|previous| previous >= key) {
            anyhow::bail!("organization hierarchy edges must be sorted and unique");
        }
        previous_edge = Some(key);
        if !children.insert(edge.child_organization_id.as_str()) {
            anyhow::bail!("organization hierarchy child must have exactly one parent");
        }
        parents.insert(
            edge.child_organization_id.as_str(),
            edge.parent_organization_id.as_str(),
        );
    }
    for child in parents.keys() {
        let mut seen = HashSet::new();
        let mut cursor = *child;
        while let Some(parent) = parents.get(cursor) {
            if !seen.insert(cursor) {
                anyhow::bail!("organization hierarchy cannot contain a cycle");
            }
            cursor = parent;
        }
    }
    Ok(())
}

fn validate_import_token(field: &str, value: &str, max_bytes: usize) -> Result<()> {
    if value.is_empty()
        || value.len() > max_bytes
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        anyhow::bail!("{field} is invalid");
    }
    Ok(())
}

use super::core::now_string;
use super::*;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

const SCHEMA_VERSION: u64 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuthoritySnapshotV2 {
    pub schema_version: u64,
    pub source_id: String,
    pub generation: u64,
    pub source_revision: String,
    pub source_ledger_digest: String,
    pub source_watermark: String,
    pub produced_at: String,
    pub principals: Vec<AuthorityPrincipalV2>,
    pub organizations: Vec<AuthorityOrganizationV2>,
    pub memberships: Vec<AuthorityMembershipV2>,
    pub applications: Vec<AuthorityApplicationV2>,
    pub effective_bindings: Vec<EffectiveBindingV2>,
    pub snapshot_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuthorityPrincipalV2 {
    pub id: String,
    pub kind: String,
    pub active: bool,
    pub external_subject: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuthorityOrganizationV2 {
    pub id: String,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuthorityMembershipV2 {
    pub id: String,
    pub principal_id: String,
    pub organization_id: String,
    pub role: String,
    pub principal_type: String,
    pub lifecycle: String,
    pub valid_until: Option<String>,
    pub source_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuthorityApplicationV2 {
    pub id: String,
    pub organization_id: String,
    pub lifecycle: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplicationScopeV2 {
    pub kind: String,
    pub application_id: Option<String>,
    pub organization_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EffectiveBindingV2 {
    pub principal_id: String,
    pub application_scope: ApplicationScopeV2,
    pub provider_id: String,
    pub action_id: String,
    pub resource_type: String,
    pub resource_id: String,
    pub effect: String,
    pub source_rule_ids: Vec<String>,
    pub derivation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthoritySnapshotV2Result {
    pub applied: bool,
    pub schema_version: u64,
    pub source_id: String,
    pub generation: u64,
    pub prior_generation: Option<u64>,
    pub snapshot_digest: String,
    pub source_revision: String,
    pub principal_count: usize,
    pub organization_count: usize,
    pub membership_count: usize,
    pub application_count: usize,
    pub effective_binding_count: usize,
    pub applied_at: String,
}

#[derive(Debug)]
struct SnapshotRow {
    prior_generation: Option<i64>,
    snapshot_digest: String,
    canonical_json: String,
    applied_at: String,
}

impl From<&mut Row<'_>> for SnapshotRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            prior_generation: row.get("prior_generation"),
            snapshot_digest: row.get("snapshot_digest"),
            canonical_json: row.get("canonical_json"),
            applied_at: row.get("applied_at"),
        }
    }
}

impl Database {
    pub async fn import_authority_snapshot_v2(
        &self,
        snapshot: AuthoritySnapshotV2,
    ) -> Result<AuthoritySnapshotV2Result> {
        validate_snapshot(&snapshot)?;
        let canonical = canonical_snapshot_json(&snapshot)?;
        if snapshot.snapshot_digest != snapshot_digest(&snapshot)? {
            anyhow::bail!("snapshotDigest does not match canonical snapshot");
        }
        let existing: Vec<SnapshotRow> = self
            .query_rows_with(
                ReadConsistency::Strong,
                "SELECT prior_generation,snapshot_digest,canonical_json,applied_at FROM authority_snapshots_v2 WHERE source_id=$1 AND generation=$2",
                params!(snapshot.source_id.clone(), snapshot.generation as i64),
                "authority v2 generation",
            )
            .await?;
        if let Some(row) = existing.into_iter().next() {
            if row.snapshot_digest != snapshot.snapshot_digest || row.canonical_json != canonical {
                anyhow::bail!("authority v2 generation conflict");
            }
            return Ok(snapshot_result(
                false,
                &snapshot,
                row.prior_generation.map(|value| value as u64),
                &row.applied_at,
            ));
        }

        let assertion = Uuid::new_v4().simple().to_string();
        let applied_at = now_string();
        let applied = self.runtime
            .transaction([
                (
                    Cow::Borrowed(
                        "INSERT INTO authority_import_assertions (id,valid) SELECT $1,CASE WHEN
                         NOT EXISTS (SELECT 1 FROM authority_snapshots_v2 WHERE source_id=$2 AND generation=$3)
                         AND COALESCE((SELECT MAX(generation) FROM authority_snapshots_v2 WHERE source_id=$2),0)+1=$3
                         THEN 1 ELSE 0 END",
                    ),
                    params!(assertion.clone(), snapshot.source_id.clone(), snapshot.generation as i64),
                ),
                (
                    Cow::Borrowed(
                        "INSERT INTO authority_snapshots_v2
                         (source_id,generation,prior_generation,snapshot_digest,canonical_json,source_revision,source_ledger_digest,source_watermark,produced_at,principal_count,organization_count,membership_count,application_count,effective_binding_count,applied_at)
                         VALUES ($1,$2,(SELECT MAX(generation) FROM authority_snapshots_v2 WHERE source_id=$1),$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)",
                    ),
                    params!(
                        snapshot.source_id.clone(),
                        snapshot.generation as i64,
                        snapshot.snapshot_digest.clone(),
                        canonical.clone(),
                        snapshot.source_revision.clone(),
                        snapshot.source_ledger_digest.clone(),
                        snapshot.source_watermark.clone(),
                        snapshot.produced_at.clone(),
                        snapshot.principals.len() as i64,
                        snapshot.organizations.len() as i64,
                        snapshot.memberships.len() as i64,
                        snapshot.applications.len() as i64,
                        snapshot.effective_bindings.len() as i64,
                        applied_at.clone()
                    ),
                ),
                (
                    Cow::Borrowed("DELETE FROM authority_import_assertions WHERE id=$1"),
                    params!(assertion),
                ),
            ])
            .await;
        if let Err(error) = applied {
            // A byte-identical concurrent retry may lose the insertion race.
            // Re-read the committed row before classifying it as a conflict.
            let rows: Vec<SnapshotRow> = self
                .query_rows_with(
                    ReadConsistency::Strong,
                    "SELECT prior_generation,snapshot_digest,canonical_json,applied_at FROM authority_snapshots_v2 WHERE source_id=$1 AND generation=$2",
                    params!(snapshot.source_id.clone(), snapshot.generation as i64),
                    "concurrent authority v2 generation",
                )
                .await?;
            if let Some(row) = rows.into_iter().next()
                && row.snapshot_digest == snapshot.snapshot_digest
                && row.canonical_json == canonical_snapshot_json(&snapshot)?
            {
                return Ok(snapshot_result(
                    false,
                    &snapshot,
                    row.prior_generation.map(|value| value as u64),
                    &row.applied_at,
                ));
            }
            return Err(error).context("import authority v2 snapshot");
        }
        let stored: Vec<SnapshotRow> = self
            .query_rows_with(
                ReadConsistency::Strong,
                "SELECT prior_generation,snapshot_digest,canonical_json,applied_at FROM authority_snapshots_v2 WHERE source_id=$1 AND generation=$2",
                params!(snapshot.source_id.clone(), snapshot.generation as i64),
                "stored authority v2 generation",
            )
            .await?;
        let stored = stored
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("stored authority v2 generation missing"))?;
        Ok(snapshot_result(
            true,
            &snapshot,
            stored.prior_generation.map(|value| value as u64),
            &stored.applied_at,
        ))
    }
}

fn snapshot_result(
    applied: bool,
    snapshot: &AuthoritySnapshotV2,
    prior_generation: Option<u64>,
    applied_at: &str,
) -> AuthoritySnapshotV2Result {
    AuthoritySnapshotV2Result {
        applied,
        schema_version: snapshot.schema_version,
        source_id: snapshot.source_id.clone(),
        generation: snapshot.generation,
        prior_generation,
        snapshot_digest: snapshot.snapshot_digest.clone(),
        source_revision: snapshot.source_revision.clone(),
        principal_count: snapshot.principals.len(),
        organization_count: snapshot.organizations.len(),
        membership_count: snapshot.memberships.len(),
        application_count: snapshot.applications.len(),
        effective_binding_count: snapshot.effective_bindings.len(),
        applied_at: applied_at.to_owned(),
    }
}

pub fn snapshot_digest(snapshot: &AuthoritySnapshotV2) -> Result<String> {
    let mut value = serde_json::to_value(snapshot)?;
    value
        .as_object_mut()
        .expect("snapshot serializes as object")
        .remove("snapshotDigest");
    let bytes = serde_json::to_vec(&value)?;
    Ok(format!("sha256:{}", hex::encode(Sha256::digest(bytes))))
}

fn canonical_snapshot_json(snapshot: &AuthoritySnapshotV2) -> Result<String> {
    Ok(serde_json::to_string(&serde_json::to_value(snapshot)?)?)
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || b"._:-".contains(&b) || b == b'-'
        })
        && value.as_bytes()[0].is_ascii_alphanumeric()
}

fn valid_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn sorted_unique<T>(values: &[T], key: impl Fn(&T) -> String) -> bool {
    values.windows(2).all(|pair| key(&pair[0]) < key(&pair[1]))
}

fn validate_snapshot(snapshot: &AuthoritySnapshotV2) -> Result<()> {
    if snapshot.schema_version != SCHEMA_VERSION || snapshot.generation == 0 {
        anyhow::bail!("unsupported authority snapshot schema or generation");
    }
    if !valid_id(&snapshot.source_id) {
        anyhow::bail!("invalid authority snapshot sourceId");
    }
    if !valid_digest(&snapshot.source_ledger_digest)
        || !valid_digest(&snapshot.source_watermark)
        || !valid_digest(&snapshot.snapshot_digest)
    {
        anyhow::bail!("snapshot digests must use sha256:<64 lowercase hex>");
    }
    DateTime::parse_from_rfc3339(&snapshot.produced_at).context("invalid producedAt")?;
    if !snapshot.produced_at.ends_with('Z')
        || snapshot.source_revision.trim() != snapshot.source_revision
        || snapshot.source_revision.is_empty()
    {
        anyhow::bail!("invalid snapshot source metadata");
    }
    if !sorted_unique(&snapshot.principals, |v| v.id.clone())
        || !sorted_unique(&snapshot.organizations, |v| v.id.clone())
        || !sorted_unique(&snapshot.memberships, |v| v.id.clone())
        || !sorted_unique(&snapshot.applications, |v| v.id.clone())
        || !sorted_unique(&snapshot.effective_bindings, |v| {
            format!(
                "{}\0{}\0{}\0{}\0{}\0{}\0{}\0{}",
                v.principal_id,
                v.application_scope.organization_id,
                v.application_scope.kind,
                v.application_scope.application_id.as_deref().unwrap_or(""),
                v.provider_id,
                v.action_id,
                v.resource_type,
                v.resource_id
            )
        })
    {
        anyhow::bail!("snapshot arrays must be sorted and unique");
    }
    let principals = snapshot
        .principals
        .iter()
        .map(|v| (v.id.as_str(), v))
        .collect::<HashMap<_, _>>();
    let mut external_subjects = HashSet::new();
    let organizations = snapshot
        .organizations
        .iter()
        .map(|v| (v.id.as_str(), v))
        .collect::<HashMap<_, _>>();
    let applications = snapshot
        .applications
        .iter()
        .map(|v| (v.id.as_str(), v))
        .collect::<HashMap<_, _>>();
    for principal in &snapshot.principals {
        if !valid_id(&principal.id)
            || principal.kind.is_empty()
            || principal.kind.trim() != principal.kind
            || principal.external_subject.is_empty()
            || !external_subjects.insert(principal.external_subject.as_str())
        {
            anyhow::bail!("invalid principal");
        }
    }
    for organization in &snapshot.organizations {
        if !valid_id(&organization.id) {
            anyhow::bail!("invalid organization");
        }
    }
    let mut semantic_memberships = HashSet::new();
    let at = DateTime::parse_from_rfc3339(&snapshot.produced_at)?;
    for membership in &snapshot.memberships {
        if !valid_id(&membership.id)
            || membership.principal_type.is_empty()
            || membership.principal_type.trim() != membership.principal_type
        {
            anyhow::bail!("invalid membership identity");
        }
        let principal = principals
            .get(membership.principal_id.as_str())
            .ok_or_else(|| anyhow!("membership principal missing"))?;
        if !organizations.contains_key(membership.organization_id.as_str())
            || principal.kind != membership.principal_type
            || membership.source_refs.is_empty()
            || !sorted_unique(&membership.source_refs, Clone::clone)
        {
            anyhow::bail!("invalid membership references");
        }
        if membership.role.is_empty()
            || membership.role.trim() != membership.role
            || !matches!(
                membership.lifecycle.as_str(),
                "active" | "disabled" | "deleted" | "expired"
            )
            || !semantic_memberships.insert((
                &membership.principal_id,
                &membership.organization_id,
                &membership.role,
                &membership.principal_type,
            ))
        {
            anyhow::bail!("invalid membership semantics");
        }
        if membership.lifecycle == "active"
            && (!principal.active
                || !organizations
                    .get(membership.organization_id.as_str())
                    .is_some_and(|organization| organization.active))
        {
            anyhow::bail!("active membership references inactive authority");
        }
        let valid = match &membership.valid_until {
            None => true,
            Some(value) => {
                if !value.ends_with('Z') {
                    anyhow::bail!("membership validUntil must be UTC");
                }
                DateTime::parse_from_rfc3339(value)? > at
            }
        };
        if membership.lifecycle == "active" && !valid {
            anyhow::bail!("active membership is expired");
        }
    }
    for application in &snapshot.applications {
        if !valid_id(&application.id)
            || !organizations.contains_key(application.organization_id.as_str())
            || !matches!(
                application.lifecycle.as_str(),
                "active" | "disabled" | "deleted"
            )
        {
            anyhow::bail!("invalid application");
        }
    }
    for binding in &snapshot.effective_bindings {
        let _principal = principals
            .get(binding.principal_id.as_str())
            .filter(|v| v.active)
            .ok_or_else(|| anyhow!("binding principal inactive or missing"))?;
        let organization = organizations
            .get(binding.application_scope.organization_id.as_str())
            .filter(|v| v.active)
            .ok_or_else(|| anyhow!("binding organization inactive or missing"))?;
        if binding.effect != "allow"
            || binding.provider_id.is_empty()
            || binding.provider_id.trim() != binding.provider_id
            || binding.action_id.is_empty()
            || binding.action_id.trim() != binding.action_id
            || binding.resource_type.is_empty()
            || binding.resource_type.trim() != binding.resource_type
            || binding.resource_id.is_empty()
            || binding.resource_id.trim() != binding.resource_id
            || binding.source_rule_ids.is_empty()
            || !sorted_unique(&binding.source_rule_ids, Clone::clone)
            || binding
                .source_rule_ids
                .iter()
                .any(|value| value.is_empty() || value.trim() != value)
        {
            anyhow::bail!("invalid effective binding");
        }
        if binding.derivation.is_empty() || binding.derivation.trim() != binding.derivation {
            anyhow::bail!("invalid binding derivation");
        }
        match binding.application_scope.kind.as_str() {
            "exact" => {
                let app_id = binding
                    .application_scope
                    .application_id
                    .as_deref()
                    .ok_or_else(|| anyhow!("exact application missing"))?;
                let app = applications
                    .get(app_id)
                    .ok_or_else(|| anyhow!("exact application missing"))?;
                if app.lifecycle != "active" || app.organization_id != organization.id {
                    anyhow::bail!("exact application scope invalid");
                }
            }
            "wildcard" if binding.application_scope.application_id.is_none() => {}
            _ => anyhow::bail!("application scope invalid"),
        }
    }
    Ok(())
}

use super::core::now_string;
use super::*;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

const MANIFEST_VERSION: &str = "grant.patch-data-authority.v1";
const PROVIDER: &str = "patch-data";
const ACTION: &str = "patch-data.plant.read";
const ROLE: &str = "role-patch-data-plant-reader";
const MAX_ENTITIES: usize = 10_000;

#[derive(Debug)]
struct Receipt {
    manifest_version: String,
    generation: String,
    ledger_digest: String,
    foundation_digest: String,
    applied_at: String,
}

impl From<&mut Row<'_>> for Receipt {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            manifest_version: row.get("manifest_version"),
            generation: row.get("generation"),
            ledger_digest: row.get("ledger_digest"),
            foundation_digest: row.get("foundation_digest"),
            applied_at: row.get("applied_at"),
        }
    }
}

impl Database {
    /// Replace the complete imported Patch Data authority snapshot atomically.
    /// Steady-state authorization uses only Grant's canonical rows.
    pub async fn import_patch_data_authority(
        &self,
        command: PatchDataAuthorityImport,
    ) -> Result<PatchDataAuthorityImportResult> {
        validate(&command)?;
        if command.manifest_digest != patch_data_manifest_digest(&command) {
            anyhow::bail!("manifestDigest does not match canonical manifest");
        }
        let existing = self.receipt().await?;
        if let Some(receipt) = existing.as_ref()
            && receipt.manifest_version == command.manifest_version
            && receipt.generation == command.generation
            && receipt.ledger_digest == command.ledger_digest
            && receipt.foundation_digest == command.manifest_digest
        {
            self.verify_snapshot(&command).await?;
            return Ok(result(false, receipt));
        }
        let previous_matches = match (
            existing.as_ref(),
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
        let receipt_assertion = Uuid::new_v4().simple().to_string();
        let mut assertions = vec![receipt_guard(
            &receipt_assertion,
            command.expected_previous_generation.as_deref(),
        )];
        assertions.push((Cow::Borrowed(
            "INSERT INTO authority_import_assertions (id,valid) SELECT $1,CASE WHEN
             NOT EXISTS (SELECT 1 FROM actions WHERE id=$2 AND (provider_id<>$3 OR code<>$2))
             AND NOT EXISTS (SELECT 1 FROM actions WHERE provider_id=$3 AND code=$2 AND id<>$2)
             AND NOT EXISTS (SELECT 1 FROM roles WHERE id=$4 AND (organization_id IS NOT NULL OR code<>$4))
             AND NOT EXISTS (SELECT 1 FROM roles WHERE organization_id IS NULL AND code=$4 AND id<>$4)
             AND NOT EXISTS (SELECT 1 FROM group_memberships gm JOIN patch_data_authority_import_entities e ON e.entity_type='membership' AND e.entity_id=gm.organization_membership_id)
             THEN 1 ELSE 0 END"), params!(Uuid::new_v4().simple().to_string(), ACTION.to_owned(), PROVIDER.to_owned(), ROLE.to_owned())));

        for user in &command.users {
            let identity_id = identity_id(&user.external_user_id);
            assertions.push((Cow::Borrowed(
                "INSERT INTO authority_import_assertions (id,valid) SELECT $1,CASE WHEN
                 NOT EXISTS (SELECT 1 FROM user_identities WHERE provider=$2 AND external_user_id=$3 AND user_id<>$4)
                 AND NOT EXISTS (SELECT 1 FROM user_identities WHERE id=$5 AND (provider<>$2 OR external_user_id<>$3 OR user_id<>$4))
                 AND (NOT EXISTS (SELECT 1 FROM users WHERE id=$4)
                      OR EXISTS (SELECT 1 FROM patch_data_authority_import_entities WHERE entity_type='user' AND entity_id=$4))
                 THEN 1 ELSE 0 END"), params!(Uuid::new_v4().simple().to_string(), PROVIDER.to_owned(), user.external_user_id.clone(), user.user_id.clone(), identity_id)));
        }
        for org in &command.organizations {
            assertions.push((Cow::Borrowed(
                "INSERT INTO authority_import_assertions (id,valid) SELECT $1,CASE WHEN
                 NOT EXISTS (SELECT 1 FROM organizations WHERE id=$2)
                 OR (EXISTS (SELECT 1 FROM patch_data_authority_import_entities WHERE entity_type='organization' AND entity_id=$2)
                     AND EXISTS (SELECT 1 FROM organizations WHERE id=$2 AND business_no IS NULL))
                 THEN 1 ELSE 0 END"), params!(Uuid::new_v4().simple().to_string(), org.organization_id.clone())));
        }
        for membership in &command.memberships {
            assertions.push((Cow::Borrowed(
                "INSERT INTO authority_import_assertions (id,valid) SELECT $1,CASE WHEN
                 (NOT EXISTS (SELECT 1 FROM organization_memberships WHERE id=$2)
                  OR EXISTS (SELECT 1 FROM patch_data_authority_import_entities WHERE entity_type='membership' AND entity_id=$2))
                 AND NOT EXISTS (SELECT 1 FROM organization_memberships WHERE organization_id=$3 AND user_id=$4 AND id<>$2)
                THEN 1 ELSE 0 END"), params!(Uuid::new_v4().simple().to_string(), membership.membership_id.clone(), membership.organization_id.clone(), membership.user_id.clone())));
        }
        for binding in &command.plant_bindings {
            assertions.push((Cow::Borrowed(
                "INSERT INTO authority_import_assertions (id,valid) SELECT $1,CASE WHEN
                 NOT EXISTS (SELECT 1 FROM role_bindings WHERE id=$2)
                 OR EXISTS (SELECT 1 FROM patch_data_authority_import_entities WHERE entity_type='plant_binding' AND entity_id=$2)
                 THEN 1 ELSE 0 END"), params!(Uuid::new_v4().simple().to_string(), binding.binding_id.clone())));
        }
        for assignment in &command.application_assignments {
            assertions.push((Cow::Borrowed(
                "INSERT INTO authority_import_assertions (id,valid) SELECT $1,CASE WHEN
                 EXISTS (SELECT 1 FROM applications WHERE id=$2 AND organization_id=$3 AND active=1 AND provisioning_status='ready')
                 AND (NOT EXISTS (SELECT 1 FROM application_actions WHERE application_id=$2 AND action_id=$4)
                      OR EXISTS (SELECT 1 FROM patch_data_authority_import_entities WHERE entity_type='application_assignment' AND entity_id=$2))
                 THEN 1 ELSE 0 END"), params!(Uuid::new_v4().simple().to_string(), assignment.application_id.clone(), assignment.organization_id.clone(), ACTION.to_owned())));
        }

        let mut writes = vec![
            (
                Cow::Borrowed(
                    "DELETE FROM role_bindings WHERE id IN (SELECT entity_id FROM patch_data_authority_import_entities WHERE entity_type='plant_binding')",
                ),
                params!(),
            ),
            (
                Cow::Borrowed(
                    "DELETE FROM organization_memberships WHERE id IN (SELECT entity_id FROM patch_data_authority_import_entities WHERE entity_type='membership')",
                ),
                params!(),
            ),
            (
                Cow::Borrowed(
                    "DELETE FROM application_actions WHERE action_id=$1 AND application_id IN (SELECT entity_id FROM patch_data_authority_import_entities WHERE entity_type='application_assignment')",
                ),
                params!(ACTION.to_owned()),
            ),
            (
                Cow::Borrowed("DELETE FROM patch_data_authority_import_entities"),
                params!(),
            ),
            (
                Cow::Borrowed("DELETE FROM organization_hierarchy_edges WHERE source_provider=$1"),
                params!(PROVIDER.to_owned()),
            ),
            (
                Cow::Borrowed(
                    "INSERT INTO service_providers (id,name,description,active,created_at,updated_at) VALUES ($1,'Patch Data','Patch Data resources imported into Grant authority',1,$2,$2) ON CONFLICT(id) DO UPDATE SET name=excluded.name,description=excluded.description,active=1,updated_at=excluded.updated_at",
                ),
                params!(PROVIDER.to_owned(), now.clone()),
            ),
            (
                Cow::Borrowed(
                    "INSERT INTO actions (id,provider_id,code,name,description,risk_level,visibility_granting,grantable_to_applications,consent_label,consent_description,created_at,updated_at) VALUES ($1,$2,$1,'Read Patch Data plant','Read one exact Patch Data plant resource','low',0,1,NULL,NULL,$3,$3) ON CONFLICT(id) DO UPDATE SET name=excluded.name,description=excluded.description,risk_level='low',visibility_granting=0,grantable_to_applications=1,updated_at=excluded.updated_at",
                ),
                params!(ACTION.to_owned(), PROVIDER.to_owned(), now.clone()),
            ),
            (
                Cow::Borrowed(
                    "INSERT INTO roles (id,organization_id,code,name,description,created_at,updated_at) VALUES ($1,NULL,$1,'Patch Data plant reader','Imported Patch Data plant read permission',$2,$2) ON CONFLICT(id) DO UPDATE SET name=excluded.name,description=excluded.description,updated_at=excluded.updated_at",
                ),
                params!(ROLE.to_owned(), now.clone()),
            ),
            (
                Cow::Borrowed(
                    "INSERT INTO role_actions (role_id,action_id) VALUES ($1,$2) ON CONFLICT(role_id,action_id) DO NOTHING",
                ),
                params!(ROLE.to_owned(), ACTION.to_owned()),
            ),
        ];
        for user in &command.users {
            writes.push((Cow::Borrowed("INSERT INTO users (id,email,display_name,active,created_at) VALUES ($1,$2,$3,$4,$5) ON CONFLICT(id) DO UPDATE SET email=excluded.email,display_name=excluded.display_name,active=excluded.active"), params!(user.user_id.clone(), user.email.clone(), user.display_name.clone(), i64::from(user.active), now.clone())));
            writes.push((Cow::Borrowed("INSERT INTO user_identities (id,user_id,provider,external_user_id,email_snapshot,display_name_snapshot,last_seen_at,created_at,updated_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$7,$7) ON CONFLICT(id) DO UPDATE SET user_id=excluded.user_id,email_snapshot=excluded.email_snapshot,display_name_snapshot=excluded.display_name_snapshot,last_seen_at=excluded.last_seen_at,updated_at=excluded.updated_at"), params!(identity_id(&user.external_user_id), user.user_id.clone(), PROVIDER.to_owned(), user.external_user_id.clone(), user.email.clone(), Some(user.display_name.clone()), Some(now.clone()))));
            writes.push(entity("user", &user.user_id, &command.generation));
        }
        for org in &command.organizations {
            writes.push((Cow::Borrowed("INSERT INTO organizations (id,business_no,name,inactivated_at,created_at,updated_at) VALUES ($1,NULL,$2,$3,$4,$4) ON CONFLICT(id) DO UPDATE SET name=excluded.name,inactivated_at=excluded.inactivated_at,updated_at=excluded.updated_at"), params!(org.organization_id.clone(), org.name.clone(), (!org.active).then(|| now.clone()), now.clone())));
            writes.push(entity(
                "organization",
                &org.organization_id,
                &command.generation,
            ));
        }
        for membership in &command.memberships {
            writes.push((Cow::Borrowed("INSERT INTO organization_memberships (id,organization_id,user_id,membership_role,status,created_at,updated_at) VALUES ($1,$2,$3,$4,$5,$6,$6)"), params!(membership.membership_id.clone(), membership.organization_id.clone(), membership.user_id.clone(), membership_role_db(&membership.role).to_owned(), membership_status_db(&membership.status).to_owned(), now.clone())));
            writes.push(entity(
                "membership",
                &membership.membership_id,
                &command.generation,
            ));
        }
        for edge in &command.hierarchy_edges {
            writes.push((Cow::Borrowed("INSERT INTO organization_hierarchy_edges (source_provider,parent_organization_id,child_organization_id,source_generation,created_at) VALUES ($1,$2,$3,$4,$5)"), params!(PROVIDER.to_owned(), edge.parent_organization_id.clone(), edge.child_organization_id.clone(), command.generation.clone(), now.clone())));
        }
        for binding in &command.plant_bindings {
            writes.push((Cow::Borrowed("INSERT INTO role_bindings (id,subject_type,subject_id,subject_organization_id,provider_id,resource_type,resource_id,role_id,agreement_id,state,created_by_user_id,created_at,updated_at) VALUES ($1,$2,$3,$4,$5,'plant',$6,$7,NULL,$8,$9,$10,$10)"), params!(binding.binding_id.clone(), binding.subject_type.as_db_str().to_owned(), binding.subject_id.clone(), binding.subject_organization_id.clone(), PROVIDER.to_owned(), binding.plant_id.clone(), ROLE.to_owned(), binding.state.as_db_str().to_owned(), self.bootstrap_user_id.clone(), now.clone())));
            writes.push(entity(
                "plant_binding",
                &binding.binding_id,
                &command.generation,
            ));
        }
        for assignment in &command.application_assignments {
            writes.push((
                Cow::Borrowed(
                    "INSERT INTO application_actions (application_id,action_id) VALUES ($1,$2)",
                ),
                params!(assignment.application_id.clone(), ACTION.to_owned()),
            ));
            writes.push(entity(
                "application_assignment",
                &assignment.application_id,
                &command.generation,
            ));
        }
        writes.push((Cow::Borrowed("INSERT INTO patch_data_authority_imports (provider_id,generation,ledger_digest,foundation_digest,applied_at,manifest_version) VALUES ($1,$2,$3,$4,$5,$6) ON CONFLICT(provider_id) DO UPDATE SET generation=excluded.generation,ledger_digest=excluded.ledger_digest,foundation_digest=excluded.foundation_digest,applied_at=excluded.applied_at,manifest_version=excluded.manifest_version"), params!(PROVIDER.to_owned(), command.generation.clone(), command.ledger_digest.clone(), command.manifest_digest.clone(), now.clone(), command.manifest_version.clone())));
        writes.push((
            Cow::Borrowed("DELETE FROM authority_import_assertions"),
            params!(),
        ));
        assertions.extend(writes);
        self.runtime
            .transaction(assertions)
            .await
            .context("import Patch Data authority transaction")?;
        let receipt = self
            .receipt()
            .await?
            .ok_or_else(|| anyhow!("Patch Data authority import receipt missing"))?;
        self.verify_snapshot(&command).await?;
        Ok(result(true, &receipt))
    }

    pub async fn import_patch_data_authority_foundation(
        &self,
        command: PatchDataAuthorityImport,
    ) -> Result<PatchDataAuthorityImportResult> {
        self.import_patch_data_authority(command).await
    }

    async fn receipt(&self) -> Result<Option<Receipt>> {
        let rows: Vec<Receipt> = self.query_rows_with(ReadConsistency::Strong,
            "SELECT manifest_version,generation,ledger_digest,foundation_digest,applied_at FROM patch_data_authority_imports WHERE provider_id=$1",
            params!(PROVIDER.to_owned()), "Patch Data authority receipt").await?;
        Ok(rows.into_iter().next())
    }

    async fn verify_snapshot(&self, command: &PatchDataAuthorityImport) -> Result<()> {
        let expected = command.users.len()
            + command.organizations.len()
            + command.memberships.len()
            + command.plant_bindings.len()
            + command.application_assignments.len();
        let counts: Vec<CountRow> = self.query_rows_with(ReadConsistency::Strong,
            "SELECT COUNT(*) AS count FROM patch_data_authority_import_entities WHERE source_generation=$1",
            params!(command.generation.clone()), "Patch Data import entities").await?;
        let actual = counts.first().map_or(0, |row| row.count);
        if actual != expected as i64 {
            anyhow::bail!("Patch Data authority import state drift");
        }
        for user in &command.users {
            if !self.exists_row_with(ReadConsistency::Strong,
                "SELECT COUNT(*) AS count FROM users u JOIN user_identities ui ON ui.user_id=u.id WHERE u.id=$1 AND u.email IS $2 AND u.display_name=$3 AND u.active=$4 AND ui.provider=$5 AND ui.external_user_id=$6 AND EXISTS (SELECT 1 FROM patch_data_authority_import_entities e WHERE e.entity_type='user' AND e.entity_id=u.id AND e.source_generation=$7)",
                params!(user.user_id.clone(), user.email.clone(), user.display_name.clone(), i64::from(user.active), PROVIDER.to_owned(), user.external_user_id.clone(), command.generation.clone()), "Patch Data user verification").await? {
                anyhow::bail!("Patch Data authority import user state drift");
            }
        }
        for org in &command.organizations {
            let expected_active = org.active;
            if !self.exists_row_with(ReadConsistency::Strong,
                "SELECT COUNT(*) AS count FROM organizations o WHERE o.id=$1 AND o.business_no IS NULL AND o.name=$2 AND (($3=1 AND o.inactivated_at IS NULL) OR ($3=0 AND o.inactivated_at IS NOT NULL)) AND EXISTS (SELECT 1 FROM patch_data_authority_import_entities e WHERE e.entity_type='organization' AND e.entity_id=o.id AND e.source_generation=$4)",
                params!(org.organization_id.clone(), org.name.clone(), i64::from(expected_active), command.generation.clone()), "Patch Data organization verification").await? {
                anyhow::bail!("Patch Data authority import organization state drift");
            }
        }
        for membership in &command.memberships {
            if !self.exists_row_with(ReadConsistency::Strong,
                "SELECT COUNT(*) AS count FROM organization_memberships m WHERE m.id=$1 AND m.organization_id=$2 AND m.user_id=$3 AND m.membership_role=$4 AND m.status=$5 AND EXISTS (SELECT 1 FROM patch_data_authority_import_entities e WHERE e.entity_type='membership' AND e.entity_id=m.id AND e.source_generation=$6)",
                params!(membership.membership_id.clone(), membership.organization_id.clone(), membership.user_id.clone(), membership_role_db(&membership.role).to_owned(), membership_status_db(&membership.status).to_owned(), command.generation.clone()), "Patch Data membership verification").await? {
                anyhow::bail!("Patch Data authority import membership state drift");
            }
        }
        for binding in &command.plant_bindings {
            if !self.exists_row_with(ReadConsistency::Strong,
                "SELECT COUNT(*) AS count FROM role_bindings b WHERE b.id=$1 AND b.subject_type=$2 AND b.subject_id=$3 AND b.subject_organization_id=$4 AND b.provider_id=$5 AND b.resource_type='plant' AND b.resource_id=$6 AND b.role_id=$7 AND b.agreement_id IS NULL AND b.state=$8 AND EXISTS (SELECT 1 FROM patch_data_authority_import_entities e WHERE e.entity_type='plant_binding' AND e.entity_id=b.id AND e.source_generation=$9)",
                params!(binding.binding_id.clone(), binding.subject_type.as_db_str().to_owned(), binding.subject_id.clone(), binding.subject_organization_id.clone(), PROVIDER.to_owned(), binding.plant_id.clone(), ROLE.to_owned(), binding.state.as_db_str().to_owned(), command.generation.clone()), "Patch Data binding verification").await? {
                anyhow::bail!("Patch Data authority import binding state drift");
            }
        }
        for assignment in &command.application_assignments {
            if !self.exists_row_with(ReadConsistency::Strong,
                "SELECT COUNT(*) AS count FROM applications a WHERE a.id=$1 AND a.organization_id=$2 AND a.active=1 AND a.provisioning_status='ready' AND EXISTS (SELECT 1 FROM application_actions aa WHERE aa.application_id=a.id AND aa.action_id=$3) AND EXISTS (SELECT 1 FROM patch_data_authority_import_entities e WHERE e.entity_type='application_assignment' AND e.entity_id=a.id AND e.source_generation=$4)",
                params!(assignment.application_id.clone(), assignment.organization_id.clone(), ACTION.to_owned(), command.generation.clone()), "Patch Data application assignment verification").await? {
                anyhow::bail!("Patch Data authority import application assignment state drift");
            }
        }
        let hierarchy_counts: Vec<CountRow> = self.query_rows_with(ReadConsistency::Strong,
            "SELECT COUNT(*) AS count FROM organization_hierarchy_edges WHERE source_provider=$1 AND source_generation=$2",
            params!(PROVIDER.to_owned(), command.generation.clone()), "Patch Data hierarchy verification").await?;
        if hierarchy_counts.first().map_or(0, |row| row.count)
            != command.hierarchy_edges.len() as i64
        {
            anyhow::bail!("Patch Data authority import state drift");
        }
        for edge in &command.hierarchy_edges {
            if !self.exists_row_with(ReadConsistency::Strong,
                "SELECT COUNT(*) AS count FROM organization_hierarchy_edges WHERE source_provider=$1 AND source_generation=$2 AND parent_organization_id=$3 AND child_organization_id=$4",
                params!(PROVIDER.to_owned(), command.generation.clone(), edge.parent_organization_id.clone(), edge.child_organization_id.clone()), "Patch Data hierarchy edge verification").await? {
                anyhow::bail!("Patch Data authority import state drift");
            }
        }
        if !self.exists_row_with(ReadConsistency::Strong,
            "SELECT COUNT(*) AS count FROM service_providers sp JOIN actions a ON a.provider_id=sp.id JOIN role_actions ra ON ra.action_id=a.id JOIN roles r ON r.id=ra.role_id WHERE sp.id=$1 AND sp.active=1 AND a.id=$2 AND a.code=$2 AND a.grantable_to_applications=1 AND r.id=$3 AND r.organization_id IS NULL",
            params!(PROVIDER.to_owned(), ACTION.to_owned(), ROLE.to_owned()), "Patch Data catalog verification").await? {
            anyhow::bail!("Patch Data authority import state drift");
        }
        Ok(())
    }
}

fn result(applied: bool, receipt: &Receipt) -> PatchDataAuthorityImportResult {
    PatchDataAuthorityImportResult {
        applied,
        manifest_version: receipt.manifest_version.clone(),
        generation: receipt.generation.clone(),
        ledger_digest: receipt.ledger_digest.clone(),
        manifest_digest: receipt.foundation_digest.clone(),
        applied_at: receipt.applied_at.clone(),
    }
}

fn receipt_guard(id: &str, expected: Option<&str>) -> (Cow<'static, str>, Vec<hiqlite::Param>) {
    match expected {
        None => (
            Cow::Borrowed(
                "INSERT INTO authority_import_assertions (id,valid) SELECT $1,CASE WHEN NOT EXISTS (SELECT 1 FROM patch_data_authority_imports WHERE provider_id=$2) THEN 1 ELSE 0 END",
            ),
            params!(id.to_owned(), PROVIDER.to_owned()),
        ),
        Some(expected) => (
            Cow::Borrowed(
                "INSERT INTO authority_import_assertions (id,valid) SELECT $1,CASE WHEN EXISTS (SELECT 1 FROM patch_data_authority_imports WHERE provider_id=$2 AND generation=$3) THEN 1 ELSE 0 END",
            ),
            params!(id.to_owned(), PROVIDER.to_owned(), expected.to_owned()),
        ),
    }
}

fn entity(kind: &str, id: &str, generation: &str) -> (Cow<'static, str>, Vec<hiqlite::Param>) {
    (
        Cow::Borrowed(
            "INSERT INTO patch_data_authority_import_entities (entity_type,entity_id,source_generation) VALUES ($1,$2,$3)",
        ),
        params!(kind.to_owned(), id.to_owned(), generation.to_owned()),
    )
}

fn identity_id(external: &str) -> String {
    format!(
        "identity-patch-data-{}",
        hex::encode(Sha256::digest(external.as_bytes()))
    )
}

pub fn patch_data_manifest_digest(command: &PatchDataAuthorityImport) -> String {
    let mut d = Sha256::new();
    frame(&mut d, command.manifest_version.as_bytes());
    frame(&mut d, command.generation.as_bytes());
    frame(&mut d, command.ledger_digest.as_bytes());
    for u in &command.users {
        frame(&mut d, b"user");
        frame(&mut d, u.user_id.as_bytes());
        frame(&mut d, u.external_user_id.as_bytes());
        frame_opt(&mut d, u.email.as_deref());
        frame(&mut d, u.display_name.as_bytes());
        frame(&mut d, &[u8::from(u.active)]);
    }
    for o in &command.organizations {
        frame(&mut d, b"organization");
        frame(&mut d, o.organization_id.as_bytes());
        frame(&mut d, o.name.as_bytes());
        frame(&mut d, &[u8::from(o.active)]);
    }
    for m in &command.memberships {
        frame(&mut d, b"membership");
        frame(&mut d, m.membership_id.as_bytes());
        frame(&mut d, m.organization_id.as_bytes());
        frame(&mut d, m.user_id.as_bytes());
        frame(&mut d, membership_role_db(&m.role).as_bytes());
        frame(&mut d, membership_status_db(&m.status).as_bytes());
    }
    for e in &command.hierarchy_edges {
        frame(&mut d, b"hierarchy");
        frame(&mut d, e.parent_organization_id.as_bytes());
        frame(&mut d, e.child_organization_id.as_bytes());
    }
    for b in &command.plant_bindings {
        frame(&mut d, b"plant_binding");
        frame(&mut d, b.binding_id.as_bytes());
        frame(&mut d, b.subject_type.as_db_str().as_bytes());
        frame(&mut d, b.subject_id.as_bytes());
        frame(&mut d, b.subject_organization_id.as_bytes());
        frame(&mut d, b.plant_id.as_bytes());
        frame(&mut d, b.state.as_db_str().as_bytes());
    }
    for assignment in &command.application_assignments {
        frame(&mut d, b"application_assignment");
        frame(&mut d, assignment.application_id.as_bytes());
        frame(&mut d, assignment.organization_id.as_bytes());
    }
    hex::encode(d.finalize())
}
fn frame(d: &mut Sha256, v: &[u8]) {
    d.update((v.len() as u64).to_be_bytes());
    d.update(v);
}
fn frame_opt(d: &mut Sha256, v: Option<&str>) {
    match v {
        Some(v) => {
            d.update([1]);
            frame(d, v.as_bytes());
        }
        None => d.update([0]),
    }
}

fn membership_role_db(value: &MembershipRole) -> &'static str {
    match value {
        MembershipRole::Owner => "owner",
        MembershipRole::Admin => "admin",
        MembershipRole::Member => "member",
    }
}
fn membership_status_db(value: &MembershipStatus) -> &'static str {
    match value {
        MembershipStatus::Active => "active",
        MembershipStatus::Disabled => "disabled",
    }
}

fn validate(c: &PatchDataAuthorityImport) -> Result<()> {
    if c.manifest_version != MANIFEST_VERSION {
        anyhow::bail!("unsupported manifestVersion");
    }
    token("generation", &c.generation, 255)?;
    digest("ledgerDigest", &c.ledger_digest)?;
    digest("manifestDigest", &c.manifest_digest)?;
    if let Some(p) = &c.expected_previous_generation {
        token("expectedPreviousGeneration", p, 255)?;
        if p == &c.generation {
            anyhow::bail!("new generation must differ from expected previous generation");
        }
    }
    if c.users.len()
        + c.organizations.len()
        + c.memberships.len()
        + c.hierarchy_edges.len()
        + c.plant_bindings.len()
        + c.application_assignments.len()
        > MAX_ENTITIES
    {
        anyhow::bail!("authority manifest exceeds entity limit");
    }
    let mut users = HashSet::new();
    let mut exts = HashSet::new();
    let mut prev = None;
    for u in &c.users {
        token("userId", &u.user_id, 255)?;
        token("externalUserId", &u.external_user_id, 255)?;
        token("displayName", &u.display_name, 255)?;
        if let Some(e) = &u.email {
            token("email", e, 320)?;
        }
        let key = (u.user_id.as_str(), u.external_user_id.as_str());
        if prev.is_some_and(|p| p >= key)
            || !users.insert(u.user_id.as_str())
            || !exts.insert(u.external_user_id.as_str())
        {
            anyhow::bail!("users must be sorted and unique");
        }
        prev = Some(key);
    }
    let mut orgs = HashSet::new();
    let mut prev = None;
    for o in &c.organizations {
        token("organizationId", &o.organization_id, 255)?;
        token("organizationName", &o.name, 255)?;
        if prev.is_some_and(|p| p >= o.organization_id.as_str())
            || !orgs.insert(o.organization_id.as_str())
        {
            anyhow::bail!("organizations must be sorted and unique");
        }
        prev = Some(o.organization_id.as_str());
    }
    let mut active = HashSet::new();
    let mut pairs = HashSet::new();
    let mut prev = None;
    for m in &c.memberships {
        token("membershipId", &m.membership_id, 255)?;
        if !orgs.contains(m.organization_id.as_str()) || !users.contains(m.user_id.as_str()) {
            anyhow::bail!("membership references an unknown user or organization");
        }
        if prev.is_some_and(|p| p >= m.membership_id.as_str())
            || !pairs.insert((m.organization_id.as_str(), m.user_id.as_str()))
        {
            anyhow::bail!("memberships must be sorted and unique");
        }
        if m.status == MembershipStatus::Active {
            active.insert((m.organization_id.as_str(), m.user_id.as_str()));
        }
        prev = Some(m.membership_id.as_str());
    }
    let mut children = HashSet::new();
    let mut parents = HashMap::new();
    let mut prev = None;
    for e in &c.hierarchy_edges {
        if !orgs.contains(e.parent_organization_id.as_str())
            || !orgs.contains(e.child_organization_id.as_str())
        {
            anyhow::bail!("organization hierarchy references an unknown organization");
        }
        let key = (
            e.parent_organization_id.as_str(),
            e.child_organization_id.as_str(),
        );
        if key.0 == key.1 || prev.is_some_and(|p| p >= key) || !children.insert(key.1) {
            anyhow::bail!("organization hierarchy is invalid");
        }
        parents.insert(key.1, key.0);
        prev = Some(key);
    }
    for child in parents.keys() {
        let mut seen = HashSet::new();
        let mut cur = *child;
        while let Some(parent) = parents.get(cur) {
            if !seen.insert(cur) {
                anyhow::bail!("organization hierarchy cannot contain a cycle");
            }
            cur = parent;
        }
    }
    let mut prev = None;
    for b in &c.plant_bindings {
        token("bindingId", &b.binding_id, 255)?;
        token("plantId", &b.plant_id, 255)?;
        if prev.is_some_and(|p| p >= b.binding_id.as_str()) {
            anyhow::bail!("plantBindings must be sorted and unique");
        }
        match b.subject_type {
            RoleBindingSubjectType::User
                if !active
                    .contains(&(b.subject_organization_id.as_str(), b.subject_id.as_str())) =>
            {
                anyhow::bail!("user plant binding requires an active imported membership")
            }
            RoleBindingSubjectType::Organization
                if b.subject_id != b.subject_organization_id
                    || !orgs.contains(b.subject_id.as_str()) =>
            {
                anyhow::bail!("organization plant binding subject is invalid")
            }
            RoleBindingSubjectType::Group => {
                anyhow::bail!("group plant bindings are not supported by manifest v1")
            }
            _ => {}
        }
        prev = Some(b.binding_id.as_str());
    }
    let mut previous_assignment = None;
    for assignment in &c.application_assignments {
        token("applicationId", &assignment.application_id, 255)?;
        if !orgs.contains(assignment.organization_id.as_str()) {
            anyhow::bail!("application assignment references an unknown organization");
        }
        if previous_assignment
            .is_some_and(|previous| previous >= assignment.application_id.as_str())
        {
            anyhow::bail!("applicationAssignments must be sorted and unique");
        }
        previous_assignment = Some(assignment.application_id.as_str());
    }
    Ok(())
}
fn digest(field: &str, v: &str) -> Result<()> {
    if v.len() != 64
        || !v
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        anyhow::bail!("{field} must be 64 lowercase hexadecimal characters");
    }
    Ok(())
}
fn token(field: &str, v: &str, max: usize) -> Result<()> {
    if v.is_empty() || v.len() > max || v.trim() != v || v.chars().any(char::is_control) {
        anyhow::bail!("{field} is invalid");
    }
    Ok(())
}

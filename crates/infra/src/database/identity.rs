use super::core::now_string;
use super::*;

#[cfg(test)]
use std::{collections::VecDeque, sync::Mutex};

#[cfg(test)]
static MATERIALIZATION_CANDIDATE_IDS: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());

fn materialization_candidate_id() -> String {
    #[cfg(test)]
    if let Some(candidate) = MATERIALIZATION_CANDIDATE_IDS
        .lock()
        .expect("materialization candidate override lock")
        .pop_front()
    {
        return candidate;
    }
    Uuid::new_v4().simple().to_string()
}

impl Database {
    /// Materialize a verified external identity without granting it any local
    /// organization access.  Linkage is exclusively provider + external id.
    pub async fn materialize_external_identity(
        &self,
        identity: ExternalIdentityMaterialization<'_>,
    ) -> Result<User> {
        let existing = self
            .resolve_existing_user_identity_with_consistency(
                identity.provider,
                identity.external_user_id,
                Some(ReadConsistency::Strong),
            )
            .await?;
        if let Some(user) = existing {
            return Ok(user);
        }
        if let Some(email) = identity.email
            && self
                .find_user_by_identity_or_email(
                    identity.provider,
                    email,
                    Some(ReadConsistency::Strong),
                )
                .await?
                .is_some()
        {
            return Err(ExternalOwnerBootstrapError::PreconditionsChanged.into());
        }
        let user_id = materialization_candidate_id();
        let identity_id = materialization_candidate_id();
        let now = now_string();
        let results = self.runtime.transaction([
            (Cow::Borrowed("INSERT INTO users (id, email, display_name, active, created_at) SELECT $1, $2, $3, 1, $4 WHERE NOT EXISTS (SELECT 1 FROM users WHERE id = $1) AND NOT EXISTS (SELECT 1 FROM user_identities WHERE provider = $5 AND external_user_id = $6) AND ($7 IS NULL OR NOT EXISTS (SELECT 1 FROM users WHERE email = $7))"), params!(user_id.clone(), identity.email.map(ToOwned::to_owned), identity.display_name.to_owned(), now.clone(), identity.provider.to_owned(), identity.external_user_id.to_owned(), identity.email.map(ToOwned::to_owned))),
            // `changes()` is the preceding user INSERT result.  It prevents a
            // UUID candidate collision from attaching a new external identity
            // to an unrelated pre-existing local user.
            (Cow::Borrowed("INSERT INTO user_identities (id, user_id, provider, external_user_id, email_snapshot, display_name_snapshot, last_seen_at, created_at, updated_at) SELECT $1, $2, $3, $4, $5, $6, $7, $7, $7 WHERE changes() = 1 AND EXISTS (SELECT 1 FROM users WHERE id = $2) AND NOT EXISTS (SELECT 1 FROM user_identities WHERE provider = $3 AND external_user_id = $4)"), params!(identity_id, user_id.clone(), identity.provider.to_owned(), identity.external_user_id.to_owned(), identity.email.map(ToOwned::to_owned), Some(identity.display_name.to_owned()), Some(now))),
        ]).await.context("materialize external identity transaction")?;
        if results.first().copied().unwrap_or_default() == 0
            || results.get(1).copied().unwrap_or_default() == 0
        {
            return Err(ExternalOwnerBootstrapError::PreconditionsChanged.into());
        }
        self.find_user_by_id(&user_id, Some(ReadConsistency::Strong))
            .await?
            .ok_or_else(|| anyhow!("materialized user missing"))
    }

    /// Atomically establishes the first owner from a provider-verified
    /// identity. Every mutation repeats the complete admission predicate, so
    /// a zero-row binding can never leave a newly-created membership behind.
    pub async fn bootstrap_external_owner_atomic(
        &self,
        command: ExternalOwnerBootstrapCommand<'_>,
    ) -> Result<ExternalOwnerBootstrapResult> {
        let identity = command.identity;
        let mapped = self
            .resolve_existing_user_identity_with_consistency(
                identity.provider,
                identity.external_user_id,
                Some(ReadConsistency::Strong),
            )
            .await?;
        if mapped.is_none()
            && identity.email.is_some()
            && self
                .find_user_by_identity_or_email(
                    identity.provider,
                    identity.email.unwrap(),
                    Some(ReadConsistency::Strong),
                )
                .await?
                .is_some()
        {
            return Err(ExternalOwnerBootstrapError::PreconditionsChanged.into());
        }
        let user_id = mapped
            .map(|u| u.id)
            .unwrap_or_else(|| Uuid::new_v4().simple().to_string());
        let identity_id = Uuid::new_v4().simple().to_string();
        let membership_id = Uuid::new_v4().simple().to_string();
        let binding_id = Uuid::new_v4().simple().to_string();
        let now = now_string();
        // This expression is repeated for each write. Existing successful
        // state is admitted, while every incompatible state is a no-op.
        let admitted = "EXISTS (SELECT 1 FROM applications a WHERE a.id = $APP AND a.organization_id = $ORG AND a.active = 1 AND a.provisioning_status = 'ready') AND (NOT EXISTS (SELECT 1 FROM organizations o WHERE o.id = $ORG) OR EXISTS (SELECT 1 FROM organizations o WHERE o.id = $ORG AND o.inactivated_at IS NULL)) AND EXISTS (SELECT 1 FROM service_providers WHERE id = 'grant' AND active = 1) AND EXISTS (SELECT 1 FROM roles WHERE id = 'role-system-bootstrap-owner') AND NOT EXISTS (SELECT 1 FROM user_identities ui WHERE ui.provider = $PROVIDER AND ui.external_user_id = $EXTERNAL AND ui.user_id <> $USER) AND (NOT EXISTS (SELECT 1 FROM users u WHERE u.id = $USER) OR EXISTS (SELECT 1 FROM users u WHERE u.id = $USER AND u.active = 1)) AND NOT EXISTS (SELECT 1 FROM organization_memberships m WHERE m.organization_id = $ORG AND m.user_id = $USER AND (m.membership_role <> 'owner' OR m.status <> 'active')) AND NOT EXISTS (SELECT 1 FROM organization_memberships m WHERE m.organization_id = $ORG AND m.user_id <> $USER AND m.membership_role = 'owner') AND NOT EXISTS (SELECT 1 FROM role_bindings b WHERE b.subject_type = 'user' AND b.subject_id = $USER AND b.subject_organization_id = $ORG AND b.provider_id = 'grant' AND b.resource_type = 'organization' AND b.resource_id = $ORG AND b.role_id = 'role-system-bootstrap-owner' AND b.state <> 'active')";
        let predicate_for =
            |app: usize, org: usize, provider: usize, external: usize, user: usize| {
                admitted
                    .replace("$APP", &format!("${app}"))
                    .replace("$ORG", &format!("${org}"))
                    .replace("$PROVIDER", &format!("${provider}"))
                    .replace("$EXTERNAL", &format!("${external}"))
                    .replace("$USER", &format!("${user}"))
            };
        // Parameter numbers in each statement are local; write them explicitly
        // to keep the conditional SQL auditable.
        let q_user = format!(
            "INSERT INTO users (id,email,display_name,active,created_at) SELECT $1,$2,$3,1,$4 WHERE {} AND NOT EXISTS (SELECT 1 FROM users WHERE id=$1)",
            predicate_for(5, 6, 7, 8, 9)
        );
        let q_identity = format!(
            "INSERT INTO user_identities (id,user_id,provider,external_user_id,email_snapshot,display_name_snapshot,last_seen_at,created_at,updated_at) SELECT $1,$2,$3,$4,$5,$6,$7,$7,$7 WHERE {} AND NOT EXISTS (SELECT 1 FROM user_identities WHERE provider=$3 AND external_user_id=$4)",
            predicate_for(8, 9, 10, 11, 12)
        );
        let identity_linked = "EXISTS (SELECT 1 FROM user_identities WHERE provider=$5 AND external_user_id=$6 AND user_id=$7)";
        let q_org = format!(
            "INSERT INTO organizations (id,business_no,name,inactivated_at,created_at,updated_at) SELECT $1,NULL,$1,NULL,$2,$2 WHERE {} AND {} AND NOT EXISTS (SELECT 1 FROM organizations WHERE id=$1)",
            predicate_for(3, 4, 5, 6, 7),
            identity_linked
        );
        let q_member = format!(
            "INSERT INTO organization_memberships (id,organization_id,user_id,membership_role,status,created_at,updated_at) SELECT $1,$2,$3,'owner','active',$4,$4 WHERE {} AND EXISTS (SELECT 1 FROM user_identities WHERE provider=$7 AND external_user_id=$8 AND user_id=$9) AND NOT EXISTS (SELECT 1 FROM organization_memberships WHERE organization_id=$2 AND user_id=$3)",
            predicate_for(5, 6, 7, 8, 9)
        );
        let q_binding = format!(
            "INSERT INTO role_bindings (id,subject_type,subject_id,subject_organization_id,provider_id,resource_type,resource_id,role_id,agreement_id,state,created_by_user_id,created_at,updated_at) SELECT $1,'user',$2,$3,'grant','organization',$3,'role-system-bootstrap-owner',NULL,'active',$2,$4,$4 WHERE {} AND EXISTS (SELECT 1 FROM user_identities WHERE provider=$7 AND external_user_id=$8 AND user_id=$9) AND NOT EXISTS (SELECT 1 FROM role_bindings WHERE subject_type='user' AND subject_id=$2 AND subject_organization_id=$3 AND provider_id='grant' AND resource_type='organization' AND resource_id=$3 AND role_id='role-system-bootstrap-owner')",
            predicate_for(5, 6, 7, 8, 9)
        );
        // A trigger or an unexpected storage condition can turn the binding
        // insert into a no-op after the membership write.  Make that state a
        // transaction error so Hiqlite rolls back every preceding statement.
        // The duplicate primary-key insert is deliberately unreachable once
        // the required active binding exists.
        let q_binding_guard = "INSERT INTO users (id,email,display_name,active,created_at) SELECT id,email,display_name,active,created_at FROM users WHERE id=$1 AND NOT EXISTS (SELECT 1 FROM role_bindings WHERE subject_type='user' AND subject_id=$2 AND subject_organization_id=$3 AND provider_id='grant' AND resource_type='organization' AND resource_id=$3 AND role_id='role-system-bootstrap-owner' AND state='active')";
        if self
            .runtime
            .transaction([
                (
                    Cow::Owned(q_user),
                    params!(
                        user_id.clone(),
                        identity.email.map(ToOwned::to_owned),
                        identity.display_name.to_owned(),
                        now.clone(),
                        command.application_id.to_owned(),
                        command.organization_id.to_owned(),
                        identity.provider.to_owned(),
                        identity.external_user_id.to_owned(),
                        user_id.clone()
                    ),
                ),
                (
                    Cow::Owned(q_identity),
                    params!(
                        identity_id,
                        user_id.clone(),
                        identity.provider.to_owned(),
                        identity.external_user_id.to_owned(),
                        identity.email.map(ToOwned::to_owned),
                        Some(identity.display_name.to_owned()),
                        Some(now.clone()),
                        command.application_id.to_owned(),
                        command.organization_id.to_owned(),
                        identity.provider.to_owned(),
                        identity.external_user_id.to_owned(),
                        user_id.clone()
                    ),
                ),
                (
                    Cow::Owned(q_org),
                    params!(
                        command.organization_id.to_owned(),
                        now.clone(),
                        command.application_id.to_owned(),
                        command.organization_id.to_owned(),
                        identity.provider.to_owned(),
                        identity.external_user_id.to_owned(),
                        user_id.clone()
                    ),
                ),
                (
                    Cow::Owned(q_member),
                    params!(
                        membership_id,
                        command.organization_id.to_owned(),
                        user_id.clone(),
                        now.clone(),
                        command.application_id.to_owned(),
                        command.organization_id.to_owned(),
                        identity.provider.to_owned(),
                        identity.external_user_id.to_owned(),
                        user_id.clone()
                    ),
                ),
                (
                    Cow::Owned(q_binding),
                    params!(
                        binding_id,
                        user_id.clone(),
                        command.organization_id.to_owned(),
                        now,
                        command.application_id.to_owned(),
                        command.organization_id.to_owned(),
                        identity.provider.to_owned(),
                        identity.external_user_id.to_owned(),
                        user_id.clone()
                    ),
                ),
                (
                    Cow::Borrowed(q_binding_guard),
                    params!(
                        self.bootstrap_user_id.clone(),
                        user_id.clone(),
                        command.organization_id.to_owned()
                    ),
                ),
            ])
            .await
            .is_err()
        {
            // All admission races and postcondition-guard failures are an
            // externally indistinguishable bootstrap conflict.
            return Err(ExternalOwnerBootstrapError::PreconditionsChanged.into());
        }
        // The transaction is the admission point.  Do not re-check mutable
        // application, membership, or binding state after it commits: an
        // immediately concurrent change must not turn a completed bootstrap
        // into an externally reported failure. `user_id` is either the
        // strong-read canonical mapping or the candidate guarded by the exact
        // provider/external-id predicate in every transactional write.
        Ok(ExternalOwnerBootstrapResult {
            user_id,
            organization_id: command.organization_id.to_owned(),
        })
    }
    pub async fn upsert_organization_with_owner(
        &self,
        organization_id: &str,
        display_name: &str,
        business_no: Option<&str>,
        legal_name: &str,
    ) -> Result<Organization> {
        let now = now_string();
        self.runtime
            .execute(
                "INSERT INTO organizations (id, business_no, name, inactivated_at, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT(id) DO UPDATE SET
                    business_no = COALESCE(excluded.business_no, organizations.business_no),
                    name = excluded.name,
                    updated_at = excluded.updated_at",
                params!(
                    organization_id.to_owned(),
                    business_no.map(ToOwned::to_owned),
                    display_name.to_owned(),
                    Option::<String>::None,
                    now.clone(),
                    now.clone()
                ),
            )
            .await
            .context("upsert organization")?;
        self.runtime
            .execute(
                "INSERT INTO organization_memberships (id, organization_id, user_id, membership_role, status, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)
                 ON CONFLICT(organization_id, user_id) DO UPDATE SET membership_role = excluded.membership_role, status = excluded.status, updated_at = excluded.updated_at",
                params!(
                    Uuid::new_v4().simple().to_string(),
                    organization_id.to_owned(),
                    self.bootstrap_user_id.clone(),
                    "owner",
                    "active",
                    now.clone(),
                    now.clone()
                ),
            )
            .await
            .context("upsert organization membership")?;
        if let Some(business_no) = business_no {
            let party_id = self
                .upsert_party_by_business_no(business_no, legal_name, &now)
                .await?;
            self.upsert_organization_party_link(
                organization_id,
                &party_id,
                organization_party_relation_type_str_legal(),
                &now,
            )
            .await?;
        }
        self.get_organization(organization_id).await
    }

    pub async fn upsert_organization_identity(
        &self,
        organization_id: &str,
        business_no: Option<&str>,
        legal_name: &str,
    ) -> Result<Organization> {
        let now = now_string();
        self.runtime
            .execute(
                "INSERT INTO organizations (id, business_no, name, inactivated_at, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT(id) DO UPDATE SET
                    business_no = COALESCE(excluded.business_no, organizations.business_no),
                    updated_at = excluded.updated_at",
                params!(
                    organization_id.to_owned(),
                    business_no.map(ToOwned::to_owned),
                    organization_id.to_owned(),
                    Option::<String>::None,
                    now.clone(),
                    now.clone()
                ),
            )
            .await
            .context("upsert organization identity")?;
        if let Some(business_no) = business_no {
            let party_id = self
                .upsert_party_by_business_no(business_no, legal_name, &now)
                .await?;
            self.upsert_organization_party_link(
                organization_id,
                &party_id,
                organization_party_relation_type_str_legal(),
                &now,
            )
            .await?;
        }
        self.get_organization(organization_id).await
    }

    async fn upsert_party_by_business_no(
        &self,
        business_no: &str,
        legal_name: &str,
        now: &str,
    ) -> Result<String> {
        let normalized = business_no.to_owned();
        let party_id = format!("party-{normalized}");
        self.runtime
            .execute(
                "INSERT INTO parties (id, business_no, legal_name, active, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT(business_no) DO UPDATE SET
                    legal_name = excluded.legal_name,
                    active = excluded.active,
                    updated_at = excluded.updated_at",
                params!(
                    party_id.clone(),
                    normalized,
                    legal_name.to_owned(),
                    1_i64,
                    now.to_owned(),
                    now.to_owned()
                ),
            )
            .await
            .context("upsert party by business number")?;
        let rows: Vec<PartyRow> = self
            .query_default_rows(
                "SELECT id, business_no, legal_name, active
                 FROM parties
                 WHERE business_no = $1
                 LIMIT 1",
                params!(business_no.to_owned()),
                "party by business number",
            )
            .await?;
        rows.into_iter()
            .next()
            .map(|row| row.id)
            .ok_or_else(|| anyhow!("party not found after upsert: {business_no}"))
    }

    async fn upsert_organization_party_link(
        &self,
        organization_id: &str,
        party_id: &str,
        relation_type: &str,
        now: &str,
    ) -> Result<()> {
        self.runtime
            .execute(
                "INSERT INTO organization_party_links (organization_id, party_id, relation_type, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT(organization_id, relation_type) DO UPDATE SET
                    party_id = excluded.party_id,
                    updated_at = excluded.updated_at",
                params!(
                    organization_id.to_owned(),
                    party_id.to_owned(),
                    relation_type.to_owned(),
                    now.to_owned(),
                    now.to_owned()
                ),
            )
            .await
            .context("upsert organization party link")?;
        Ok(())
    }

    pub async fn resolve_or_create_user_identity(
        &self,
        provider: &str,
        external_user_id: &str,
        email: Option<&str>,
        display_name: &str,
    ) -> Result<User> {
        self.resolve_or_create_user_identity_with_consistency(
            provider,
            external_user_id,
            email,
            display_name,
            None,
        )
        .await
    }

    pub async fn resolve_or_create_user_identity_with_consistency(
        &self,
        provider: &str,
        external_user_id: &str,
        email: Option<&str>,
        display_name: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<User> {
        let now = now_string();
        let read_consistency = self.validation_read_consistency_with_override(consistency.clone());
        let existing: Option<UserIdentityJoinRow> = self
            .runtime
            .query_rows(
                read_consistency.clone(),
                "SELECT ui.id as identity_id,
                        ui.user_id as user_id
                 FROM user_identities ui
                 WHERE ui.provider = $1 AND ui.external_user_id = $2",
                params!(provider.to_owned(), external_user_id.to_owned()),
            )
            .await
            .context("query user identity")?
            .into_iter()
            .next();

        let email_user = if existing.is_none() {
            match email {
                Some(email) => {
                    self.find_user_by_identity_or_email(provider, email, consistency.clone())
                        .await?
                }
                None => None,
            }
        } else {
            None
        };
        let existing_user = if existing.is_none() && email_user.is_none() {
            self.find_user_by_id(external_user_id, consistency.clone())
                .await?
        } else {
            None
        };

        let user_id = existing
            .as_ref()
            .map(|row| row.user_id.clone())
            .or_else(|| email_user.as_ref().map(|user| user.id.clone()))
            .or_else(|| existing_user.as_ref().map(|user| user.id.clone()))
            .unwrap_or_else(|| format!("{provider}:{external_user_id}"));
        let identity_id = existing
            .as_ref()
            .map(|row| row.identity_id.clone())
            .unwrap_or_else(|| Uuid::new_v4().simple().to_string());

        self.runtime
            .execute(
                "INSERT INTO users (id, email, display_name, active, created_at)
                 VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT(id) DO UPDATE SET
                    email = excluded.email,
                    display_name = excluded.display_name",
                params!(
                    user_id.clone(),
                    email.map(ToOwned::to_owned),
                    display_name.to_owned(),
                    1_i64,
                    now.clone()
                ),
            )
            .await
            .context("upsert user from identity")?;

        self.runtime
            .execute(
                "INSERT INTO user_identities (
                    id, user_id, provider, external_user_id, email_snapshot, display_name_snapshot, last_seen_at, created_at, updated_at
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                 ON CONFLICT(provider, external_user_id) DO UPDATE SET
                    user_id = excluded.user_id,
                    email_snapshot = excluded.email_snapshot,
                    display_name_snapshot = excluded.display_name_snapshot,
                    last_seen_at = excluded.last_seen_at,
                    updated_at = excluded.updated_at",
                params!(
                    identity_id,
                    user_id.clone(),
                    provider.to_owned(),
                    external_user_id.to_owned(),
                    email.map(ToOwned::to_owned),
                    Some(display_name.to_owned()),
                    Some(now.clone()),
                    now.clone(),
                    now.clone()
                ),
            )
            .await
            .context("upsert user identity")?;

        self.find_user_by_id(&user_id, consistency)
            .await?
            .ok_or_else(|| anyhow!("user not found after identity sync: {user_id}"))
    }

    pub async fn resolve_existing_user_identity_with_consistency(
        &self,
        provider: &str,
        external_user_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Option<User>> {
        let read_consistency = self.validation_read_consistency_with_override(consistency.clone());
        let existing: Option<UserIdentityJoinRow> = self
            .runtime
            .query_rows(
                read_consistency,
                "SELECT ui.id as identity_id,
                        ui.user_id as user_id
                 FROM user_identities ui
                 WHERE ui.provider = $1 AND ui.external_user_id = $2",
                params!(provider.to_owned(), external_user_id.to_owned()),
            )
            .await
            .context("query user identity")?
            .into_iter()
            .next();

        if let Some(existing) = existing {
            return self
                .find_user_by_id(&existing.user_id, consistency.clone())
                .await;
        }
        Ok(None)
    }

    pub async fn bind_identity_to_user(&self, input: BindIdentityInput<'_>) -> Result<User> {
        let now = now_string();
        let existing: Option<UserIdentityJoinRow> = self
            .runtime
            .query_rows(
                self.validation_read_consistency(),
                "SELECT ui.id as identity_id,
                        ui.user_id as user_id
                 FROM user_identities ui
                 WHERE ui.provider = $1 AND ui.external_user_id = $2",
                params!(input.provider.to_owned(), input.external_user_id.to_owned()),
            )
            .await
            .context("query user identity for binding")?
            .into_iter()
            .next();

        if !input.allow_rebind
            && let Some(existing) = existing.as_ref()
            && existing.user_id != input.user_id
        {
            anyhow::bail!(
                "identity {}:{} is already bound to another user {}",
                input.provider,
                input.external_user_id,
                existing.user_id
            );
        }

        let target_user = self
            .find_user_by_id(input.user_id, None)
            .await?
            .ok_or_else(|| anyhow::anyhow!("target user {} does not exist", input.user_id))?;

        let identity_id = existing
            .as_ref()
            .map(|row| row.identity_id.clone())
            .unwrap_or_else(|| Uuid::new_v4().simple().to_string());

        self.runtime
            .execute(
                "INSERT INTO users (id, email, display_name, active, created_at)
                 VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT(id) DO UPDATE SET
                    email = excluded.email,
                    display_name = excluded.display_name",
                params!(
                    target_user.id.clone(),
                    input
                        .email
                        .map(ToOwned::to_owned)
                        .or(target_user.email.clone()),
                    input.display_name.to_owned(),
                    if target_user.active { 1_i64 } else { 0_i64 },
                    now.clone()
                ),
            )
            .await
            .context("upsert user for identity binding")?;

        self.runtime
            .execute(
                "INSERT INTO user_identities (
                    id, user_id, provider, external_user_id, email_snapshot, display_name_snapshot, last_seen_at, created_at, updated_at
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                 ON CONFLICT(provider, external_user_id) DO UPDATE SET
                    user_id = excluded.user_id,
                    email_snapshot = excluded.email_snapshot,
                    display_name_snapshot = excluded.display_name_snapshot,
                    last_seen_at = excluded.last_seen_at,
                    updated_at = excluded.updated_at",
                params!(
                    identity_id,
                    input.user_id.to_owned(),
                    input.provider.to_owned(),
                    input.external_user_id.to_owned(),
                    input.email.map(ToOwned::to_owned),
                    Some(input.display_name.to_owned()),
                    Some(now.clone()),
                    now.clone(),
                    now.clone()
                ),
            )
            .await
            .context("bind identity to user")?;

        self.find_user_by_id(input.user_id, Some(ReadConsistency::Strong))
            .await?
            .ok_or_else(|| anyhow!("user not found after identity binding: {}", input.user_id))
    }

    async fn find_user_by_id(
        &self,
        user_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Option<User>> {
        let rows: Vec<UserLookupRow> = self
            .runtime
            .query_rows(
                self.validation_read_consistency_with_override(consistency),
                "SELECT id, email, display_name, active
                 FROM users
                 WHERE id = $1
                 LIMIT 1",
                params!(user_id.to_owned()),
            )
            .await
            .context("find user by id")?;
        Ok(rows.into_iter().next().map(|row| User {
            id: row.id,
            email: row.email,
            display_name: row.display_name,
            active: row.active != 0,
        }))
    }

    pub async fn enroll_active_member(
        &self,
        organization_id: &str,
        user_id: &str,
    ) -> Result<OrganizationMembership> {
        let now = now_string();
        let affected = self
            .runtime
            .execute(
                "INSERT INTO organization_memberships (id, organization_id, user_id, membership_role, status, created_at, updated_at)
                 SELECT $1, $2, $3, 'member', 'active', $4, $5
                 WHERE EXISTS (
                    SELECT 1
                    FROM users u
                    JOIN organizations o ON o.id = $2
                    WHERE u.id = $3
                      AND u.active = 1
                      AND o.inactivated_at IS NULL
                 )
                 ON CONFLICT(organization_id, user_id) DO UPDATE SET
                    status = 'active',
                    updated_at = excluded.updated_at
                 WHERE organization_memberships.membership_role = 'member'
                   AND EXISTS (
                     SELECT 1
                     FROM users u
                     JOIN organizations o ON o.id = excluded.organization_id
                     WHERE u.id = excluded.user_id
                       AND u.active = 1
                       AND o.inactivated_at IS NULL
                   )",
                params!(
                    Uuid::new_v4().simple().to_string(),
                    organization_id.to_owned(),
                    user_id.to_owned(),
                    now.clone(),
                    now,
                ),
            )
            .await
            .context("enroll active organization member")?;
        if affected == 0 {
            let target = self
                .find_user_by_id(user_id, Some(ReadConsistency::Strong))
                .await?;
            match target {
                None => return Err(OrganizationMembershipWriteError::TargetUserNotFound.into()),
                Some(target) if !target.active => {
                    return Err(OrganizationMembershipWriteError::TargetUserInactive.into());
                }
                Some(_) => {}
            }
            let organization_active: CountRow = self
                .runtime
                .query_rows(
                    ReadConsistency::Strong,
                    "SELECT COUNT(*) as count
                     FROM organizations
                     WHERE id = $1 AND inactivated_at IS NULL",
                    params!(organization_id.to_owned()),
                )
                .await
                .context("check enrollment organization state")?
                .into_iter()
                .next()
                .ok_or_else(|| anyhow!("organization state query returned no row"))?;
            if organization_active.count == 0 {
                return Err(OrganizationMembershipWriteError::OrganizationInactive.into());
            }
        }
        let rows: Vec<OrganizationMembershipRow> = self
            .runtime
            .query_rows(
                ReadConsistency::Strong,
                "SELECT id, organization_id, user_id, membership_role, status
                 FROM organization_memberships
                 WHERE organization_id = $1 AND user_id = $2
                 LIMIT 1",
                params!(organization_id.to_owned(), user_id.to_owned()),
            )
            .await
            .context("read enrolled organization member")?;
        let membership = match rows.into_iter().next() {
            Some(membership) => membership,
            None => return Err(OrganizationMembershipWriteError::PreconditionsChanged.into()),
        };
        let membership = OrganizationMembership::try_from(membership)?;
        if membership.membership_role != MembershipRole::Member {
            return Err(OrganizationMembershipWriteError::ExistingNonMemberRole.into());
        }
        if membership.status != MembershipStatus::Active {
            return Err(OrganizationMembershipWriteError::PreconditionsChanged.into());
        }
        Ok(membership)
    }

    /// Creates the first active owner for an external organization and grants the
    /// system bootstrap-owner role. Repeating the operation for that same owner
    /// repairs a missing binding, but it never promotes a member/admin or
    /// replaces another owner.
    pub async fn bootstrap_external_owner(
        &self,
        organization_id: &str,
        user_id: &str,
    ) -> Result<()> {
        let now = now_string();
        let binding_id = format!("binding-system-external-owner-{organization_id}-{user_id}");
        let results = self
            .runtime
            .transaction([
                (
                    Cow::Borrowed(
                        "INSERT INTO organization_memberships (
                            id, organization_id, user_id, membership_role, status, created_at, updated_at
                         )
                         SELECT $1, $2, $3, 'owner', 'active', $4, $5
                         WHERE EXISTS (
                            SELECT 1 FROM users u
                            JOIN organizations o ON o.id = $2
                            WHERE u.id = $3
                              AND u.active = 1
                              AND o.inactivated_at IS NULL
                         )
                           AND NOT EXISTS (
                               SELECT 1 FROM organization_memberships existing
                               WHERE existing.organization_id = $2
                                 AND existing.user_id = $3
                                 AND existing.membership_role <> 'owner'
                           )
                           AND NOT EXISTS (
                               SELECT 1 FROM organization_memberships owner
                               WHERE owner.organization_id = $2
                                 AND owner.user_id <> $3
                                 AND owner.membership_role = 'owner'
                           )
                         ON CONFLICT(organization_id, user_id) DO UPDATE SET
                            status = 'active',
                            updated_at = excluded.updated_at
                         WHERE organization_memberships.membership_role = 'owner'
                           AND NOT EXISTS (
                               SELECT 1 FROM organization_memberships owner
                               WHERE owner.organization_id = excluded.organization_id
                                 AND owner.user_id <> excluded.user_id
                                 AND owner.membership_role = 'owner'
                           )",
                    ),
                    params!(
                        Uuid::new_v4().simple().to_string(),
                        organization_id.to_owned(),
                        user_id.to_owned(),
                        now.clone(),
                        now.clone(),
                    ),
                ),
                (
                    Cow::Borrowed(
                        "INSERT INTO role_bindings (
                            id, subject_type, subject_id, subject_organization_id, provider_id, resource_type,
                            resource_id, role_id, agreement_id, state, created_by_user_id, created_at, updated_at
                         )
                         SELECT $1, 'user', $2, $3, 'grant', 'organization', $3,
                                'role-system-bootstrap-owner', NULL, 'active', $2, $4, $5
                         WHERE EXISTS (
                            SELECT 1 FROM organization_memberships
                            WHERE organization_id = $3
                              AND user_id = $2
                              AND membership_role = 'owner'
                              AND status = 'active'
                         )
                           AND EXISTS (
                               SELECT 1 FROM organizations
                               WHERE id = $3 AND inactivated_at IS NULL
                           )
                           AND EXISTS (
                               SELECT 1 FROM service_providers
                               WHERE id = 'grant' AND active = 1
                           )
                           AND EXISTS (
                               SELECT 1 FROM roles
                               WHERE id = 'role-system-bootstrap-owner'
                           )
                         ON CONFLICT(id) DO UPDATE SET
                            state = 'active',
                            updated_at = excluded.updated_at
                         ON CONFLICT(subject_type, subject_id, subject_organization_id, provider_id, resource_type, resource_id, role_id)
                         DO UPDATE SET
                            state = 'active',
                            updated_at = excluded.updated_at",
                    ),
                    params!(
                        binding_id,
                        user_id.to_owned(),
                        organization_id.to_owned(),
                        now.clone(),
                        now,
                    ),
                ),
            ])
            .await
            .context("bootstrap external organization owner transaction")?;
        if results.first().copied().unwrap_or_default() == 0 {
            return Err(self
                .external_owner_bootstrap_conflict(organization_id, user_id)
                .await);
        }
        if results.get(1).copied().unwrap_or_default() == 0 {
            return Err(ExternalOwnerBootstrapError::PreconditionsChanged.into());
        }
        Ok(())
    }

    async fn external_owner_bootstrap_conflict(
        &self,
        organization_id: &str,
        user_id: &str,
    ) -> anyhow::Error {
        let memberships: Vec<OrganizationMembershipRow> = match self
            .runtime
            .query_rows(
                ReadConsistency::Strong,
                "SELECT id, organization_id, user_id, membership_role, status
                 FROM organization_memberships
                 WHERE organization_id = $1",
                params!(organization_id.to_owned()),
            )
            .await
        {
            Ok(memberships) => memberships,
            Err(_) => return ExternalOwnerBootstrapError::PreconditionsChanged.into(),
        };
        if let Some(membership) = memberships
            .iter()
            .find(|membership| membership.user_id == user_id)
        {
            return match membership.membership_role.as_str() {
                "member" => ExternalOwnerBootstrapError::ExistingMemberRole.into(),
                "admin" => ExternalOwnerBootstrapError::ExistingAdminRole.into(),
                _ => ExternalOwnerBootstrapError::PreconditionsChanged.into(),
            };
        }
        if let Some(owner) = memberships
            .iter()
            .find(|membership| membership.membership_role == "owner")
        {
            return ExternalOwnerBootstrapError::ExistingOwner {
                user_id: owner.user_id.clone(),
            }
            .into();
        }
        ExternalOwnerBootstrapError::PreconditionsChanged.into()
    }

    async fn find_user_by_identity_or_email(
        &self,
        provider: &str,
        email: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Option<User>> {
        let read_consistency = self.validation_read_consistency_with_override(consistency.clone());
        let identity_rows: Vec<UserIdentityLookupRow> = self
            .runtime
            .query_rows(
                read_consistency.clone(),
                "SELECT ui.user_id
                 FROM user_identities ui
                 WHERE ui.provider = $1
                   AND ui.email_snapshot = $2
                 ORDER BY ui.updated_at DESC, ui.created_at DESC
                 LIMIT 2",
                params!(provider.to_owned(), email.to_owned()),
            )
            .await
            .context("find user identity by provider/email")?;
        if identity_rows.len() > 1 {
            anyhow::bail!(
                "multiple local users matched {} identity email {}",
                provider,
                email
            );
        }
        if let Some(row) = identity_rows.into_iter().next() {
            return self
                .find_user_by_id(&row.user_id, consistency.clone())
                .await;
        }

        let user_rows: Vec<UserLookupRow> = self
            .runtime
            .query_rows(
                read_consistency,
                "SELECT id, email, display_name, active
                 FROM users
                 WHERE email = $1
                 ORDER BY created_at ASC, id ASC
                 LIMIT 2",
                params!(email.to_owned()),
            )
            .await
            .context("find user by email")?;
        if user_rows.len() > 1 {
            anyhow::bail!("multiple local users matched email {}", email);
        }
        Ok(user_rows.into_iter().next().map(|row| User {
            id: row.id,
            email: row.email,
            display_name: row.display_name,
            active: row.active != 0,
        }))
    }

    pub async fn get_cached_external_identity(
        &self,
        token_hash: &str,
        now: DateTime<Utc>,
        consistency: Option<ReadConsistency>,
    ) -> Result<Option<CachedExternalIdentity>> {
        let rows: Vec<CachedIdentityWithClientRow> = self
            .runtime
            .query_rows(
                self.validation_read_consistency_with_override(consistency),
                "SELECT provider, external_user_id, client_id, email, display_name, active, expires_at
                 FROM auth_identity_cache
                 WHERE token_hash = $1
                   AND expires_at > $2
                   AND client_id IS NOT NULL
                   AND TRIM(client_id) <> ''
                 LIMIT 1",
                params!(token_hash.to_owned(), now.to_rfc3339()),
            )
            .await
            .context("query shared auth identity cache")?;

        Ok(rows.into_iter().next().map(|row| CachedExternalIdentity {
            provider: row.provider,
            external_user_id: row.external_user_id,
            client_id: row.client_id.expect("query requires non-null client_id"),
            email: row.email,
            display_name: row.display_name,
            active: row.active != 0,
            expires_at: DateTime::parse_from_rfc3339(&row.expires_at)
                .map(|ts| ts.with_timezone(&Utc))
                .unwrap_or(now),
        }))
    }

    pub async fn put_cached_external_identity(
        &self,
        token_hash: &str,
        identity: &CachedExternalIdentity,
        expires_at: DateTime<Utc>,
    ) -> Result<()> {
        let now = now_string();
        self.runtime
            .execute(
                "DELETE FROM auth_identity_cache WHERE expires_at <= $1",
                params!(now.clone()),
            )
            .await
            .context("purge expired auth identity cache")?;
        self.runtime
            .execute(
                "INSERT INTO auth_identity_cache (
                    token_hash, provider, external_user_id, client_id, email, display_name, active, expires_at, updated_at
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                 ON CONFLICT(token_hash) DO UPDATE SET
                    provider = excluded.provider,
                    external_user_id = excluded.external_user_id,
                    client_id = excluded.client_id,
                    email = excluded.email,
                    display_name = excluded.display_name,
                    active = excluded.active,
                    expires_at = excluded.expires_at,
                    updated_at = excluded.updated_at",
                params!(
                    token_hash.to_owned(),
                    identity.provider.clone(),
                    identity.external_user_id.clone(),
                    identity.client_id.clone(),
                    identity.email.clone(),
                    identity.display_name.clone(),
                    if identity.active { 1_i64 } else { 0_i64 },
                    expires_at.to_rfc3339(),
                    now.clone()
                ),
            )
            .await
            .context("upsert shared auth identity cache")?;
        Ok(())
    }

    pub async fn set_user_active(&self, user_id: &str, active: bool) -> Result<User> {
        let updated = self
            .runtime
            .execute(
                "UPDATE users SET active = $1 WHERE id = $2",
                params!(if active { 1_i64 } else { 0_i64 }, user_id.to_owned()),
            )
            .await
            .context("set user active")?;
        if updated == 0 {
            return Err(anyhow!("user not found: {user_id}"));
        }
        self.find_user_by_id(user_id, Some(ReadConsistency::Strong))
            .await?
            .ok_or_else(|| anyhow!("user not found after active update: {user_id}"))
    }

    pub async fn has_active_organization_membership(
        &self,
        user_id: &str,
        organization_id: &str,
    ) -> Result<bool> {
        self.has_active_organization_membership_with_consistency(user_id, organization_id, None)
            .await
    }

    pub async fn has_organization_membership_with_consistency(
        &self,
        user_id: &str,
        organization_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<bool> {
        let row: CountRow = self
            .runtime
            .query_rows(
                self.validation_read_consistency_with_override(consistency),
                "SELECT COUNT(*) as count
                 FROM organization_memberships
                 WHERE user_id = $1
                   AND organization_id = $2
                   AND status = 'active'",
                params!(user_id.to_owned(), organization_id.to_owned()),
            )
            .await
            .context("check organization membership including inactive organization")?
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("membership count query returned no row"))?;
        Ok(row.count > 0)
    }

    /// Returns the local membership state without granting access.  This is
    /// used only by onboarding's deliberately narrow response.
    pub async fn organization_membership_status_with_consistency(
        &self,
        user_id: &str,
        organization_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Option<MembershipStatus>> {
        let rows: Vec<OrganizationMembershipRow> = self.runtime.query_rows(
            self.validation_read_consistency_with_override(consistency),
            "SELECT id, organization_id, user_id, membership_role, status, created_at, updated_at FROM organization_memberships WHERE user_id=$1 AND organization_id=$2 LIMIT 1",
            params!(user_id.to_owned(), organization_id.to_owned()),
        ).await.context("read organization membership status")?;
        rows.into_iter()
            .next()
            .map(OrganizationMembership::try_from)
            .transpose()
            .map(|value| value.map(|membership| membership.status))
    }

    pub async fn has_active_organization_membership_with_consistency(
        &self,
        user_id: &str,
        organization_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<bool> {
        let row: CountRow = self
            .runtime
            .query_rows(
                self.validation_read_consistency_with_override(consistency),
                "SELECT COUNT(*) as count
                 FROM organization_memberships om
                 JOIN organizations o ON o.id = om.organization_id
                 WHERE om.user_id = $1
                   AND om.organization_id = $2
                   AND om.status = 'active'
                   AND o.inactivated_at IS NULL",
                params!(user_id.to_owned(), organization_id.to_owned()),
            )
            .await
            .context("check organization membership")?
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("membership count query returned no row"))?;
        Ok(row.count > 0)
    }

    pub async fn get_user(&self, user_id: &str) -> Result<User> {
        self.find_user_by_id(user_id, None)
            .await?
            .ok_or_else(|| anyhow!("user not found: {user_id}"))
    }

    pub async fn list_active_organizations_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<Organization>> {
        self.list_active_organizations_for_user_with_consistency(user_id, None)
            .await
    }

    pub async fn list_active_organizations_for_user_with_consistency(
        &self,
        user_id: &str,
        consistency: Option<ReadConsistency>,
    ) -> Result<Vec<Organization>> {
        let rows: Vec<OrganizationRow> = self
            .runtime
            .query_rows(
                self.validation_read_consistency_with_override(consistency),
                "SELECT o.id, opl.party_id, o.business_no, o.name, o.inactivated_at
                 FROM organizations o
                 LEFT JOIN organization_party_links opl
                   ON opl.organization_id = o.id
                  AND opl.relation_type = 'legal'
                 JOIN organization_memberships om ON om.organization_id = o.id
                 WHERE om.user_id = $1
                   AND om.status = 'active'
                   AND o.inactivated_at IS NULL
                 ORDER BY o.name ASC, o.id ASC",
                params!(user_id.to_owned()),
            )
            .await
            .context("list active organizations for user")?;
        Ok(rows.into_iter().map(Organization::from).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::{Database, ExternalIdentityMaterialization, MATERIALIZATION_CANDIDATE_IDS};
    use crate::{config::ReadConsistency, runtime::HiqliteRuntime};
    use hiqlite::{Node, NodeConfig, Row, s3::EncKeys};
    use std::{collections::VecDeque, net::TcpListener};

    struct CountRow(i64);

    impl From<&mut Row<'_>> for CountRow {
        fn from(row: &mut Row<'_>) -> Self {
            Self(row.get("count"))
        }
    }

    fn free_address() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("ephemeral address");
        let address = listener.local_addr().expect("local address");
        drop(listener);
        address.to_string()
    }

    async fn test_database() -> (HiqliteRuntime, Database, tempfile::TempDir) {
        let directory = tempfile::tempdir().expect("temporary database directory");
        let config = NodeConfig {
            node_id: 1,
            nodes: vec![Node {
                id: 1,
                addr_raft: free_address(),
                addr_api: free_address(),
            }],
            listen_addr_api: "127.0.0.1".into(),
            listen_addr_raft: "127.0.0.1".into(),
            data_dir: directory.path().to_string_lossy().into_owned().into(),
            filename_db: "identity-test.sqlite".into(),
            secret_api: "0123456789abcdef0123456789abcdef".to_owned(),
            secret_raft: "fedcba9876543210fedcba9876543210".to_owned(),
            enc_keys: EncKeys::generate().expect("encryption keys"),
            health_check_delay_secs: 0,
            raft_config: NodeConfig::default_raft_config(128),
            ..Default::default()
        };
        let runtime = HiqliteRuntime::embedded(
            hiqlite::start_node(config).await.expect("start hiqlite"),
            ReadConsistency::Local,
        );
        let database = Database::new(
            runtime.clone(),
            "identity-test-bootstrap".to_owned(),
            "Test".to_owned(),
        );
        database.ensure_schema().await.expect("schema");
        (runtime, database, directory)
    }

    #[tokio::test]
    async fn materialization_candidate_user_id_collision_cannot_link_identity_to_existing_user() {
        let (runtime, database, _directory) = test_database().await;
        runtime.execute(
            "INSERT INTO users (id,email,display_name,active,created_at) VALUES ('candidate-user','existing@example.test','Existing',1,'2026-01-01T00:00:00Z')",
            vec![],
        ).await.expect("seed exact candidate collision");
        *MATERIALIZATION_CANDIDATE_IDS
            .lock()
            .expect("candidate override lock") =
            VecDeque::from(["candidate-user".to_owned(), "candidate-identity".to_owned()]);

        let error = database
            .materialize_external_identity(ExternalIdentityMaterialization {
                provider: "patch-data",
                external_user_id: "new-external-subject",
                email: Some("new@example.test"),
                display_name: "New User",
            })
            .await
            .expect_err("candidate collision must reject");
        assert_eq!(
            error.downcast_ref::<super::ExternalOwnerBootstrapError>(),
            Some(&super::ExternalOwnerBootstrapError::PreconditionsChanged)
        );
        let identities: Vec<CountRow> = runtime.query_rows(
            ReadConsistency::Strong,
            "SELECT COUNT(*) AS count FROM user_identities WHERE provider='patch-data' AND external_user_id='new-external-subject'",
            vec![],
        ).await.expect("identity count");
        assert_eq!(
            identities[0].0, 0,
            "identity must not attach to the collided user id"
        );
    }
}

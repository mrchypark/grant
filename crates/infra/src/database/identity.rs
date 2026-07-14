use super::core::now_string;
use super::*;

#[derive(Debug, Clone)]
struct CachedIdentityWithClientRow {
    provider: String,
    external_user_id: String,
    client_id: Option<String>,
    email: Option<String>,
    display_name: String,
    active: i64,
    expires_at: String,
}

impl From<&mut Row<'_>> for CachedIdentityWithClientRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            provider: row.get("provider"),
            external_user_id: row.get("external_user_id"),
            client_id: row.get("client_id"),
            email: row.get("email"),
            display_name: row.get("display_name"),
            active: row.get("active"),
            expires_at: row.get("expires_at"),
        }
    }
}

impl Database {
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

use super::codec::{
    action_risk_level_from_str, agreement_status_from_str, agreement_type_from_str,
    application_app_type_from_str, application_consent_policy_from_str,
    application_grant_status_from_str, application_provisioning_status_from_str,
    application_type_from_str, role_binding_state_from_str, role_binding_subject_type_from_str,
};
use anyhow::Result;
use domain::{
    Action, Agreement, Application, ApplicationGrant, ApplicationRedirectUri, Group, GroupMember,
    Organization, Party, Role, RoleBinding, ServiceProvider,
};
use hiqlite::Row;

#[derive(Debug, Clone)]
pub(super) struct CountRow {
    pub(super) count: i64,
}

#[derive(Debug, Clone)]
pub(super) struct UserIdentityJoinRow {
    pub(super) identity_id: String,
    pub(super) user_id: String,
}

#[derive(Debug, Clone)]
pub(super) struct UserIdentityLookupRow {
    pub(super) user_id: String,
}

#[derive(Debug, Clone)]
pub(super) struct CachedIdentityWithClientRow {
    pub(super) provider: String,
    pub(super) external_user_id: String,
    pub(super) client_id: Option<String>,
    pub(super) email: Option<String>,
    pub(super) display_name: String,
    pub(super) active: i64,
    pub(super) expires_at: String,
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

impl From<&mut Row<'_>> for UserIdentityLookupRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            user_id: row.get("user_id"),
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct UserLookupRow {
    pub(super) id: String,
    pub(super) email: Option<String>,
    pub(super) display_name: String,
    pub(super) active: i64,
}

#[derive(Debug, Clone)]
pub(super) struct RoleBindingRow {
    pub(super) id: String,
    pub(super) subject_type: String,
    pub(super) subject_id: String,
    pub(super) subject_organization_id: String,
    pub(super) provider_id: String,
    pub(super) resource_type: String,
    pub(super) resource_id: String,
    pub(super) role_id: String,
    pub(super) agreement_id: Option<String>,
    pub(super) state: String,
    pub(super) created_by_user_id: String,
    pub(super) created_at: String,
    pub(super) updated_at: String,
}

#[derive(Debug, Clone)]
pub(super) struct ServiceProviderRow {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) description: Option<String>,
    pub(super) active: i64,
}

#[derive(Debug, Clone)]
pub(super) struct AgreementRow {
    pub(super) id: String,
    pub(super) agreement_type: String,
    pub(super) grantor_organization_id: String,
    pub(super) grantee_organization_id: String,
    pub(super) status: String,
    pub(super) title: String,
    pub(super) reference_no: Option<String>,
    pub(super) starts_at: Option<String>,
    pub(super) ends_at: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct RoleOrgRow {
    pub(super) organization_id: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct OrganizationRow {
    pub(super) id: String,
    pub(super) party_id: Option<String>,
    pub(super) business_no: Option<String>,
    pub(super) name: String,
    pub(super) inactivated_at: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct GroupRow {
    pub(super) id: String,
    pub(super) organization_id: String,
    pub(super) name: String,
    pub(super) description: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct GroupMemberRow {
    pub(super) group_id: String,
    pub(super) organization_membership_id: String,
    pub(super) user_id: String,
}

#[derive(Debug, Clone)]
pub(super) struct PartyRow {
    pub(super) id: String,
    pub(super) business_no: String,
    pub(super) legal_name: String,
    pub(super) active: i64,
}

#[derive(Debug, Clone)]
pub(super) struct ActionRow {
    pub(super) id: String,
    pub(super) provider_id: String,
    pub(super) code: String,
    pub(super) name: String,
    pub(super) description: Option<String>,
    pub(super) risk_level: String,
    pub(super) visibility_granting: i64,
    pub(super) grantable_to_applications: i64,
    pub(super) consent_label: Option<String>,
    pub(super) consent_description: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct RoleRow {
    pub(super) id: String,
    pub(super) organization_id: Option<String>,
    pub(super) code: String,
    pub(super) name: String,
    pub(super) description: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct ApplicationRow {
    pub(super) id: String,
    pub(super) organization_id: String,
    pub(super) slug: String,
    pub(super) name: String,
    pub(super) description: Option<String>,
    pub(super) application_type: String,
    pub(super) app_type: String,
    pub(super) first_party: i64,
    pub(super) consent_policy: String,
    pub(super) active: i64,
    pub(super) provisioning_status: String,
    pub(super) rauthy_client_id: Option<String>,
    pub(super) created_by_user_id: String,
}

#[derive(Debug, Clone)]
pub(super) struct ApplicationRedirectUriRow {
    pub(super) application_id: String,
    pub(super) redirect_uri: String,
    pub(super) created_at: String,
}

#[derive(Debug, Clone)]
pub(super) struct ApplicationGrantLookupRow {
    pub(super) id: String,
}

#[derive(Debug, Clone)]
pub(super) struct ApplicationGrantRow {
    pub(super) id: String,
    pub(super) user_id: String,
    pub(super) application_id: String,
    pub(super) organization_id: String,
    pub(super) status: String,
    pub(super) consented_at: String,
    pub(super) revoked_at: Option<String>,
    pub(super) revoked_by_user_id: Option<String>,
    pub(super) created_at: String,
    pub(super) updated_at: String,
}

#[derive(Debug, Clone)]
pub(super) struct ActionLookupRow {
    pub(super) id: String,
}

#[derive(Debug, Clone)]
pub(super) struct UserActionDecisionRow {
    pub(super) idx: i64,
    pub(super) rbac_allowed: i64,
    pub(super) allowed: i64,
    pub(super) trusted_application: i64,
}

#[derive(Debug, Clone)]
pub(super) struct ColumnNameRow {
    pub(super) name: String,
}

#[derive(Debug, Clone)]
pub(super) struct TableSqlRow {
    pub(super) sql: Option<String>,
}

impl From<&mut Row<'_>> for CountRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            count: row.get("count"),
        }
    }
}

impl From<&mut Row<'_>> for UserIdentityJoinRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            identity_id: row.get("identity_id"),
            user_id: row.get("user_id"),
        }
    }
}

impl From<&mut Row<'_>> for UserLookupRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            id: row.get("id"),
            email: row.get("email"),
            display_name: row.get("display_name"),
            active: row.get("active"),
        }
    }
}

impl From<&mut Row<'_>> for RoleBindingRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            id: row.get("id"),
            subject_type: row.get("subject_type"),
            subject_id: row.get("subject_id"),
            subject_organization_id: row.get("subject_organization_id"),
            provider_id: row.get("provider_id"),
            resource_type: row.get("resource_type"),
            resource_id: row.get("resource_id"),
            role_id: row.get("role_id"),
            agreement_id: row.get("agreement_id"),
            state: row.get("state"),
            created_by_user_id: row.get("created_by_user_id"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
        }
    }
}

impl From<&mut Row<'_>> for ServiceProviderRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            id: row.get("id"),
            name: row.get("name"),
            description: row.get("description"),
            active: row.get("active"),
        }
    }
}

impl From<&mut Row<'_>> for AgreementRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            id: row.get("id"),
            agreement_type: row.get("agreement_type"),
            grantor_organization_id: row.get("grantor_organization_id"),
            grantee_organization_id: row.get("grantee_organization_id"),
            status: row.get("status"),
            title: row.get("title"),
            reference_no: row.get("reference_no"),
            starts_at: row.get("starts_at"),
            ends_at: row.get("ends_at"),
        }
    }
}

impl From<&mut Row<'_>> for RoleOrgRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            organization_id: row.get("organization_id"),
        }
    }
}

impl From<&mut Row<'_>> for OrganizationRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            id: row.get("id"),
            party_id: row.get("party_id"),
            business_no: row.get("business_no"),
            name: row.get("name"),
            inactivated_at: row.get("inactivated_at"),
        }
    }
}

impl From<&mut Row<'_>> for GroupRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            id: row.get("id"),
            organization_id: row.get("organization_id"),
            name: row.get("name"),
            description: row.get("description"),
        }
    }
}

impl From<&mut Row<'_>> for GroupMemberRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            group_id: row.get("group_id"),
            organization_membership_id: row.get("organization_membership_id"),
            user_id: row.get("user_id"),
        }
    }
}

impl From<&mut Row<'_>> for PartyRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            id: row.get("id"),
            business_no: row.get("business_no"),
            legal_name: row.get("legal_name"),
            active: row.get("active"),
        }
    }
}

impl From<&mut Row<'_>> for ActionRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            id: row.get("id"),
            provider_id: row.get("provider_id"),
            code: row.get("code"),
            name: row.get("name"),
            description: row.get("description"),
            risk_level: row.get("risk_level"),
            visibility_granting: row.get("visibility_granting"),
            grantable_to_applications: row.get("grantable_to_applications"),
            consent_label: row.get("consent_label"),
            consent_description: row.get("consent_description"),
        }
    }
}

impl From<&mut Row<'_>> for RoleRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            id: row.get("id"),
            organization_id: row.get("organization_id"),
            code: row.get("code"),
            name: row.get("name"),
            description: row.get("description"),
        }
    }
}

impl From<&mut Row<'_>> for ApplicationRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            id: row.get("id"),
            organization_id: row.get("organization_id"),
            slug: row.get("slug"),
            name: row.get("name"),
            description: row.get("description"),
            application_type: row.get("application_type"),
            app_type: row.get("app_type"),
            first_party: row.get("first_party"),
            consent_policy: row.get("consent_policy"),
            active: row.get("active"),
            provisioning_status: row.get("provisioning_status"),
            rauthy_client_id: row.get("rauthy_client_id"),
            created_by_user_id: row.get("created_by_user_id"),
        }
    }
}

impl From<&mut Row<'_>> for ApplicationRedirectUriRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            application_id: row.get("application_id"),
            redirect_uri: row.get("redirect_uri"),
            created_at: row.get("created_at"),
        }
    }
}

impl From<&mut Row<'_>> for ApplicationGrantLookupRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self { id: row.get("id") }
    }
}

impl From<&mut Row<'_>> for ApplicationGrantRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            id: row.get("id"),
            user_id: row.get("user_id"),
            application_id: row.get("application_id"),
            organization_id: row.get("organization_id"),
            status: row.get("status"),
            consented_at: row.get("consented_at"),
            revoked_at: row.get("revoked_at"),
            revoked_by_user_id: row.get("revoked_by_user_id"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
        }
    }
}

impl From<&mut Row<'_>> for ActionLookupRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self { id: row.get("id") }
    }
}

impl From<&mut Row<'_>> for UserActionDecisionRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            idx: row.get("idx"),
            rbac_allowed: row.get("rbac_allowed"),
            allowed: row.get("allowed"),
            trusted_application: row.get("trusted_application"),
        }
    }
}

impl From<&mut Row<'_>> for ColumnNameRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            name: row.get("name"),
        }
    }
}

impl From<&mut Row<'_>> for TableSqlRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            sql: row.get("sql"),
        }
    }
}

impl From<OrganizationRow> for Organization {
    fn from(value: OrganizationRow) -> Self {
        Self {
            id: value.id,
            party_id: value.party_id,
            business_no: value.business_no,
            name: value.name,
            inactivated_at: value.inactivated_at,
        }
    }
}

impl From<PartyRow> for Party {
    fn from(value: PartyRow) -> Self {
        Self {
            id: value.id,
            business_no: value.business_no,
            legal_name: value.legal_name,
            active: value.active == 1,
        }
    }
}

impl From<ServiceProviderRow> for ServiceProvider {
    fn from(value: ServiceProviderRow) -> Self {
        Self {
            id: value.id,
            name: value.name,
            description: value.description,
            active: value.active == 1,
        }
    }
}

impl From<RoleRow> for Role {
    fn from(value: RoleRow) -> Self {
        Self {
            id: value.id,
            organization_id: value.organization_id,
            code: value.code,
            name: value.name,
            description: value.description,
        }
    }
}

impl TryFrom<ActionRow> for Action {
    type Error = anyhow::Error;

    fn try_from(value: ActionRow) -> Result<Self> {
        Ok(Self {
            id: value.id,
            provider_id: value.provider_id,
            code: value.code,
            name: value.name,
            description: value.description,
            risk_level: action_risk_level_from_str(&value.risk_level)?,
            visibility_granting: value.visibility_granting == 1,
            grantable_to_applications: value.grantable_to_applications == 1,
            consent_label: value.consent_label,
            consent_description: value.consent_description,
        })
    }
}

impl TryFrom<ApplicationRow> for Application {
    type Error = anyhow::Error;

    fn try_from(value: ApplicationRow) -> Result<Self> {
        Ok(Self {
            id: value.id,
            organization_id: value.organization_id,
            slug: value.slug,
            name: value.name,
            description: value.description,
            application_type: application_type_from_str(&value.application_type)?,
            app_type: application_app_type_from_str(&value.app_type)?,
            first_party: value.first_party == 1,
            consent_policy: application_consent_policy_from_str(&value.consent_policy)?,
            active: value.active == 1,
            provisioning_status: application_provisioning_status_from_str(
                &value.provisioning_status,
            )?,
            rauthy_client_id: value.rauthy_client_id,
            created_by_user_id: value.created_by_user_id,
        })
    }
}

impl From<GroupRow> for Group {
    fn from(value: GroupRow) -> Self {
        Self {
            id: value.id,
            organization_id: value.organization_id,
            name: value.name,
            description: value.description,
        }
    }
}

impl From<GroupMemberRow> for GroupMember {
    fn from(value: GroupMemberRow) -> Self {
        Self {
            group_id: value.group_id,
            organization_membership_id: value.organization_membership_id,
            user_id: value.user_id,
        }
    }
}

impl From<ApplicationRedirectUriRow> for ApplicationRedirectUri {
    fn from(value: ApplicationRedirectUriRow) -> Self {
        Self {
            application_id: value.application_id,
            redirect_uri: value.redirect_uri,
            created_at: value.created_at,
        }
    }
}

impl TryFrom<ApplicationGrantRow> for ApplicationGrant {
    type Error = anyhow::Error;

    fn try_from(value: ApplicationGrantRow) -> Result<Self> {
        Ok(Self {
            id: value.id,
            user_id: value.user_id,
            application_id: value.application_id,
            organization_id: value.organization_id,
            status: application_grant_status_from_str(&value.status)?,
            consented_at: value.consented_at,
            revoked_at: value.revoked_at,
            revoked_by_user_id: value.revoked_by_user_id,
            created_at: value.created_at,
            updated_at: value.updated_at,
        })
    }
}

impl TryFrom<RoleBindingRow> for RoleBinding {
    type Error = anyhow::Error;

    fn try_from(value: RoleBindingRow) -> Result<Self> {
        Ok(Self {
            id: value.id,
            subject_type: role_binding_subject_type_from_str(&value.subject_type)?,
            subject_id: value.subject_id,
            subject_organization_id: value.subject_organization_id,
            provider_id: value.provider_id,
            resource_type: value.resource_type,
            resource_id: value.resource_id,
            role_id: value.role_id,
            agreement_id: value.agreement_id,
            state: role_binding_state_from_str(&value.state)?,
            created_by_user_id: value.created_by_user_id,
            created_at: value.created_at,
            updated_at: value.updated_at,
        })
    }
}

impl TryFrom<AgreementRow> for Agreement {
    type Error = anyhow::Error;

    fn try_from(value: AgreementRow) -> Result<Self> {
        Ok(Self {
            id: value.id,
            agreement_type: agreement_type_from_str(&value.agreement_type)?,
            grantor_organization_id: value.grantor_organization_id,
            grantee_organization_id: value.grantee_organization_id,
            status: agreement_status_from_str(&value.status)?,
            title: value.title,
            reference_no: value.reference_no,
            starts_at: value.starts_at,
            ends_at: value.ends_at,
        })
    }
}

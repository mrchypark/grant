use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HealthStatus {
    pub status: &'static str,
    #[serde(rename = "readReady")]
    pub read_ready: bool,
    #[serde(rename = "writeReady")]
    pub write_ready: bool,
    #[serde(rename = "runtimeMode")]
    pub runtime_mode: &'static str,
    #[serde(rename = "defaultReadConsistency")]
    pub default_read_consistency: &'static str,
}

impl HealthStatus {
    pub const fn ok() -> Self {
        Self {
            status: "ok",
            read_ready: true,
            write_ready: true,
            runtime_mode: "embedded",
            default_read_consistency: "local",
        }
    }

    pub const fn new(
        status: &'static str,
        read_ready: bool,
        write_ready: bool,
        runtime_mode: &'static str,
        default_read_consistency: &'static str,
    ) -> Self {
        Self {
            status,
            read_ready,
            write_ready,
            runtime_mode,
            default_read_consistency,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Organization {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub party_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub business_no: Option<String>,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inactivated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OrganizationPartyLink {
    pub organization_id: String,
    pub party_id: String,
    pub relation_type: OrganizationPartyRelationType,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OrganizationPartyRelationType {
    Legal,
    Billing,
    Contract,
    Owner,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OrganizationMembership {
    pub id: String,
    pub organization_id: String,
    pub user_id: String,
    pub membership_role: MembershipRole,
    pub status: MembershipStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MembershipRole {
    Owner,
    Admin,
    Member,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MembershipStatus {
    Active,
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub id: String,
    pub organization_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GroupMember {
    pub group_id: String,
    pub organization_membership_id: String,
    pub user_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Agreement {
    pub id: String,
    pub agreement_type: AgreementType,
    pub grantor_organization_id: String,
    pub grantee_organization_id: String,
    pub status: AgreementStatus,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_no: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub starts_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ends_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgreementType {
    Delegation,
    Epc,
    Om,
    Distributor,
    Dealer,
}

impl AgreementType {
    pub fn as_db_str(&self) -> &'static str {
        match self {
            Self::Delegation => "delegation",
            Self::Epc => "epc",
            Self::Om => "om",
            Self::Distributor => "distributor",
            Self::Dealer => "dealer",
        }
    }

    pub fn from_db_str(value: &str) -> Result<Self, String> {
        match value {
            "delegation" => Ok(Self::Delegation),
            "epc" => Ok(Self::Epc),
            "om" => Ok(Self::Om),
            "distributor" => Ok(Self::Distributor),
            "dealer" => Ok(Self::Dealer),
            other => Err(format!("invalid agreement type {other}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgreementStatus {
    Draft,
    Active,
    Expired,
    Terminated,
}

impl AgreementStatus {
    pub fn as_db_str(&self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Active => "active",
            Self::Expired => "expired",
            Self::Terminated => "terminated",
        }
    }

    pub fn from_db_str(value: &str) -> Result<Self, String> {
        match value {
            "draft" => Ok(Self::Draft),
            "active" => Ok(Self::Active),
            "expired" => Ok(Self::Expired),
            "terminated" => Ok(Self::Terminated),
            other => Err(format!("invalid agreement status {other}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Action {
    pub id: String,
    pub provider_id: String,
    pub code: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub risk_level: ActionRiskLevel,
    pub visibility_granting: bool,
    pub grantable_to_applications: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consent_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consent_description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionRiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

impl ActionRiskLevel {
    pub fn as_db_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }

    pub fn from_db_str(value: &str) -> Result<Self, String> {
        match value {
            "low" => Ok(Self::Low),
            "medium" => Ok(Self::Medium),
            "high" => Ok(Self::High),
            "critical" => Ok(Self::Critical),
            other => Err(format!("invalid action risk level {other}")),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum FieldViewMode {
    None,
    Anonymized,
    Real,
}

impl FieldViewMode {
    pub fn as_positive_db_str(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Anonymized => Some("anonymized"),
            Self::Real => Some("real"),
        }
    }

    pub fn from_db_str(value: &str) -> Result<Self, String> {
        match value {
            "anonymized" => Ok(Self::Anonymized),
            "real" => Ok(Self::Real),
            other => Err(format!("invalid field view mode {other}")),
        }
    }
}

pub const MAX_FIELD_VIEW_RESOURCE_TYPE_BYTES: usize = 128;
pub const MAX_FIELD_VIEW_SCHEMA_KEY_BYTES: usize = 128;
pub const MAX_FIELD_VIEW_POINTER_BYTES: usize = 512;

pub fn normalize_field_view_pointer(pointer: &str) -> Result<String, String> {
    if pointer.is_empty() || !pointer.starts_with('/') {
        return Err("field view pointer must be a non-root RFC 6901 JSON Pointer".to_owned());
    }

    let mut canonical = String::with_capacity(pointer.len());
    for encoded_token in pointer.split('/').skip(1) {
        let mut decoded = String::with_capacity(encoded_token.len());
        let mut characters = encoded_token.chars();
        while let Some(character) = characters.next() {
            if character != '~' {
                decoded.push(character);
                continue;
            }
            match characters.next() {
                Some('0') => decoded.push('~'),
                Some('1') => decoded.push('/'),
                _ => {
                    return Err(format!(
                        "field view pointer contains an invalid RFC 6901 escape: {pointer}"
                    ));
                }
            }
        }
        canonical.push('/');
        for character in decoded.chars() {
            match character {
                '~' => canonical.push_str("~0"),
                '/' => canonical.push_str("~1"),
                _ => canonical.push(character),
            }
        }
    }
    Ok(canonical)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Application {
    pub id: String,
    pub organization_id: String,
    pub slug: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub application_type: ApplicationType,
    pub app_type: ApplicationAppType,
    pub first_party: bool,
    pub consent_policy: ApplicationConsentPolicy,
    pub active: bool,
    pub provisioning_status: ApplicationProvisioningStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rauthy_client_id: Option<String>,
    pub created_by_user_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApplicationProvisioningStatus {
    Unprovisioned,
    Ready,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationRedirectUri {
    pub application_id: String,
    pub redirect_uri: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApplicationType {
    Confidential,
    Public,
}

impl ApplicationType {
    pub fn as_db_str(&self) -> &'static str {
        match self {
            Self::Confidential => "confidential",
            Self::Public => "public",
        }
    }

    pub fn from_db_str(value: &str) -> Result<Self, String> {
        match value {
            "confidential" => Ok(Self::Confidential),
            "public" => Ok(Self::Public),
            other => Err(format!("invalid application type {other}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApplicationAppType {
    Web,
    Mobile,
    Server,
    Internal,
}

impl ApplicationAppType {
    pub fn as_db_str(&self) -> &'static str {
        match self {
            Self::Web => "web",
            Self::Mobile => "mobile",
            Self::Server => "server",
            Self::Internal => "internal",
        }
    }

    pub fn from_db_str(value: &str) -> Result<Self, String> {
        match value {
            "web" => Ok(Self::Web),
            "mobile" => Ok(Self::Mobile),
            "server" => Ok(Self::Server),
            "internal" => Ok(Self::Internal),
            other => Err(format!("invalid application app type {other}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApplicationConsentPolicy {
    Explicit,
    AutoApprove,
}

impl ApplicationConsentPolicy {
    pub fn as_db_str(&self) -> &'static str {
        match self {
            Self::Explicit => "explicit",
            Self::AutoApprove => "auto_approve",
        }
    }

    pub fn from_db_str(value: &str) -> Result<Self, String> {
        match value {
            "explicit" => Ok(Self::Explicit),
            "auto_approve" => Ok(Self::AutoApprove),
            other => Err(format!("invalid application consent policy {other}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationAction {
    pub application_id: String,
    pub action_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationGrant {
    pub id: String,
    pub user_id: String,
    pub application_id: String,
    pub organization_id: String,
    pub status: ApplicationGrantStatus,
    pub consented_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revoked_by_user_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApplicationGrantStatus {
    Active,
    Revoked,
    Expired,
}

impl ApplicationGrantStatus {
    pub fn as_db_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Revoked => "revoked",
            Self::Expired => "expired",
        }
    }

    pub fn from_db_str(value: &str) -> Result<Self, String> {
        match value {
            "active" => Ok(Self::Active),
            "revoked" => Ok(Self::Revoked),
            "expired" => Ok(Self::Expired),
            other => Err(format!("invalid application grant status {other}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationGrantAction {
    pub application_grant_id: String,
    pub action_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Role {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organization_id: Option<String>,
    pub code: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RoleBinding {
    pub id: String,
    pub subject_type: RoleBindingSubjectType,
    pub subject_id: String,
    pub subject_organization_id: String,
    pub provider_id: String,
    pub resource_type: String,
    pub resource_id: String,
    pub role_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agreement_id: Option<String>,
    pub state: RoleBindingState,
    pub created_by_user_id: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RoleBindingSubjectType {
    Organization,
    Group,
    User,
}

impl RoleBindingSubjectType {
    pub fn as_db_str(&self) -> &'static str {
        match self {
            Self::Organization => "organization",
            Self::Group => "group",
            Self::User => "user",
        }
    }

    pub fn from_db_str(value: &str) -> Result<Self, String> {
        match value {
            "organization" => Ok(Self::Organization),
            "group" => Ok(Self::Group),
            "user" => Ok(Self::User),
            other => Err(format!("invalid role binding subject type {other}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ServiceProvider {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RoleBindingState {
    Active,
    Disabled,
}

impl RoleBindingState {
    pub fn as_db_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Disabled => "disabled",
        }
    }

    pub fn from_db_str(value: &str) -> Result<Self, String> {
        match value {
            "active" => Ok(Self::Active),
            "disabled" => Ok(Self::Disabled),
            other => Err(format!("invalid role binding state {other}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct User {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    pub display_name: String,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Party {
    pub id: String,
    pub business_no: String,
    pub legal_name: String,
    pub active: bool,
}

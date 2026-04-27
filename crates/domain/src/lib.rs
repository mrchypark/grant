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
#[serde(rename_all = "camelCase")]
pub struct AgreementDocument {
    pub id: String,
    pub agreement_id: String,
    pub document_type: AgreementDocumentType,
    pub file_name: String,
    pub object_key: String,
    pub content_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_no: Option<String>,
    pub uploaded_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgreementDocumentType {
    MasterContract,
    Appendix,
    Amendment,
    AuthorizationLetter,
    InvoiceBasis,
    Other,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgreementStatus {
    Draft,
    Active,
    Expired,
    Terminated,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Action {
    pub id: String,
    pub code: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub risk_level: ActionRiskLevel,
    pub visibility_granting: bool,
    pub grantable_to_clients: bool,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Client {
    pub id: String,
    pub organization_id: String,
    pub slug: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub client_type: ClientType,
    pub app_type: ClientAppType,
    pub first_party: bool,
    pub consent_policy: ClientConsentPolicy,
    pub active: bool,
    pub created_by_user_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClientType {
    Confidential,
    Public,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClientAppType {
    Web,
    Mobile,
    Server,
    Internal,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClientConsentPolicy {
    Explicit,
    AutoApprove,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ClientAction {
    pub client_id: String,
    pub action_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ClientGrant {
    pub id: String,
    pub user_id: String,
    pub client_id: String,
    pub organization_id: String,
    pub status: ClientGrantStatus,
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
pub enum ClientGrantStatus {
    Active,
    Revoked,
    Expired,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ClientGrantAction {
    pub client_grant_id: String,
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
    pub resource_type: RoleBindingResourceType,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RoleBindingResourceType {
    Organization,
    Asset,
    Plant,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RoleBindingState {
    Active,
    Disabled,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Asset {
    pub id: String,
    pub owner_organization_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_by_organization_id: Option<String>,
    pub name: String,
    pub country_code: String,
    pub address: String,
    pub capacity_kw: f64,
    pub timezone: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AssetProfile {
    pub id: String,
    pub asset_id: String,
    pub country_code: String,
    pub provider: String,
    pub profile_type: String,
    pub external_id: String,
    pub metadata_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Registry {
    pub id: String,
    pub registry_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country_code: Option<String>,
    pub provider: String,
    pub name: String,
    pub metadata_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RegistryAccount {
    pub id: String,
    pub registry_id: String,
    pub organization_id: String,
    pub external_account_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub status: String,
    pub metadata_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RegistryProject {
    pub id: String,
    pub registry_id: String,
    pub asset_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registry_account_id: Option<String>,
    pub external_project_id: String,
    pub project_type: String,
    pub status: String,
    pub metadata_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DataSource {
    pub id: String,
    pub asset_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registry_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registry_project_id: Option<String>,
    pub source_type: String,
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_source_id: Option<String>,
    pub unit: String,
    pub metadata_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GenerationDatum {
    pub id: String,
    pub asset_id: String,
    pub source_id: String,
    pub period_start: String,
    pub period_end: String,
    pub metric_type: String,
    pub value: f64,
    pub unit: String,
    pub quality: String,
    pub metadata_json: String,
    pub recorded_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RateDatum {
    pub id: String,
    pub asset_id: String,
    pub source_id: String,
    pub period_start: String,
    pub period_end: String,
    pub metric_type: String,
    pub value: String,
    pub unit: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    pub quality: String,
    pub metadata_json: String,
    pub recorded_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EnergyCertificate {
    pub id: String,
    pub certificate_type: String,
    pub registry_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registry_project_id: Option<String>,
    pub asset_id: String,
    pub serial_number: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vintage_start: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vintage_end: Option<String>,
    pub quantity_mwh: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_account_id: Option<String>,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issued_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retired_at: Option<String>,
    pub metadata_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AuthCredential {
    pub id: String,
    pub organization_id: String,
    pub credential_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country_code: Option<String>,
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_subject_id: Option<String>,
    pub active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CredentialBinding {
    pub credential_id: String,
    pub registry_account_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Plant {
    pub id: String,
    pub facility_code: String,
    pub name: String,
    pub owner_organization_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_by_organization_id: Option<String>,
    pub contract_no: String,
    pub address: String,
    pub installed_kw: f64,
    pub rec_start_date: String,
    pub confirm_issued: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SyncMode {
    Delta,
    Full,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SyncRun {
    pub id: String,
    pub organization_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plant_id: Option<String>,
    pub mode: SyncMode,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    pub duration_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GenerationRecord {
    pub month: String,
    pub generation_kwh: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub self_consumption_kwh: Option<f64>,
    pub supply_kwh: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub application_deadline: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RateRecord {
    pub month: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub self_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub national_average: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nearby_average: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nearby_highest: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecRecord {
    pub month: String,
    pub application_no: String,
    pub supply_kwh: i64,
    pub rec_quantity: i64,
    pub fee_krw: i64,
    pub issued_date: String,
    pub expiry_date: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub virtual_account: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bank_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum DocumentCategory {
    PlantInfo,
    Permit,
    SiteLand,
    Drawings,
    Photos,
    Equipment,
    Finance,
    Development,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DocumentArtifact {
    pub id: String,
    pub category: DocumentCategory,
    pub file_class: String,
    pub file_sec_name: String,
    pub file_name: String,
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_url: Option<String>,
}

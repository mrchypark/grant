use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HealthStatus {
    pub status: &'static str,
}

impl HealthStatus {
    pub const fn ok() -> Self {
        Self { status: "ok" }
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
pub struct LegalEntity {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub representative_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BusinessRole {
    Owner,
    Admin,
    Viewer,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Business {
    pub id: String,
    pub display_name: String,
    pub business_no: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legal_entity_id: Option<String>,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BusinessMembership {
    pub user_id: String,
    pub business_id: String,
    pub role: BusinessRole,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Plant {
    pub code: String,
    pub name: String,
    pub business_no: String,
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
    pub business_no: String,
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

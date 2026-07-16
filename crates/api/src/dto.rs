use domain::{
    Action, ActionRiskLevel, AgreementStatus, AgreementType, ApplicationAppType, ApplicationGrant,
    ApplicationType, FieldViewMode, Organization, RoleBindingState, RoleBindingSubjectType, User,
};
use serde::{Deserialize, Deserializer, Serialize, de::MapAccess, de::Visitor};
use std::{
    collections::{BTreeMap, HashSet},
    fmt,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateApplicationRequest {
    pub(crate) id: String,
    pub(crate) slug: String,
    pub(crate) name: String,
    pub(crate) description: Option<String>,
    pub(crate) application_type: ApplicationType,
    pub(crate) app_type: ApplicationAppType,
    pub(crate) active: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReplaceApplicationConsentRequest {
    pub(crate) action_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApplicationConsentResponse {
    pub(crate) grant: ApplicationGrant,
    pub(crate) actions: Vec<Action>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateGroupRequest {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) description: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateGroupRequest {
    pub(crate) name: String,
    pub(crate) description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SetApplicationTrustRequest {
    pub(crate) trusted: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SetUserActiveRequest {
    pub(crate) active: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReplaceApplicationActionsRequest {
    pub(crate) action_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReplaceActionsRequest {
    pub(crate) action_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReplaceProviderActionsRequest {
    pub(crate) actions: Vec<ProviderActionRequest>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderActionRequest {
    pub(crate) id: String,
    pub(crate) code: String,
    pub(crate) name: String,
    pub(crate) description: Option<String>,
    pub(crate) risk_level: ActionRiskLevel,
    pub(crate) visibility_granting: bool,
    pub(crate) grantable_to_applications: bool,
    pub(crate) consent_label: Option<String>,
    pub(crate) consent_description: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpsertAgreementRequest {
    pub(crate) id: String,
    pub(crate) agreement_type: AgreementType,
    pub(crate) grantor_organization_id: String,
    pub(crate) grantee_organization_id: String,
    pub(crate) status: AgreementStatus,
    pub(crate) title: String,
    pub(crate) reference_no: Option<String>,
    pub(crate) starts_at: Option<String>,
    pub(crate) ends_at: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateRoleRequest {
    pub(crate) id: String,
    pub(crate) code: String,
    pub(crate) name: String,
    pub(crate) description: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateRoleBindingRequest {
    pub(crate) id: String,
    pub(crate) subject_type: RoleBindingSubjectType,
    pub(crate) subject_id: String,
    pub(crate) subject_organization_id: String,
    pub(crate) provider_id: String,
    pub(crate) resource_type: String,
    pub(crate) resource_id: String,
    pub(crate) role_id: String,
    pub(crate) agreement_id: Option<String>,
    pub(crate) state: RoleBindingState,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FieldViewPolicyQuery {
    pub(crate) resource_type: String,
    pub(crate) schema_key: String,
    pub(crate) consistency: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReplaceFieldViewPolicyRequest {
    pub(crate) resource_type: String,
    pub(crate) schema_key: String,
    #[serde(deserialize_with = "deserialize_field_view_entries")]
    pub(crate) field_views: Vec<(String, FieldViewMode)>,
}

fn deserialize_field_view_entries<'de, D>(
    deserializer: D,
) -> Result<Vec<(String, FieldViewMode)>, D::Error>
where
    D: Deserializer<'de>,
{
    struct FieldViewEntriesVisitor;

    impl<'de> Visitor<'de> for FieldViewEntriesVisitor {
        type Value = Vec<(String, FieldViewMode)>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a map of JSON Pointer keys to field view modes")
        }

        fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
        where
            A: MapAccess<'de>,
        {
            let mut entries = Vec::with_capacity(map.size_hint().unwrap_or(0));
            let mut seen = HashSet::new();
            while let Some(pointer) = map.next_key::<String>()? {
                if !seen.insert(pointer.clone()) {
                    return Err(serde::de::Error::custom(format!(
                        "fieldViews contains duplicate pointer {pointer}"
                    )));
                }
                entries.push((pointer, map.next_value()?));
            }
            Ok(entries)
        }
    }

    deserializer.deserialize_map(FieldViewEntriesVisitor)
}

#[derive(Debug, Deserialize)]
pub(crate) struct AuthzSubjectRequest {
    #[serde(rename = "type")]
    pub(crate) subject_type: String,
    pub(crate) id: String,
    #[serde(rename = "org_id", alias = "orgId")]
    pub(crate) organization_id: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub(crate) struct AuthzResourceRequest {
    pub(crate) provider: String,
    #[serde(rename = "type")]
    pub(crate) resource_type: String,
    pub(crate) id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) view: Option<AuthzResourceViewRequest>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AuthzResourceViewRequest {
    pub(crate) schema_key: String,
    pub(crate) fields: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AuthzCheckRequest {
    pub(crate) subject: AuthzSubjectRequest,
    pub(crate) action: String,
    pub(crate) resource: AuthzResourceRequest,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AuthzBatchCheckRequest {
    pub(crate) checks: Vec<AuthzCheckRequest>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AuthzFilterRequest {
    pub(crate) subject: AuthzSubjectRequest,
    pub(crate) action: String,
    pub(crate) resources: Vec<AuthzResourceRequest>,
}

#[derive(Debug, Serialize)]
pub(crate) struct AuthzCheckResponse {
    pub(crate) allowed: bool,
    pub(crate) reason: String,
    #[serde(rename = "fieldViews", skip_serializing_if = "Option::is_none")]
    pub(crate) field_views: Option<BTreeMap<String, FieldViewMode>>,
}

#[derive(Debug, Serialize)]
pub(crate) struct AuthzBatchCheckResponse {
    pub(crate) results: Vec<AuthzCheckResponse>,
}

#[derive(Debug, Serialize)]
pub(crate) struct AuthzFilterResponse {
    pub(crate) resource_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ApplicationAuthzCheckRequest {
    pub(crate) action: String,
    pub(crate) resource: AuthzResourceRequest,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ApplicationAuthzBatchCheckRequest {
    pub(crate) checks: Vec<ApplicationAuthzCheckRequest>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ApplicationAuthzFilterRequest {
    pub(crate) action: String,
    pub(crate) resources: Vec<AuthzResourceRequest>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BackupListResponse {
    pub(crate) items: Vec<BackupItemResponse>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BackupItemResponse {
    pub(crate) name: String,
    pub(crate) last_modified: i64,
    pub(crate) size: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BootstrapOwnerResponse {
    pub(crate) user_id: String,
    pub(crate) organization_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CurrentUserResponse {
    pub(crate) provider: String,
    pub(crate) external_user_id: String,
    pub(crate) user: User,
    pub(crate) organizations: Vec<Organization>,
    pub(crate) active_organization_id: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct ReadConsistencyQuery {
    pub(crate) consistency: Option<String>,
}

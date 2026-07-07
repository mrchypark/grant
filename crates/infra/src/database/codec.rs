use anyhow::{Result, anyhow};
use domain::{
    Action, ActionRiskLevel, ApplicationAppType, ApplicationConsentPolicy, ApplicationGrantStatus,
    ApplicationType, RoleBinding, RoleBindingState, RoleBindingSubjectType,
};
use std::collections::HashSet;

pub(super) fn validate_provider_action_manifest(
    provider_id: &str,
    actions: &[Action],
) -> Result<()> {
    let mut ids = HashSet::new();
    let mut codes = HashSet::new();

    for action in actions {
        if action.provider_id != provider_id {
            return Err(anyhow!(
                "action {} provider {} does not match path provider {}",
                action.id,
                action.provider_id,
                provider_id
            ));
        }
        if !codes.insert(action.code.as_str()) {
            return Err(anyhow!(
                "duplicate action code in provider manifest: {}",
                action.code
            ));
        }
        if !ids.insert(action.id.as_str()) {
            return Err(anyhow!(
                "duplicate action id in provider manifest: {}",
                action.id
            ));
        }
    }

    Ok(())
}

pub(super) fn validate_role_binding(binding: &RoleBinding) -> Result<()> {
    if binding.subject_organization_id.trim().is_empty() {
        return Err(anyhow!(
            "role binding subject organization id must not be empty"
        ));
    }
    if binding.provider_id.trim().is_empty() {
        return Err(anyhow!("role binding provider id must not be empty"));
    }
    if binding.resource_type.trim().is_empty() {
        return Err(anyhow!("role binding resource type must not be empty"));
    }
    if binding.resource_id.trim().is_empty() {
        return Err(anyhow!("role binding resource id must not be empty"));
    }
    if matches!(binding.subject_type, RoleBindingSubjectType::Organization)
        && binding.subject_id != binding.subject_organization_id
    {
        return Err(anyhow!(
            "organization role binding must use the same subject id and subject organization id"
        ));
    }
    if !matches!(binding.subject_type, RoleBindingSubjectType::Organization)
        && binding.agreement_id.is_some()
    {
        return Err(anyhow!(
            "group/user role bindings must not carry agreement references"
        ));
    }
    if binding.provider_id == "grant"
        && binding.resource_type == "organization"
        && binding.resource_id != binding.subject_organization_id
        && binding.agreement_id.is_none()
    {
        return Err(anyhow!(
            "cross-organization role binding requires an agreement"
        ));
    }
    Ok(())
}

pub(super) fn role_binding_subject_type_str(subject_type: &RoleBindingSubjectType) -> &'static str {
    subject_type.as_db_str()
}

pub(super) fn role_binding_subject_type_from_str(value: &str) -> Result<RoleBindingSubjectType> {
    RoleBindingSubjectType::from_db_str(value).map_err(anyhow::Error::msg)
}

pub(super) fn organization_party_relation_type_str_legal() -> &'static str {
    "legal"
}

pub(super) fn role_binding_state_str(state: &RoleBindingState) -> &'static str {
    state.as_db_str()
}

pub(super) fn role_binding_state_from_str(value: &str) -> Result<RoleBindingState> {
    RoleBindingState::from_db_str(value).map_err(anyhow::Error::msg)
}

pub(super) fn agreement_type_str(agreement_type: &domain::AgreementType) -> &'static str {
    agreement_type.as_db_str()
}

pub(super) fn agreement_type_from_str(value: &str) -> Result<domain::AgreementType> {
    domain::AgreementType::from_db_str(value).map_err(anyhow::Error::msg)
}

pub(super) fn agreement_status_str(status: &domain::AgreementStatus) -> &'static str {
    status.as_db_str()
}

pub(super) fn agreement_status_from_str(value: &str) -> Result<domain::AgreementStatus> {
    domain::AgreementStatus::from_db_str(value).map_err(anyhow::Error::msg)
}

pub(super) fn application_type_str(application_type: &ApplicationType) -> &'static str {
    application_type.as_db_str()
}

pub(super) fn application_app_type_str(app_type: &ApplicationAppType) -> &'static str {
    app_type.as_db_str()
}

pub(super) fn application_consent_policy_str(policy: &ApplicationConsentPolicy) -> &'static str {
    policy.as_db_str()
}

pub(super) fn application_grant_status_str(status: &ApplicationGrantStatus) -> &'static str {
    status.as_db_str()
}

pub(super) fn application_grant_status_from_str(value: &str) -> Result<ApplicationGrantStatus> {
    ApplicationGrantStatus::from_db_str(value).map_err(anyhow::Error::msg)
}

pub(super) fn action_risk_level_str(value: &ActionRiskLevel) -> &'static str {
    value.as_db_str()
}

pub(super) fn action_risk_level_from_str(value: &str) -> Result<ActionRiskLevel> {
    ActionRiskLevel::from_db_str(value).map_err(anyhow::Error::msg)
}

pub(super) fn application_type_from_str(value: &str) -> Result<ApplicationType> {
    ApplicationType::from_db_str(value).map_err(anyhow::Error::msg)
}

pub(super) fn application_app_type_from_str(value: &str) -> Result<ApplicationAppType> {
    ApplicationAppType::from_db_str(value).map_err(anyhow::Error::msg)
}

pub(super) fn application_consent_policy_from_str(value: &str) -> Result<ApplicationConsentPolicy> {
    ApplicationConsentPolicy::from_db_str(value).map_err(anyhow::Error::msg)
}

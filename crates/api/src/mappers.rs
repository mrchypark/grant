use domain::Action;
use infra::service::{
    AuthorizationCheck, AuthorizationDecision, AuthorizationResource, AuthorizationResourceView,
    AuthorizationSubject, BackupInfo, CreateApplicationInput, CreateRoleBindingInput,
    CreateRoleInput, ReplaceFieldViewPolicyInput, UpsertAgreementInput, UpsertGroupInput,
};

use crate::dto::{
    AuthzCheckRequest, AuthzCheckResponse, AuthzFilterRequest, AuthzResourceRequest,
    AuthzSubjectRequest, BackupItemResponse, CreateApplicationRequest, CreateGroupRequest,
    CreateRoleBindingRequest, CreateRoleRequest, ProviderActionRequest,
    ReplaceFieldViewPolicyRequest, UpdateGroupRequest, UpsertAgreementRequest,
};

pub(crate) fn create_application_input(body: CreateApplicationRequest) -> CreateApplicationInput {
    CreateApplicationInput {
        id: body.id,
        slug: body.slug,
        name: body.name,
        description: body.description,
        application_type: body.application_type,
        app_type: body.app_type,
        active: body.active,
    }
}

pub(crate) fn create_group_input(body: CreateGroupRequest) -> UpsertGroupInput {
    UpsertGroupInput {
        id: body.id,
        name: body.name,
        description: body.description,
    }
}

pub(crate) fn update_group_input(group_id: String, body: UpdateGroupRequest) -> UpsertGroupInput {
    UpsertGroupInput {
        id: group_id,
        name: body.name,
        description: body.description,
    }
}

pub(crate) fn create_role_input(body: CreateRoleRequest) -> CreateRoleInput {
    CreateRoleInput {
        id: body.id,
        code: body.code,
        name: body.name,
        description: body.description,
    }
}

pub(crate) fn create_role_binding_input(body: CreateRoleBindingRequest) -> CreateRoleBindingInput {
    CreateRoleBindingInput {
        id: body.id,
        subject_type: body.subject_type,
        subject_id: body.subject_id,
        subject_organization_id: body.subject_organization_id,
        provider_id: body.provider_id,
        resource_type: body.resource_type,
        resource_id: body.resource_id,
        role_id: body.role_id,
        agreement_id: body.agreement_id,
        state: body.state,
    }
}

pub(crate) fn replace_field_view_policy_input(
    body: ReplaceFieldViewPolicyRequest,
) -> ReplaceFieldViewPolicyInput {
    ReplaceFieldViewPolicyInput {
        resource_type: body.resource_type,
        schema_key: body.schema_key,
        field_views: body.field_views,
    }
}

pub(crate) fn map_backup(item: BackupInfo) -> BackupItemResponse {
    BackupItemResponse {
        name: item.name,
        last_modified: item.last_modified,
        size: item.size,
    }
}

pub(crate) fn provider_action_input(provider_id: &str, body: ProviderActionRequest) -> Action {
    Action {
        id: body.id,
        provider_id: provider_id.to_owned(),
        code: body.code,
        name: body.name,
        description: body.description,
        risk_level: body.risk_level,
        visibility_granting: body.visibility_granting,
        grantable_to_applications: body.grantable_to_applications,
        consent_label: body.consent_label,
        consent_description: body.consent_description,
    }
}

pub(crate) fn agreement_input(body: UpsertAgreementRequest) -> UpsertAgreementInput {
    UpsertAgreementInput {
        id: body.id,
        agreement_type: body.agreement_type,
        grantor_organization_id: body.grantor_organization_id,
        grantee_organization_id: body.grantee_organization_id,
        status: body.status,
        title: body.title,
        reference_no: body.reference_no,
        starts_at: body.starts_at,
        ends_at: body.ends_at,
    }
}

pub(crate) fn authz_check_input(body: AuthzCheckRequest) -> AuthorizationCheck {
    AuthorizationCheck {
        subject: authz_subject_input(body.subject),
        action: body.action,
        resource: authz_resource_input(body.resource),
    }
}

pub(crate) fn authz_filter_inputs(
    body: AuthzFilterRequest,
) -> (Vec<String>, Vec<AuthorizationCheck>) {
    let subject = authz_subject_input(body.subject);
    let action = body.action;
    let mut requested_resource_ids = Vec::with_capacity(body.resources.len());
    let checks = body
        .resources
        .into_iter()
        .map(|resource| {
            let resource_id = resource.id.clone();
            requested_resource_ids.push(resource_id);
            AuthorizationCheck {
                subject: subject.clone(),
                action: action.clone(),
                resource: authz_resource_input(resource),
            }
        })
        .collect::<Vec<_>>();
    (requested_resource_ids, checks)
}

fn authz_subject_input(body: AuthzSubjectRequest) -> AuthorizationSubject {
    AuthorizationSubject {
        subject_type: body.subject_type,
        id: body.id,
        organization_id: body.organization_id,
    }
}

fn authz_resource_input(body: AuthzResourceRequest) -> AuthorizationResource {
    AuthorizationResource {
        provider_id: body.provider,
        resource_type: body.resource_type,
        id: body.id,
        view: body.view.map(|view| AuthorizationResourceView {
            schema_key: view.schema_key,
            fields: view.fields,
        }),
    }
}

pub(crate) fn authz_check_response(decision: AuthorizationDecision) -> AuthzCheckResponse {
    AuthzCheckResponse {
        allowed: decision.allowed,
        reason: decision.reason,
        field_views: decision.field_views,
    }
}

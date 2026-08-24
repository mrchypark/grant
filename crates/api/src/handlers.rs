use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use domain::{
    Action, Agreement, Application, ApplicationGrant, Group, GroupMember, HealthStatus,
    Organization, OrganizationMembership, Role, RoleBinding, ServiceProvider,
};
use infra::{
    config::ReadConsistency,
    database::{PatchDataAuthorityImport, PatchDataAuthorityImportResult},
    service::AppService,
};
use std::sync::Arc;

use crate::{
    dto::{
        ApplicationConsentResponse, ApplicationResponse, BootstrapOwnerResponse,
        CreateApplicationRequest, CreateGroupRequest, CreateRoleBindingRequest, CreateRoleRequest,
        CurrentUserResponse, MaterializeUserResponse, ReadConsistencyQuery, ReplaceActionsRequest,
        ReplaceApplicationActionsRequest, ReplaceApplicationConsentRequest,
        ReplaceProviderActionsRequest, SetApplicationTrustRequest, SetUserActiveRequest,
        UpdateGroupRequest, UpsertAgreementRequest,
    },
    error::{ApiError, ApiResult},
    mappers::{
        agreement_input, create_application_input, create_group_input, create_role_binding_input,
        create_role_input, provider_action_input, update_group_input,
    },
    request::{
        bearer_token, ensure_active_organization_path_matches, ensure_request_context_organization,
        header_value, optional_header_value, read_consistency_override, request_context,
        request_context_for_inactive_organization, require_master_key,
    },
};

pub(crate) async fn healthz(State(service): State<Arc<AppService>>) -> Json<HealthStatus> {
    Json(service.health_status().await)
}

pub(crate) async fn readyz(
    State(service): State<Arc<AppService>>,
) -> (StatusCode, Json<HealthStatus>) {
    let status = service.health_status().await;
    let http_status = if service.is_ready().await {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (http_status, Json(status))
}

pub(crate) async fn get_current_user(
    State(service): State<Arc<AppService>>,
    Query(query): Query<ReadConsistencyQuery>,
    headers: HeaderMap,
) -> ApiResult<Json<CurrentUserResponse>> {
    let bearer_token = bearer_token(&headers)?;
    let active_organization_id = optional_header_value(&headers, "x-organization-id")?;
    let read_consistency = read_consistency_override(&query)?;
    let current = service
        .current_user(
            bearer_token.as_deref(),
            active_organization_id.as_deref(),
            read_consistency,
        )
        .await
        .map_err(ApiError::from_auth)?;
    Ok(Json(CurrentUserResponse {
        provider: current.provider,
        external_user_id: current.external_user_id,
        user: current.user,
        organizations: current.organizations,
        active_organization_id: current.active_organization_id,
    }))
}

pub(crate) async fn materialize_current_user(
    State(service): State<Arc<AppService>>,
    headers: HeaderMap,
) -> ApiResult<Json<MaterializeUserResponse>> {
    let bearer_token = bearer_token(&headers)?;
    let organization_id = optional_header_value(&headers, "x-organization-id")?;
    let current = service
        .materialize_current_user(bearer_token.as_deref(), organization_id.as_deref())
        .await
        .map_err(ApiError::from_auth)?;
    Ok(Json(MaterializeUserResponse {
        user_id: current.user_id,
        organization_id: current.organization_id,
        membership_status: current.membership_status,
    }))
}

pub(crate) async fn list_providers(
    State(service): State<Arc<AppService>>,
    Query(query): Query<ReadConsistencyQuery>,
) -> ApiResult<Json<Vec<ServiceProvider>>> {
    let read_consistency = read_consistency_override(&query)?;
    let providers = service
        .list_service_providers(read_consistency)
        .await
        .map_err(ApiError::from_domain)?;
    Ok(Json(providers))
}

pub(crate) async fn upsert_provider(
    State(service): State<Arc<AppService>>,
    headers: HeaderMap,
    Json(body): Json<ServiceProvider>,
) -> ApiResult<(StatusCode, Json<ServiceProvider>)> {
    require_master_key(service.as_ref(), &headers)?;
    let provider = service
        .upsert_service_provider_admin(body)
        .await
        .map_err(ApiError::from_domain)?;
    Ok((StatusCode::CREATED, Json(provider)))
}

pub(crate) async fn replace_provider_actions(
    State(service): State<Arc<AppService>>,
    Path(provider_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<ReplaceProviderActionsRequest>,
) -> ApiResult<Json<Vec<Action>>> {
    require_master_key(service.as_ref(), &headers)?;
    if body.actions.iter().any(|action| action.visibility_granting) {
        return Err(ApiError::bad_request(anyhow::anyhow!(
            "visibilityGranting=true is not supported by the coarse authorization API"
        )));
    }
    let actions = body
        .actions
        .into_iter()
        .map(|action| provider_action_input(&provider_id, action))
        .collect();
    let actions = service
        .replace_provider_actions_admin(&provider_id, actions)
        .await
        .map_err(ApiError::from_domain)?;
    Ok(Json(actions))
}

pub(crate) async fn list_applications(
    State(service): State<Arc<AppService>>,
    Path(organization_id): Path<String>,
    Query(query): Query<ReadConsistencyQuery>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<Application>>> {
    let read_consistency = read_consistency_override(&query)?;
    let ctx = request_context(service.as_ref(), &headers, read_consistency.clone()).await?;
    ensure_request_context_organization(&organization_id, &ctx)?;
    let applications = service
        .list_applications_authorized(&ctx.user_id, &ctx.organization_id, read_consistency)
        .await
        .map_err(ApiError::from_domain)?;
    Ok(Json(applications))
}

pub(crate) async fn create_application(
    State(service): State<Arc<AppService>>,
    Path(organization_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<CreateApplicationRequest>,
) -> ApiResult<(StatusCode, Json<ApplicationResponse>)> {
    let ctx = request_context(service.as_ref(), &headers, Some(ReadConsistency::Strong)).await?;
    ensure_request_context_organization(&organization_id, &ctx)?;
    let application = service
        .create_application(
            &ctx.user_id,
            &ctx.organization_id,
            create_application_input(body),
        )
        .await
        .map_err(ApiError::from_domain)?;
    Ok((
        StatusCode::CREATED,
        Json(ApplicationResponse {
            application: application.application,
            redirect_uri: application.redirect_uri,
        }),
    ))
}

pub(crate) async fn get_application(
    State(service): State<Arc<AppService>>,
    Path((organization_id, application_id)): Path<(String, String)>,
    Query(query): Query<ReadConsistencyQuery>,
    headers: HeaderMap,
) -> ApiResult<Json<ApplicationResponse>> {
    let read_consistency = read_consistency_override(&query)?;
    let ctx = request_context(service.as_ref(), &headers, read_consistency.clone()).await?;
    ensure_request_context_organization(&organization_id, &ctx)?;
    let application = service
        .get_application_detail_authorized(
            &ctx.user_id,
            &ctx.organization_id,
            &application_id,
            read_consistency,
        )
        .await
        .map_err(ApiError::from_domain)?;
    Ok(Json(ApplicationResponse {
        application: application.application,
        redirect_uri: application.redirect_uri,
    }))
}

pub(crate) async fn list_application_actions(
    State(service): State<Arc<AppService>>,
    Path((organization_id, application_id)): Path<(String, String)>,
    Query(query): Query<ReadConsistencyQuery>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<Action>>> {
    let read_consistency = read_consistency_override(&query)?;
    let ctx = request_context(service.as_ref(), &headers, read_consistency.clone()).await?;
    ensure_request_context_organization(&organization_id, &ctx)?;
    let actions = service
        .list_application_actions_authorized(
            &ctx.user_id,
            &ctx.organization_id,
            &application_id,
            read_consistency,
        )
        .await
        .map_err(ApiError::from_domain)?;
    Ok(Json(actions))
}

pub(crate) async fn replace_application_actions(
    State(service): State<Arc<AppService>>,
    Path((organization_id, application_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<ReplaceApplicationActionsRequest>,
) -> ApiResult<Json<Vec<Action>>> {
    let ctx = request_context(service.as_ref(), &headers, Some(ReadConsistency::Strong)).await?;
    ensure_request_context_organization(&organization_id, &ctx)?;
    let actions = service
        .replace_application_actions_authorized(
            &ctx.user_id,
            &ctx.organization_id,
            &application_id,
            &body.action_ids,
        )
        .await
        .map_err(ApiError::from_domain)?;
    Ok(Json(actions))
}

pub(crate) async fn list_application_grants(
    State(service): State<Arc<AppService>>,
    Path(organization_id): Path<String>,
    Query(query): Query<ReadConsistencyQuery>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<ApplicationGrant>>> {
    let read_consistency = read_consistency_override(&query)?;
    let ctx = request_context(service.as_ref(), &headers, read_consistency.clone()).await?;
    ensure_request_context_organization(&organization_id, &ctx)?;
    let grants = service
        .list_application_grants_authorized(&ctx.user_id, &ctx.organization_id, read_consistency)
        .await
        .map_err(ApiError::from_domain)?;
    Ok(Json(grants))
}

pub(crate) async fn get_application_consent(
    State(service): State<Arc<AppService>>,
    Path((organization_id, application_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> ApiResult<Json<ApplicationConsentResponse>> {
    let ctx = request_context(service.as_ref(), &headers, Some(ReadConsistency::Strong)).await?;
    ensure_request_context_organization(&organization_id, &ctx)?;
    let Some((grant, actions)) = service
        .get_application_consent(&ctx.user_id, &ctx.organization_id, &application_id)
        .await
        .map_err(ApiError::from_domain)?
    else {
        return Err(ApiError {
            status: StatusCode::NOT_FOUND,
            message: "application consent not found".to_owned(),
        });
    };
    Ok(Json(ApplicationConsentResponse { grant, actions }))
}

pub(crate) async fn replace_application_consent(
    State(service): State<Arc<AppService>>,
    Path((organization_id, application_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<ReplaceApplicationConsentRequest>,
) -> ApiResult<Json<ApplicationConsentResponse>> {
    let ctx = request_context(service.as_ref(), &headers, Some(ReadConsistency::Strong)).await?;
    ensure_request_context_organization(&organization_id, &ctx)?;
    let (grant, actions) = service
        .replace_application_consent(
            &ctx.user_id,
            &ctx.organization_id,
            &application_id,
            &body.action_ids,
        )
        .await
        .map_err(ApiError::from_domain)?;
    Ok(Json(ApplicationConsentResponse { grant, actions }))
}

pub(crate) async fn revoke_application_consent(
    State(service): State<Arc<AppService>>,
    Path((organization_id, application_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> ApiResult<StatusCode> {
    let ctx = request_context(service.as_ref(), &headers, Some(ReadConsistency::Strong)).await?;
    ensure_request_context_organization(&organization_id, &ctx)?;
    service
        .revoke_application_consent(&ctx.user_id, &ctx.organization_id, &application_id)
        .await
        .map_err(ApiError::from_domain)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn set_application_trust(
    State(service): State<Arc<AppService>>,
    Path((organization_id, application_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<SetApplicationTrustRequest>,
) -> ApiResult<Json<Application>> {
    require_master_key(service.as_ref(), &headers)?;
    let application = service
        .set_application_trust_admin(&organization_id, &application_id, body.trusted)
        .await
        .map_err(ApiError::from_domain)?;
    Ok(Json(application))
}

pub(crate) async fn verify_application_provisioning(
    State(service): State<Arc<AppService>>,
    Path((organization_id, application_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> ApiResult<Json<Application>> {
    require_master_key(service.as_ref(), &headers)?;
    let application = service
        .verify_application_provisioning_admin(&organization_id, &application_id)
        .await
        .map_err(ApiError::from_domain)?;
    Ok(Json(application))
}

pub(crate) async fn set_user_active(
    State(service): State<Arc<AppService>>,
    Path(user_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<SetUserActiveRequest>,
) -> ApiResult<Json<domain::User>> {
    require_master_key(service.as_ref(), &headers)?;
    let user = service
        .set_user_active_admin(&user_id, body.active)
        .await
        .map_err(ApiError::from_domain)?;
    Ok(Json(user))
}

pub(crate) async fn list_groups(
    State(service): State<Arc<AppService>>,
    Path(organization_id): Path<String>,
    Query(query): Query<ReadConsistencyQuery>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<Group>>> {
    let consistency = read_consistency_override(&query)?;
    let ctx = request_context(service.as_ref(), &headers, consistency.clone()).await?;
    ensure_request_context_organization(&organization_id, &ctx)?;
    let groups = service
        .list_groups_authorized(&ctx.user_id, &ctx.organization_id, consistency)
        .await
        .map_err(ApiError::from_domain)?;
    Ok(Json(groups))
}

pub(crate) async fn create_group(
    State(service): State<Arc<AppService>>,
    Path(organization_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<CreateGroupRequest>,
) -> ApiResult<(StatusCode, Json<Group>)> {
    let ctx = request_context(service.as_ref(), &headers, Some(ReadConsistency::Strong)).await?;
    ensure_request_context_organization(&organization_id, &ctx)?;
    let group = service
        .create_group_authorized(&ctx.user_id, &ctx.organization_id, create_group_input(body))
        .await
        .map_err(ApiError::from_domain)?;
    Ok((StatusCode::CREATED, Json(group)))
}

pub(crate) async fn get_group(
    State(service): State<Arc<AppService>>,
    Path((organization_id, group_id)): Path<(String, String)>,
    Query(query): Query<ReadConsistencyQuery>,
    headers: HeaderMap,
) -> ApiResult<Json<Group>> {
    let consistency = read_consistency_override(&query)?;
    let ctx = request_context(service.as_ref(), &headers, consistency.clone()).await?;
    ensure_request_context_organization(&organization_id, &ctx)?;
    let group = service
        .get_group_authorized(&ctx.user_id, &ctx.organization_id, &group_id, consistency)
        .await
        .map_err(ApiError::from_domain)?;
    Ok(Json(group))
}

pub(crate) async fn update_group(
    State(service): State<Arc<AppService>>,
    Path((organization_id, group_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<UpdateGroupRequest>,
) -> ApiResult<Json<Group>> {
    let ctx = request_context(service.as_ref(), &headers, Some(ReadConsistency::Strong)).await?;
    ensure_request_context_organization(&organization_id, &ctx)?;
    let group = service
        .update_group_authorized(
            &ctx.user_id,
            &ctx.organization_id,
            &group_id,
            update_group_input(group_id.clone(), body),
        )
        .await
        .map_err(ApiError::from_domain)?;
    Ok(Json(group))
}

pub(crate) async fn delete_group(
    State(service): State<Arc<AppService>>,
    Path((organization_id, group_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> ApiResult<StatusCode> {
    let ctx = request_context(service.as_ref(), &headers, Some(ReadConsistency::Strong)).await?;
    ensure_request_context_organization(&organization_id, &ctx)?;
    service
        .delete_group_authorized(&ctx.user_id, &ctx.organization_id, &group_id)
        .await
        .map_err(ApiError::from_domain)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn list_group_members(
    State(service): State<Arc<AppService>>,
    Path((organization_id, group_id)): Path<(String, String)>,
    Query(query): Query<ReadConsistencyQuery>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<GroupMember>>> {
    let consistency = read_consistency_override(&query)?;
    let ctx = request_context(service.as_ref(), &headers, consistency.clone()).await?;
    ensure_request_context_organization(&organization_id, &ctx)?;
    let members = service
        .list_group_members_authorized(&ctx.user_id, &ctx.organization_id, &group_id, consistency)
        .await
        .map_err(ApiError::from_domain)?;
    Ok(Json(members))
}

pub(crate) async fn add_group_member(
    State(service): State<Arc<AppService>>,
    Path((organization_id, group_id, user_id)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> ApiResult<Json<GroupMember>> {
    let ctx = request_context(service.as_ref(), &headers, Some(ReadConsistency::Strong)).await?;
    ensure_request_context_organization(&organization_id, &ctx)?;
    let member = service
        .add_group_member_authorized(&ctx.user_id, &ctx.organization_id, &group_id, &user_id)
        .await
        .map_err(ApiError::from_domain)?;
    Ok(Json(member))
}

pub(crate) async fn remove_group_member(
    State(service): State<Arc<AppService>>,
    Path((organization_id, group_id, user_id)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> ApiResult<StatusCode> {
    let ctx = request_context(service.as_ref(), &headers, Some(ReadConsistency::Strong)).await?;
    ensure_request_context_organization(&organization_id, &ctx)?;
    service
        .remove_group_member_authorized(&ctx.user_id, &ctx.organization_id, &group_id, &user_id)
        .await
        .map_err(ApiError::from_domain)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn enroll_organization_member(
    State(service): State<Arc<AppService>>,
    Path((organization_id, user_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> ApiResult<Json<OrganizationMembership>> {
    let ctx = request_context(service.as_ref(), &headers, Some(ReadConsistency::Strong)).await?;
    ensure_request_context_organization(&organization_id, &ctx)?;
    let membership = service
        .enroll_organization_member_authorized(&ctx.user_id, &ctx.organization_id, &user_id)
        .await
        .map_err(ApiError::from_domain)?;
    Ok(Json(membership))
}

pub(crate) async fn list_agreements(
    State(service): State<Arc<AppService>>,
    Path(organization_id): Path<String>,
    Query(query): Query<ReadConsistencyQuery>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<Agreement>>> {
    let read_consistency = read_consistency_override(&query)?;
    let ctx = request_context(service.as_ref(), &headers, read_consistency.clone()).await?;
    ensure_request_context_organization(&organization_id, &ctx)?;
    let agreements = service
        .list_agreements_authorized(&ctx.user_id, &ctx.organization_id, read_consistency)
        .await
        .map_err(ApiError::from_domain)?;
    Ok(Json(agreements))
}

pub(crate) async fn create_agreement(
    State(service): State<Arc<AppService>>,
    Path(organization_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<UpsertAgreementRequest>,
) -> ApiResult<(StatusCode, Json<Agreement>)> {
    let ctx = request_context(service.as_ref(), &headers, Some(ReadConsistency::Strong)).await?;
    ensure_request_context_organization(&organization_id, &ctx)?;
    let agreement = service
        .create_agreement_authorized(&ctx.user_id, &ctx.organization_id, agreement_input(body))
        .await
        .map_err(ApiError::from_domain)?;
    Ok((StatusCode::CREATED, Json(agreement)))
}

pub(crate) async fn get_agreement(
    State(service): State<Arc<AppService>>,
    Path((organization_id, agreement_id)): Path<(String, String)>,
    Query(query): Query<ReadConsistencyQuery>,
    headers: HeaderMap,
) -> ApiResult<Json<Agreement>> {
    let read_consistency = read_consistency_override(&query)?;
    let ctx = request_context(service.as_ref(), &headers, read_consistency.clone()).await?;
    ensure_request_context_organization(&organization_id, &ctx)?;
    let agreement = service
        .get_agreement_authorized(
            &ctx.user_id,
            &ctx.organization_id,
            &agreement_id,
            read_consistency,
        )
        .await
        .map_err(ApiError::from_domain)?;
    Ok(Json(agreement))
}

pub(crate) async fn update_agreement(
    State(service): State<Arc<AppService>>,
    Path((organization_id, agreement_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<UpsertAgreementRequest>,
) -> ApiResult<Json<Agreement>> {
    let ctx = request_context(service.as_ref(), &headers, Some(ReadConsistency::Strong)).await?;
    ensure_request_context_organization(&organization_id, &ctx)?;
    let agreement = service
        .update_agreement_authorized(
            &ctx.user_id,
            &ctx.organization_id,
            &agreement_id,
            agreement_input(body),
        )
        .await
        .map_err(ApiError::from_domain)?;
    Ok(Json(agreement))
}

pub(crate) async fn list_roles(
    State(service): State<Arc<AppService>>,
    Path(organization_id): Path<String>,
    Query(query): Query<ReadConsistencyQuery>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<Role>>> {
    let read_consistency = read_consistency_override(&query)?;
    let ctx = request_context(service.as_ref(), &headers, read_consistency.clone()).await?;
    ensure_request_context_organization(&organization_id, &ctx)?;
    let roles = service
        .list_roles_authorized(&ctx.user_id, &ctx.organization_id, read_consistency)
        .await
        .map_err(ApiError::from_domain)?;
    Ok(Json(roles))
}

pub(crate) async fn create_role(
    State(service): State<Arc<AppService>>,
    Path(organization_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<CreateRoleRequest>,
) -> ApiResult<(StatusCode, Json<Role>)> {
    let ctx = request_context(service.as_ref(), &headers, Some(ReadConsistency::Strong)).await?;
    ensure_request_context_organization(&organization_id, &ctx)?;
    let role = service
        .create_role_authorized(&ctx.user_id, &ctx.organization_id, create_role_input(body))
        .await
        .map_err(ApiError::from_domain)?;
    Ok((StatusCode::CREATED, Json(role)))
}

pub(crate) async fn replace_role_actions(
    State(service): State<Arc<AppService>>,
    Path((organization_id, role_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<ReplaceActionsRequest>,
) -> ApiResult<Json<Vec<Action>>> {
    let ctx = request_context(service.as_ref(), &headers, Some(ReadConsistency::Strong)).await?;
    ensure_request_context_organization(&organization_id, &ctx)?;
    let actions = service
        .replace_role_actions_authorized(
            &ctx.user_id,
            &ctx.organization_id,
            &role_id,
            &body.action_ids,
        )
        .await
        .map_err(ApiError::from_domain)?;
    Ok(Json(actions))
}

pub(crate) async fn list_role_bindings(
    State(service): State<Arc<AppService>>,
    Path(organization_id): Path<String>,
    Query(query): Query<ReadConsistencyQuery>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<RoleBinding>>> {
    let read_consistency = read_consistency_override(&query)?;
    let ctx = request_context(service.as_ref(), &headers, read_consistency.clone()).await?;
    ensure_request_context_organization(&organization_id, &ctx)?;
    let bindings = service
        .list_role_bindings_authorized(&ctx.user_id, &ctx.organization_id, read_consistency)
        .await
        .map_err(ApiError::from_domain)?;
    Ok(Json(bindings))
}

pub(crate) async fn create_role_binding(
    State(service): State<Arc<AppService>>,
    Path(organization_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<CreateRoleBindingRequest>,
) -> ApiResult<(StatusCode, Json<RoleBinding>)> {
    let ctx = request_context(service.as_ref(), &headers, Some(ReadConsistency::Strong)).await?;
    ensure_request_context_organization(&organization_id, &ctx)?;
    let binding = service
        .create_role_binding_authorized(
            &ctx.user_id,
            &ctx.organization_id,
            create_role_binding_input(body),
        )
        .await
        .map_err(ApiError::from_domain)?;
    Ok((StatusCode::CREATED, Json(binding)))
}

pub(crate) async fn delete_role_binding(
    State(service): State<Arc<AppService>>,
    Path((organization_id, binding_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> ApiResult<StatusCode> {
    let ctx = request_context(service.as_ref(), &headers, Some(ReadConsistency::Strong)).await?;
    ensure_request_context_organization(&organization_id, &ctx)?;
    service
        .delete_role_binding_authorized(&ctx.user_id, &ctx.organization_id, &binding_id)
        .await
        .map_err(ApiError::from_domain)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn deactivate_organization(
    State(service): State<Arc<AppService>>,
    Path(path_organization_id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<Json<Organization>> {
    let ctx = request_context(service.as_ref(), &headers, Some(ReadConsistency::Strong)).await?;
    ensure_active_organization_path_matches(&path_organization_id, &ctx.organization_id)?;
    let organization = service
        .deactivate_organization(&ctx.user_id, &ctx.organization_id, &path_organization_id)
        .await
        .map_err(ApiError::from_domain)?;
    Ok(Json(organization))
}

pub(crate) async fn reactivate_organization(
    State(service): State<Arc<AppService>>,
    Path(path_organization_id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<Json<Organization>> {
    let ctx = request_context_for_inactive_organization(
        service.as_ref(),
        &headers,
        &path_organization_id,
        Some(ReadConsistency::Strong),
    )
    .await?;
    let organization = service
        .reactivate_organization(&ctx.user_id, &ctx.organization_id, &path_organization_id)
        .await
        .map_err(ApiError::from_domain)?;
    Ok(Json(organization))
}

pub(crate) async fn bootstrap_rauthy_owner(
    State(service): State<Arc<AppService>>,
    headers: HeaderMap,
) -> ApiResult<(StatusCode, Json<BootstrapOwnerResponse>)> {
    require_master_key(service.as_ref(), &headers)?;
    let bearer_token = bearer_token(&headers)?;
    let principal = service
        .bootstrap_rauthy_owner(bearer_token.as_deref())
        .await
        .map_err(ApiError::from_auth)?;
    Ok((
        StatusCode::CREATED,
        Json(BootstrapOwnerResponse {
            user_id: principal.user_id,
            organization_id: principal.organization_id,
        }),
    ))
}

pub(crate) async fn bootstrap_external_owner(
    State(service): State<Arc<AppService>>,
    headers: HeaderMap,
) -> ApiResult<(StatusCode, Json<BootstrapOwnerResponse>)> {
    require_master_key(service.as_ref(), &headers)?;
    let organization_id = header_value(&headers, "x-organization-id")?;
    let bearer_token = bearer_token(&headers)?;
    let principal = service
        .bootstrap_external_owner(bearer_token.as_deref(), Some(&organization_id))
        .await
        .map_err(ApiError::from_auth)?;
    Ok((
        StatusCode::CREATED,
        Json(BootstrapOwnerResponse {
            user_id: principal.user_id,
            organization_id: principal.organization_id,
        }),
    ))
}

pub(crate) async fn import_patch_data_authority(
    State(service): State<Arc<AppService>>,
    headers: HeaderMap,
    Json(command): Json<PatchDataAuthorityImport>,
) -> ApiResult<Json<PatchDataAuthorityImportResult>> {
    require_master_key(service.as_ref(), &headers)?;
    let receipt = service
        .import_patch_data_authority(command)
        .await
        .map_err(map_patch_data_import_error)?;
    Ok(Json(receipt))
}

fn map_patch_data_import_error(error: anyhow::Error) -> ApiError {
    let message = error.to_string();
    let chain = error
        .chain()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(": ");
    if message.contains("generation conflict") || chain.contains("authority_import_assertion_valid")
    {
        return ApiError {
            status: StatusCode::CONFLICT,
            message,
        };
    }
    if [
        "manifest",
        "unsupported manifestVersion",
        "unknown user",
        "unknown organization",
        "must be sorted",
        "hierarchy",
        "requires an active",
        "not supported",
        "entity limit",
        "must be 64",
        "is invalid",
        "new generation",
    ]
    .iter()
    .any(|needle| message.contains(needle))
    {
        return ApiError::bad_request(anyhow::anyhow!(message));
    }
    ApiError::internal(error)
}

pub(crate) async fn requires_voter_node() -> ApiResult<StatusCode> {
    Err(ApiError::requires_voter())
}

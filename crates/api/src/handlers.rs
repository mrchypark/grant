use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use domain::{
    Action, Agreement, Application, ApplicationGrant, ApplicationRedirectUri, HealthStatus,
    Organization, Role, RoleBinding, ServiceProvider,
};
use infra::{config::ReadConsistency, service::AppService};
use std::sync::Arc;

use crate::{
    dto::{
        ApplicationRedirectUriRequest, BootstrapOwnerResponse, CreateApplicationGrantRequest,
        CreateApplicationRequest, CreateRoleBindingRequest, CreateRoleRequest, CurrentUserResponse,
        ReadConsistencyQuery, ReplaceActionsRequest, ReplaceApplicationActionsRequest,
        ReplaceProviderActionsRequest, RotateClientSecretResponse, UpdateApplicationRequest,
        UpsertAgreementRequest,
    },
    error::{ApiError, ApiResult},
    mappers::{
        agreement_input, create_application_grant_input, create_application_input,
        create_role_binding_input, create_role_input, provider_action_input,
        update_application_input,
    },
    request::{
        bearer_token, ensure_active_organization_path_matches, ensure_request_context_organization,
        optional_header_value, read_consistency_override, request_context,
        request_context_for_inactive_organization, require_master_key,
    },
};

pub(crate) async fn healthz(State(service): State<Arc<AppService>>) -> Json<HealthStatus> {
    Json(service.health_status().await)
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
        .map_err(ApiError::from_domain)?;
    Ok(Json(CurrentUserResponse {
        provider: current.provider,
        external_user_id: current.external_user_id,
        user: current.user,
        organizations: current.organizations,
        active_organization_id: current.active_organization_id,
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
) -> ApiResult<(StatusCode, Json<Application>)> {
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
    Ok((StatusCode::CREATED, Json(application)))
}

pub(crate) async fn get_application(
    State(service): State<Arc<AppService>>,
    Path((organization_id, application_id)): Path<(String, String)>,
    Query(query): Query<ReadConsistencyQuery>,
    headers: HeaderMap,
) -> ApiResult<Json<Application>> {
    let read_consistency = read_consistency_override(&query)?;
    let ctx = request_context(service.as_ref(), &headers, read_consistency.clone()).await?;
    ensure_request_context_organization(&organization_id, &ctx)?;
    let application = service
        .get_application_authorized(
            &ctx.user_id,
            &ctx.organization_id,
            &application_id,
            read_consistency,
        )
        .await
        .map_err(ApiError::from_domain)?;
    Ok(Json(application))
}

pub(crate) async fn update_application(
    State(service): State<Arc<AppService>>,
    Path((organization_id, application_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<UpdateApplicationRequest>,
) -> ApiResult<Json<Application>> {
    let ctx = request_context(service.as_ref(), &headers, Some(ReadConsistency::Strong)).await?;
    ensure_request_context_organization(&organization_id, &ctx)?;
    let application = service
        .update_application(
            &ctx.user_id,
            &ctx.organization_id,
            &application_id,
            update_application_input(body),
        )
        .await
        .map_err(ApiError::from_domain)?;
    Ok(Json(application))
}

pub(crate) async fn list_application_redirect_uris(
    State(service): State<Arc<AppService>>,
    Path((organization_id, application_id)): Path<(String, String)>,
    Query(query): Query<ReadConsistencyQuery>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<ApplicationRedirectUri>>> {
    let read_consistency = read_consistency_override(&query)?;
    let ctx = request_context(service.as_ref(), &headers, read_consistency.clone()).await?;
    ensure_request_context_organization(&organization_id, &ctx)?;
    let redirect_uris = service
        .list_application_redirect_uris_authorized(
            &ctx.user_id,
            &ctx.organization_id,
            &application_id,
            read_consistency,
        )
        .await
        .map_err(ApiError::from_domain)?;
    Ok(Json(redirect_uris))
}

pub(crate) async fn create_application_redirect_uri(
    State(service): State<Arc<AppService>>,
    Path((organization_id, application_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<ApplicationRedirectUriRequest>,
) -> ApiResult<(StatusCode, Json<ApplicationRedirectUri>)> {
    let ctx = request_context(service.as_ref(), &headers, Some(ReadConsistency::Strong)).await?;
    ensure_request_context_organization(&organization_id, &ctx)?;
    let redirect_uri = service
        .create_application_redirect_uri_authorized(
            &ctx.user_id,
            &ctx.organization_id,
            &application_id,
            &body.redirect_uri,
        )
        .await
        .map_err(ApiError::from_domain)?;
    Ok((StatusCode::CREATED, Json(redirect_uri)))
}

pub(crate) async fn delete_application_redirect_uri(
    State(service): State<Arc<AppService>>,
    Path((organization_id, application_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<ApplicationRedirectUriRequest>,
) -> ApiResult<StatusCode> {
    let ctx = request_context(service.as_ref(), &headers, Some(ReadConsistency::Strong)).await?;
    ensure_request_context_organization(&organization_id, &ctx)?;
    service
        .delete_application_redirect_uri_authorized(
            &ctx.user_id,
            &ctx.organization_id,
            &application_id,
            &body.redirect_uri,
        )
        .await
        .map_err(ApiError::from_domain)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn rotate_application_client_secret(
    State(service): State<Arc<AppService>>,
    Path((organization_id, application_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> ApiResult<Json<RotateClientSecretResponse>> {
    let ctx = request_context(service.as_ref(), &headers, Some(ReadConsistency::Strong)).await?;
    ensure_request_context_organization(&organization_id, &ctx)?;
    let rotated = service
        .rotate_application_client_secret_authorized(
            &ctx.user_id,
            &ctx.organization_id,
            &application_id,
        )
        .await
        .map_err(ApiError::from_domain)?;
    Ok(Json(RotateClientSecretResponse {
        client_secret: rotated.client_secret,
        secret_rotated_at: rotated.secret_rotated_at,
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

pub(crate) async fn create_application_grant(
    State(service): State<Arc<AppService>>,
    Path(organization_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<CreateApplicationGrantRequest>,
) -> ApiResult<(StatusCode, Json<ApplicationGrant>)> {
    let ctx = request_context(service.as_ref(), &headers, Some(ReadConsistency::Strong)).await?;
    ensure_request_context_organization(&organization_id, &ctx)?;
    let grant = service
        .create_application_grant_authorized(
            &ctx.user_id,
            &ctx.organization_id,
            create_application_grant_input(body),
        )
        .await
        .map_err(ApiError::from_domain)?;
    Ok((StatusCode::CREATED, Json(grant)))
}

pub(crate) async fn replace_application_grant_actions(
    State(service): State<Arc<AppService>>,
    Path((organization_id, grant_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<ReplaceActionsRequest>,
) -> ApiResult<Json<Vec<Action>>> {
    let ctx = request_context(service.as_ref(), &headers, Some(ReadConsistency::Strong)).await?;
    ensure_request_context_organization(&organization_id, &ctx)?;
    let actions = service
        .replace_application_grant_actions_authorized(
            &ctx.user_id,
            &ctx.organization_id,
            &grant_id,
            &body.action_ids,
        )
        .await
        .map_err(ApiError::from_domain)?;
    Ok(Json(actions))
}

pub(crate) async fn revoke_application_grant(
    State(service): State<Arc<AppService>>,
    Path((organization_id, grant_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> ApiResult<Json<ApplicationGrant>> {
    let ctx = request_context(service.as_ref(), &headers, Some(ReadConsistency::Strong)).await?;
    ensure_request_context_organization(&organization_id, &ctx)?;
    let grant = service
        .revoke_application_grant_authorized(&ctx.user_id, &ctx.organization_id, &grant_id)
        .await
        .map_err(ApiError::from_domain)?;
    Ok(Json(grant))
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

pub(crate) async fn requires_voter_node() -> ApiResult<StatusCode> {
    Err(ApiError::requires_voter())
}

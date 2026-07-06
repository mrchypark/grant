use anyhow::anyhow;
use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
    routing::{any, delete, get, post, put},
};
use domain::{
    Action, ActionRiskLevel, Agreement, AgreementStatus, AgreementType, Application,
    ApplicationAppType, ApplicationConsentPolicy, ApplicationGrant, ApplicationType, HealthStatus,
    Organization, Role, RoleBinding, RoleBindingState, RoleBindingSubjectType, ServiceProvider,
    User,
};
use infra::{
    config::ReadConsistency,
    service::{
        AppService, AuthorizationResource, AuthorizationSubject, BackupInfo,
        CreateApplicationGrantInput, CreateApplicationInput, CreateRoleBindingInput,
        CreateRoleInput, ServiceError, ServiceErrorKind, UpdateApplicationInput,
        UpsertAgreementInput,
    },
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub fn build_router(service: Arc<AppService>) -> Router {
    build_shared_router()
        .route("/api/v1/admin/providers", post(upsert_provider))
        .route(
            "/api/v1/admin/providers/{providerId}/actions",
            put(replace_provider_actions),
        )
        .route(
            "/api/v1/orgs/{organizationId}/apps",
            post(create_application),
        )
        .route(
            "/api/v1/orgs/{organizationId}/apps/{applicationId}",
            put(update_application),
        )
        .route(
            "/api/v1/orgs/{organizationId}/apps/{applicationId}/actions",
            put(replace_application_actions),
        )
        .route(
            "/api/v1/orgs/{organizationId}/app-grants",
            post(create_application_grant),
        )
        .route(
            "/api/v1/orgs/{organizationId}/app-grants/{grantId}/actions",
            put(replace_application_grant_actions),
        )
        .route(
            "/api/v1/orgs/{organizationId}/app-grants/{grantId}/revoke",
            post(revoke_application_grant),
        )
        .route(
            "/api/v1/orgs/{organizationId}/agreements",
            post(create_agreement),
        )
        .route(
            "/api/v1/orgs/{organizationId}/agreements/{agreementId}",
            put(update_agreement),
        )
        .route("/api/v1/orgs/{organizationId}/roles", post(create_role))
        .route(
            "/api/v1/orgs/{organizationId}/roles/{roleId}/actions",
            put(replace_role_actions),
        )
        .route(
            "/api/v1/orgs/{organizationId}/bindings",
            post(create_role_binding),
        )
        .route(
            "/api/v1/orgs/{organizationId}/bindings/{bindingId}",
            delete(delete_role_binding),
        )
        .route(
            "/api/v1/orgs/{organizationId}/deactivate",
            post(deactivate_organization),
        )
        .route(
            "/api/v1/orgs/{organizationId}/reactivate",
            post(reactivate_organization),
        )
        .route(
            "/api/v1/admin/backups",
            get(list_backups_local).post(trigger_backup),
        )
        .route("/api/v1/admin/backups/s3", get(list_backups_s3))
        .route(
            "/api/v1/admin/bootstrap/trailbase-owner",
            post(bootstrap_trailbase_owner),
        )
        .with_state(service)
}

pub fn build_learner_router(service: Arc<AppService>) -> Router {
    build_shared_router()
        .route("/api/v1/admin/providers", any(requires_voter_node))
        .route(
            "/api/v1/admin/providers/{providerId}/actions",
            any(requires_voter_node),
        )
        .route(
            "/api/v1/orgs/{organizationId}/apps",
            post(requires_voter_node),
        )
        .route(
            "/api/v1/orgs/{organizationId}/apps/{applicationId}",
            put(requires_voter_node),
        )
        .route(
            "/api/v1/orgs/{organizationId}/apps/{applicationId}/actions",
            put(requires_voter_node),
        )
        .route(
            "/api/v1/orgs/{organizationId}/app-grants",
            post(requires_voter_node),
        )
        .route(
            "/api/v1/orgs/{organizationId}/app-grants/{grantId}/actions",
            put(requires_voter_node),
        )
        .route(
            "/api/v1/orgs/{organizationId}/app-grants/{grantId}/revoke",
            post(requires_voter_node),
        )
        .route(
            "/api/v1/orgs/{organizationId}/agreements",
            post(requires_voter_node),
        )
        .route(
            "/api/v1/orgs/{organizationId}/agreements/{agreementId}",
            put(requires_voter_node),
        )
        .route(
            "/api/v1/orgs/{organizationId}/roles",
            post(requires_voter_node),
        )
        .route(
            "/api/v1/orgs/{organizationId}/roles/{roleId}/actions",
            put(requires_voter_node),
        )
        .route(
            "/api/v1/orgs/{organizationId}/bindings",
            post(requires_voter_node),
        )
        .route(
            "/api/v1/orgs/{organizationId}/bindings/{bindingId}",
            delete(requires_voter_node),
        )
        .route(
            "/api/v1/orgs/{organizationId}/deactivate",
            post(requires_voter_node),
        )
        .route(
            "/api/v1/orgs/{organizationId}/reactivate",
            post(requires_voter_node),
        )
        .route("/api/v1/admin/backups", any(requires_voter_node))
        .route("/api/v1/admin/backups/s3", any(requires_voter_node))
        .route(
            "/api/v1/admin/bootstrap/trailbase-owner",
            any(requires_voter_node),
        )
        .with_state(service)
}

fn build_shared_router() -> Router<Arc<AppService>> {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/auth", get(auth_root))
        .route("/auth/{*trailbasePath}", any(proxy_trailbase_auth_ui))
        .route("/_/auth/{*trailbasePath}", any(proxy_trailbase_passthrough))
        .route(
            "/api/auth/v1/{*trailbasePath}",
            any(proxy_trailbase_passthrough),
        )
        .route("/api/v1/me", get(get_current_user))
        .route("/api/v1/providers", get(list_providers))
        .route("/api/v1/orgs/{organizationId}/apps", get(list_applications))
        .route(
            "/api/v1/orgs/{organizationId}/apps/{applicationId}",
            get(get_application),
        )
        .route(
            "/api/v1/orgs/{organizationId}/apps/{applicationId}/actions",
            get(list_application_actions),
        )
        .route(
            "/api/v1/orgs/{organizationId}/app-grants",
            get(list_application_grants),
        )
        .route(
            "/api/v1/orgs/{organizationId}/agreements",
            get(list_agreements),
        )
        .route(
            "/api/v1/orgs/{organizationId}/agreements/{agreementId}",
            get(get_agreement),
        )
        .route("/api/v1/orgs/{organizationId}/roles", get(list_roles))
        .route(
            "/api/v1/orgs/{organizationId}/bindings",
            get(list_role_bindings),
        )
        .route("/api/v1/authz/check", post(authz_check))
        .route("/api/v1/authz/batch-check", post(authz_batch_check))
        .route("/api/v1/authz/filter", post(authz_filter))
}

async fn healthz(State(service): State<Arc<AppService>>) -> Json<HealthStatus> {
    Json(service.health_status().await)
}

async fn auth_root() -> Redirect {
    Redirect::temporary("/auth/login")
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateApplicationRequest {
    id: String,
    slug: String,
    name: String,
    description: Option<String>,
    application_type: ApplicationType,
    app_type: ApplicationAppType,
    first_party: bool,
    consent_policy: ApplicationConsentPolicy,
    active: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateApplicationRequest {
    slug: String,
    name: String,
    description: Option<String>,
    application_type: ApplicationType,
    app_type: ApplicationAppType,
    first_party: bool,
    consent_policy: ApplicationConsentPolicy,
    active: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReplaceApplicationActionsRequest {
    action_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReplaceActionsRequest {
    action_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReplaceProviderActionsRequest {
    actions: Vec<ProviderActionRequest>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProviderActionRequest {
    id: String,
    code: String,
    name: String,
    description: Option<String>,
    risk_level: ActionRiskLevel,
    visibility_granting: bool,
    grantable_to_applications: bool,
    consent_label: Option<String>,
    consent_description: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateApplicationGrantRequest {
    id: String,
    user_id: String,
    application_id: String,
    action_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpsertAgreementRequest {
    id: String,
    agreement_type: AgreementType,
    grantor_organization_id: String,
    grantee_organization_id: String,
    status: AgreementStatus,
    title: String,
    reference_no: Option<String>,
    starts_at: Option<String>,
    ends_at: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateRoleRequest {
    id: String,
    code: String,
    name: String,
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateRoleBindingRequest {
    id: String,
    subject_type: RoleBindingSubjectType,
    subject_id: String,
    subject_organization_id: String,
    provider_id: String,
    resource_type: String,
    resource_id: String,
    role_id: String,
    agreement_id: Option<String>,
    state: RoleBindingState,
}

#[derive(Debug, Deserialize)]
struct AuthzSubjectRequest {
    #[serde(rename = "type")]
    subject_type: String,
    id: String,
    #[serde(rename = "org_id", alias = "orgId")]
    organization_id: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct AuthzResourceRequest {
    provider: String,
    #[serde(rename = "type")]
    resource_type: String,
    id: String,
}

#[derive(Debug, Deserialize)]
struct AuthzCheckRequest {
    subject: AuthzSubjectRequest,
    action: String,
    resource: AuthzResourceRequest,
}

#[derive(Debug, Deserialize)]
struct AuthzBatchCheckRequest {
    checks: Vec<AuthzCheckRequest>,
}

#[derive(Debug, Deserialize)]
struct AuthzFilterRequest {
    subject: AuthzSubjectRequest,
    action: String,
    resources: Vec<AuthzResourceRequest>,
}

#[derive(Debug, Serialize)]
struct AuthzCheckResponse {
    allowed: bool,
    reason: String,
}

#[derive(Debug, Serialize)]
struct AuthzBatchCheckResponse {
    results: Vec<AuthzCheckResponse>,
}

#[derive(Debug, Serialize)]
struct AuthzFilterResponse {
    resource_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BackupListResponse {
    items: Vec<BackupItemResponse>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BackupItemResponse {
    name: String,
    last_modified: i64,
    size: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BootstrapOwnerResponse {
    user_id: String,
    organization_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CurrentUserResponse {
    provider: String,
    external_user_id: String,
    user: User,
    organizations: Vec<Organization>,
    active_organization_id: Option<String>,
}

#[derive(Debug, Clone)]
struct RequestContext {
    user_id: String,
    organization_id: String,
}

#[derive(Debug, Default, Deserialize)]
struct ReadConsistencyQuery {
    consistency: Option<String>,
}

async fn get_current_user(
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

async fn list_providers(
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

async fn upsert_provider(
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

async fn replace_provider_actions(
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

async fn list_applications(
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

async fn create_application(
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
            CreateApplicationInput {
                id: body.id,
                slug: body.slug,
                name: body.name,
                description: body.description,
                application_type: body.application_type,
                app_type: body.app_type,
                first_party: body.first_party,
                consent_policy: body.consent_policy,
                active: body.active,
            },
        )
        .await
        .map_err(ApiError::from_domain)?;
    Ok((StatusCode::CREATED, Json(application)))
}

async fn get_application(
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

async fn update_application(
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
            UpdateApplicationInput {
                slug: body.slug,
                name: body.name,
                description: body.description,
                application_type: body.application_type,
                app_type: body.app_type,
                first_party: body.first_party,
                consent_policy: body.consent_policy,
                active: body.active,
            },
        )
        .await
        .map_err(ApiError::from_domain)?;
    Ok(Json(application))
}

async fn list_application_actions(
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

async fn replace_application_actions(
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

async fn list_application_grants(
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

async fn create_application_grant(
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
            CreateApplicationGrantInput {
                id: body.id,
                user_id: body.user_id,
                application_id: body.application_id,
                action_ids: body.action_ids,
            },
        )
        .await
        .map_err(ApiError::from_domain)?;
    Ok((StatusCode::CREATED, Json(grant)))
}

async fn replace_application_grant_actions(
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

async fn revoke_application_grant(
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

async fn list_agreements(
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

async fn create_agreement(
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

async fn get_agreement(
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

async fn update_agreement(
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

async fn list_roles(
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

async fn create_role(
    State(service): State<Arc<AppService>>,
    Path(organization_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<CreateRoleRequest>,
) -> ApiResult<(StatusCode, Json<Role>)> {
    let ctx = request_context(service.as_ref(), &headers, Some(ReadConsistency::Strong)).await?;
    ensure_request_context_organization(&organization_id, &ctx)?;
    let role = service
        .create_role_authorized(
            &ctx.user_id,
            &ctx.organization_id,
            CreateRoleInput {
                id: body.id,
                code: body.code,
                name: body.name,
                description: body.description,
            },
        )
        .await
        .map_err(ApiError::from_domain)?;
    Ok((StatusCode::CREATED, Json(role)))
}

async fn replace_role_actions(
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

async fn list_role_bindings(
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

async fn create_role_binding(
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
            },
        )
        .await
        .map_err(ApiError::from_domain)?;
    Ok((StatusCode::CREATED, Json(binding)))
}

async fn delete_role_binding(
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

async fn authz_check(
    State(service): State<Arc<AppService>>,
    Query(query): Query<ReadConsistencyQuery>,
    headers: HeaderMap,
    Json(body): Json<AuthzCheckRequest>,
) -> ApiResult<Json<AuthzCheckResponse>> {
    require_master_key(service.as_ref(), &headers)?;
    let response =
        run_authz_check(service.as_ref(), body, read_consistency_override(&query)?).await?;
    Ok(Json(response))
}

async fn authz_batch_check(
    State(service): State<Arc<AppService>>,
    Query(query): Query<ReadConsistencyQuery>,
    headers: HeaderMap,
    Json(body): Json<AuthzBatchCheckRequest>,
) -> ApiResult<Json<AuthzBatchCheckResponse>> {
    require_master_key(service.as_ref(), &headers)?;
    let read_consistency = read_consistency_override(&query)?;
    let mut results = Vec::with_capacity(body.checks.len());
    for check in body.checks {
        results.push(run_authz_check(service.as_ref(), check, read_consistency.clone()).await?);
    }
    Ok(Json(AuthzBatchCheckResponse { results }))
}

async fn authz_filter(
    State(service): State<Arc<AppService>>,
    Query(query): Query<ReadConsistencyQuery>,
    headers: HeaderMap,
    Json(body): Json<AuthzFilterRequest>,
) -> ApiResult<Json<AuthzFilterResponse>> {
    require_master_key(service.as_ref(), &headers)?;
    let read_consistency = read_consistency_override(&query)?;
    let mut resource_ids = Vec::new();
    for resource in body.resources {
        let resource_id = resource.id.clone();
        let check = AuthzCheckRequest {
            subject: AuthzSubjectRequest {
                subject_type: body.subject.subject_type.clone(),
                id: body.subject.id.clone(),
                organization_id: body.subject.organization_id.clone(),
            },
            action: body.action.clone(),
            resource,
        };
        let decision = run_authz_check(service.as_ref(), check, read_consistency.clone()).await?;
        if decision.allowed {
            resource_ids.push(resource_id);
        }
    }
    Ok(Json(AuthzFilterResponse { resource_ids }))
}

async fn deactivate_organization(
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

async fn reactivate_organization(
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

async fn trigger_backup(
    State(service): State<Arc<AppService>>,
    headers: HeaderMap,
) -> ApiResult<StatusCode> {
    require_master_key(service.as_ref(), &headers)?;
    service.trigger_backup().await.map_err(ApiError::internal)?;
    Ok(StatusCode::ACCEPTED)
}

async fn list_backups_local(
    State(service): State<Arc<AppService>>,
    headers: HeaderMap,
) -> ApiResult<Json<BackupListResponse>> {
    require_master_key(service.as_ref(), &headers)?;
    let items = service
        .list_backups_local()
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(BackupListResponse {
        items: items.into_iter().map(map_backup).collect(),
    }))
}

async fn list_backups_s3(
    State(service): State<Arc<AppService>>,
    headers: HeaderMap,
) -> ApiResult<Json<BackupListResponse>> {
    require_master_key(service.as_ref(), &headers)?;
    let items = service
        .list_backups_s3()
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(BackupListResponse {
        items: items.into_iter().map(map_backup).collect(),
    }))
}

async fn bootstrap_trailbase_owner(
    State(service): State<Arc<AppService>>,
    headers: HeaderMap,
) -> ApiResult<(StatusCode, Json<BootstrapOwnerResponse>)> {
    require_master_key(service.as_ref(), &headers)?;
    let bearer_token = bearer_token(&headers)?;
    let principal = service
        .bootstrap_trailbase_owner(bearer_token.as_deref())
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

async fn requires_voter_node() -> ApiResult<StatusCode> {
    Err(ApiError::requires_voter())
}

async fn proxy_trailbase_auth_ui(
    State(service): State<Arc<AppService>>,
    Path(trailbase_path): Path<String>,
    request: axum::extract::Request,
) -> ApiResult<Response> {
    proxy_trailbase_request(
        service.as_ref(),
        request,
        &format!("/_/auth/{}", trailbase_path.trim_start_matches('/')),
    )
    .await
}

async fn proxy_trailbase_passthrough(
    State(service): State<Arc<AppService>>,
    Path(trailbase_path): Path<String>,
    request: axum::extract::Request,
) -> ApiResult<Response> {
    let original_path = request.uri().path();
    let upstream_path = if original_path.starts_with("/_/auth/") {
        format!("/_/auth/{}", trailbase_path.trim_start_matches('/'))
    } else {
        format!("/api/auth/v1/{}", trailbase_path.trim_start_matches('/'))
    };
    proxy_trailbase_request(service.as_ref(), request, &upstream_path).await
}

async fn proxy_trailbase_request(
    service: &AppService,
    request: axum::extract::Request,
    upstream_path: &str,
) -> ApiResult<Response> {
    let (parts, body) = request.into_parts();
    let mut url = format!(
        "{}{}",
        service.trailbase_base_url().trim_end_matches('/'),
        upstream_path
    );
    if let Some(query) = parts.uri.query() {
        url.push('?');
        url.push_str(query);
    }

    let body_bytes = to_bytes(body, 10 * 1024 * 1024)
        .await
        .map_err(ApiError::internal)?;
    let http_client = reqwest::Client::new();
    let mut upstream = http_client.request(parts.method.clone(), url);
    for (name, value) in &parts.headers {
        if name == header::HOST || name == header::CONTENT_LENGTH {
            continue;
        }
        upstream = upstream.header(name, value);
    }
    let upstream_response = upstream
        .body(body_bytes.clone())
        .send()
        .await
        .map_err(ApiError::internal)?;

    let status = upstream_response.status();
    let headers = upstream_response.headers().clone();
    let bytes = upstream_response
        .bytes()
        .await
        .map_err(ApiError::internal)?;

    let mut response = Response::new(Body::from(bytes));
    *response.status_mut() = StatusCode::from_u16(status.as_u16()).map_err(ApiError::internal)?;
    let response_headers = response.headers_mut();
    for (name, value) in &headers {
        if name == header::CONTENT_LENGTH {
            continue;
        }
        response_headers.append(name, value.clone());
    }
    Ok(response)
}

fn map_backup(item: BackupInfo) -> BackupItemResponse {
    BackupItemResponse {
        name: item.name,
        last_modified: item.last_modified,
        size: item.size,
    }
}

fn provider_action_input(provider_id: &str, body: ProviderActionRequest) -> Action {
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

fn agreement_input(body: UpsertAgreementRequest) -> UpsertAgreementInput {
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

async fn run_authz_check(
    service: &AppService,
    body: AuthzCheckRequest,
    read_consistency: Option<ReadConsistency>,
) -> ApiResult<AuthzCheckResponse> {
    let decision = service
        .check_authorization(
            &AuthorizationSubject {
                subject_type: body.subject.subject_type,
                id: body.subject.id,
                organization_id: body.subject.organization_id,
            },
            &body.action,
            &AuthorizationResource {
                provider_id: body.resource.provider,
                resource_type: body.resource.resource_type,
                id: body.resource.id,
            },
            read_consistency,
        )
        .await
        .map_err(ApiError::from_domain)?;
    Ok(AuthzCheckResponse {
        allowed: decision.allowed,
        reason: decision.reason,
    })
}

async fn request_context(
    service: &AppService,
    headers: &HeaderMap,
    read_consistency: Option<ReadConsistency>,
) -> ApiResult<RequestContext> {
    let organization_id = header_value(headers, "x-organization-id")?;
    let bearer_token = bearer_token(headers)?;
    let principal = service
        .authenticate_request(
            bearer_token.as_deref(),
            Some(&organization_id),
            read_consistency,
        )
        .await
        .map_err(ApiError::from_auth)?;
    Ok(RequestContext {
        user_id: principal.user_id,
        organization_id: principal.organization_id,
    })
}

async fn request_context_for_inactive_organization(
    service: &AppService,
    headers: &HeaderMap,
    target_organization_id: &str,
    read_consistency: Option<ReadConsistency>,
) -> ApiResult<RequestContext> {
    let bearer_token = bearer_token(headers)?;
    let current = service
        .current_user(bearer_token.as_deref(), None, read_consistency.clone())
        .await
        .map_err(ApiError::from_auth)?;
    let has_membership = service
        .db()
        .has_organization_membership_with_consistency(
            &current.user.id,
            target_organization_id,
            read_consistency,
        )
        .await
        .map_err(ApiError::internal)?;
    if !has_membership {
        return Err(ApiError {
            status: StatusCode::FORBIDDEN,
            message: format!(
                "forbidden: user {} is not an active member of organization {}",
                current.user.id, target_organization_id
            ),
        });
    }
    Ok(RequestContext {
        user_id: current.user.id,
        organization_id: target_organization_id.to_owned(),
    })
}

fn read_consistency_override(query: &ReadConsistencyQuery) -> ApiResult<Option<ReadConsistency>> {
    let Some(value) = query.consistency.as_deref() else {
        return Ok(None);
    };
    match value.trim().to_ascii_lowercase().as_str() {
        "strong" => Ok(Some(ReadConsistency::Strong)),
        other => Err(ApiError::bad_request(anyhow!(
            "invalid consistency: {other}"
        ))),
    }
}

fn ensure_active_organization_path_matches(
    path_organization_id: &str,
    active_organization_id: &str,
) -> ApiResult<()> {
    if path_organization_id == active_organization_id {
        return Ok(());
    }
    Err(ApiError {
        status: StatusCode::FORBIDDEN,
        message: format!(
            "forbidden: organization path {} does not match active organization {}",
            path_organization_id, active_organization_id
        ),
    })
}

fn ensure_request_context_organization(
    path_organization_id: &str,
    ctx: &RequestContext,
) -> ApiResult<()> {
    ensure_active_organization_path_matches(path_organization_id, &ctx.organization_id)
}

fn require_master_key(service: &AppService, headers: &HeaderMap) -> ApiResult<()> {
    let provided = header_value(headers, "x-certd-master-key")?;
    if provided != service.master_key() {
        return Err(ApiError {
            status: StatusCode::UNAUTHORIZED,
            message: "unauthorized: invalid master key".to_owned(),
        });
    }
    Ok(())
}

fn header_value(headers: &HeaderMap, key: &'static str) -> ApiResult<String> {
    optional_header_value(headers, key)?
        .ok_or_else(|| ApiError::bad_request(anyhow!("missing {key} header")))
}

fn optional_header_value(headers: &HeaderMap, key: &'static str) -> ApiResult<Option<String>> {
    let value = headers.get(key);
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value
        .to_str()
        .map_err(|_| ApiError::bad_request(anyhow!("{key} header is not valid UTF-8")))?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ApiError::bad_request(anyhow!(
            "{key} header must not be empty"
        )));
    }
    Ok(Some(trimmed.to_owned()))
}

fn bearer_token(headers: &HeaderMap) -> ApiResult<Option<String>> {
    let value = match headers.get(header::AUTHORIZATION) {
        Some(value) => value,
        None => return Ok(None),
    };
    let value = value
        .to_str()
        .map_err(|_| ApiError::bad_request(anyhow!("authorization header is not valid UTF-8")))?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ApiError::bad_request(anyhow!(
            "authorization header must not be empty"
        )));
    }
    let token = trimmed
        .strip_prefix("Bearer ")
        .or_else(|| trimmed.strip_prefix("bearer "))
        .ok_or_else(|| ApiError::bad_request(anyhow!("authorization header must use Bearer")))?;
    let token = token.trim();
    if token.is_empty() {
        return Err(ApiError::bad_request(anyhow!(
            "authorization bearer token must not be empty"
        )));
    }
    Ok(Some(token.to_owned()))
}

type ApiResult<T> = Result<T, ApiError>;

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn requires_voter() -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: "endpoint requires voter node".to_owned(),
        }
    }

    fn bad_request(err: anyhow::Error) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: err.to_string(),
        }
    }

    fn internal<E>(err: E) -> Self
    where
        E: ToString,
    {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: err.to_string(),
        }
    }

    fn from_domain(err: anyhow::Error) -> Self {
        if let Some(status) = structured_status(&err) {
            return Self {
                status,
                message: err.to_string(),
            };
        }
        let message = err.to_string();
        let contains_chain =
            |needle: &str| err.chain().any(|cause| cause.to_string().contains(needle));
        let status = if message.contains("forbidden:") {
            StatusCode::FORBIDDEN
        } else if contains_chain("already exists")
            || contains_chain("UNIQUE constraint failed")
            || contains_chain("unique constraint failed")
        {
            StatusCode::CONFLICT
        } else if message.contains("unavailable:") {
            StatusCode::SERVICE_UNAVAILABLE
        } else if contains_chain("not found") {
            StatusCode::NOT_FOUND
        } else if contains_chain("must not be empty")
            || contains_chain("is not grantable to applications")
            || contains_chain("expired")
            || contains_chain("consumed")
            || contains_chain("payload mismatch")
            || contains_chain("mismatch")
            || contains_chain("must be included")
            || contains_chain("already used")
        {
            StatusCode::BAD_REQUEST
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };
        Self { status, message }
    }

    fn from_auth(err: anyhow::Error) -> Self {
        if let Some(status) = structured_status(&err) {
            return Self {
                status,
                message: err.to_string(),
            };
        }
        let message = err.to_string();
        let status = if message.contains("unauthorized:") {
            StatusCode::UNAUTHORIZED
        } else if message.contains("forbidden:") {
            StatusCode::FORBIDDEN
        } else if message.contains("unavailable:") {
            StatusCode::SERVICE_UNAVAILABLE
        } else if message.contains("missing ") || message.contains("header") {
            StatusCode::BAD_REQUEST
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };
        Self { status, message }
    }
}

fn structured_status(err: &anyhow::Error) -> Option<StatusCode> {
    err.chain().find_map(|cause| {
        cause
            .downcast_ref::<ServiceError>()
            .map(|err| match err.kind() {
                ServiceErrorKind::BadRequest => StatusCode::BAD_REQUEST,
                ServiceErrorKind::Unauthorized => StatusCode::UNAUTHORIZED,
                ServiceErrorKind::Forbidden => StatusCode::FORBIDDEN,
                ServiceErrorKind::NotFound => StatusCode::NOT_FOUND,
                ServiceErrorKind::Conflict => StatusCode::CONFLICT,
                ServiceErrorKind::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            })
    })
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = Json(serde_json::json!({
            "error": self.message,
        }));
        (self.status, body).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::{ApiError, ensure_active_organization_path_matches};
    use anyhow::anyhow;
    use axum::http::StatusCode;
    use infra::service::{ServiceError, ServiceErrorKind};

    #[test]
    fn auth_unavailable_errors_map_to_service_unavailable() {
        let err = ApiError::from_auth(anyhow!("unavailable: quorum read path is not ready"));
        assert_eq!(err.status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn domain_unavailable_errors_map_to_service_unavailable() {
        let err =
            ApiError::from_domain(anyhow!("unavailable: quorum validation path is not ready"));
        assert_eq!(err.status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn structured_service_errors_override_string_matching() {
        let err = ApiError::from_domain(anyhow::Error::new(ServiceError::new(
            ServiceErrorKind::BadRequest,
            "challengeSignatureBase64 must not be empty",
        )));
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert_eq!(err.message, "challengeSignatureBase64 must not be empty");
    }

    #[test]
    fn generic_invalid_domain_errors_stay_internal() {
        let err = ApiError::from_domain(anyhow!("invalid internal state"));
        assert_eq!(err.status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn organization_path_match_helper_returns_forbidden_on_mismatch() {
        let err = ensure_active_organization_path_matches("org-b", "org-a")
            .expect_err("mismatched organization path should be rejected");
        assert_eq!(err.status, StatusCode::FORBIDDEN);
        assert_eq!(
            err.message,
            "forbidden: organization path org-b does not match active organization org-a"
        );
    }
}

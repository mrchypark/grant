use anyhow::anyhow;
use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
    routing::{any, get, post, put},
};
use domain::{
    Action, Application, ApplicationAppType, ApplicationConsentPolicy, ApplicationType,
    HealthStatus, Organization, User,
};
use infra::{
    config::ReadConsistency,
    service::{
        AppService, BackupInfo, CreateApplicationInput, ServiceError, ServiceErrorKind,
        UpdateApplicationInput,
    },
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub fn build_router(service: Arc<AppService>) -> Router {
    build_shared_router()
        .route(
            "/api/v1/organizations/{organizationId}/applications",
            post(create_application),
        )
        .route(
            "/api/v1/organizations/{organizationId}/applications/{applicationId}",
            put(update_application),
        )
        .route(
            "/api/v1/organizations/{organizationId}/applications/{applicationId}/actions",
            put(replace_application_actions),
        )
        .route(
            "/api/v1/organizations/{organizationId}/deactivate",
            post(deactivate_organization),
        )
        .route(
            "/api/v1/organizations/{organizationId}/reactivate",
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
        .route(
            "/api/v1/organizations/{organizationId}/applications",
            any(requires_voter_node),
        )
        .route(
            "/api/v1/organizations/{organizationId}/applications/{applicationId}",
            any(requires_voter_node),
        )
        .route(
            "/api/v1/organizations/{organizationId}/applications/{applicationId}/actions",
            any(requires_voter_node),
        )
        .route(
            "/api/v1/organizations/{organizationId}/deactivate",
            any(requires_voter_node),
        )
        .route(
            "/api/v1/organizations/{organizationId}/reactivate",
            any(requires_voter_node),
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
        .route(
            "/api/v1/organizations/{organizationId}/applications",
            get(list_applications),
        )
        .route(
            "/api/v1/organizations/{organizationId}/applications/{applicationId}",
            get(get_application),
        )
        .route(
            "/api/v1/organizations/{organizationId}/applications/{applicationId}/actions",
            get(list_application_actions),
        )
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

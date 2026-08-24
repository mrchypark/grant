use anyhow::anyhow;
use axum::{
    Json,
    extract::{Query, State},
    http::HeaderMap,
};
use infra::{
    config::ReadConsistency,
    service::{AppService, ApplicationAuthorizationCheck, AuthorizationResource},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::{
    dto::{
        ApplicationAuthzBatchCheckRequest, ApplicationAuthzCheckRequest,
        ApplicationAuthzFilterRequest, AuthzBatchCheckRequest, AuthzBatchCheckResponse,
        AuthzCheckRequest, AuthzCheckResponse, AuthzFilterRequest, AuthzFilterResponse,
        ReadConsistencyQuery,
    },
    error::{ApiError, ApiResult},
    mappers::{authz_check_input, authz_check_response, authz_filter_inputs},
    request::{
        bearer_token, optional_header_value, read_consistency_override, require_app_auth_key,
        require_master_key,
    },
};

const MAX_AUTHZ_BATCH_CHECKS: usize = 1000;
const MAX_AUTHZ_FILTER_RESOURCES: usize = 1000;
const FIELD_VIEW_UNSUPPORTED: &str =
    "resource.view is not supported by the coarse authorization API";
const EXPOSURE_ARTIFACT_DIGEST_PREFIX: &str = "sha256:";
const EXPOSURE_ARTIFACT_DIGEST_HEX_LENGTH: usize = 64;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ExposureReadAuthorizationRequest {
    exposure_id: String,
    exposure_version: u64,
    artifact_digest: String,
    resource_id: String,
    request_binding: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExposureReadAuthorizationResponse {
    allowed: bool,
    reason: String,
    exposure_id: String,
    exposure_version: u64,
    artifact_digest: String,
    resource_id: String,
    request_binding: String,
    principal_digest: String,
    issued_at: String,
    expires_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PlantReadAuthorizationRequest {
    exposure_id: String,
    exposure_version: u64,
    artifact_digest: String,
    resource_id: String,
    request_binding: String,
    operation_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PlantReadAuthorizationResponse {
    allowed: bool,
    reason: String,
    exposure_id: String,
    exposure_version: u64,
    artifact_digest: String,
    resource_id: String,
    request_binding: String,
    principal_digest: String,
    issued_at: String,
    expires_at: String,
    operation_id: &'static str,
    provider_id: &'static str,
    action_id: &'static str,
    resource_type: &'static str,
}

fn application_check_input(body: ApplicationAuthzCheckRequest) -> ApplicationAuthorizationCheck {
    ApplicationAuthorizationCheck {
        action: body.action,
        resource: AuthorizationResource {
            provider_id: body.resource.provider,
            resource_type: body.resource.resource_type,
            id: body.resource.id,
        },
    }
}

fn application_auth_headers(
    service: &AppService,
    headers: &HeaderMap,
) -> ApiResult<(String, String)> {
    require_app_auth_key(service, headers)?;
    let token = bearer_token(headers)?.ok_or_else(|| ApiError {
        status: axum::http::StatusCode::UNAUTHORIZED,
        message: "unauthorized: missing authorization header".to_owned(),
    })?;
    let organization_id =
        optional_header_value(headers, "x-organization-id")?.ok_or_else(|| ApiError {
            status: axum::http::StatusCode::BAD_REQUEST,
            message: "missing x-organization-id header".to_owned(),
        })?;
    Ok((token, organization_id))
}

pub(crate) async fn application_authz_check(
    State(service): State<Arc<AppService>>,
    headers: HeaderMap,
    Json(body): Json<ApplicationAuthzCheckRequest>,
) -> ApiResult<Json<AuthzCheckResponse>> {
    let (token, organization_id) = application_auth_headers(service.as_ref(), &headers)?;
    reject_resource_views(std::iter::once(&body.resource))?;
    let decision = service
        .check_application_authorizations(
            Some(&token),
            &organization_id,
            &[application_check_input(body)],
        )
        .await
        .map_err(ApiError::from_auth)?
        .into_iter()
        .next()
        .ok_or_else(|| ApiError::internal("application authorization check failed"))?;
    Ok(Json(authz_check_response(decision)))
}

pub(crate) async fn exposure_read_authorization(
    State(service): State<Arc<AppService>>,
    headers: HeaderMap,
    Json(body): Json<ExposureReadAuthorizationRequest>,
) -> ApiResult<Json<ExposureReadAuthorizationResponse>> {
    let (token, organization_id) = application_auth_headers(service.as_ref(), &headers)?;
    validate_exposure_read_request(&body)?;
    let decision = service
        .check_exposure_read_authorization(
            Some(&token),
            &organization_id,
            &body.exposure_id,
            &body.resource_id,
        )
        .await
        .map_err(ApiError::from_auth)?;
    Ok(Json(ExposureReadAuthorizationResponse {
        allowed: decision.allowed,
        reason: decision.reason,
        exposure_id: body.exposure_id,
        exposure_version: body.exposure_version,
        artifact_digest: body.artifact_digest,
        resource_id: body.resource_id,
        request_binding: body.request_binding,
        principal_digest: decision.principal_digest,
        issued_at: decision.issued_at,
        expires_at: decision.expires_at,
    }))
}

pub(crate) async fn plant_read_authorization(
    State(service): State<Arc<AppService>>,
    headers: HeaderMap,
    Json(body): Json<PlantReadAuthorizationRequest>,
) -> ApiResult<Json<PlantReadAuthorizationResponse>> {
    let (token, organization_id) = application_auth_headers(service.as_ref(), &headers)?;
    validate_exposure_read_fields(
        &body.exposure_id,
        body.exposure_version,
        &body.artifact_digest,
        &body.resource_id,
        &body.request_binding,
    )?;
    if body.operation_id != "plant.get" {
        return Err(ApiError::bad_request(anyhow!(
            "operationId must be plant.get"
        )));
    }
    let decision = service
        .check_patch_data_plant_read_authorization(
            Some(&token),
            &organization_id,
            &body.resource_id,
        )
        .await
        .map_err(ApiError::from_auth)?;
    Ok(Json(PlantReadAuthorizationResponse {
        allowed: decision.allowed,
        reason: decision.reason,
        exposure_id: body.exposure_id,
        exposure_version: body.exposure_version,
        artifact_digest: body.artifact_digest,
        resource_id: body.resource_id,
        request_binding: body.request_binding,
        principal_digest: decision.principal_digest,
        issued_at: decision.issued_at,
        expires_at: decision.expires_at,
        operation_id: "plant.get",
        provider_id: "patch-data",
        action_id: "patch-data.plant.read",
        resource_type: "plant",
    }))
}

fn validate_exposure_read_request(body: &ExposureReadAuthorizationRequest) -> ApiResult<()> {
    validate_exposure_read_fields(
        &body.exposure_id,
        body.exposure_version,
        &body.artifact_digest,
        &body.resource_id,
        &body.request_binding,
    )
}

fn validate_exposure_read_fields(
    exposure_id: &str,
    exposure_version: u64,
    artifact_digest: &str,
    resource_id: &str,
    request_binding: &str,
) -> ApiResult<()> {
    require_exact_non_empty("exposureId", exposure_id)?;
    if exposure_version == 0 {
        return Err(ApiError::bad_request(anyhow!(
            "exposureVersion must be greater than zero"
        )));
    }
    let digest_hex = artifact_digest.strip_prefix(EXPOSURE_ARTIFACT_DIGEST_PREFIX);
    if artifact_digest.len()
        != EXPOSURE_ARTIFACT_DIGEST_PREFIX.len() + EXPOSURE_ARTIFACT_DIGEST_HEX_LENGTH
        || digest_hex.is_none_or(|digest_hex| {
            digest_hex.len() != EXPOSURE_ARTIFACT_DIGEST_HEX_LENGTH
                || !digest_hex
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
    {
        return Err(ApiError::bad_request(anyhow!(
            "artifactDigest must be the literal sha256: followed by exactly 64 lowercase hexadecimal characters"
        )));
    }
    require_exact_non_empty("resourceId", resource_id)?;
    require_exact_non_empty("requestBinding", request_binding)?;
    Ok(())
}

fn require_exact_non_empty(field: &str, value: &str) -> ApiResult<()> {
    if value.is_empty() || value.trim() != value {
        return Err(ApiError::bad_request(anyhow!(
            "{field} must be non-empty and must not have surrounding whitespace"
        )));
    }
    Ok(())
}

pub(crate) async fn application_authz_batch_check(
    State(service): State<Arc<AppService>>,
    headers: HeaderMap,
    Json(body): Json<ApplicationAuthzBatchCheckRequest>,
) -> ApiResult<Json<AuthzBatchCheckResponse>> {
    let (token, organization_id) = application_auth_headers(service.as_ref(), &headers)?;
    reject_resource_views(body.checks.iter().map(|check| &check.resource))?;
    if body.checks.len() > MAX_AUTHZ_BATCH_CHECKS {
        return Err(ApiError::bad_request(anyhow!(
            "checks must contain at most {MAX_AUTHZ_BATCH_CHECKS} items"
        )));
    }
    let checks = body
        .checks
        .into_iter()
        .map(application_check_input)
        .collect::<Vec<_>>();
    let results = service
        .check_application_authorizations(Some(&token), &organization_id, &checks)
        .await
        .map_err(ApiError::from_auth)?
        .into_iter()
        .map(authz_check_response)
        .collect::<Vec<_>>();
    if results.len() != checks.len() {
        return Err(ApiError::internal(
            "application authorization batch result length mismatch",
        ));
    }
    Ok(Json(AuthzBatchCheckResponse { results }))
}

pub(crate) async fn application_authz_filter(
    State(service): State<Arc<AppService>>,
    headers: HeaderMap,
    Json(body): Json<ApplicationAuthzFilterRequest>,
) -> ApiResult<Json<AuthzFilterResponse>> {
    let (token, organization_id) = application_auth_headers(service.as_ref(), &headers)?;
    reject_resource_views(&body.resources)?;
    if body.resources.len() > MAX_AUTHZ_FILTER_RESOURCES {
        return Err(ApiError::bad_request(anyhow!(
            "resources must contain at most {MAX_AUTHZ_FILTER_RESOURCES} items"
        )));
    }
    let mut ids = Vec::with_capacity(body.resources.len());
    let mut checks = Vec::with_capacity(body.resources.len());
    for resource in body.resources {
        ids.push(resource.id.clone());
        checks.push(application_check_input(ApplicationAuthzCheckRequest {
            action: body.action.clone(),
            resource,
        }));
    }
    let decisions = service
        .check_application_authorizations(Some(&token), &organization_id, &checks)
        .await
        .map_err(ApiError::from_auth)?;
    let resource_ids = ids
        .into_iter()
        .zip(decisions)
        .filter_map(|(id, decision)| decision.allowed.then_some(id))
        .collect();
    Ok(Json(AuthzFilterResponse { resource_ids }))
}

pub(crate) async fn authz_check(
    State(service): State<Arc<AppService>>,
    Query(query): Query<ReadConsistencyQuery>,
    headers: HeaderMap,
    Json(body): Json<AuthzCheckRequest>,
) -> ApiResult<Json<AuthzCheckResponse>> {
    require_master_key(service.as_ref(), &headers)?;
    reject_resource_views(std::iter::once(&body.resource))?;
    let response =
        run_authz_check(service.as_ref(), body, read_consistency_override(&query)?).await?;
    Ok(Json(response))
}

pub(crate) async fn authz_batch_check(
    State(service): State<Arc<AppService>>,
    Query(query): Query<ReadConsistencyQuery>,
    headers: HeaderMap,
    Json(body): Json<AuthzBatchCheckRequest>,
) -> ApiResult<Json<AuthzBatchCheckResponse>> {
    require_master_key(service.as_ref(), &headers)?;
    reject_resource_views(body.checks.iter().map(|check| &check.resource))?;
    if body.checks.len() > MAX_AUTHZ_BATCH_CHECKS {
        return Err(ApiError::bad_request(anyhow!(
            "checks must contain at most {MAX_AUTHZ_BATCH_CHECKS} items"
        )));
    }
    let read_consistency = read_consistency_override(&query)?;
    let checks = body
        .checks
        .into_iter()
        .map(authz_check_input)
        .collect::<Vec<_>>();
    let results = service
        .check_authorizations(&checks, read_consistency)
        .await
        .map_err(ApiError::from_domain)?
        .into_iter()
        .map(authz_check_response)
        .collect::<Vec<_>>();
    if results.len() != checks.len() {
        return Err(ApiError::internal(
            "authorization batch result length mismatch",
        ));
    }
    Ok(Json(AuthzBatchCheckResponse { results }))
}

pub(crate) async fn authz_filter(
    State(service): State<Arc<AppService>>,
    Query(query): Query<ReadConsistencyQuery>,
    headers: HeaderMap,
    Json(body): Json<AuthzFilterRequest>,
) -> ApiResult<Json<AuthzFilterResponse>> {
    require_master_key(service.as_ref(), &headers)?;
    reject_resource_views(&body.resources)?;
    if body.resources.len() > MAX_AUTHZ_FILTER_RESOURCES {
        return Err(ApiError::bad_request(anyhow!(
            "resources must contain at most {MAX_AUTHZ_FILTER_RESOURCES} items"
        )));
    }
    let read_consistency = read_consistency_override(&query)?;
    let (requested_resource_ids, checks) = authz_filter_inputs(body);
    let decisions = service
        .check_authorizations(&checks, read_consistency)
        .await
        .map_err(ApiError::from_domain)?;
    let resource_ids = requested_resource_ids
        .into_iter()
        .zip(decisions)
        .filter_map(|(resource_id, decision)| {
            if decision.allowed {
                Some(resource_id)
            } else {
                None
            }
        })
        .collect();
    Ok(Json(AuthzFilterResponse { resource_ids }))
}

fn reject_resource_views<'a>(
    resources: impl IntoIterator<Item = &'a crate::dto::AuthzResourceRequest>,
) -> ApiResult<()> {
    if resources
        .into_iter()
        .any(|resource| resource.field_view_present)
    {
        return Err(ApiError::bad_request(anyhow!(FIELD_VIEW_UNSUPPORTED)));
    }
    Ok(())
}

async fn run_authz_check(
    service: &AppService,
    body: AuthzCheckRequest,
    read_consistency: Option<ReadConsistency>,
) -> ApiResult<AuthzCheckResponse> {
    let decision = service
        .check_authorizations(&[authz_check_input(body)], read_consistency)
        .await
        .map_err(ApiError::from_domain)?
        .into_iter()
        .next()
        .ok_or_else(|| ApiError::internal("authorization check failed"))?;
    Ok(authz_check_response(decision))
}

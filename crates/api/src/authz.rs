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

fn application_check_input(body: ApplicationAuthzCheckRequest) -> ApplicationAuthorizationCheck {
    ApplicationAuthorizationCheck {
        action: body.action,
        resource: AuthorizationResource {
            provider_id: body.resource.provider,
            resource_type: body.resource.resource_type,
            id: body.resource.id,
            view: body
                .resource
                .view
                .map(|view| infra::service::AuthorizationResourceView {
                    schema_key: view.schema_key,
                    fields: view.fields,
                }),
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

pub(crate) async fn application_authz_batch_check(
    State(service): State<Arc<AppService>>,
    headers: HeaderMap,
    Json(body): Json<ApplicationAuthzBatchCheckRequest>,
) -> ApiResult<Json<AuthzBatchCheckResponse>> {
    let (token, organization_id) = application_auth_headers(service.as_ref(), &headers)?;
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
    if body.resources.len() > MAX_AUTHZ_FILTER_RESOURCES {
        return Err(ApiError::bad_request(anyhow!(
            "resources must contain at most {MAX_AUTHZ_FILTER_RESOURCES} items"
        )));
    }
    reject_filter_views(&body.resources)?;
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
    if body.resources.len() > MAX_AUTHZ_FILTER_RESOURCES {
        return Err(ApiError::bad_request(anyhow!(
            "resources must contain at most {MAX_AUTHZ_FILTER_RESOURCES} items"
        )));
    }
    reject_filter_views(&body.resources)?;
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

fn reject_filter_views(resources: &[crate::dto::AuthzResourceRequest]) -> ApiResult<()> {
    if resources.iter().any(|resource| resource.view.is_some()) {
        return Err(ApiError::bad_request(anyhow!(
            "resource.view is not supported by authorization filters"
        )));
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

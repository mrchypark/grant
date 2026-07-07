use anyhow::anyhow;
use axum::{
    Json,
    extract::{Query, State},
    http::HeaderMap,
};
use infra::{config::ReadConsistency, service::AppService};
use std::sync::Arc;

use crate::{
    dto::{
        AuthzBatchCheckRequest, AuthzBatchCheckResponse, AuthzCheckRequest, AuthzCheckResponse,
        AuthzFilterRequest, AuthzFilterResponse, ReadConsistencyQuery,
    },
    error::{ApiError, ApiResult},
    mappers::{authz_check_input, authz_check_response, authz_filter_inputs},
    request::{read_consistency_override, require_master_key},
};

const MAX_AUTHZ_BATCH_CHECKS: usize = 1000;
const MAX_AUTHZ_FILTER_RESOURCES: usize = 1000;

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
        .collect();
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

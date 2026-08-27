use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use infra::{
    database::{
        ActivateHttpCapabilityGeneration, ActiveHttpCapabilityCatalog, HttpCapabilityActivation,
        ProviderPepClient, StageHttpCapabilityGeneration, StagedHttpCapabilityGeneration,
        UpsertProviderPepClient,
    },
    service::{AppService, HttpCapabilityDecision, HttpCapabilityDecisionInput},
};
use std::sync::Arc;

use crate::{
    error::{ApiError, ApiResult},
    request::{bearer_token, header_value, require_app_auth_key, require_master_key},
};

pub(crate) async fn stage_http_capability_generation(
    State(service): State<Arc<AppService>>,
    Path(provider_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<StageHttpCapabilityGeneration>,
) -> ApiResult<(StatusCode, Json<StagedHttpCapabilityGeneration>)> {
    require_master_key(service.as_ref(), &headers)?;
    let result = service
        .stage_http_capability_generation_admin(&provider_id, body)
        .await
        .map_err(ApiError::from_domain)?;
    Ok((StatusCode::CREATED, Json(result)))
}

pub(crate) async fn activate_http_capability_generation(
    State(service): State<Arc<AppService>>,
    Path(provider_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<ActivateHttpCapabilityGeneration>,
) -> ApiResult<Json<HttpCapabilityActivation>> {
    require_master_key(service.as_ref(), &headers)?;
    let result = service
        .activate_http_capability_generation_admin(&provider_id, body)
        .await
        .map_err(ApiError::from_domain)?;
    Ok(Json(result))
}

pub(crate) async fn set_provider_pep_client(
    State(service): State<Arc<AppService>>,
    Path((provider_id, pep_client_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<UpsertProviderPepClient>,
) -> ApiResult<Json<ProviderPepClient>> {
    require_master_key(service.as_ref(), &headers)?;
    let result = service
        .set_provider_pep_client_admin(&provider_id, &pep_client_id, body)
        .await
        .map_err(ApiError::from_domain)?;
    Ok(Json(result))
}

pub(crate) async fn get_active_http_capability_catalog(
    State(service): State<Arc<AppService>>,
    Path(provider_id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<Json<ActiveHttpCapabilityCatalog>> {
    require_app_auth_key(service.as_ref(), &headers)?;
    let result = service
        .active_http_capability_catalog(&provider_id)
        .await
        .map_err(ApiError::from_auth)?;
    Ok(Json(result))
}

pub(crate) async fn decide_http_capability(
    State(service): State<Arc<AppService>>,
    headers: HeaderMap,
    Json(body): Json<HttpCapabilityDecisionInput>,
) -> ApiResult<Json<HttpCapabilityDecision>> {
    require_app_auth_key(service.as_ref(), &headers)?;
    let bearer = bearer_token(&headers)?;
    let organization_id = header_value(&headers, "x-organization-id")?;
    let result = service
        .decide_http_capability(bearer.as_deref(), &organization_id, body)
        .await
        .map_err(ApiError::from_auth)?;
    Ok(Json(result))
}

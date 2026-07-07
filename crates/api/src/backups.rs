use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use infra::service::AppService;
use std::sync::Arc;

use crate::{
    dto::BackupListResponse,
    error::{ApiError, ApiResult},
    mappers::map_backup,
    request::require_master_key,
};

pub(crate) async fn trigger_backup(
    State(service): State<Arc<AppService>>,
    headers: HeaderMap,
) -> ApiResult<StatusCode> {
    require_master_key(service.as_ref(), &headers)?;
    service.trigger_backup().await.map_err(ApiError::internal)?;
    Ok(StatusCode::ACCEPTED)
}

pub(crate) async fn list_backups_local(
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

pub(crate) async fn list_backups_s3(
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

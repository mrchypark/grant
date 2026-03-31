use anyhow::anyhow;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use domain::{DocumentArtifact, GenerationRecord, HealthStatus, Plant, RateRecord, RecRecord, SyncMode, SyncRun};
use infra::{
    database::StoredCertificate,
    service::{AppService, BackupInfo, PlantView, PutCertificateInput, SyncInput},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub fn build_router(service: Arc<AppService>) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route(
            "/api/v1/businesses/{businessNo}/certificate",
            put(put_business_certificate),
        )
        .route("/api/v1/businesses/{businessNo}/sync", post(sync_business))
        .route("/api/v1/businesses/{businessNo}/plants", get(list_business_plants))
        .route("/api/v1/plants/{facilityCode}", get(get_plant))
        .route(
            "/api/v1/plants/{facilityCode}/generation",
            get(list_plant_generation),
        )
        .route("/api/v1/plants/{facilityCode}/rates", get(list_plant_rates))
        .route("/api/v1/plants/{facilityCode}/rec", get(list_plant_rec))
        .route(
            "/api/v1/plants/{facilityCode}/documents",
            get(list_plant_documents),
        )
        .route("/api/v1/documents/{documentId}/download", get(download_document))
        .route("/api/v1/admin/backups", get(list_backups_local).post(trigger_backup))
        .route("/api/v1/admin/backups/s3", get(list_backups_s3))
        .with_state(service)
}

async fn healthz() -> Json<HealthStatus> {
    Json(HealthStatus::ok())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CertificateUploadRequest {
    password: String,
    sign_cert_der_base64: String,
    sign_pri_key_base64: String,
    km_cert_der_base64: String,
    km_pri_key_base64: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CertificateResponse {
    id: String,
    business_no: String,
    common_name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SyncRequest {
    facility_code: Option<String>,
    start_month: Option<String>,
    end_month: Option<String>,
    mode: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SyncResponse {
    id: String,
    business_no: String,
    plant_id: Option<String>,
    mode: String,
    status: String,
    started_at: Option<String>,
    finished_at: Option<String>,
    duration_ms: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PlantResponse {
    plant: Plant,
    last_synced_at: Option<String>,
    last_sync: Option<SyncResponse>,
    generation: PlantMetricF64,
    rates: PlantMetricCount,
    rec: PlantMetricI64,
    documents: PlantDocumentMetric,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PlantMetricF64 {
    count: i64,
    total_kwh: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PlantMetricCount {
    count: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PlantMetricI64 {
    count: i64,
    total: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PlantDocumentMetric {
    count: i64,
    categories: std::collections::BTreeMap<String, i64>,
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

async fn put_business_certificate(
    State(service): State<Arc<AppService>>,
    Path(business_no): Path<String>,
    Json(body): Json<CertificateUploadRequest>,
) -> ApiResult<Json<CertificateResponse>> {
    let cert = service
        .put_certificate(PutCertificateInput {
            business_no,
            password: body.password,
            sign_cert_der_b64: body.sign_cert_der_base64,
            sign_pri_key_b64: body.sign_pri_key_base64,
            km_cert_der_b64: body.km_cert_der_base64,
            km_pri_key_b64: body.km_pri_key_base64,
        })
        .await
        .map_err(ApiError::bad_request_if_known)?;
    Ok(Json(map_certificate(cert)))
}

async fn sync_business(
    State(service): State<Arc<AppService>>,
    Path(business_no): Path<String>,
    Json(body): Json<SyncRequest>,
) -> ApiResult<(StatusCode, Json<SyncResponse>)> {
    let mode = parse_sync_mode(body.mode.as_deref())?;
    let run = service
        .sync(SyncInput {
            business_no,
            facility_code: body.facility_code,
            start_month: body.start_month,
            end_month: body.end_month,
            mode,
        })
        .await
        .map_err(ApiError::internal)?;
    Ok((StatusCode::CREATED, Json(map_sync(run))))
}

async fn list_business_plants(
    State(service): State<Arc<AppService>>,
    Path(business_no): Path<String>,
) -> ApiResult<Json<Vec<Plant>>> {
    let plants = service
        .list_plants(&business_no)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(plants))
}

async fn get_plant(
    State(service): State<Arc<AppService>>,
    Path(facility_code): Path<String>,
) -> ApiResult<Json<PlantResponse>> {
    let plant = service
        .get_plant(&facility_code)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(map_plant(plant)))
}

async fn list_plant_generation(
    State(service): State<Arc<AppService>>,
    Path(facility_code): Path<String>,
) -> ApiResult<Json<Vec<GenerationRecord>>> {
    let items = service
        .list_generation(&facility_code)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(items))
}

async fn list_plant_rates(
    State(service): State<Arc<AppService>>,
    Path(facility_code): Path<String>,
) -> ApiResult<Json<Vec<RateRecord>>> {
    let items = service
        .list_rates(&facility_code)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(items))
}

async fn list_plant_rec(
    State(service): State<Arc<AppService>>,
    Path(facility_code): Path<String>,
) -> ApiResult<Json<Vec<RecRecord>>> {
    let items = service
        .list_rec(&facility_code)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(items))
}

async fn list_plant_documents(
    State(service): State<Arc<AppService>>,
    Path(facility_code): Path<String>,
) -> ApiResult<Json<Vec<DocumentArtifact>>> {
    let items = service
        .list_documents(&facility_code)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(items))
}

async fn download_document(
    State(service): State<Arc<AppService>>,
    Path(document_id): Path<String>,
) -> ApiResult<Response> {
    let (doc, bytes) = service
        .download_document(&document_id)
        .await
        .map_err(ApiError::internal)?;

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&doc.content_type).map_err(ApiError::internal)?,
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename*=UTF-8''{}", urlencoding::encode(&doc.artifact.file_name)))
            .map_err(ApiError::internal)?,
    );
    Ok((headers, bytes).into_response())
}

async fn trigger_backup(State(service): State<Arc<AppService>>) -> ApiResult<StatusCode> {
    service.trigger_backup().await.map_err(ApiError::internal)?;
    Ok(StatusCode::ACCEPTED)
}

async fn list_backups_local(State(service): State<Arc<AppService>>) -> ApiResult<Json<BackupListResponse>> {
    let items = service
        .list_backups_local()
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(BackupListResponse {
        items: items.into_iter().map(map_backup).collect(),
    }))
}

async fn list_backups_s3(State(service): State<Arc<AppService>>) -> ApiResult<Json<BackupListResponse>> {
    let items = service
        .list_backups_s3()
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(BackupListResponse {
        items: items.into_iter().map(map_backup).collect(),
    }))
}

fn parse_sync_mode(value: Option<&str>) -> ApiResult<SyncMode> {
    match value.unwrap_or("delta").trim().to_ascii_lowercase().as_str() {
        "delta" => Ok(SyncMode::Delta),
        "full" => Ok(SyncMode::Full),
        other => Err(ApiError::bad_request(anyhow!("invalid mode: {other}"))),
    }
}

fn map_certificate(cert: StoredCertificate) -> CertificateResponse {
    CertificateResponse {
        id: cert.id,
        business_no: cert.business_id,
        common_name: cert.common_name,
    }
}

fn map_sync(run: SyncRun) -> SyncResponse {
    SyncResponse {
        id: run.id,
        business_no: run.business_no,
        plant_id: run.plant_id,
        mode: match run.mode {
            SyncMode::Delta => "delta".to_owned(),
            SyncMode::Full => "full".to_owned(),
        },
        status: run.status,
        started_at: run.started_at,
        finished_at: run.finished_at,
        duration_ms: run.duration_ms,
    }
}

fn map_plant(view: PlantView) -> PlantResponse {
    PlantResponse {
        plant: view.plant,
        last_synced_at: view.last_synced_at,
        last_sync: view.last_sync.map(map_sync),
        generation: PlantMetricF64 {
            count: view.generation_count,
            total_kwh: view.generation_total_kwh,
        },
        rates: PlantMetricCount {
            count: view.rates_count,
        },
        rec: PlantMetricI64 {
            count: view.rec_count,
            total: view.rec_total,
        },
        documents: PlantDocumentMetric {
            count: view.document_count,
            categories: view.document_categories,
        },
    }
}

fn map_backup(item: BackupInfo) -> BackupItemResponse {
    BackupItemResponse {
        name: item.name,
        last_modified: item.last_modified,
        size: item.size,
    }
}

type ApiResult<T> = Result<T, ApiError>;

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn bad_request(err: anyhow::Error) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: err.to_string(),
        }
    }

    fn bad_request_if_known(err: anyhow::Error) -> Self {
        let message = err.to_string();
        let status = if message.contains("businessNo") || message.contains("base64") || message.contains("certificate") {
            StatusCode::BAD_REQUEST
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };
        Self { status, message }
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
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = Json(serde_json::json!({
            "error": self.message,
        }));
        (self.status, body).into_response()
    }
}

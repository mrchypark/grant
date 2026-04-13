use anyhow::anyhow;
use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
    routing::{any, get, post, put},
};
use base64::Engine;
use domain::{
    Action, Client, ClientAppType, ClientConsentPolicy, ClientType, DocumentArtifact,
    GenerationRecord, HealthStatus, Organization, Plant, RateRecord, RecRecord, SyncMode, SyncRun,
    User,
};
use infra::{
    config::ReadConsistency,
    crypto::CertificateBundle,
    database::StoredCertificate,
    service::{
        AppService, BackupInfo, CreateCertGatewayChallengeInput, CreateClientInput,
        CreatePlantInput, ExecuteEphemeralCrawlInput, IssueCrawlTaskTokenInput, PlantView,
        PutCertificateInput, RegisterCertificateFromProofInput, ServiceError, ServiceErrorKind,
        StartCrawlWithTaskTokenInput, SyncInput, UpdateClientInput,
    },
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub fn build_router(service: Arc<AppService>) -> Router {
    build_shared_router()
        .route(
            "/api/v1/organizations/{organizationId}/clients",
            post(create_client),
        )
        .route(
            "/api/v1/organizations/{organizationId}/clients/{clientId}",
            put(update_client),
        )
        .route(
            "/api/v1/organizations/{organizationId}/clients/{clientId}/actions",
            put(replace_client_actions),
        )
        .route(
            "/api/v1/organizations/{organizationId}/certificate",
            put(put_organization_certificate),
        )
        .route(
            "/api/v1/organizations/{organizationId}/sync",
            post(sync_organization),
        )
        .route(
            "/api/v1/organizations/{organizationId}/cert-gateway/challenges",
            post(create_cert_gateway_challenge),
        )
        .route(
            "/api/v1/organizations/{organizationId}/cert-gateway/ephemeral-executions",
            post(execute_ephemeral_crawl),
        )
        .route(
            "/api/v1/organizations/{organizationId}/certificates",
            get(list_stored_certificates).post(register_stored_certificate),
        )
        .route(
            "/api/v1/organizations/{organizationId}/plants",
            post(create_plant),
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
            "/api/v1/organizations/{organizationId}/certificates/{certificateId}/crawl-tokens",
            post(issue_crawl_task_token),
        )
        .route(
            "/api/v1/organizations/{organizationId}/crawls",
            post(start_crawl),
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
            "/api/v1/organizations/{organizationId}/clients",
            any(requires_voter_node),
        )
        .route(
            "/api/v1/organizations/{organizationId}/clients/{clientId}",
            any(requires_voter_node),
        )
        .route(
            "/api/v1/organizations/{organizationId}/clients/{clientId}/actions",
            any(requires_voter_node),
        )
        .route(
            "/api/v1/organizations/{organizationId}/certificate",
            any(requires_voter_node),
        )
        .route(
            "/api/v1/organizations/{organizationId}/sync",
            any(requires_voter_node),
        )
        .route(
            "/api/v1/organizations/{organizationId}/cert-gateway/challenges",
            any(requires_voter_node),
        )
        .route(
            "/api/v1/organizations/{organizationId}/cert-gateway/ephemeral-executions",
            any(requires_voter_node),
        )
        .route(
            "/api/v1/organizations/{organizationId}/certificates",
            any(requires_voter_node),
        )
        .route(
            "/api/v1/organizations/{organizationId}/plants",
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
        .route(
            "/api/v1/organizations/{organizationId}/certificates/{certificateId}/crawl-tokens",
            any(requires_voter_node),
        )
        .route(
            "/api/v1/organizations/{organizationId}/crawls",
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
            "/api/v1/organizations/{organizationId}/clients",
            get(list_clients),
        )
        .route(
            "/api/v1/organizations/{organizationId}/clients/{clientId}",
            get(get_client),
        )
        .route(
            "/api/v1/organizations/{organizationId}/clients/{clientId}/actions",
            get(list_client_actions),
        )
        .route("/api/v1/plants", get(list_plants))
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
        .route(
            "/api/v1/documents/{documentId}/download",
            get(download_document),
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
struct CertificateUploadRequest {
    business_no: Option<String>,
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
    organization_id: String,
    party_id: String,
    business_no: Option<String>,
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateCertGatewayChallengeRequest {
    mode: String,
    facility_code: Option<String>,
    ttl_seconds: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CertGatewayChallengeResponse {
    challenge_id: String,
    mode: String,
    nonce: String,
    payload: String,
    expires_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProofBundleRequest {
    business_no: String,
    password: String,
    common_name: String,
    sign_cert_der_base64: String,
    sign_pri_key_base64: String,
    km_cert_der_base64: String,
    km_pri_key_base64: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RegisterStoredCertificateRequest {
    challenge_id: String,
    challenge_payload: String,
    challenge_signature_base64: String,
    bundle: ProofBundleRequest,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExecuteEphemeralCrawlRequest {
    challenge_id: String,
    challenge_payload: String,
    challenge_signature_base64: String,
    bundle: ProofBundleRequest,
    facility_code: Option<String>,
    start_month: Option<String>,
    end_month: Option<String>,
    mode: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IssueCrawlTaskTokenRequest {
    facility_code: Option<String>,
    start_month: Option<String>,
    end_month: Option<String>,
    mode: Option<String>,
    ttl_seconds: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CrawlTaskTokenResponse {
    crawl_task_token: String,
    expires_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreatePlantRequest {
    facility_code: String,
    name: String,
    contract_no: String,
    address: String,
    installed_kw: f64,
    rec_start_date: String,
    confirm_issued: String,
    owner_organization_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateClientRequest {
    id: String,
    slug: String,
    name: String,
    description: Option<String>,
    client_type: ClientType,
    app_type: ClientAppType,
    first_party: bool,
    consent_policy: ClientConsentPolicy,
    active: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateClientRequest {
    slug: String,
    name: String,
    description: Option<String>,
    client_type: ClientType,
    app_type: ClientAppType,
    first_party: bool,
    consent_policy: ClientConsentPolicy,
    active: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReplaceClientActionsRequest {
    action_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SyncResponse {
    id: String,
    organization_id: String,
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

async fn put_organization_certificate(
    State(service): State<Arc<AppService>>,
    Path(organization_id): Path<String>,
    Json(body): Json<CertificateUploadRequest>,
) -> ApiResult<Json<CertificateResponse>> {
    let cert = service
        .put_certificate(PutCertificateInput {
            organization_id: organization_id.clone(),
            business_no: body.business_no,
            password: body.password,
            sign_cert_der_b64: body.sign_cert_der_base64,
            sign_pri_key_b64: body.sign_pri_key_base64,
            km_cert_der_b64: body.km_cert_der_base64,
            km_pri_key_b64: body.km_pri_key_base64,
        })
        .await
        .map_err(ApiError::bad_request_if_known)?;
    Ok(Json(map_certificate(cert, &organization_id)))
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

async fn sync_organization(
    State(service): State<Arc<AppService>>,
    Path(organization_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<SyncRequest>,
) -> ApiResult<(StatusCode, Json<SyncResponse>)> {
    let ctx = request_context(service.as_ref(), &headers, Some(ReadConsistency::Strong)).await?;
    ensure_active_organization_path_matches(&organization_id, &ctx.organization_id)?;
    let mode = parse_sync_mode(body.mode.as_deref())?;
    let run = service
        .sync_authorized(
            &ctx.user_id,
            &ctx.organization_id,
            SyncInput {
                organization_id,
                facility_code: body.facility_code,
                start_month: body.start_month,
                end_month: body.end_month,
                mode,
            },
        )
        .await
        .map_err(ApiError::from_domain)?;
    Ok((StatusCode::CREATED, Json(map_sync(run))))
}

async fn list_clients(
    State(service): State<Arc<AppService>>,
    Path(organization_id): Path<String>,
    Query(query): Query<ReadConsistencyQuery>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<Client>>> {
    let read_consistency = read_consistency_override(&query)?;
    let ctx = request_context(service.as_ref(), &headers, read_consistency.clone()).await?;
    ensure_request_context_organization(&organization_id, &ctx)?;
    let clients = service
        .list_clients_authorized(&ctx.user_id, &ctx.organization_id, read_consistency)
        .await
        .map_err(ApiError::from_domain)?;
    Ok(Json(clients))
}

async fn create_client(
    State(service): State<Arc<AppService>>,
    Path(organization_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<CreateClientRequest>,
) -> ApiResult<(StatusCode, Json<Client>)> {
    let ctx = request_context(service.as_ref(), &headers, Some(ReadConsistency::Strong)).await?;
    ensure_request_context_organization(&organization_id, &ctx)?;
    let client = service
        .create_client(
            &ctx.user_id,
            &ctx.organization_id,
            CreateClientInput {
                id: body.id,
                slug: body.slug,
                name: body.name,
                description: body.description,
                client_type: body.client_type,
                app_type: body.app_type,
                first_party: body.first_party,
                consent_policy: body.consent_policy,
                active: body.active,
            },
        )
        .await
        .map_err(ApiError::from_domain)?;
    Ok((StatusCode::CREATED, Json(client)))
}

async fn get_client(
    State(service): State<Arc<AppService>>,
    Path((organization_id, client_id)): Path<(String, String)>,
    Query(query): Query<ReadConsistencyQuery>,
    headers: HeaderMap,
) -> ApiResult<Json<Client>> {
    let read_consistency = read_consistency_override(&query)?;
    let ctx = request_context(service.as_ref(), &headers, read_consistency.clone()).await?;
    ensure_request_context_organization(&organization_id, &ctx)?;
    let client = service
        .get_client_authorized(
            &ctx.user_id,
            &ctx.organization_id,
            &client_id,
            read_consistency,
        )
        .await
        .map_err(ApiError::from_domain)?;
    Ok(Json(client))
}

async fn update_client(
    State(service): State<Arc<AppService>>,
    Path((organization_id, client_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<UpdateClientRequest>,
) -> ApiResult<Json<Client>> {
    let ctx = request_context(service.as_ref(), &headers, Some(ReadConsistency::Strong)).await?;
    ensure_request_context_organization(&organization_id, &ctx)?;
    let client = service
        .update_client(
            &ctx.user_id,
            &ctx.organization_id,
            &client_id,
            UpdateClientInput {
                slug: body.slug,
                name: body.name,
                description: body.description,
                client_type: body.client_type,
                app_type: body.app_type,
                first_party: body.first_party,
                consent_policy: body.consent_policy,
                active: body.active,
            },
        )
        .await
        .map_err(ApiError::from_domain)?;
    Ok(Json(client))
}

async fn list_client_actions(
    State(service): State<Arc<AppService>>,
    Path((organization_id, client_id)): Path<(String, String)>,
    Query(query): Query<ReadConsistencyQuery>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<Action>>> {
    let read_consistency = read_consistency_override(&query)?;
    let ctx = request_context(service.as_ref(), &headers, read_consistency.clone()).await?;
    ensure_request_context_organization(&organization_id, &ctx)?;
    let actions = service
        .list_client_actions_authorized(
            &ctx.user_id,
            &ctx.organization_id,
            &client_id,
            read_consistency,
        )
        .await
        .map_err(ApiError::from_domain)?;
    Ok(Json(actions))
}

async fn replace_client_actions(
    State(service): State<Arc<AppService>>,
    Path((organization_id, client_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<ReplaceClientActionsRequest>,
) -> ApiResult<Json<Vec<Action>>> {
    let ctx = request_context(service.as_ref(), &headers, Some(ReadConsistency::Strong)).await?;
    ensure_request_context_organization(&organization_id, &ctx)?;
    let actions = service
        .replace_client_actions_authorized(
            &ctx.user_id,
            &ctx.organization_id,
            &client_id,
            &body.action_ids,
        )
        .await
        .map_err(ApiError::from_domain)?;
    Ok(Json(actions))
}

async fn create_cert_gateway_challenge(
    State(service): State<Arc<AppService>>,
    Path(organization_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<CreateCertGatewayChallengeRequest>,
) -> ApiResult<(StatusCode, Json<CertGatewayChallengeResponse>)> {
    let ctx = request_context(service.as_ref(), &headers, Some(ReadConsistency::Strong)).await?;
    ensure_active_organization_path_matches(&organization_id, &ctx.organization_id)?;
    let challenge = service
        .create_cert_gateway_challenge(CreateCertGatewayChallengeInput {
            organization_id,
            mode: body.mode,
            facility_code: body.facility_code,
            created_by_user_id: ctx.user_id,
            ttl_seconds: body.ttl_seconds.unwrap_or(120),
        })
        .await
        .map_err(ApiError::from_domain)?;
    Ok((
        StatusCode::CREATED,
        Json(CertGatewayChallengeResponse {
            challenge_id: challenge.id,
            mode: challenge.mode,
            nonce: challenge.nonce,
            payload: challenge.payload,
            expires_at: challenge.expires_at,
        }),
    ))
}

async fn execute_ephemeral_crawl(
    State(service): State<Arc<AppService>>,
    Path(organization_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<ExecuteEphemeralCrawlRequest>,
) -> ApiResult<(StatusCode, Json<SyncResponse>)> {
    let ctx = request_context(service.as_ref(), &headers, Some(ReadConsistency::Strong)).await?;
    ensure_active_organization_path_matches(&organization_id, &ctx.organization_id)?;
    let facility_code = service
        .authorize_gateway_sync_scope(
            &ctx.user_id,
            &organization_id,
            body.facility_code.as_deref(),
        )
        .await
        .map_err(ApiError::from_domain)?;
    let run = service
        .execute_ephemeral_crawl(ExecuteEphemeralCrawlInput {
            organization_id,
            challenge_id: body.challenge_id,
            challenge_payload: body.challenge_payload,
            challenge_signature_base64: body.challenge_signature_base64,
            bundle: map_proof_bundle(body.bundle)?,
            facility_code,
            start_month: body.start_month,
            end_month: body.end_month,
            mode: parse_sync_mode(body.mode.as_deref())?,
        })
        .await
        .map_err(ApiError::from_domain)?;
    Ok((StatusCode::ACCEPTED, Json(map_sync(run))))
}

async fn register_stored_certificate(
    State(service): State<Arc<AppService>>,
    Path(organization_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<RegisterStoredCertificateRequest>,
) -> ApiResult<(StatusCode, Json<CertificateResponse>)> {
    let ctx = request_context(service.as_ref(), &headers, Some(ReadConsistency::Strong)).await?;
    ensure_active_organization_path_matches(&organization_id, &ctx.organization_id)?;
    service
        .authorize_certificate_registration(&ctx.user_id, &organization_id)
        .await
        .map_err(ApiError::from_domain)?;
    let cert = service
        .register_certificate_from_proof(RegisterCertificateFromProofInput {
            organization_id: organization_id.clone(),
            challenge_id: body.challenge_id,
            challenge_payload: body.challenge_payload,
            challenge_signature_base64: body.challenge_signature_base64,
            bundle: map_proof_bundle(body.bundle)?,
        })
        .await
        .map_err(ApiError::from_domain)?;
    Ok((
        StatusCode::CREATED,
        Json(map_certificate(cert, &organization_id)),
    ))
}

async fn list_stored_certificates(
    State(service): State<Arc<AppService>>,
    Path(organization_id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<CertificateResponse>>> {
    let ctx = request_context(service.as_ref(), &headers, Some(ReadConsistency::Strong)).await?;
    ensure_active_organization_path_matches(&organization_id, &ctx.organization_id)?;
    let certificates = service
        .list_stored_certificates(&organization_id)
        .await
        .map_err(ApiError::from_domain)?;
    Ok(Json(
        certificates
            .into_iter()
            .map(|cert| map_certificate(cert, &organization_id))
            .collect(),
    ))
}

async fn issue_crawl_task_token(
    State(service): State<Arc<AppService>>,
    Path((organization_id, certificate_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<IssueCrawlTaskTokenRequest>,
) -> ApiResult<(StatusCode, Json<CrawlTaskTokenResponse>)> {
    let ctx = request_context(service.as_ref(), &headers, Some(ReadConsistency::Strong)).await?;
    ensure_active_organization_path_matches(&organization_id, &ctx.organization_id)?;
    let facility_code = service
        .authorize_gateway_sync_scope(
            &ctx.user_id,
            &organization_id,
            body.facility_code.as_deref(),
        )
        .await
        .map_err(ApiError::from_domain)?;
    let issued = service
        .issue_crawl_task_token(IssueCrawlTaskTokenInput {
            organization_id,
            certificate_id,
            facility_code,
            start_month: body.start_month,
            end_month: body.end_month,
            mode: parse_sync_mode(body.mode.as_deref())?,
            created_by_user_id: ctx.user_id,
            ttl_seconds: body.ttl_seconds.unwrap_or(60),
        })
        .await
        .map_err(ApiError::from_domain)?;
    Ok((
        StatusCode::CREATED,
        Json(CrawlTaskTokenResponse {
            crawl_task_token: issued.token,
            expires_at: issued.expires_at,
        }),
    ))
}

async fn start_crawl(
    State(service): State<Arc<AppService>>,
    Path(organization_id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<(StatusCode, Json<SyncResponse>)> {
    let token = bearer_token(&headers)?
        .ok_or_else(|| ApiError::bad_request(anyhow!("missing authorization header")))?;
    let run = service
        .consume_crawl_task_token_and_start_crawl(StartCrawlWithTaskTokenInput {
            organization_id,
            token,
        })
        .await
        .map_err(ApiError::from_domain)?;
    Ok((StatusCode::ACCEPTED, Json(map_sync(run))))
}

async fn list_plants(
    State(service): State<Arc<AppService>>,
    Query(query): Query<ReadConsistencyQuery>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<Plant>>> {
    let read_consistency = read_consistency_override(&query)?;
    let ctx = request_context(service.as_ref(), &headers, read_consistency.clone()).await?;
    let plants = service
        .list_visible_plants(&ctx.user_id, &ctx.organization_id, read_consistency)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(plants))
}

async fn get_plant(
    State(service): State<Arc<AppService>>,
    Path(facility_code): Path<String>,
    Query(query): Query<ReadConsistencyQuery>,
    headers: HeaderMap,
) -> ApiResult<Json<PlantResponse>> {
    let read_consistency = read_consistency_override(&query)?;
    let ctx = request_context(service.as_ref(), &headers, read_consistency.clone()).await?;
    let plant = service
        .get_plant_profile(
            &ctx.user_id,
            &ctx.organization_id,
            &facility_code,
            read_consistency,
        )
        .await
        .map_err(ApiError::from_domain)?;
    Ok(Json(map_plant(plant)))
}

async fn create_plant(
    State(service): State<Arc<AppService>>,
    Path(path_organization_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<CreatePlantRequest>,
) -> ApiResult<(StatusCode, Json<Plant>)> {
    let ctx = request_context(service.as_ref(), &headers, Some(ReadConsistency::Strong)).await?;
    ensure_active_organization_path_matches(&path_organization_id, &ctx.organization_id)?;

    let plant = service
        .create_plant(
            &ctx.user_id,
            &ctx.organization_id,
            CreatePlantInput {
                facility_code: body.facility_code,
                name: body.name,
                contract_no: body.contract_no,
                address: body.address,
                installed_kw: body.installed_kw,
                rec_start_date: body.rec_start_date,
                confirm_issued: body.confirm_issued,
                owner_organization_id: body.owner_organization_id,
            },
        )
        .await
        .map_err(ApiError::from_domain)?;
    Ok((StatusCode::CREATED, Json(plant)))
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

async fn list_plant_generation(
    State(service): State<Arc<AppService>>,
    Path(facility_code): Path<String>,
    Query(query): Query<ReadConsistencyQuery>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<GenerationRecord>>> {
    let read_consistency = read_consistency_override(&query)?;
    let ctx = request_context(service.as_ref(), &headers, read_consistency.clone()).await?;
    let items = service
        .list_generation_authorized(
            &ctx.user_id,
            &ctx.organization_id,
            &facility_code,
            read_consistency,
        )
        .await
        .map_err(ApiError::from_domain)?;
    Ok(Json(items))
}

async fn list_plant_rates(
    State(service): State<Arc<AppService>>,
    Path(facility_code): Path<String>,
    Query(query): Query<ReadConsistencyQuery>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<RateRecord>>> {
    let read_consistency = read_consistency_override(&query)?;
    let ctx = request_context(service.as_ref(), &headers, read_consistency.clone()).await?;
    let items = service
        .list_rates_authorized(
            &ctx.user_id,
            &ctx.organization_id,
            &facility_code,
            read_consistency,
        )
        .await
        .map_err(ApiError::from_domain)?;
    Ok(Json(items))
}

async fn list_plant_rec(
    State(service): State<Arc<AppService>>,
    Path(facility_code): Path<String>,
    Query(query): Query<ReadConsistencyQuery>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<RecRecord>>> {
    let read_consistency = read_consistency_override(&query)?;
    let ctx = request_context(service.as_ref(), &headers, read_consistency.clone()).await?;
    let items = service
        .list_rec_authorized(
            &ctx.user_id,
            &ctx.organization_id,
            &facility_code,
            read_consistency,
        )
        .await
        .map_err(ApiError::from_domain)?;
    Ok(Json(items))
}

async fn list_plant_documents(
    State(service): State<Arc<AppService>>,
    Path(facility_code): Path<String>,
    Query(query): Query<ReadConsistencyQuery>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<DocumentArtifact>>> {
    let read_consistency = read_consistency_override(&query)?;
    let ctx = request_context(service.as_ref(), &headers, read_consistency.clone()).await?;
    let items = service
        .list_documents_authorized(
            &ctx.user_id,
            &ctx.organization_id,
            &facility_code,
            read_consistency,
        )
        .await
        .map_err(ApiError::from_domain)?;
    Ok(Json(items))
}

async fn download_document(
    State(service): State<Arc<AppService>>,
    Path(document_id): Path<String>,
    Query(query): Query<ReadConsistencyQuery>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let read_consistency = read_consistency_override(&query)?;
    let ctx = request_context(service.as_ref(), &headers, read_consistency.clone()).await?;
    let (doc, bytes) = service
        .download_document_authorized(
            &ctx.user_id,
            &ctx.organization_id,
            &document_id,
            read_consistency,
        )
        .await
        .map_err(ApiError::from_domain)?;

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&doc.content_type).map_err(ApiError::internal)?,
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!(
            "attachment; filename*=UTF-8''{}",
            urlencoding::encode(&doc.artifact.file_name)
        ))
        .map_err(ApiError::internal)?,
    );
    Ok((headers, bytes).into_response())
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
    let client = reqwest::Client::new();
    let mut upstream = client.request(parts.method.clone(), url);
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

fn parse_sync_mode(value: Option<&str>) -> ApiResult<SyncMode> {
    match value
        .unwrap_or("delta")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "delta" => Ok(SyncMode::Delta),
        "full" => Ok(SyncMode::Full),
        other => Err(ApiError::bad_request(anyhow!("invalid mode: {other}"))),
    }
}

fn map_proof_bundle(body: ProofBundleRequest) -> ApiResult<CertificateBundle> {
    Ok(CertificateBundle {
        business_no: body.business_no,
        password: body.password,
        common_name: body.common_name,
        sign_cert_der: decode_base64_field("signCertDerBase64", &body.sign_cert_der_base64)?,
        sign_pri_key: decode_base64_field("signPriKeyBase64", &body.sign_pri_key_base64)?,
        km_cert_der: decode_base64_field("kmCertDerBase64", &body.km_cert_der_base64)?,
        km_pri_key: decode_base64_field("kmPriKeyBase64", &body.km_pri_key_base64)?,
    })
}

fn decode_base64_field(field_name: &'static str, value: &str) -> ApiResult<Vec<u8>> {
    base64::engine::general_purpose::STANDARD
        .decode(value.trim())
        .map_err(|_| ApiError::bad_request(anyhow!("invalid base64 for {field_name}")))
}

fn map_certificate(cert: StoredCertificate, organization_id: &str) -> CertificateResponse {
    CertificateResponse {
        id: cert.id,
        organization_id: organization_id.to_owned(),
        party_id: cert.party_id,
        business_no: cert.business_no,
        common_name: cert.common_name,
    }
}

fn map_sync(run: SyncRun) -> SyncResponse {
    SyncResponse {
        id: run.id,
        organization_id: run.organization_id,
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

    fn bad_request_if_known(err: anyhow::Error) -> Self {
        if let Some(status) = structured_status(&err) {
            return Self {
                status,
                message: err.to_string(),
            };
        }
        let message = err.to_string();
        let status = if message.contains("businessNo")
            || message.contains("organizationId")
            || message.contains("base64")
            || message.contains("certificate")
        {
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

    fn from_domain(err: anyhow::Error) -> Self {
        if let Some(status) = structured_status(&err) {
            return Self {
                status,
                message: err.to_string(),
            };
        }
        let message = err.to_string();
        let status = if message.contains("forbidden:") {
            StatusCode::FORBIDDEN
        } else if message.contains("already exists")
            || message.contains("UNIQUE constraint failed")
            || message.contains("unique constraint failed")
        {
            StatusCode::CONFLICT
        } else if message.contains("unavailable:") {
            StatusCode::SERVICE_UNAVAILABLE
        } else if message.contains("not found") {
            StatusCode::NOT_FOUND
        } else if message.contains("must not be empty")
            || message.contains("is not grantable to clients")
            || message.contains("expired")
            || message.contains("consumed")
            || message.contains("payload mismatch")
            || message.contains("mismatch")
            || message.contains("already used")
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
    use super::{
        ApiError, decode_base64_field, ensure_active_organization_path_matches, map_certificate,
    };
    use anyhow::anyhow;
    use axum::http::StatusCode;
    use infra::database::StoredCertificate;
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
    fn certificate_response_exposes_party_identity() {
        let response = map_certificate(
            StoredCertificate {
                id: "cert-1".to_owned(),
                party_id: "party-2768603291".to_owned(),
                business_no: Some("2768603291".to_owned()),
                common_name: "주식회사 커널로그에너지".to_owned(),
                encrypted_blob: "blob".to_owned(),
                wrapped_dek: Vec::new(),
                kms_key_id: "kms-key-v1".to_owned(),
                active: 1,
                last_used_at: None,
                created_at: "2026-04-12T00:00:00Z".to_owned(),
            },
            "org-2768603291",
        );

        assert_eq!(response.organization_id, "org-2768603291");
        assert_eq!(response.party_id, "party-2768603291");
        assert_eq!(response.business_no.as_deref(), Some("2768603291"));
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
    fn decode_base64_field_trims_surrounding_whitespace() {
        let decoded = decode_base64_field("signCertDerBase64", "  Zm9vYmFy\n")
            .expect("trimmed base64 should decode");
        assert_eq!(decoded, b"foobar");
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

mod support;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
};
use domain::{
    Action, ActionRiskLevel, Application, ApplicationAction, ApplicationAppType,
    ApplicationConsentPolicy, ApplicationProvisioningStatus, ApplicationType, Role, RoleBinding,
    RoleBindingState, RoleBindingSubjectType, ServiceProvider,
};
use hiqlite::Row;
use http_body_util::BodyExt;
use infra::{
    auth::AuthResolver,
    config::{
        AppConfig, AuthConfig, AuthProviderMode, HiqliteConfig, LocalNodeRole, PatchDataAuthConfig,
        RauthyAuthConfig, ReadConsistency, RuntimeMode, SecurityConfig, ServerConfig,
    },
    database::{
        Database, PatchDataAuthorityImport, PatchDataImportedApplicationAssignment,
        PatchDataImportedMembership, PatchDataImportedOrganization, PatchDataImportedPlantBinding,
        PatchDataImportedUser, patch_data_manifest_digest,
    },
    service::AppService,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::time::Duration;
use std::{collections::HashMap, future::pending, process::Command, sync::Arc};
use tempfile::TempDir;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::mpsc,
    task::JoinHandle,
};
use tower::ServiceExt;

const AUTHORIZATION: &str = "Bearer exact.original+credential/with=padding";
const REFRESH_PATH: &str = "/api/collections/users/auth-refresh";
const PLANT_READ_CONTRACT_SHA256: &str =
    "1da9293bbac25c94acfd154083ea057d0dd3aa321703aa0b6d15545bcad2bb2b";

struct CountRow {
    count: i64,
}

impl From<&mut Row<'_>> for CountRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            count: row.get("count"),
        }
    }
}

#[derive(Debug)]
struct CapturedRequest {
    method: String,
    path: String,
    headers: HashMap<String, String>,
}

enum Reply {
    Response {
        status: &'static str,
        headers: Vec<(&'static str, String)>,
        body: String,
    },
    Chunked {
        status: &'static str,
        headers: Vec<(&'static str, String)>,
        chunks: Vec<String>,
    },
    Stall,
}

struct LocalPatchData {
    base_url: String,
    requests: mpsc::UnboundedReceiver<CapturedRequest>,
    task: JoinHandle<()>,
}

impl Drop for LocalPatchData {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl LocalPatchData {
    async fn start(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind local Patch Data fixture");
        let base_url = format!("http://{}", listener.local_addr().expect("local address"));
        let (request_tx, requests) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            for reply in replies {
                let (mut stream, _) = listener.accept().await.expect("accept request");
                let request = read_request(&mut stream).await;
                request_tx.send(request).expect("test receives request");
                match reply {
                    Reply::Response {
                        status,
                        headers,
                        body,
                    } => write_response(&mut stream, status, &headers, &body).await,
                    Reply::Chunked {
                        status,
                        headers,
                        chunks,
                    } => write_chunked_response(&mut stream, status, &headers, &chunks).await,
                    Reply::Stall => pending::<()>().await,
                }
            }
        });
        Self {
            base_url,
            requests,
            task,
        }
    }

    async fn next_request(&mut self) -> CapturedRequest {
        self.requests
            .recv()
            .await
            .expect("captured Patch Data request")
    }
}

async fn write_chunked_response(
    stream: &mut TcpStream,
    status: &str,
    headers: &[(&str, String)],
    chunks: &[String],
) {
    let mut response =
        format!("HTTP/1.1 {status}\r\nConnection: close\r\nTransfer-Encoding: chunked\r\n");
    for (name, value) in headers {
        response.push_str(name);
        response.push_str(": ");
        response.push_str(value);
        response.push_str("\r\n");
    }
    response.push_str("\r\n");
    stream
        .write_all(response.as_bytes())
        .await
        .expect("headers");
    for chunk in chunks {
        stream
            .write_all(format!("{:x}\r\n", chunk.len()).as_bytes())
            .await
            .expect("chunk size");
        stream.write_all(chunk.as_bytes()).await.expect("chunk");
        stream.write_all(b"\r\n").await.expect("chunk end");
    }
    stream.write_all(b"0\r\n\r\n").await.expect("end chunks");
}

async fn read_request(stream: &mut TcpStream) -> CapturedRequest {
    let mut raw = Vec::new();
    let mut chunk = [0_u8; 1024];
    while !raw.windows(4).any(|window| window == b"\r\n\r\n") {
        let count = stream.read(&mut chunk).await.expect("read request");
        assert_ne!(count, 0, "request ended before headers");
        raw.extend_from_slice(&chunk[..count]);
    }
    let head = String::from_utf8(raw).expect("request headers are UTF-8");
    let mut lines = head.split("\r\n");
    let request_line = lines.next().expect("request line");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().expect("method").to_owned();
    let path = parts.next().expect("path").to_owned();
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_owned()))
        .collect();
    CapturedRequest {
        method,
        path,
        headers,
    }
}

async fn write_response(
    stream: &mut TcpStream,
    status: &str,
    headers: &[(&str, String)],
    body: &str,
) {
    let mut response = format!(
        "HTTP/1.1 {status}\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (name, value) in headers {
        response.push_str(name);
        response.push_str(": ");
        response.push_str(value);
        response.push_str("\r\n");
    }
    response.push_str("\r\n");
    response.push_str(body);
    stream
        .write_all(response.as_bytes())
        .await
        .expect("write response");
}

fn resolver(base_url: String) -> AuthResolver {
    AuthResolver::new_with_patch_data_request_timeout(
        &AuthConfig {
            provider_mode: AuthProviderMode::PatchData,
            rauthy: RauthyAuthConfig {
                base_url: "http://unused.test".to_owned(),
                introspect_path: "/unused".to_owned(),
                userinfo_path: "/unused".to_owned(),
                client_id: String::new(),
                client_secret: String::new(),
                client_read_api_key: String::new(),
                cache_ttl_seconds: 1,
            },
            patch_data: Some(PatchDataAuthConfig {
                base_url,
                exposure_application_id: "grant-exposure".to_owned(),
            }),
        },
        Duration::from_millis(100),
    )
}

fn valid_response(id: &str, organization_id: &str) -> String {
    json!({
        "token": "provider-issued-token-with-an-expiry-grant-must-not-interpret",
        "record": {
            "id": id,
            "email": "USER@PatchData.test",
            "verified": true,
            "name": "  Patch Data User  ",
            "organizations": [organization_id],
        }
    })
    .to_string()
}

fn valid_response_with_email(id: &str, email: &str, organization_id: &str) -> String {
    json!({
        "token": "provider-issued-token-with-an-expiry-grant-must-not-interpret",
        "record": {
            "id": id,
            "email": email,
            "verified": true,
            "name": "Patch Data User",
            "organizations": [organization_id],
        }
    })
    .to_string()
}

fn ok(body: String) -> Reply {
    Reply::Response {
        status: "200 OK",
        headers: vec![("Content-Type", "application/json".to_owned())],
        body,
    }
}

async fn resolve(
    resolver: &AuthResolver,
    authorization: Option<&str>,
    organization_id: Option<&str>,
) -> anyhow::Result<infra::auth::ExternalIdentity> {
    resolver
        .resolve_external_identity_for_organization(None, authorization, organization_id, None)
        .await
}

async fn resolve_bootstrap(
    resolver: &AuthResolver,
    authorization: Option<&str>,
    organization_id: Option<&str>,
) -> anyhow::Result<infra::auth::ExternalIdentity> {
    resolver
        .resolve_external_identity_for_bootstrap_organization(authorization, organization_id)
        .await
}

async fn patch_data_api(
    base_url: String,
) -> (
    axum::Router,
    Database,
    infra::runtime::HiqliteRuntime,
    TempDir,
) {
    let (runtime, db_dir) = support::test_runtime(ReadConsistency::Local).await;
    let cfg = AppConfig {
        server: ServerConfig {
            bind_addr: "127.0.0.1:0".to_owned(),
        },
        security: SecurityConfig {
            master_key: support::MASTER_KEY.to_owned(),
            app_auth_key: support::APP_AUTH_KEY.to_owned(),
            bootstrap_user_id: support::BOOTSTRAP_USER_ID.to_owned(),
            bootstrap_user_name: support::BOOTSTRAP_USER_NAME.to_owned(),
        },
        auth: AuthConfig {
            provider_mode: AuthProviderMode::PatchData,
            rauthy: RauthyAuthConfig {
                base_url: "http://unused.test".to_owned(),
                introspect_path: "/unused".to_owned(),
                userinfo_path: "/unused".to_owned(),
                client_id: String::new(),
                client_secret: String::new(),
                client_read_api_key: String::new(),
                cache_ttl_seconds: 1,
            },
            patch_data: Some(PatchDataAuthConfig {
                base_url,
                exposure_application_id: "grant-exposure".to_owned(),
            }),
        },
        hiqlite: HiqliteConfig {
            runtime_mode: RuntimeMode::Embedded,
            local_node_role: LocalNodeRole::Voter,
            default_read_consistency: ReadConsistency::Local,
            remote_nodes: vec![],
            remote_tls: false,
            remote_tls_no_verify: false,
            remote_api_secret: None,
            remote_with_proxy: false,
        },
    };
    let db = Database::new(
        runtime.clone(),
        cfg.security.bootstrap_user_id.clone(),
        cfg.security.bootstrap_user_name.clone(),
    );
    let service = Arc::new(AppService::new(cfg, db.clone()));
    service.initialize().await.expect("initialize");
    (api::build_router(service), db, runtime, db_dir)
}

async fn seed_patch_data_membership(
    db: &Database,
    runtime: &infra::runtime::HiqliteRuntime,
    organization_id: &str,
) {
    db.upsert_organization_identity(organization_id, None, organization_id)
        .await
        .expect("organization");
    let user = db
        .resolve_or_create_user_identity(
            "patch-data",
            "patch-user",
            Some("user@patchdata.test"),
            "Patch Data User",
        )
        .await
        .expect("user");
    support::seed_org_user_membership(runtime, "patch-membership", organization_id, &user.id).await;
}

async fn insert_exposure_application(
    db: &Database,
    organization_id: &str,
    active: bool,
    provisioning_status: ApplicationProvisioningStatus,
) {
    db.insert_application(&Application {
        id: "grant-exposure".to_owned(),
        organization_id: organization_id.to_owned(),
        slug: "grant-exposure".to_owned(),
        name: "Grant Exposure".to_owned(),
        description: None,
        application_type: ApplicationType::Confidential,
        app_type: ApplicationAppType::Server,
        first_party: false,
        consent_policy: ApplicationConsentPolicy::Explicit,
        active,
        provisioning_status,
        rauthy_client_id: Some("irrelevant-rauthy-client".to_owned()),
        created_by_user_id: support::BOOTSTRAP_USER_ID.to_owned(),
    })
    .await
    .expect("exposure application");
}

async fn grant_privileged_exposure_access(db: &Database, user_id: &str) {
    db.upsert_service_provider(&ServiceProvider {
        id: "exposure".to_owned(),
        name: "Exposure".to_owned(),
        description: None,
        active: true,
    })
    .await
    .expect("exposure provider");
    db.upsert_action(&Action {
        id: "exposure.read".to_owned(),
        provider_id: "exposure".to_owned(),
        code: "exposure.read".to_owned(),
        name: "Read exposure".to_owned(),
        description: None,
        risk_level: ActionRiskLevel::Low,
        visibility_granting: false,
        grantable_to_applications: true,
        consent_label: None,
        consent_description: None,
    })
    .await
    .expect("exposure action");
    db.upsert_role(&Role {
        id: "collision-exposure-reader".to_owned(),
        organization_id: Some("patch-org".to_owned()),
        code: "collision.exposure.reader".to_owned(),
        name: "Collision exposure reader".to_owned(),
        description: None,
    })
    .await
    .expect("exposure role");
    db.upsert_role_action("collision-exposure-reader", "exposure.read")
        .await
        .expect("role action");
    db.save_role_binding(&RoleBinding {
        id: "collision-exposure-binding".to_owned(),
        subject_type: RoleBindingSubjectType::User,
        subject_id: user_id.to_owned(),
        subject_organization_id: "patch-org".to_owned(),
        provider_id: "exposure".to_owned(),
        resource_type: "customer-profile".to_owned(),
        resource_id: "customer-123".to_owned(),
        role_id: "collision-exposure-reader".to_owned(),
        agreement_id: None,
        state: RoleBindingState::Active,
        created_by_user_id: user_id.to_owned(),
        created_at: support::FIXED_TEST_TIME.to_owned(),
        updated_at: support::FIXED_TEST_TIME.to_owned(),
    })
    .await
    .expect("exposure binding");
    db.upsert_application_action(&ApplicationAction {
        application_id: "grant-exposure".to_owned(),
        action_id: "exposure.read".to_owned(),
    })
    .await
    .expect("application action");
    db.set_application_trust("patch-org", "grant-exposure", true)
        .await
        .expect("trusted application");
}

fn application_authz_request(app_auth_key: Option<&str>) -> Request<Body> {
    let mut request = Request::builder()
        .method(Method::POST)
        .uri("/api/v1/application-authz/check")
        .header("content-type", "application/json")
        .header("authorization", AUTHORIZATION)
        .header("x-organization-id", "patch-org");
    if let Some(app_auth_key) = app_auth_key {
        request = request.header("x-grant-app-auth-key", app_auth_key);
    }
    request
        .body(Body::from(
            json!({
                "action": "organization.profile.read",
                "resource": {"provider": "grant", "type": "organization", "id": "patch-org"}
            })
            .to_string(),
        ))
        .expect("request")
}

fn exposure_read_request() -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri("/api/v1/exposure-authz/read")
        .header("content-type", "application/json")
        .header("authorization", AUTHORIZATION)
        .header("x-organization-id", "patch-org")
        .header("x-grant-app-auth-key", support::APP_AUTH_KEY)
        .body(Body::from(
            json!({
                "exposureId": "customer-profile",
                "exposureVersion": 1,
                "artifactDigest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "resourceId": "customer-123",
                "requestBinding": "collision-test"
            })
            .to_string(),
        ))
        .expect("exposure request")
}

fn materialize_request(
    authorization: Option<&str>,
    organization_id: Option<&str>,
) -> Request<Body> {
    let mut request = Request::builder()
        .method(Method::POST)
        .uri("/api/v1/me/materialize");
    if let Some(authorization) = authorization {
        request = request.header("authorization", authorization);
    }
    if let Some(organization_id) = organization_id {
        request = request.header("x-organization-id", organization_id);
    }
    request.body(Body::empty()).expect("materialize request")
}

#[tokio::test]
async fn patch_data_me_returns_grant_authoritative_active_memberships() {
    let mut patch_data =
        LocalPatchData::start(vec![ok(valid_response("patch-user", "patch-org"))]).await;
    let (app, db, runtime, _db_dir) = patch_data_api(patch_data.base_url.clone()).await;
    seed_patch_data_membership(&db, &runtime, "patch-org").await;
    db.upsert_organization_identity("other-org", None, "Other Org")
        .await
        .expect("other organization");
    let user = db
        .resolve_or_create_user_identity(
            "patch-data",
            "patch-user",
            Some("user@patchdata.test"),
            "Patch Data User",
        )
        .await
        .expect("existing Patch Data user");
    support::seed_org_user_membership(&runtime, "other-membership", "other-org", &user.id).await;

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/v1/me")
                .header("authorization", AUTHORIZATION)
                .header("x-organization-id", "patch-org")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let json: Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(json["provider"], "patch-data");
    assert_eq!(json["externalUserId"], "patch-user");
    assert_eq!(json["user"]["email"], "user@patchdata.test");
    assert_eq!(json["activeOrganizationId"], "patch-org");
    let organizations = json["organizations"]
        .as_array()
        .expect("Grant organizations");
    assert_eq!(organizations.len(), 2);
    assert!(organizations.iter().any(|item| item["id"] == "patch-org"));
    assert!(organizations.iter().any(|item| item["id"] == "other-org"));
    assert_eq!(patch_data.next_request().await.path, REFRESH_PATH);
}

#[tokio::test]
async fn patch_data_materialize_forwards_refresh_and_returns_private_minimal_view() {
    let mut patch_data = LocalPatchData::start(vec![ok(valid_response(
        "materialize-user",
        "materialize-org",
    ))])
    .await;
    let (app, _db, runtime, _db_dir) = patch_data_api(patch_data.base_url.clone()).await;

    let response = app
        .oneshot(materialize_request(
            Some(AUTHORIZATION),
            Some("materialize-org"),
        ))
        .await
        .expect("materialize response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("application/json")
    );
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let json: Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(json["organizationId"], "materialize-org");
    assert_eq!(json["membershipStatus"], "none");
    assert!(json["userId"].as_str().is_some_and(|id| !id.is_empty()));
    let keys = json.as_object().expect("materialize response object");
    assert_eq!(keys.len(), 3, "materialize response must remain narrow");
    for private in [
        "provider",
        "externalUserId",
        "email",
        "displayName",
        "token",
        "roles",
    ] {
        assert!(json.get(private).is_none(), "{private} must not be exposed");
    }
    let request = patch_data.next_request().await;
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, REFRESH_PATH);
    assert_eq!(
        request.headers.get("authorization").map(String::as_str),
        Some(AUTHORIZATION)
    );
    let identities: Vec<CountRow> = runtime.query_rows(
        ReadConsistency::Strong,
        "SELECT COUNT(*) AS count FROM user_identities WHERE provider='patch-data' AND external_user_id='materialize-user'",
        vec![],
    ).await.expect("materialized identity");
    assert_eq!(identities[0].count, 1);
    let memberships: Vec<CountRow> = runtime.query_rows(
        ReadConsistency::Strong,
        "SELECT COUNT(*) AS count FROM organization_memberships WHERE organization_id='materialize-org'",
        vec![],
    ).await.expect("no implicit membership");
    assert_eq!(memberships[0].count, 0);
}

#[tokio::test]
async fn patch_data_owner_bootstrap_requires_and_materializes_only_the_selected_organization() {
    let mut patch_data = LocalPatchData::start(vec![
        ok(valid_response("bootstrap-owner", "owned-org")),
        ok(valid_response("bootstrap-owner", "owned-org")),
    ])
    .await;
    let (app, db, _runtime, _db_dir) = patch_data_api(patch_data.base_url.clone()).await;
    let request = |organization_id: Option<&str>| {
        let mut request = Request::builder()
            .method(Method::POST)
            .uri("/api/v1/admin/bootstrap/external-owner")
            .header("authorization", AUTHORIZATION)
            .header("x-grant-master-key", support::MASTER_KEY);
        if let Some(organization_id) = organization_id {
            request = request.header("x-organization-id", organization_id);
        }
        request.body(Body::empty()).expect("bootstrap request")
    };

    let missing = app
        .clone()
        .oneshot(request(None))
        .await
        .expect("missing organization response");
    assert_eq!(missing.status(), StatusCode::BAD_REQUEST);
    let wrong = app
        .clone()
        .oneshot(request(Some("other-org")))
        .await
        .expect("wrong organization response");
    assert_eq!(wrong.status(), StatusCode::FORBIDDEN);
    let selected = app
        .oneshot(request(Some("owned-org")))
        .await
        .expect("selected organization response");
    assert_eq!(selected.status(), StatusCode::CREATED);
    let application = db
        .get_application_with_consistency("grant-exposure", Some(ReadConsistency::Strong))
        .await
        .expect("fixed Patch Data application");
    assert_eq!(application.organization_id, "owned-org");
    assert_eq!(application.name, "Patch Data Exposure");
    assert!(application.active);
    assert_eq!(application.rauthy_client_id, None);
    assert_eq!(patch_data.next_request().await.path, REFRESH_PATH);
    assert_eq!(patch_data.next_request().await.path, REFRESH_PATH);
}

#[tokio::test]
async fn patch_data_materialize_requires_local_scope_but_not_patch_data_membership() {
    let mut patch_data =
        LocalPatchData::start(vec![ok(valid_response("materialize-user", "other-org"))]).await;
    let (app, _db, _runtime, _db_dir) = patch_data_api(patch_data.base_url.clone()).await;

    let missing_token = app
        .clone()
        .oneshot(materialize_request(None, Some("materialize-org")))
        .await
        .expect("missing token response");
    assert_eq!(missing_token.status(), StatusCode::UNAUTHORIZED);
    let missing_organization = app
        .clone()
        .oneshot(materialize_request(Some(AUTHORIZATION), None))
        .await
        .expect("missing org response");
    assert_eq!(missing_organization.status(), StatusCode::FORBIDDEN);
    let locally_scoped = app
        .oneshot(materialize_request(
            Some(AUTHORIZATION),
            Some("materialize-org"),
        ))
        .await
        .expect("locally scoped response");
    assert_eq!(locally_scoped.status(), StatusCode::OK);
    assert_eq!(patch_data.next_request().await.path, REFRESH_PATH);
}

#[tokio::test]
async fn patch_data_organization_relation_changes_cannot_grant_or_revoke_local_access() {
    let mut patch_data = LocalPatchData::start(vec![
        ok(valid_response("patch-user", "provider-other-org")),
        ok(valid_response("patch-user", "patch-org")),
        ok(valid_response("patch-user", "provider-other-org")),
    ])
    .await;
    let (app, db, runtime, _db_dir) = patch_data_api(patch_data.base_url.clone()).await;
    seed_patch_data_membership(&db, &runtime, "patch-org").await;
    let request = || {
        Request::builder()
            .method(Method::GET)
            .uri("/api/v1/me")
            .header("authorization", AUTHORIZATION)
            .header("x-organization-id", "patch-org")
            .body(Body::empty())
            .expect("request")
    };

    let locally_allowed = app
        .clone()
        .oneshot(request())
        .await
        .expect("local membership response");
    assert_eq!(locally_allowed.status(), StatusCode::OK);

    runtime
        .execute(
            "UPDATE organization_memberships SET status='disabled' WHERE organization_id='patch-org'",
            vec![],
        )
        .await
        .expect("disable local membership");
    let provider_cannot_grant = app
        .clone()
        .oneshot(request())
        .await
        .expect("disabled response");
    assert_eq!(provider_cannot_grant.status(), StatusCode::FORBIDDEN);

    runtime
        .execute(
            "UPDATE organization_memberships SET status='active' WHERE organization_id='patch-org'",
            vec![],
        )
        .await
        .expect("restore local membership");
    let provider_cannot_revoke = app.oneshot(request()).await.expect("restored response");
    assert_eq!(provider_cannot_revoke.status(), StatusCode::OK);
    for _ in 0..3 {
        assert_eq!(patch_data.next_request().await.path, REFRESH_PATH);
    }
}

#[tokio::test]
async fn patch_data_materialize_respects_the_local_user_active_kill_switch() {
    let mut patch_data =
        LocalPatchData::start(vec![ok(valid_response("patch-user", "patch-org"))]).await;
    let (app, db, runtime, _db_dir) = patch_data_api(patch_data.base_url.clone()).await;
    seed_patch_data_membership(&db, &runtime, "patch-org").await;
    runtime
        .execute(
            "UPDATE users SET active=0 WHERE id=(SELECT user_id FROM user_identities WHERE provider='patch-data' AND external_user_id='patch-user')",
            vec![],
        )
        .await
        .expect("deactivate local user");
    let before: Vec<CountRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT COUNT(*) AS count FROM organization_memberships WHERE organization_id='patch-org'",
            vec![],
        )
        .await
        .expect("membership count before request");

    let response = app
        .oneshot(materialize_request(Some(AUTHORIZATION), Some("patch-org")))
        .await
        .expect("materialize response");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let after: Vec<CountRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT COUNT(*) AS count FROM organization_memberships WHERE organization_id='patch-org'",
            vec![],
        )
        .await
        .expect("membership count after request");
    assert_eq!(
        after[0].count, before[0].count,
        "request must not enroll an inactive user"
    );
    assert_eq!(patch_data.next_request().await.path, REFRESH_PATH);
}

#[tokio::test]
async fn patch_data_mode_hides_both_rauthy_proxy_routes() {
    let patch_data = LocalPatchData::start(vec![]).await;
    let (app, _db, _runtime, _db_dir) = patch_data_api(patch_data.base_url.clone()).await;

    for path in ["/auth/v1", "/auth/v1/oidc/authorize"] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(path)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
    }
}

#[tokio::test]
async fn patch_data_configured_exposure_application_must_exist_be_active_ready_and_match_org() {
    let cases = [
        (None, true, ApplicationProvisioningStatus::Ready),
        (
            Some("patch-org"),
            false,
            ApplicationProvisioningStatus::Ready,
        ),
        (
            Some("patch-org"),
            true,
            ApplicationProvisioningStatus::Unprovisioned,
        ),
        (
            Some("other-org"),
            true,
            ApplicationProvisioningStatus::Ready,
        ),
    ];

    for (application_org, active, provisioning_status) in cases {
        let mut patch_data =
            LocalPatchData::start(vec![ok(valid_response("patch-user", "patch-org"))]).await;
        let (app, db, runtime, _db_dir) = patch_data_api(patch_data.base_url.clone()).await;
        seed_patch_data_membership(&db, &runtime, "patch-org").await;
        if let Some(application_org) = application_org {
            if application_org != "patch-org" {
                db.upsert_organization_identity(application_org, None, application_org)
                    .await
                    .expect("other application organization");
            }
            insert_exposure_application(&db, application_org, active, provisioning_status).await;
        }

        let response = app
            .oneshot(application_authz_request(Some(support::APP_AUTH_KEY)))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(patch_data.next_request().await.path, REFRESH_PATH);
    }
}

#[tokio::test]
async fn patch_data_application_auth_key_uses_configured_exposure_application_not_token_client() {
    let mut patch_data = LocalPatchData::start(vec![
        ok(valid_response("patch-user", "patch-org")),
        ok(valid_response("patch-user", "patch-org")),
    ])
    .await;
    let (app, db, runtime, _db_dir) = patch_data_api(patch_data.base_url.clone()).await;
    seed_patch_data_membership(&db, &runtime, "patch-org").await;
    insert_exposure_application(&db, "patch-org", true, ApplicationProvisioningStatus::Ready).await;

    let missing_key = app
        .clone()
        .oneshot(application_authz_request(None))
        .await
        .expect("missing key response");
    assert_eq!(missing_key.status(), StatusCode::UNAUTHORIZED);

    let allowed = app
        .oneshot(application_authz_request(Some(support::APP_AUTH_KEY)))
        .await
        .expect("configured application response");
    assert_eq!(allowed.status(), StatusCode::OK);
    assert_eq!(patch_data.next_request().await.path, REFRESH_PATH);
}

fn authority_manifest(
    generation: &str,
    previous: Option<&str>,
    assign_application: bool,
) -> PatchDataAuthorityImport {
    let mut manifest = PatchDataAuthorityImport {
        manifest_version: "grant.patch-data-authority.v1".to_owned(),
        generation: generation.to_owned(),
        ledger_digest: if previous.is_some() { "b" } else { "a" }.repeat(64),
        manifest_digest: "0".repeat(64),
        expected_previous_generation: previous.map(ToOwned::to_owned),
        users: vec![PatchDataImportedUser {
            user_id: "imported-user".to_owned(),
            external_user_id: "imported-external".to_owned(),
            email: Some("user@patchdata.test".to_owned()),
            display_name: "Imported User".to_owned(),
            active: true,
        }],
        organizations: vec![PatchDataImportedOrganization {
            organization_id: "imported-org".to_owned(),
            name: "Imported Org".to_owned(),
            active: true,
        }],
        memberships: vec![PatchDataImportedMembership {
            membership_id: "imported-membership".to_owned(),
            organization_id: "imported-org".to_owned(),
            user_id: "imported-user".to_owned(),
            role: domain::MembershipRole::Owner,
            status: domain::MembershipStatus::Active,
        }],
        hierarchy_edges: vec![],
        plant_bindings: vec![PatchDataImportedPlantBinding {
            binding_id: "imported-plant-binding".to_owned(),
            subject_type: RoleBindingSubjectType::User,
            subject_id: "imported-user".to_owned(),
            subject_organization_id: "imported-org".to_owned(),
            plant_id: "plant-a".to_owned(),
            state: RoleBindingState::Active,
        }],
        application_assignments: assign_application
            .then(|| PatchDataImportedApplicationAssignment {
                application_id: "grant-exposure".to_owned(),
                organization_id: "imported-org".to_owned(),
            })
            .into_iter()
            .collect(),
    };
    manifest.manifest_digest = patch_data_manifest_digest(&manifest);
    manifest
}

fn import_request(manifest: &PatchDataAuthorityImport) -> Request<Body> {
    support::master_json_request(
        Method::POST,
        "/api/v1/admin/imports/patch-data-authority",
        serde_json::to_value(manifest).expect("manifest json"),
    )
}

fn imported_plant_request() -> Request<Body> {
    let fixture = plant_read_contract_fixture();
    Request::builder()
        .method(Method::POST)
        .uri(fixture["path"].as_str().expect("fixture path"))
        .header("content-type", "application/json")
        .header("authorization", AUTHORIZATION)
        .header("x-organization-id", "imported-org")
        .header("x-grant-app-auth-key", support::APP_AUTH_KEY)
        .body(Body::from(fixture["request"].to_string()))
        .expect("plant request")
}

fn plant_read_contract_fixture() -> Value {
    let bytes = include_bytes!("fixtures/plant_read_contract.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        PLANT_READ_CONTRACT_SHA256,
        "Grant and Exposure must deliberately update the shared wire fixture together"
    );
    serde_json::from_slice(bytes).expect("shared plant-read contract fixture")
}

#[tokio::test]
async fn public_import_assignment_enables_plant_read_and_omission_revokes_it() {
    let mut patch_data = LocalPatchData::start(vec![
        ok(valid_response("imported-external", "imported-org")),
        ok(valid_response("imported-external", "imported-org")),
        ok(valid_response("imported-external", "imported-org")),
    ])
    .await;
    let (app, _db, _runtime, _db_dir) = patch_data_api(patch_data.base_url.clone()).await;

    let first = authority_manifest("generation-a", None, false);
    let imported = app
        .clone()
        .oneshot(import_request(&first))
        .await
        .expect("first import");
    assert_eq!(imported.status(), StatusCode::OK);

    let application = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/admin/bootstrap/external-owner")
                .header("authorization", AUTHORIZATION)
                .header("x-organization-id", "imported-org")
                .header("x-grant-master-key", support::MASTER_KEY)
                .body(Body::empty())
                .expect("bootstrap request"),
        )
        .await
        .expect("bootstrap application");
    assert_eq!(application.status(), StatusCode::CREATED);

    let assigned = authority_manifest("generation-b", Some("generation-a"), true);
    let assigned_response = app
        .clone()
        .oneshot(import_request(&assigned))
        .await
        .expect("assigned import");
    assert_eq!(assigned_response.status(), StatusCode::OK);

    let allowed = app
        .clone()
        .oneshot(imported_plant_request())
        .await
        .expect("allowed plant decision");
    assert_eq!(allowed.status(), StatusCode::OK);
    let allowed = support::response_json(allowed).await;
    assert_eq!(allowed["allowed"], true, "decision: {allowed}");
    let contract = plant_read_contract_fixture();
    for key in ["operationId", "providerId", "actionId", "resourceType"] {
        assert_eq!(
            allowed[key], contract["expectedResponseSemantics"][key],
            "semantic mismatch for {key}"
        );
    }

    let omitted = authority_manifest("generation-c", Some("generation-b"), false);
    let omitted_response = app
        .clone()
        .oneshot(import_request(&omitted))
        .await
        .expect("omitted import");
    assert_eq!(omitted_response.status(), StatusCode::OK);

    let denied = app
        .clone()
        .oneshot(imported_plant_request())
        .await
        .expect("denied plant decision");
    assert_eq!(denied.status(), StatusCode::OK);
    let denied = support::response_json(denied).await;
    assert_eq!(denied["allowed"], false, "decision: {denied}");
    for key in ["operationId", "providerId", "actionId", "resourceType"] {
        assert_eq!(
            denied[key], contract["expectedResponseSemantics"][key],
            "semantic mismatch for {key}"
        );
    }

    for _ in 0..3 {
        assert_eq!(patch_data.next_request().await.path, REFRESH_PATH);
    }
}

#[tokio::test]
async fn patch_data_uses_fixed_refresh_path_and_forwards_authorization_verbatim() {
    let mut patch_data = LocalPatchData::start(vec![ok(valid_response("user-1", "org-1"))]).await;
    let resolver = resolver(patch_data.base_url.clone());

    let identity = resolve(&resolver, Some(AUTHORIZATION), Some("org-1"))
        .await
        .expect("Patch Data identity");

    assert_eq!(identity.external_user_id, "user-1");
    assert_eq!(identity.email.as_deref(), Some("user@patchdata.test"));
    assert_eq!(identity.display_name, "Patch Data User");
    assert_eq!(identity.organization_id, None);
    let request = patch_data.next_request().await;
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, REFRESH_PATH);
    assert_eq!(
        request.headers.get("authorization").map(String::as_str),
        Some(AUTHORIZATION),
    );
}

#[test]
fn patch_data_proxy_child() {
    let Ok(base_url) = std::env::var("GRANT_TEST_PATCH_DATA_PROXY_CHILD_BASE_URL") else {
        return;
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("child runtime");
    runtime.block_on(async move {
        let identity = resolve(&resolver(base_url), Some(AUTHORIZATION), Some("org-1"))
            .await
            .expect("Patch Data identity must bypass environment proxies");
        assert_eq!(identity.external_user_id, "proxy-direct-user");
    });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn patch_data_bearer_bypasses_http_proxy_and_all_proxy() {
    let mut intended =
        LocalPatchData::start(vec![ok(valid_response("proxy-direct-user", "org-1"))]).await;
    let mut trap = LocalPatchData::start(vec![Reply::Response {
        status: "401 Unauthorized",
        headers: vec![],
        body: String::new(),
    }])
    .await;

    let current_exe = std::env::current_exe().expect("current integration-test executable");
    let intended_base_url = intended.base_url.clone();
    let trap_base_url = trap.base_url.clone();
    let child = tokio::task::spawn_blocking(move || {
        Command::new(current_exe)
            .arg("--exact")
            .arg("patch_data_proxy_child")
            .arg("--nocapture")
            .arg("--test-threads=1")
            .env(
                "GRANT_TEST_PATCH_DATA_PROXY_CHILD_BASE_URL",
                intended_base_url,
            )
            .env("HTTP_PROXY", &trap_base_url)
            .env("http_proxy", &trap_base_url)
            .env("ALL_PROXY", &trap_base_url)
            .env("all_proxy", &trap_base_url)
            .env_remove("NO_PROXY")
            .env_remove("no_proxy")
            .output()
            .expect("run isolated proxy child")
    })
    .await
    .expect("proxy child join");

    assert!(
        child.status.success(),
        "proxy child failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&child.stdout),
        String::from_utf8_lossy(&child.stderr)
    );
    let request = intended.next_request().await;
    assert_eq!(request.path, REFRESH_PATH);
    assert_eq!(
        request.headers.get("authorization").map(String::as_str),
        Some(AUTHORIZATION)
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(100), trap.requests.recv())
            .await
            .is_err(),
        "Patch Data bearer must never reach an environment proxy"
    );
}

#[tokio::test]
async fn patch_data_rejects_unauthorized_and_redirect_responses() {
    let mut patch_data = LocalPatchData::start(vec![
        Reply::Response {
            status: "401 Unauthorized",
            headers: vec![],
            body: String::new(),
        },
        Reply::Response {
            status: "302 Found",
            headers: vec![("Location", "/not-the-refresh-route".to_owned())],
            body: String::new(),
        },
    ])
    .await;
    let resolver = resolver(patch_data.base_url.clone());

    let rejected = resolve(&resolver, Some(AUTHORIZATION), Some("org-1"))
        .await
        .expect_err("401 must be unauthorized");
    assert_eq!(
        rejected.to_string(),
        "unauthorized: Patch Data token rejected"
    );
    assert_eq!(patch_data.next_request().await.path, REFRESH_PATH);

    let redirected = resolve(&resolver, Some(AUTHORIZATION), Some("org-1"))
        .await
        .expect_err("redirects must not be followed or accepted");
    assert!(!redirected.to_string().is_empty());
    assert_eq!(patch_data.next_request().await.path, REFRESH_PATH);
}

#[tokio::test]
async fn patch_data_times_out_when_auth_refresh_never_responds() {
    let mut patch_data = LocalPatchData::start(vec![Reply::Stall]).await;
    let resolver = resolver(patch_data.base_url.clone());
    let resolution =
        tokio::spawn(async move { resolve(&resolver, Some(AUTHORIZATION), Some("org-1")).await });
    assert_eq!(patch_data.next_request().await.path, REFRESH_PATH);
    let error = resolution
        .await
        .expect("resolver task")
        .expect_err("stalled refresh must time out");
    assert_eq!(
        error.to_string(),
        "unavailable: Patch Data auth refresh request failed"
    );
}

#[tokio::test]
async fn patch_data_rejects_responses_larger_than_64_kib() {
    let mut patch_data = LocalPatchData::start(vec![Reply::Response {
        status: "200 OK",
        headers: vec![("Content-Type", "application/json".to_owned())],
        body: "x".repeat(64 * 1024 + 1),
    }])
    .await;
    let resolver = resolver(patch_data.base_url.clone());

    let error = resolve(&resolver, Some(AUTHORIZATION), Some("org-1"))
        .await
        .expect_err("oversized refresh body must be rejected");
    assert_eq!(
        error.to_string(),
        "unavailable: Patch Data auth refresh response is too large"
    );
    assert_eq!(patch_data.next_request().await.path, REFRESH_PATH);
}

#[tokio::test]
async fn patch_data_streaming_limit_rejects_chunked_response_without_content_length() {
    let mut patch_data = LocalPatchData::start(vec![Reply::Chunked {
        status: "200 OK",
        headers: vec![("Content-Type", "application/json".to_owned())],
        chunks: vec!["x".repeat(64 * 1024), "y".to_owned()],
    }])
    .await;
    let resolver = resolver(patch_data.base_url.clone());

    let error = resolve(&resolver, Some(AUTHORIZATION), Some("org-1"))
        .await
        .expect_err("chunked response over the hard bound must be rejected");
    assert_eq!(
        error.to_string(),
        "unavailable: Patch Data auth refresh response is too large"
    );
    assert_eq!(patch_data.next_request().await.path, REFRESH_PATH);
}

#[tokio::test]
async fn patch_data_streaming_limit_rejects_dishonest_content_length() {
    let mut patch_data = LocalPatchData::start(vec![Reply::Chunked {
        status: "200 OK",
        headers: vec![
            ("Content-Type", "application/json".to_owned()),
            ("Content-Length", "1".to_owned()),
        ],
        chunks: vec!["x".repeat(64 * 1024), "y".to_owned()],
    }])
    .await;
    let resolver = resolver(patch_data.base_url.clone());

    let error = resolve(&resolver, Some(AUTHORIZATION), Some("org-1"))
        .await
        .expect_err("dishonest content length must not bypass the hard bound");
    assert_eq!(
        error.to_string(),
        "unavailable: Patch Data auth refresh response is too large"
    );
    assert_eq!(patch_data.next_request().await.path, REFRESH_PATH);
}

#[tokio::test]
async fn patch_data_non_auth_upstream_statuses_are_sanitized_as_unavailable() {
    let mut patch_data = LocalPatchData::start(vec![
        Reply::Response {
            status: "302 Found",
            headers: vec![],
            body: String::new(),
        },
        Reply::Response {
            status: "500 Internal Server Error",
            headers: vec![],
            body: "secret upstream body".to_owned(),
        },
    ])
    .await;
    let resolver = resolver(patch_data.base_url.clone());
    for _ in 0..2 {
        let error = resolve(&resolver, Some(AUTHORIZATION), Some("org-1"))
            .await
            .expect_err("non-auth upstream status must fail closed");
        assert_eq!(
            error.to_string(),
            "unavailable: Patch Data auth refresh is unavailable"
        );
        assert_eq!(patch_data.next_request().await.path, REFRESH_PATH);
    }
}

#[tokio::test]
async fn patch_data_rejects_invalid_response_json_shapes_and_types() {
    let invalid_payloads = vec![
        "not json".to_owned(),
        "[]".to_owned(),
        json!({ "token": "token", "record": [] }).to_string(),
        json!({ "token": ["token"], "record": {}}).to_string(),
        json!({
            "token": "token",
            "record": {"id": 1, "email": "user@example.test", "verified": true, "organizationId": "org-1"}
        })
        .to_string(),
        json!({
            "token": "token",
            "record": {"id": "user-1", "email": "user@example.test", "verified": "true", "organizationId": "org-1"}
        })
        .to_string(),
    ];
    let mut patch_data =
        LocalPatchData::start(invalid_payloads.into_iter().map(ok).collect()).await;
    let resolver = resolver(patch_data.base_url.clone());

    for _ in 0..6 {
        let error = resolve(&resolver, Some(AUTHORIZATION), Some("org-1"))
            .await
            .expect_err("invalid Patch Data response must be rejected");
        assert_eq!(
            error.to_string(),
            "unavailable: Patch Data auth refresh response was invalid"
        );
        assert_eq!(patch_data.next_request().await.path, REFRESH_PATH);
    }
}

#[tokio::test]
async fn patch_data_rejects_missing_empty_or_invalid_identity_fields() {
    let invalid_payloads = vec![
        json!({ "token": "token", "record": {"email": "user@example.test", "verified": true, "organizationId": "org-1"}}),
        json!({ "token": "token", "record": {"id": " ", "email": "user@example.test", "verified": true, "organizationId": "org-1"}}),
        json!({ "token": "token", "record": {"id": "user-1", "verified": true, "organizationId": "org-1"}}),
        json!({ "token": "token", "record": {"id": "user-1", "email": " ", "verified": true, "organizationId": "org-1"}}),
        json!({ "token": "token", "record": {"id": "user-1", "email": "not-an-email", "verified": true, "organizationId": "org-1"}}),
    ]
    .into_iter()
    .map(|payload| payload.to_string())
    .collect::<Vec<_>>();
    let mut patch_data =
        LocalPatchData::start(invalid_payloads.into_iter().map(ok).collect()).await;
    let resolver = resolver(patch_data.base_url.clone());

    for _ in 0..5 {
        assert!(
            resolve(&resolver, Some(AUTHORIZATION), Some("org-1"))
                .await
                .is_err()
        );
        assert_eq!(patch_data.next_request().await.path, REFRESH_PATH);
    }
}

#[tokio::test]
async fn patch_data_bootstrap_requires_verified_email_and_explicit_matching_organization() {
    let missing_organization = json!({
        "token": "token",
        "record": {"id": "user-1", "email": "user@example.test", "verified": true}
    })
    .to_string();
    let mut patch_data = LocalPatchData::start(vec![
        ok(json!({
            "token": "token",
            "record": {"id": "user-1", "email": "user@example.test", "verified": false, "organizations": ["org-1"]}
        }).to_string()),
        ok(json!({
            "token": "token",
            "record": {"id": "user-1", "email": "user@example.test", "organizations": ["org-1"]}
        }).to_string()),
        ok(missing_organization),
        ok(valid_response("user-1", "other-org")),
        ok(json!({
            "token": "token",
            "record": {"id": "user-1", "email": "user@example.test", "verified": true, "organizations": ["org-2", "org-1", "org-1"]}
        }).to_string()),
        ok(valid_response("user-1", "org-1")),
    ])
    .await;
    let resolver = resolver(patch_data.base_url.clone());

    for _ in 0..3 {
        assert!(
            resolve_bootstrap(&resolver, Some(AUTHORIZATION), Some("org-1"))
                .await
                .is_err()
        );
        assert_eq!(patch_data.next_request().await.path, REFRESH_PATH);
    }
    let wrong_organization = resolve_bootstrap(&resolver, Some(AUTHORIZATION), Some("org-1"))
        .await
        .expect_err("wrong organization must be forbidden");
    assert_eq!(
        wrong_organization.to_string(),
        "forbidden: Patch Data user is not a member of the requested organization"
    );
    assert_eq!(patch_data.next_request().await.path, REFRESH_PATH);
    let selected = resolve_bootstrap(&resolver, Some(AUTHORIZATION), Some("org-1"))
        .await
        .expect("deduplicated multi-organization membership");
    assert_eq!(selected.organization_id.as_deref(), Some("org-1"));
    assert_eq!(patch_data.next_request().await.path, REFRESH_PATH);
    let missing_selection = resolve_bootstrap(&resolver, Some(AUTHORIZATION), None)
        .await
        .expect_err("Patch Data must never select the first organization implicitly");
    assert_eq!(
        missing_selection.to_string(),
        "forbidden: Patch Data organization selection is required"
    );
    assert_eq!(patch_data.next_request().await.path, REFRESH_PATH);
}

#[tokio::test]
async fn patch_data_rejects_unbounded_or_malformed_organization_relations() {
    let record = |organizations: Value| {
        ok(json!({
            "token": "token",
            "record": {
                "id": "user-1",
                "email": "user@example.test",
                "verified": true,
                "organizations": organizations
            }
        })
        .to_string())
    };
    let mut patch_data = LocalPatchData::start(vec![
        record(json!(
            (0..101)
                .map(|index| format!("org-{index}"))
                .collect::<Vec<_>>()
        )),
        record(json!(["x".repeat(256)])),
        record(json!([" org-1"])),
        record(json!(["org-1\u{0}"])),
        ok(r#"{"token":"token","record":{"id":"user-1","email":"user@example.test","verified":true,"organizations":[],"organizations":["org-1"]}}"#.to_owned()),
    ])
    .await;
    let resolver = resolver(patch_data.base_url.clone());
    for _ in 0..5 {
        let error = resolve_bootstrap(&resolver, Some(AUTHORIZATION), Some("org-1"))
            .await
            .expect_err("malformed organizations relation must fail closed");
        assert_eq!(
            error.to_string(),
            "unavailable: Patch Data auth refresh response was invalid"
        );
        assert_eq!(patch_data.next_request().await.path, REFRESH_PATH);
    }
}

#[tokio::test]
async fn patch_data_authentication_accepts_missing_or_empty_organization_relations() {
    let mut patch_data = LocalPatchData::start(vec![
        ok(json!({
            "token": "token",
            "record": {"id": "user-1", "email": "user@example.test", "verified": true}
        })
        .to_string()),
        ok(json!({
            "token": "token",
            "record": {"id": "user-1", "email": "user@example.test", "verified": true, "organizations": []}
        })
        .to_string()),
    ])
    .await;
    let resolver = resolver(patch_data.base_url.clone());
    for _ in 0..2 {
        let identity = resolve(&resolver, Some(AUTHORIZATION), Some("local-org"))
            .await
            .expect("authn must not depend on the Patch Data relation");
        assert_eq!(identity.external_user_id, "user-1");
        assert_eq!(identity.organization_id, None);
        assert_eq!(patch_data.next_request().await.path, REFRESH_PATH);
    }
}

#[tokio::test]
async fn patch_data_authentication_ignores_malformed_or_unbounded_organization_relations() {
    let record = |organizations: Value| {
        ok(json!({
            "token": "token",
            "record": {
                "id": "user-1",
                "email": "user@example.test",
                "verified": true,
                "organizations": organizations
            }
        })
        .to_string())
    };
    let mut patch_data = LocalPatchData::start(vec![
        record(json!(
            (0..101)
                .map(|index| format!("org-{index}"))
                .collect::<Vec<_>>()
        )),
        record(json!([" org-1"])),
        record(json!({"unexpected": "shape"})),
        ok(r#"{"token":"token","record":{"id":"user-1","email":"user@example.test","verified":true,"organizations":[],"organizations":["org-1"]}}"#.to_owned()),
    ])
    .await;
    let resolver = resolver(patch_data.base_url.clone());
    for _ in 0..4 {
        let identity = resolve(&resolver, Some(AUTHORIZATION), Some("local-org"))
            .await
            .expect("relation-only changes cannot revoke Grant-local access");
        assert_eq!(identity.external_user_id, "user-1");
        assert_eq!(identity.organization_id, None);
        assert_eq!(patch_data.next_request().await.path, REFRESH_PATH);
    }
}

#[tokio::test]
async fn patch_data_requires_a_nonempty_provider_token_and_revalidates_every_request() {
    let mut patch_data = LocalPatchData::start(vec![
        ok(json!({
            "token": "",
            "record": {"id": "user-1", "email": "user@example.test", "verified": true, "organizations": ["org-1"]}
        }).to_string()),
        ok(valid_response("user-1", "org-1")),
        ok(valid_response("user-2", "org-1")),
    ])
    .await;
    let resolver = resolver(patch_data.base_url.clone());

    for missing in [None, Some(""), Some("  ")] {
        assert!(resolve(&resolver, missing, Some("org-1")).await.is_err());
    }
    assert!(
        resolve(&resolver, Some(AUTHORIZATION), Some("org-1"))
            .await
            .is_err(),
        "empty provider token must be rejected"
    );
    assert_eq!(patch_data.next_request().await.path, REFRESH_PATH);

    let first = resolve(&resolver, Some(AUTHORIZATION), Some("org-1"))
        .await
        .expect("first revalidation");
    let second = resolve(&resolver, Some(AUTHORIZATION), Some("org-1"))
        .await
        .expect("second revalidation");
    assert_eq!(first.external_user_id, "user-1");
    assert_eq!(second.external_user_id, "user-2");
    assert_eq!(patch_data.next_request().await.path, REFRESH_PATH);
    assert_eq!(patch_data.next_request().await.path, REFRESH_PATH);
}

#[tokio::test]
async fn patch_data_owner_can_reactivate_the_requested_local_inactive_organization() {
    let mut patch_data =
        LocalPatchData::start(vec![ok(valid_response("patch-user", "patch-org"))]).await;
    let (runtime, _dir) = support::test_runtime(ReadConsistency::Local).await;
    let cfg = AppConfig {
        server: ServerConfig {
            bind_addr: "127.0.0.1:0".to_owned(),
        },
        security: SecurityConfig {
            master_key: support::MASTER_KEY.to_owned(),
            app_auth_key: support::APP_AUTH_KEY.to_owned(),
            bootstrap_user_id: support::BOOTSTRAP_USER_ID.to_owned(),
            bootstrap_user_name: support::BOOTSTRAP_USER_NAME.to_owned(),
        },
        auth: AuthConfig {
            provider_mode: AuthProviderMode::PatchData,
            rauthy: RauthyAuthConfig {
                base_url: "http://unused.test".to_owned(),
                introspect_path: "/unused".to_owned(),
                userinfo_path: "/unused".to_owned(),
                client_id: String::new(),
                client_secret: String::new(),
                client_read_api_key: String::new(),
                cache_ttl_seconds: 1,
            },
            patch_data: Some(PatchDataAuthConfig {
                base_url: patch_data.base_url.clone(),
                exposure_application_id: "grant-exposure".to_owned(),
            }),
        },
        hiqlite: HiqliteConfig {
            runtime_mode: RuntimeMode::Embedded,
            local_node_role: LocalNodeRole::Voter,
            default_read_consistency: ReadConsistency::Local,
            remote_nodes: vec![],
            remote_tls: false,
            remote_tls_no_verify: false,
            remote_api_secret: None,
            remote_with_proxy: false,
        },
    };
    let db = Database::new(
        runtime.clone(),
        cfg.security.bootstrap_user_id.clone(),
        cfg.security.bootstrap_user_name.clone(),
    );
    let service = Arc::new(AppService::new(cfg, db.clone()));
    service.initialize().await.expect("initialize");
    db.upsert_organization_identity("patch-org", None, "Patch Org")
        .await
        .expect("organization");
    let user = db
        .resolve_or_create_user_identity(
            "patch-data",
            "patch-user",
            Some("user@patchdata.test"),
            "Patch Data User",
        )
        .await
        .expect("user");
    support::seed_org_user_membership(&runtime, "patch-membership", "patch-org", &user.id).await;
    service
        .save_role_binding(&RoleBinding {
            id: "patch-owner-binding".to_owned(),
            subject_type: RoleBindingSubjectType::User,
            subject_id: user.id.clone(),
            subject_organization_id: "patch-org".to_owned(),
            provider_id: "grant".to_owned(),
            resource_type: "organization".to_owned(),
            resource_id: "patch-org".to_owned(),
            role_id: "role-system-bootstrap-owner".to_owned(),
            agreement_id: None,
            state: RoleBindingState::Active,
            created_by_user_id: user.id.clone(),
            created_at: support::FIXED_TEST_TIME.to_owned(),
            updated_at: support::FIXED_TEST_TIME.to_owned(),
        })
        .await
        .expect("owner binding");
    db.deactivate_organization("patch-org")
        .await
        .expect("deactivate");

    let principal = service
        .authenticate_request_for_inactive_organization(
            Some(AUTHORIZATION),
            "patch-org",
            Some(ReadConsistency::Strong),
        )
        .await
        .expect("inactive organization authentication");
    let organization = service
        .reactivate_organization(&principal.user_id, &principal.organization_id, "patch-org")
        .await
        .expect("reactivate");
    assert_eq!(organization.id, "patch-org");
    assert_eq!(patch_data.next_request().await.path, REFRESH_PATH);
}

#[tokio::test]
async fn patch_data_inactive_org_recovery_never_inherits_email_or_bare_id_membership() {
    for (case, external_id, email) in [
        ("same-email", "new-patch-subject", "collision@example.test"),
        ("same-bare-id", "patch-user", "different@example.test"),
    ] {
        let mut patch_data = LocalPatchData::start(vec![ok(valid_response_with_email(
            external_id,
            email,
            "patch-org",
        ))])
        .await;
        let (app, db, runtime, _db_dir) = patch_data_api(patch_data.base_url.clone()).await;
        db.upsert_organization_identity("patch-org", None, "Patch Org")
            .await
            .expect("organization");
        let local_user_id = if case == "same-email" {
            db.resolve_or_create_user_identity(
                "rauthy",
                "legacy-subject",
                Some(email),
                "Legacy Owner",
            )
            .await
            .expect("legacy user")
            .id
        } else {
            runtime
                .execute(
                    "INSERT INTO users (id,email,display_name,active,created_at) VALUES ('patch-user',$1,'Bare Id Owner',1,$2)",
                    vec![email.into(), support::FIXED_TEST_TIME.into()],
                )
                .await
                .expect("bare-id local user");
            "patch-user".to_owned()
        };
        support::seed_org_user_membership(
            &runtime,
            "collision-membership",
            "patch-org",
            &local_user_id,
        )
        .await;
        db.save_role_binding(&RoleBinding {
            id: "collision-owner-binding".to_owned(),
            subject_type: RoleBindingSubjectType::User,
            subject_id: local_user_id.clone(),
            subject_organization_id: "patch-org".to_owned(),
            provider_id: "grant".to_owned(),
            resource_type: "organization".to_owned(),
            resource_id: "patch-org".to_owned(),
            role_id: "role-system-bootstrap-owner".to_owned(),
            agreement_id: None,
            state: RoleBindingState::Active,
            created_by_user_id: local_user_id,
            created_at: support::FIXED_TEST_TIME.to_owned(),
            updated_at: support::FIXED_TEST_TIME.to_owned(),
        })
        .await
        .expect("privileged legacy binding");
        db.deactivate_organization("patch-org")
            .await
            .expect("deactivate organization");

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/orgs/patch-org/reactivate")
                    .header("authorization", AUTHORIZATION)
                    .header("x-organization-id", "patch-org")
                    .body(Body::empty())
                    .expect("reactivate request"),
            )
            .await
            .expect("reactivate response");
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE, "{case}");
        let identities: Vec<CountRow> = runtime
            .query_rows(
                ReadConsistency::Strong,
                "SELECT COUNT(*) AS count FROM user_identities WHERE provider='patch-data' AND external_user_id=$1",
                vec![external_id.into()],
            )
            .await
            .expect("Patch Data identity count");
        assert_eq!(identities[0].count, 0, "{case} must not write an identity");
        let inactive: Vec<CountRow> = runtime
            .query_rows(ReadConsistency::Strong, "SELECT COUNT(*) AS count FROM organizations WHERE id='patch-org' AND inactivated_at IS NOT NULL", vec![])
            .await
            .expect("inactive organization");
        assert_eq!(inactive[0].count, 1, "{case} must not inherit owner access");
        assert_eq!(patch_data.next_request().await.path, REFRESH_PATH);
    }
}

#[tokio::test]
async fn patch_data_exposure_authz_never_inherits_email_or_bare_id_role_or_application_access() {
    for (case, external_id, email) in [
        ("same-email", "new-patch-subject", "collision@example.test"),
        ("same-bare-id", "patch-user", "different@example.test"),
    ] {
        let mut patch_data = LocalPatchData::start(vec![ok(valid_response_with_email(
            external_id,
            email,
            "patch-org",
        ))])
        .await;
        let (app, db, runtime, _db_dir) = patch_data_api(patch_data.base_url.clone()).await;
        db.upsert_organization_identity("patch-org", None, "Patch Org")
            .await
            .expect("organization");
        let local_user_id = if case == "same-email" {
            db.resolve_or_create_user_identity(
                "rauthy",
                "legacy-subject",
                Some(email),
                "Legacy Reader",
            )
            .await
            .expect("legacy user")
            .id
        } else {
            runtime
                .execute(
                    "INSERT INTO users (id,email,display_name,active,created_at) VALUES ('patch-user',$1,'Bare Id Reader',1,$2)",
                    vec![email.into(), support::FIXED_TEST_TIME.into()],
                )
                .await
                .expect("bare-id local user");
            "patch-user".to_owned()
        };
        support::seed_org_user_membership(
            &runtime,
            "collision-exposure-membership",
            "patch-org",
            &local_user_id,
        )
        .await;
        insert_exposure_application(&db, "patch-org", true, ApplicationProvisioningStatus::Ready)
            .await;
        grant_privileged_exposure_access(&db, &local_user_id).await;

        let response = app
            .oneshot(exposure_read_request())
            .await
            .expect("exposure response");
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE, "{case}");
        let identities: Vec<CountRow> = runtime
            .query_rows(
                ReadConsistency::Strong,
                "SELECT COUNT(*) AS count FROM user_identities WHERE provider='patch-data' AND external_user_id=$1",
                vec![external_id.into()],
            )
            .await
            .expect("Patch Data identity count");
        assert_eq!(
            identities[0].count, 0,
            "{case} must not materialize a colliding identity"
        );
        assert_eq!(patch_data.next_request().await.path, REFRESH_PATH);
    }
}

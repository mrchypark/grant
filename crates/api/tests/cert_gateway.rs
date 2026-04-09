mod support;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use chrono::Utc;
use hiqlite::{Node, NodeConfig, s3::EncKeys};
use http_body_util::BodyExt;
use infra::{
    artifacts::ArtifactStore,
    bridge::{
        BridgeFacility, BridgeGenerationRecord, BridgeRateRecord, BridgeRecRecord, BridgeRun,
        BridgeSnapshot,
    },
    config::{
        AppConfig, ArtifactBackendConfig, ArtifactConfig, HiqliteConfig, LocalNodeRole,
        ReadConsistency, RuntimeMode, SecurityConfig, ServerConfig,
    },
    database::Database,
    runtime::HiqliteRuntime,
    service::AppService,
};
use serde_json::{Value, json};
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;

fn free_local_addr() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    drop(listener);
    addr.to_string()
}

async fn test_runtime() -> (HiqliteRuntime, TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let api_addr = free_local_addr();
    let raft_addr = free_local_addr();
    let mut cfg = NodeConfig::default();
    cfg.node_id = 1;
    cfg.nodes = vec![Node {
        id: 1,
        addr_raft: raft_addr.clone(),
        addr_api: api_addr.clone(),
    }];
    cfg.listen_addr_api = "127.0.0.1".into();
    cfg.listen_addr_raft = "127.0.0.1".into();
    cfg.data_dir = dir.path().to_string_lossy().into_owned().into();
    cfg.filename_db = "test.sqlite".into();
    cfg.secret_api = "0123456789abcdef0123456789abcdef".to_owned();
    cfg.secret_raft = "fedcba9876543210fedcba9876543210".to_owned();
    cfg.enc_keys = EncKeys::generate().expect("enc keys");
    cfg.health_check_delay_secs = 0;
    cfg.raft_config = NodeConfig::default_raft_config(128);

    let client = hiqlite::start_node(cfg).await.expect("start hiqlite");
    (
        HiqliteRuntime::embedded(client, ReadConsistency::Local),
        dir,
    )
}

async fn build_app() -> (axum::Router, TempDir, TempDir) {
    let trailbase_base_url = support::start_mock_trailbase().await;
    let (runtime, db_dir) = test_runtime().await;
    let artifacts_dir = tempfile::tempdir().expect("artifact tempdir");
    let cfg = AppConfig {
        server: ServerConfig {
            bind_addr: "127.0.0.1:0".to_owned(),
        },
        security: SecurityConfig {
            master_key: "dev-only-master-key".to_owned(),
            bootstrap_user_id: "bootstrap-user".to_owned(),
            bootstrap_user_name: "Bootstrap User".to_owned(),
        },
        auth: support::trailbase_auth_config(trailbase_base_url),
        kms: support::test_kms_config(),
        artifacts: ArtifactConfig {
            backend: ArtifactBackendConfig::FileSystem {
                root_dir: artifacts_dir.path().to_path_buf(),
            },
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
        runtime,
        cfg.security.bootstrap_user_id.clone(),
        cfg.security.bootstrap_user_name.clone(),
    );
    let artifacts = ArtifactStore::new(&cfg.artifacts)
        .await
        .expect("artifact store");
    let service = Arc::new(AppService::new_with_collector(
        cfg,
        db,
        artifacts,
        mock_collect_snapshot,
    ));
    service.initialize().await.expect("initialize");
    (api::build_router(service), db_dir, artifacts_dir)
}

async fn mock_collect_snapshot(opts: infra::bridge::BridgeOptions) -> anyhow::Result<BridgeRun> {
    let facility_code = opts
        .facility_code
        .clone()
        .unwrap_or_else(|| "FAC-MOCK".to_owned());
    Ok(BridgeRun {
        temp_dir: tempfile::tempdir().expect("collector tempdir"),
        snapshot: BridgeSnapshot {
            collected_at: Utc::now().fixed_offset(),
            facility: BridgeFacility {
                code: facility_code,
                name: "Mock Facility".to_owned(),
                business_no: opts.business_no,
                contract_no: "CONTRACT-001".to_owned(),
                address: "Seoul".to_owned(),
                installed_kw: 123.4,
                rec_start_date: "2026-01-01".to_owned(),
                confirm_issued: "Y".to_owned(),
            },
            generation: vec![BridgeGenerationRecord {
                month: "2026-03".to_owned(),
                generation_kwh: 1000.0,
                self_consumption_kwh: Some(100.0),
                supply_kwh: 900.0,
                application_deadline: Some("2026-04-15".to_owned()),
            }],
            rates: vec![BridgeRateRecord {
                month: "2026-03".to_owned(),
                self_rate: Some(110.0),
                national_average: Some(120.0),
                nearby_average: Some(118.0),
                nearby_highest: Some(130.0),
                published: true,
                source_control_flag: Some("mock".to_owned()),
            }],
            rec: vec![BridgeRecRecord {
                month: "2026-03".to_owned(),
                application_no: "APP-001".to_owned(),
                supply_kwh: 900,
                rec_quantity: 1,
                fee_krw: 1000,
                issued_date: "2026-04-01".to_owned(),
                expiry_date: "2027-04-01".to_owned(),
                virtual_account: None,
                bank_name: None,
                status: Some("issued".to_owned()),
            }],
            documents: Vec::new(),
        },
    })
}

fn auth_request(
    method: &str,
    path: &str,
    token: &str,
    organization_id: &str,
    body: Value,
) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", support::bearer_header(token))
        .header("x-organization-id", organization_id)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request")
}

fn task_token_request(path: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .expect("request")
}

fn proof_bundle() -> Value {
    json!({
        "businessNo": "2768603291",
        "password": "secret-password",
        "commonName": "Test Certificate",
        "signCertDerBase64": "AQID",
        "signPriKeyBase64": "BAUG",
        "kmCertDerBase64": "BwgJ",
        "kmPriKeyBase64": "CgsM"
    })
}

async fn json_body(response: axum::response::Response) -> Value {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("json body")
}

#[tokio::test]
async fn cert_gateway_routes_cover_stored_and_ephemeral_flows() {
    let (app, _db_dir, _artifacts_dir) = build_app().await;

    let challenge_response = app
        .clone()
        .oneshot(auth_request(
            "POST",
            "/api/v1/organizations/bootstrap-user/cert-gateway/challenges",
            "bootstrap-user",
            "bootstrap-user",
            json!({ "mode": "stored", "facilityCode": "FAC-001" }),
        ))
        .await
        .expect("challenge response");
    assert_eq!(challenge_response.status(), StatusCode::CREATED);
    let challenge_json = json_body(challenge_response).await;
    let challenge_id = challenge_json["challengeId"]
        .as_str()
        .expect("challenge id")
        .to_owned();
    let challenge_payload = challenge_json["payload"]
        .as_str()
        .expect("challenge payload")
        .to_owned();

    let register_response = app
        .clone()
        .oneshot(auth_request(
            "POST",
            "/api/v1/organizations/bootstrap-user/certificates",
            "bootstrap-user",
            "bootstrap-user",
            json!({
                "challengeId": challenge_id,
                "challengePayload": challenge_payload,
                "bundle": proof_bundle()
            }),
        ))
        .await
        .expect("register response");
    assert_eq!(register_response.status(), StatusCode::CREATED);
    let register_json = json_body(register_response).await;
    let certificate_id = register_json["id"]
        .as_str()
        .expect("certificate id")
        .to_owned();

    let list_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/organizations/bootstrap-user/certificates")
                .header("authorization", support::bearer_header("bootstrap-user"))
                .header("x-organization-id", "bootstrap-user")
                .body(Body::empty())
                .expect("list request"),
        )
        .await
        .expect("list response");
    assert_eq!(list_response.status(), StatusCode::OK);
    let list_json = json_body(list_response).await;
    assert_eq!(list_json.as_array().expect("list array").len(), 1);

    let issue_response = app
        .clone()
        .oneshot(auth_request(
            "POST",
            &format!(
                "/api/v1/organizations/bootstrap-user/certificates/{certificate_id}/crawl-tokens"
            ),
            "bootstrap-user",
            "bootstrap-user",
            json!({
                "facilityCode": "FAC-003",
                "startMonth": "2026-01",
                "endMonth": "2026-04",
                "mode": "full"
            }),
        ))
        .await
        .expect("issue response");
    assert_eq!(issue_response.status(), StatusCode::CREATED);
    let issue_json = json_body(issue_response).await;
    let crawl_task_token = issue_json["crawlTaskToken"]
        .as_str()
        .expect("crawl task token")
        .to_owned();

    let start_response = app
        .clone()
        .oneshot(task_token_request(
            "/api/v1/organizations/bootstrap-user/crawls",
            &crawl_task_token,
        ))
        .await
        .expect("start response");
    assert_eq!(start_response.status(), StatusCode::ACCEPTED);
    let start_json = json_body(start_response).await;
    assert_eq!(start_json["status"], "completed");
    assert_eq!(start_json["plantId"], "FAC-003");

    let reuse_response = app
        .clone()
        .oneshot(task_token_request(
            "/api/v1/organizations/bootstrap-user/crawls",
            &crawl_task_token,
        ))
        .await
        .expect("reuse response");
    assert_eq!(reuse_response.status(), StatusCode::BAD_REQUEST);

    let ephemeral_challenge = app
        .clone()
        .oneshot(auth_request(
            "POST",
            "/api/v1/organizations/bootstrap-user/cert-gateway/challenges",
            "bootstrap-user",
            "bootstrap-user",
            json!({ "mode": "ephemeral", "facilityCode": "FAC-002" }),
        ))
        .await
        .expect("ephemeral challenge");
    assert_eq!(ephemeral_challenge.status(), StatusCode::CREATED);
    let ephemeral_challenge_json = json_body(ephemeral_challenge).await;

    let ephemeral_response = app
        .oneshot(auth_request(
            "POST",
            "/api/v1/organizations/bootstrap-user/cert-gateway/ephemeral-executions",
            "bootstrap-user",
            "bootstrap-user",
            json!({
                "challengeId": ephemeral_challenge_json["challengeId"],
                "challengePayload": ephemeral_challenge_json["payload"],
                "bundle": proof_bundle(),
                "facilityCode": "FAC-002",
                "startMonth": "2026-01",
                "endMonth": "2026-03",
                "mode": "delta"
            }),
        ))
        .await
        .expect("ephemeral response");
    assert_eq!(ephemeral_response.status(), StatusCode::ACCEPTED);
    let ephemeral_json = json_body(ephemeral_response).await;
    assert_eq!(ephemeral_json["status"], "completed");
    assert_eq!(ephemeral_json["plantId"], "FAC-002");
}

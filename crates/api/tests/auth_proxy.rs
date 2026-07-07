mod support;

use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use hiqlite::{Node, NodeConfig, s3::EncKeys};
use http_body_util::BodyExt;
use infra::{
    config::{
        AppConfig, HiqliteConfig, LocalNodeRole, ReadConsistency, RuntimeMode, SecurityConfig,
        ServerConfig,
    },
    database::Database,
    runtime::HiqliteRuntime,
    service::AppService,
};
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
    let cfg = NodeConfig {
        node_id: 1,
        nodes: vec![Node {
            id: 1,
            addr_raft: raft_addr.clone(),
            addr_api: api_addr.clone(),
        }],
        listen_addr_api: "127.0.0.1".into(),
        listen_addr_raft: "127.0.0.1".into(),
        data_dir: dir.path().to_string_lossy().into_owned().into(),
        filename_db: "test.sqlite".into(),
        secret_api: "0123456789abcdef0123456789abcdef".to_owned(),
        secret_raft: "fedcba9876543210fedcba9876543210".to_owned(),
        enc_keys: EncKeys::generate().expect("enc keys"),
        health_check_delay_secs: 0,
        raft_config: NodeConfig::default_raft_config(128),
        ..Default::default()
    };

    let client = hiqlite::start_node(cfg).await.expect("start hiqlite");
    (
        HiqliteRuntime::embedded(client, ReadConsistency::Local),
        dir,
    )
}

async fn build_app() -> axum::Router {
    let trailbase_base_url = support::start_mock_trailbase().await;
    let (runtime, _db_dir) = test_runtime().await;
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
        hiqlite: HiqliteConfig {
            runtime_mode: RuntimeMode::Embedded,
            local_node_role: LocalNodeRole::Voter,
            default_read_consistency: ReadConsistency::Local,
            remote_nodes: Vec::new(),
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
    let service = Arc::new(AppService::new(cfg, db));
    service.initialize().await.expect("initialize service");
    api::build_router(service)
}

#[tokio::test]
async fn auth_login_proxy_returns_trailbase_ui() {
    let app = build_app().await;
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/auth/login")
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
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("TrailBase Login"));
    assert!(html.contains("/api/auth/v1/login"));
    assert!(html.contains("/_/auth/app.css"));
}

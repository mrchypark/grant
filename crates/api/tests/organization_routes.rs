use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use hiqlite::{Node, NodeConfig, s3::EncKeys};
use infra::{
    artifacts::ArtifactStore,
    config::{
        AppConfig, ArtifactBackendConfig, ArtifactConfig, HiqliteConfig, LocalNodeRole,
        ReadConsistency, RuntimeMode, SecurityConfig, ServerConfig,
    },
    database::Database,
    runtime::HiqliteRuntime,
    service::AppService,
};
use serde_json::json;
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

async fn test_service() -> (Arc<AppService>, Database, TempDir) {
    let (runtime, dir) = test_runtime().await;
    let artifacts_root = dir.path().join("artifacts");
    let app_cfg = AppConfig {
        server: ServerConfig {
            bind_addr: free_local_addr(),
        },
        security: SecurityConfig {
            master_key: "dev-only-master-key".to_owned(),
            bootstrap_user_id: "bootstrap-user".to_owned(),
            bootstrap_user_name: "Bootstrap User".to_owned(),
        },
        artifacts: ArtifactConfig {
            backend: ArtifactBackendConfig::FileSystem {
                root_dir: artifacts_root,
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
        app_cfg.security.bootstrap_user_id.clone(),
        app_cfg.security.bootstrap_user_name.clone(),
    );
    let db_for_tests = db.clone();
    let artifacts = ArtifactStore::new(&app_cfg.artifacts)
        .await
        .expect("artifact store");
    let service = Arc::new(AppService::new(app_cfg, db, artifacts));
    service.initialize().await.expect("initialize");
    (service, db_for_tests, dir)
}

#[tokio::test]
async fn organization_certificate_route_replaces_business_route() {
    let (service, _db, _dir) = test_service().await;
    let app = api::build_router(service);

    let new_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/organizations/org-1/certificate")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "businessNo": "276-86-03291",
                        "password": "secret",
                        "signCertDerBase64": "not-base64",
                        "signPriKeyBase64": "not-base64",
                        "kmCertDerBase64": "not-base64",
                        "kmPriKeyBase64": "not-base64"
                    })
                    .to_string(),
                ))
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(new_response.status(), StatusCode::BAD_REQUEST);

    let old_response = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/businesses/2768603291/certificate")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(old_response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn organization_sync_route_replaces_business_route() {
    let (service, _db, _dir) = test_service().await;
    let app = api::build_router(service);

    let new_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/organizations/org-1/sync")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "mode": "bogus"
                    })
                    .to_string(),
                ))
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(new_response.status(), StatusCode::BAD_REQUEST);

    let old_response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/businesses/2768603291/sync")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(old_response.status(), StatusCode::NOT_FOUND);
}

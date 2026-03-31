use infra::config::{AppConfig, ArtifactBackendConfig};

#[test]
fn loads_default_config() {
    let cfg = AppConfig::from_env().expect("config");

    assert_eq!(cfg.server.bind_addr, "0.0.0.0:8080");
    assert_eq!(cfg.security.bootstrap_user_id, "conalog-account");
    match cfg.artifacts.backend {
        ArtifactBackendConfig::FileSystem { .. } => {}
        other => panic!("expected filesystem artifacts backend, got {other:?}"),
    }
}

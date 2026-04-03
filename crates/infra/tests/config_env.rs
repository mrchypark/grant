use infra::config::{
    AppConfig, ArtifactBackendConfig, LocalNodeRole, ReadConsistency, RuntimeMode,
};
use std::sync::{Mutex, OnceLock};

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn clear_certd_env() {
    for key in [
        "CERTD_RUNTIME_MODE",
        "CERTD_READ_CONSISTENCY",
        "CERTD_LOCAL_NODE_ROLE",
        "CERTD_REMOTE_NODES",
        "CERTD_REMOTE_TLS",
        "CERTD_REMOTE_TLS_NO_VERIFY",
        "CERTD_REMOTE_API_SECRET",
        "CERTD_REMOTE_WITH_PROXY",
    ] {
        unsafe { std::env::remove_var(key) };
    }
}

#[test]
fn loads_default_config() {
    let _guard = env_lock().lock().expect("env lock");
    clear_certd_env();
    let cfg = AppConfig::from_env().expect("config");

    assert_eq!(cfg.server.bind_addr, "0.0.0.0:8080");
    assert_eq!(cfg.security.bootstrap_user_id, "conalog-account");
    assert_eq!(cfg.hiqlite.runtime_mode, RuntimeMode::Embedded);
    assert_eq!(cfg.hiqlite.local_node_role, LocalNodeRole::Voter);
    assert_eq!(cfg.hiqlite.default_read_consistency, ReadConsistency::Local);
    match cfg.artifacts.backend {
        ArtifactBackendConfig::FileSystem { .. } => {}
        other => panic!("expected filesystem artifacts backend, got {other:?}"),
    }
}

#[test]
fn loads_split_runtime_config_from_env() {
    let _guard = env_lock().lock().expect("env lock");
    clear_certd_env();
    unsafe {
        std::env::set_var("CERTD_RUNTIME_MODE", "split");
        std::env::set_var("CERTD_LOCAL_NODE_ROLE", "learner");
        std::env::set_var("CERTD_READ_CONSISTENCY", "prefer-strong");
        std::env::set_var("CERTD_REMOTE_NODES", "http://a:8100,http://b:8100");
        std::env::set_var("CERTD_REMOTE_TLS", "true");
        std::env::set_var("CERTD_REMOTE_TLS_NO_VERIFY", "true");
        std::env::set_var("CERTD_REMOTE_API_SECRET", "secret-token");
        std::env::set_var("CERTD_REMOTE_WITH_PROXY", "true");
    }

    let cfg = AppConfig::from_env().expect("config");

    assert_eq!(cfg.hiqlite.runtime_mode, RuntimeMode::Split);
    assert_eq!(cfg.hiqlite.local_node_role, LocalNodeRole::Learner);
    assert_eq!(
        cfg.hiqlite.default_read_consistency,
        ReadConsistency::PreferStrong
    );
    assert_eq!(
        cfg.hiqlite.remote_nodes,
        vec!["a:8100".to_owned(), "b:8100".to_owned()]
    );
    assert!(cfg.hiqlite.remote_tls);
    assert!(cfg.hiqlite.remote_tls_no_verify);
    assert_eq!(cfg.hiqlite.remote_api_secret.as_deref(), Some("secret-token"));
    assert!(cfg.hiqlite.remote_with_proxy);
}

#[test]
fn normalizes_remote_nodes_without_scheme_changes() {
    let _guard = env_lock().lock().expect("env lock");
    clear_certd_env();
    unsafe {
        std::env::set_var("CERTD_RUNTIME_MODE", "remote");
        std::env::set_var(
            "CERTD_REMOTE_NODES",
            "http://a:8100, b:8200 ,https://c:8300",
        );
    }

    let cfg = AppConfig::from_env().expect("config");

    assert_eq!(
        cfg.hiqlite.remote_nodes,
        vec![
            "a:8100".to_owned(),
            "b:8200".to_owned(),
            "c:8300".to_owned()
        ]
    );
}

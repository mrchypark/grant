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
        "CERTD_TRAILBASE_BASE_URL",
        "CERTD_TRAILBASE_INTROSPECT_PATH",
        "CERTD_TRAILBASE_INTERNAL_SECRET",
        "CERTD_TRAILBASE_CACHE_TTL_SECONDS",
        "CERTD_MASTER_KEY",
        "CERTD_ALLOW_UNTRUSTED_CERTIFICATES",
        "CERTD_CERT_TRUST_BUNDLE_PATH",
        "CERTD_KMS_ACTIVE_KEY_ID",
        "CERTD_KMS_MASTER_KEYS",
    ] {
        unsafe { std::env::remove_var(key) };
    }
}

fn set_test_kms_env() {
    unsafe {
        std::env::set_var("CERTD_KMS_ACTIVE_KEY_ID", "test-kms-key");
        std::env::set_var("CERTD_KMS_MASTER_KEYS", "test-kms-key:dGVzdC1rbXMta2V5");
    }
}

#[test]
fn loads_default_config() {
    let _guard = env_lock().lock().expect("env lock");
    clear_certd_env();
    set_test_kms_env();
    unsafe {
        std::env::set_var("CERTD_TRAILBASE_INTERNAL_SECRET", "default-secret");
    }
    let cfg = AppConfig::from_env().expect("config");

    assert_eq!(cfg.server.bind_addr, "0.0.0.0:8080");
    assert_eq!(cfg.security.bootstrap_user_id, "conalog-account");
    assert_eq!(cfg.auth.trailbase.base_url, "http://127.0.0.1:4000");
    assert_eq!(
        cfg.auth.trailbase.internal_secret,
        "default-secret".to_owned()
    );
    assert_eq!(cfg.kms.active_key_id, "test-kms-key");
    assert_eq!(cfg.hiqlite.runtime_mode, RuntimeMode::Embedded);
    assert!(!cfg.security.allow_untrusted_certificates);
    assert_eq!(cfg.security.certificate_trust_bundle_path, None);
    assert_eq!(cfg.hiqlite.local_node_role, LocalNodeRole::Voter);
    assert_eq!(cfg.hiqlite.default_read_consistency, ReadConsistency::Local);
    match cfg.artifacts.backend {
        ArtifactBackendConfig::FileSystem { .. } => {}
        other => panic!("expected filesystem artifacts backend, got {other:?}"),
    }
}

#[test]
fn loads_security_certificate_trust_bundle_path_from_env() {
    let _guard = env_lock().lock().expect("env lock");
    clear_certd_env();
    set_test_kms_env();
    unsafe {
        std::env::set_var("CERTD_TRAILBASE_INTERNAL_SECRET", "trust-secret");
        std::env::set_var("CERTD_CERT_TRUST_BUNDLE_PATH", "/etc/ssl/certs/custom.pem");
    }

    let cfg = AppConfig::from_env().expect("config");

    assert_eq!(
        cfg.security.certificate_trust_bundle_path.as_deref(),
        Some(std::path::Path::new("/etc/ssl/certs/custom.pem"))
    );
}

#[test]
fn loads_trailbase_auth_config_from_env() {
    let _guard = env_lock().lock().expect("env lock");
    clear_certd_env();
    set_test_kms_env();
    unsafe {
        std::env::set_var("CERTD_TRAILBASE_BASE_URL", "http://trailbase.internal:4000");
        std::env::set_var("CERTD_TRAILBASE_INTROSPECT_PATH", "/api/certd/introspect");
        std::env::set_var("CERTD_TRAILBASE_INTERNAL_SECRET", "shared-secret");
        std::env::set_var("CERTD_TRAILBASE_CACHE_TTL_SECONDS", "45");
    }

    let cfg = AppConfig::from_env().expect("config");

    assert_eq!(
        cfg.auth.trailbase.base_url,
        "http://trailbase.internal:4000"
    );
    assert_eq!(cfg.auth.trailbase.introspect_path, "/api/certd/introspect");
    assert_eq!(
        cfg.auth.trailbase.internal_secret,
        "shared-secret".to_owned()
    );
    assert_eq!(cfg.auth.trailbase.cache_ttl_seconds, 45);
}

#[test]
fn missing_trailbase_internal_secret_is_rejected() {
    let _guard = env_lock().lock().expect("env lock");
    clear_certd_env();
    set_test_kms_env();

    let err = AppConfig::from_env().expect_err("missing internal secret should fail");
    assert!(err.to_string().contains("CERTD_TRAILBASE_INTERNAL_SECRET"));
}

#[test]
fn loads_split_runtime_config_from_env() {
    let _guard = env_lock().lock().expect("env lock");
    clear_certd_env();
    set_test_kms_env();
    unsafe {
        std::env::set_var("CERTD_TRAILBASE_INTERNAL_SECRET", "split-secret");
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
    assert_eq!(
        cfg.hiqlite.remote_api_secret.as_deref(),
        Some("secret-token")
    );
    assert!(cfg.hiqlite.remote_with_proxy);
}

#[test]
fn normalizes_remote_nodes_without_scheme_changes() {
    let _guard = env_lock().lock().expect("env lock");
    clear_certd_env();
    set_test_kms_env();
    unsafe {
        std::env::set_var("CERTD_TRAILBASE_INTERNAL_SECRET", "remote-secret");
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

#[test]
fn falls_back_to_legacy_master_key_when_kms_envs_are_absent() {
    let _guard = env_lock().lock().expect("env lock");
    clear_certd_env();
    unsafe {
        std::env::set_var("CERTD_TRAILBASE_INTERNAL_SECRET", "fallback-secret");
        std::env::set_var("CERTD_MASTER_KEY", "legacy-master-key-material");
    }

    let cfg = AppConfig::from_env().expect("config");

    assert_eq!(cfg.kms.active_key_id, "legacy-master-key");
    assert_eq!(
        cfg.kms.master_keys,
        vec![(
            "legacy-master-key".to_owned(),
            b"legacy-master-key-material".to_vec()
        )]
    );
}

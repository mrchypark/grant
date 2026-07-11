use infra::config::{AppConfig, LocalNodeRole, ReadConsistency, RuntimeMode};
use std::sync::{Mutex, MutexGuard, OnceLock};

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn clear_grant_env() {
    for key in [
        "GRANT_RUNTIME_MODE",
        "GRANT_READ_CONSISTENCY",
        "GRANT_LOCAL_NODE_ROLE",
        "GRANT_REMOTE_NODES",
        "GRANT_REMOTE_TLS",
        "GRANT_REMOTE_TLS_NO_VERIFY",
        "GRANT_REMOTE_API_SECRET",
        "GRANT_REMOTE_WITH_PROXY",
        "GRANT_RAUTHY_BASE_URL",
        "GRANT_RAUTHY_INTROSPECT_PATH",
        "GRANT_RAUTHY_USERINFO_PATH",
        "GRANT_RAUTHY_CLIENT_ID",
        "GRANT_RAUTHY_CLIENT_SECRET",
        "GRANT_RAUTHY_ADMIN_API_KEY",
        "GRANT_RAUTHY_CACHE_TTL_SECONDS",
        "GRANT_MASTER_KEY",
    ] {
        unsafe { std::env::remove_var(key) };
    }
}

struct EnvGuard {
    _guard: MutexGuard<'static, ()>,
}

impl EnvGuard {
    fn new() -> Self {
        let guard = env_lock().lock().expect("env lock");
        clear_grant_env();
        Self { _guard: guard }
    }

    fn set(&self, key: &str, value: &str) {
        unsafe { std::env::set_var(key, value) };
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        clear_grant_env();
    }
}

#[test]
fn loads_default_config() {
    let env = EnvGuard::new();
    env.set("GRANT_RAUTHY_CLIENT_ID", "grant");
    env.set("GRANT_RAUTHY_CLIENT_SECRET", "default-secret");
    env.set("GRANT_RAUTHY_ADMIN_API_KEY", "grant-admin-key");
    let cfg = AppConfig::from_env().expect("config");

    assert_eq!(cfg.server.bind_addr, "0.0.0.0:8080");
    assert_eq!(cfg.security.bootstrap_user_id, "conalog-grant");
    assert_eq!(cfg.auth.rauthy.base_url, "http://127.0.0.1:8080");
    assert_eq!(cfg.auth.rauthy.client_id, "grant".to_owned());
    assert_eq!(cfg.auth.rauthy.client_secret, "default-secret".to_owned());
    assert_eq!(cfg.auth.rauthy.admin_api_key, "grant-admin-key".to_owned());
    assert_eq!(cfg.hiqlite.runtime_mode, RuntimeMode::Embedded);
    assert_eq!(cfg.hiqlite.local_node_role, LocalNodeRole::Voter);
    assert_eq!(cfg.hiqlite.default_read_consistency, ReadConsistency::Local);
}

#[test]
fn loads_rauthy_auth_config_from_env() {
    let env = EnvGuard::new();
    env.set("GRANT_RAUTHY_BASE_URL", "http://rauthy.internal:8080");
    env.set("GRANT_RAUTHY_INTROSPECT_PATH", "/auth/v1/oidc/introspect");
    env.set("GRANT_RAUTHY_USERINFO_PATH", "/auth/v1/oidc/userinfo");
    env.set("GRANT_RAUTHY_CLIENT_ID", "grant");
    env.set("GRANT_RAUTHY_CLIENT_SECRET", "shared-secret");
    env.set("GRANT_RAUTHY_ADMIN_API_KEY", "shared-admin-key");
    env.set("GRANT_RAUTHY_CACHE_TTL_SECONDS", "45");

    let cfg = AppConfig::from_env().expect("config");

    assert_eq!(cfg.auth.rauthy.base_url, "http://rauthy.internal:8080");
    assert_eq!(cfg.auth.rauthy.introspect_path, "/auth/v1/oidc/introspect");
    assert_eq!(cfg.auth.rauthy.userinfo_path, "/auth/v1/oidc/userinfo");
    assert_eq!(cfg.auth.rauthy.client_id, "grant");
    assert_eq!(cfg.auth.rauthy.client_secret, "shared-secret".to_owned());
    assert_eq!(cfg.auth.rauthy.admin_api_key, "shared-admin-key".to_owned());
    assert_eq!(cfg.auth.rauthy.cache_ttl_seconds, 45);
}

#[test]
fn missing_rauthy_client_id_is_rejected() {
    let _env = EnvGuard::new();

    let err = AppConfig::from_env().expect_err("missing client id should fail");
    assert!(err.to_string().contains("GRANT_RAUTHY_CLIENT_ID"));
}

#[test]
fn missing_rauthy_client_secret_is_rejected() {
    let env = EnvGuard::new();
    env.set("GRANT_RAUTHY_CLIENT_ID", "grant");

    let err = AppConfig::from_env().expect_err("missing client secret should fail");
    assert!(err.to_string().contains("GRANT_RAUTHY_CLIENT_SECRET"));
}

#[test]
fn missing_rauthy_admin_api_key_is_rejected() {
    let env = EnvGuard::new();
    env.set("GRANT_RAUTHY_CLIENT_ID", "grant");
    env.set("GRANT_RAUTHY_CLIENT_SECRET", "shared-secret");

    let err = AppConfig::from_env().expect_err("missing admin api key should fail");
    assert!(err.to_string().contains("GRANT_RAUTHY_ADMIN_API_KEY"));
}

#[test]
fn loads_split_runtime_config_from_env() {
    let env = EnvGuard::new();
    env.set("GRANT_RAUTHY_CLIENT_ID", "grant");
    env.set("GRANT_RAUTHY_CLIENT_SECRET", "split-secret");
    env.set("GRANT_RAUTHY_ADMIN_API_KEY", "split-admin-key");
    env.set("GRANT_RUNTIME_MODE", "split");
    env.set("GRANT_LOCAL_NODE_ROLE", "learner");
    env.set("GRANT_READ_CONSISTENCY", "prefer-strong");
    env.set("GRANT_REMOTE_NODES", "http://a:8100,http://b:8100");
    env.set("GRANT_REMOTE_TLS", "true");
    env.set("GRANT_REMOTE_TLS_NO_VERIFY", "true");
    env.set("GRANT_REMOTE_API_SECRET", "secret-token");
    env.set("GRANT_REMOTE_WITH_PROXY", "true");

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
fn rejects_invalid_runtime_mode() {
    let _guard = env_lock().lock().expect("env lock");
    clear_grant_env();
    unsafe {
        std::env::set_var("GRANT_RAUTHY_CLIENT_ID", "grant");
        std::env::set_var("GRANT_RAUTHY_CLIENT_SECRET", "runtime-secret");
        std::env::set_var("GRANT_RAUTHY_ADMIN_API_KEY", "runtime-admin-key");
        std::env::set_var("GRANT_RUNTIME_MODE", "invalid");
    }

    let err = AppConfig::from_env().expect_err("invalid runtime mode should fail");
    assert!(err.to_string().contains("GRANT_RUNTIME_MODE"));
}

#[test]
fn rejects_invalid_read_consistency() {
    let _guard = env_lock().lock().expect("env lock");
    clear_grant_env();
    unsafe {
        std::env::set_var("GRANT_RAUTHY_CLIENT_ID", "grant");
        std::env::set_var("GRANT_RAUTHY_CLIENT_SECRET", "consistency-secret");
        std::env::set_var("GRANT_RAUTHY_ADMIN_API_KEY", "consistency-admin-key");
        std::env::set_var("GRANT_READ_CONSISTENCY", "eventual");
    }

    let err = AppConfig::from_env().expect_err("invalid read consistency should fail");
    assert!(err.to_string().contains("GRANT_READ_CONSISTENCY"));
}

#[test]
fn rejects_invalid_local_node_role() {
    let _guard = env_lock().lock().expect("env lock");
    clear_grant_env();
    unsafe {
        std::env::set_var("GRANT_RAUTHY_CLIENT_ID", "grant");
        std::env::set_var("GRANT_RAUTHY_CLIENT_SECRET", "role-secret");
        std::env::set_var("GRANT_RAUTHY_ADMIN_API_KEY", "role-admin-key");
        std::env::set_var("GRANT_LOCAL_NODE_ROLE", "observer");
    }

    let err = AppConfig::from_env().expect_err("invalid local node role should fail");
    assert!(err.to_string().contains("GRANT_LOCAL_NODE_ROLE"));
}

#[test]
fn normalizes_remote_nodes_without_scheme_changes() {
    let env = EnvGuard::new();
    env.set("GRANT_RAUTHY_CLIENT_ID", "grant");
    env.set("GRANT_RAUTHY_CLIENT_SECRET", "remote-secret");
    env.set("GRANT_RAUTHY_ADMIN_API_KEY", "remote-admin-key");
    env.set("GRANT_RUNTIME_MODE", "remote");
    env.set(
        "GRANT_REMOTE_NODES",
        "http://a:8100, b:8200 ,https://c:8300",
    );

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

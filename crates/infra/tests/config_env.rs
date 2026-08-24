use infra::config::{AppConfig, AuthProviderMode, LocalNodeRole, ReadConsistency, RuntimeMode};
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
        "GRANT_RAUTHY_CLIENT_READ_API_KEY",
        "GRANT_RAUTHY_CACHE_TTL_SECONDS",
        "GRANT_AUTH_PROVIDER",
        "GRANT_PATCH_DATA_BASE_URL",
        "GRANT_PATCH_DATA_EXPOSURE_APPLICATION_ID",
        "GRANT_MASTER_KEY",
        "GRANT_APP_AUTH_KEY",
    ] {
        unsafe { std::env::remove_var(key) };
    }
}

#[test]
fn patch_data_mode_requires_only_its_internal_auth_configuration() {
    let env = EnvGuard::new();
    env.set("GRANT_AUTH_PROVIDER", "patch-data");
    env.set(
        "GRANT_PATCH_DATA_BASE_URL",
        "http://patch-data.patch.svc.cluster.local:8090",
    );
    env.set(
        "GRANT_PATCH_DATA_EXPOSURE_APPLICATION_ID",
        "patch-data-exposure",
    );
    let cfg = AppConfig::from_env().expect("Patch Data config");
    assert_eq!(cfg.auth.provider_mode, AuthProviderMode::PatchData);
    let patch_data = cfg.auth.patch_data.expect("Patch Data settings");
    assert_eq!(patch_data.exposure_application_id, "patch-data-exposure");
}

#[test]
fn patch_data_mode_accepts_only_exact_cluster_service_or_loopback_origins() {
    for base_url in [
        "http://patch-data.patch.svc.cluster.local:8090",
        "http://patch-data.grant-compos-exposure-v1.svc.cluster.local:8090",
        "http://127.0.0.1:8090",
        "http://[::1]:8090",
    ] {
        let env = EnvGuard::new();
        env.set("GRANT_AUTH_PROVIDER", "patch-data");
        env.set("GRANT_PATCH_DATA_BASE_URL", base_url);
        env.set(
            "GRANT_PATCH_DATA_EXPOSURE_APPLICATION_ID",
            "patch-data-exposure",
        );
        AppConfig::from_env().unwrap_or_else(|error| panic!("{base_url}: {error}"));
    }
}

#[test]
fn patch_data_mode_rejects_every_other_origin_shape() {
    for base_url in [
        "file:///tmp/patch-data",
        "https://patch-data.patch.svc.cluster.local:8090",
        "http://patch-data.patch.svc.cluster.local",
        "http://patch-data.patch.svc.cluster.local:80",
        "http://patch-data.patch.svc.cluster.local:8090/",
        "http://patch-data.patch.svc.cluster.local:8090/api",
        "http://patch-data.patch.svc.cluster.local:8090/%2f",
        "http://patch-data.patch.svc.cluster.local:8090?query=1",
        "http://patch-data.patch.svc.cluster.local:8090#fragment",
        "http://user@patch-data.patch.svc.cluster.local:8090",
        "http://user:password@patch-data.patch.svc.cluster.local:8090",
        "http://Patch-Data.patch.svc.cluster.local:8090",
        "http://patch-data.PATCH.svc.cluster.local:8090",
        "HTTP://patch-data.patch.svc.cluster.local:8090",
        "http://patch-data.svc.cluster.local:8090",
        "http://patch-data.patch.svc:8090",
        "http://other.patch.svc.cluster.local:8090",
        "http://patch-data.patch.other.cluster.local:8090",
        "http://patch-data.a.b.svc.cluster.local:8090",
        "http://patch-data.-patch.svc.cluster.local:8090",
        "http://patch-data.patch-.svc.cluster.local:8090",
        "http://patch-data.patch_.svc.cluster.local:8090",
        "http://patch-data.*.svc.cluster.local:8090",
        "http://localhost:8090",
        "http://10.0.0.1:8090",
        "http://169.254.169.254:8090",
        "http://[fd00::1]:8090",
        "http://127.0.0.1:8080",
        "http://[::1]:8080",
        " http://patch-data.patch.svc.cluster.local:8090",
        "http://patch-data.patch.svc.cluster.local:8090 ",
    ] {
        let env = EnvGuard::new();
        env.set("GRANT_AUTH_PROVIDER", "patch-data");
        env.set("GRANT_PATCH_DATA_BASE_URL", base_url);
        env.set(
            "GRANT_PATCH_DATA_EXPOSURE_APPLICATION_ID",
            "patch-data-exposure",
        );
        let error = match AppConfig::from_env() {
            Ok(_) => panic!("unexpectedly accepted {base_url}"),
            Err(error) => error,
        };
        assert!(
            error.to_string().contains("GRANT_PATCH_DATA_BASE_URL"),
            "{base_url}: {error}"
        );
    }
}

struct EnvGuard {
    _guard: MutexGuard<'static, ()>,
}

impl EnvGuard {
    fn new() -> Self {
        let guard = env_lock().lock().expect("env lock");
        clear_grant_env();
        unsafe { std::env::set_var("GRANT_APP_AUTH_KEY", "dev-only-app-auth-key") };
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
    env.set("GRANT_RAUTHY_CLIENT_READ_API_KEY", "grant-client-read-key");
    let cfg = AppConfig::from_env().expect("config");

    assert_eq!(cfg.server.bind_addr, "0.0.0.0:8080");
    assert_eq!(cfg.security.bootstrap_user_id, "conalog-grant");
    assert_eq!(cfg.security.app_auth_key, "dev-only-app-auth-key");
    assert_eq!(cfg.auth.rauthy.base_url, "http://127.0.0.1:8080");
    assert_eq!(cfg.auth.rauthy.client_id, "grant".to_owned());
    assert_eq!(cfg.auth.rauthy.client_secret, "default-secret".to_owned());
    assert_eq!(
        cfg.auth.rauthy.client_read_api_key,
        "grant-client-read-key".to_owned()
    );
    assert_eq!(cfg.hiqlite.runtime_mode, RuntimeMode::Embedded);
    assert_eq!(cfg.hiqlite.local_node_role, LocalNodeRole::Voter);
    assert_eq!(cfg.hiqlite.default_read_consistency, ReadConsistency::Local);
}

#[test]
fn missing_app_auth_key_is_rejected() {
    let _env = EnvGuard::new();
    unsafe { std::env::remove_var("GRANT_APP_AUTH_KEY") };

    let err = AppConfig::from_env().expect_err("missing app auth key should fail");
    assert!(err.to_string().contains("GRANT_APP_AUTH_KEY"));
}

#[test]
fn app_auth_key_must_differ_from_master_key() {
    let env = EnvGuard::new();
    env.set("GRANT_MASTER_KEY", "same-key");
    env.set("GRANT_APP_AUTH_KEY", "same-key");

    let err = AppConfig::from_env().expect_err("shared admin and app auth key should fail");
    assert!(
        err.to_string()
            .contains("GRANT_APP_AUTH_KEY must differ from GRANT_MASTER_KEY")
    );
}

#[test]
fn loads_rauthy_auth_config_from_env() {
    let env = EnvGuard::new();
    env.set("GRANT_RAUTHY_BASE_URL", "http://rauthy.internal:8080");
    env.set("GRANT_RAUTHY_INTROSPECT_PATH", "/auth/v1/oidc/introspect");
    env.set("GRANT_RAUTHY_USERINFO_PATH", "/auth/v1/oidc/userinfo");
    env.set("GRANT_RAUTHY_CLIENT_ID", "grant");
    env.set("GRANT_RAUTHY_CLIENT_SECRET", "shared-secret");
    env.set("GRANT_RAUTHY_CLIENT_READ_API_KEY", "shared-client-read-key");
    env.set("GRANT_RAUTHY_CACHE_TTL_SECONDS", "45");

    let cfg = AppConfig::from_env().expect("config");

    assert_eq!(cfg.auth.rauthy.base_url, "http://rauthy.internal:8080");
    assert_eq!(cfg.auth.rauthy.introspect_path, "/auth/v1/oidc/introspect");
    assert_eq!(cfg.auth.rauthy.userinfo_path, "/auth/v1/oidc/userinfo");
    assert_eq!(cfg.auth.rauthy.client_id, "grant");
    assert_eq!(cfg.auth.rauthy.client_secret, "shared-secret".to_owned());
    assert_eq!(
        cfg.auth.rauthy.client_read_api_key,
        "shared-client-read-key".to_owned()
    );
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
fn missing_rauthy_client_read_api_key_is_rejected() {
    let env = EnvGuard::new();
    env.set("GRANT_RAUTHY_CLIENT_ID", "grant");
    env.set("GRANT_RAUTHY_CLIENT_SECRET", "shared-secret");

    let err = AppConfig::from_env().expect_err("missing client read api key should fail");
    assert!(err.to_string().contains("GRANT_RAUTHY_CLIENT_READ_API_KEY"));
}

#[test]
fn loads_split_runtime_config_from_env() {
    let env = EnvGuard::new();
    env.set("GRANT_RAUTHY_CLIENT_ID", "grant");
    env.set("GRANT_RAUTHY_CLIENT_SECRET", "split-secret");
    env.set("GRANT_RAUTHY_CLIENT_READ_API_KEY", "split-client-read-key");
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
        std::env::set_var("GRANT_APP_AUTH_KEY", "dev-only-app-auth-key");
        std::env::set_var("GRANT_RAUTHY_CLIENT_ID", "grant");
        std::env::set_var("GRANT_RAUTHY_CLIENT_SECRET", "runtime-secret");
        std::env::set_var(
            "GRANT_RAUTHY_CLIENT_READ_API_KEY",
            "runtime-client-read-key",
        );
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
        std::env::set_var("GRANT_APP_AUTH_KEY", "dev-only-app-auth-key");
        std::env::set_var("GRANT_RAUTHY_CLIENT_ID", "grant");
        std::env::set_var("GRANT_RAUTHY_CLIENT_SECRET", "consistency-secret");
        std::env::set_var(
            "GRANT_RAUTHY_CLIENT_READ_API_KEY",
            "consistency-client-read-key",
        );
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
        std::env::set_var("GRANT_APP_AUTH_KEY", "dev-only-app-auth-key");
        std::env::set_var("GRANT_RAUTHY_CLIENT_ID", "grant");
        std::env::set_var("GRANT_RAUTHY_CLIENT_SECRET", "role-secret");
        std::env::set_var("GRANT_RAUTHY_CLIENT_READ_API_KEY", "role-client-read-key");
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
    env.set("GRANT_RAUTHY_CLIENT_READ_API_KEY", "remote-client-read-key");
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

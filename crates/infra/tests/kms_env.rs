use infra::{
    config::AppConfig,
    kms::{EnvKmsProvider, KmsProvider},
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
        "CERTD_KMS_ACTIVE_KEY_ID",
        "CERTD_KMS_MASTER_KEYS",
    ] {
        unsafe { std::env::remove_var(key) };
    }
}

#[test]
fn loads_kms_config_from_env() {
    let _guard = env_lock().lock().expect("env lock");
    clear_certd_env();
    unsafe {
        std::env::set_var("CERTD_TRAILBASE_INTERNAL_SECRET", "kms-secret");
        std::env::set_var("CERTD_KMS_ACTIVE_KEY_ID", "dev-key-2");
        std::env::set_var(
            "CERTD_KMS_MASTER_KEYS",
            "dev-key-1:Zmlyc3Qta2V5,dev-key-2:c2Vjb25kLWtleQ==",
        );
    }

    let cfg = AppConfig::from_env().expect("config");

    assert_eq!(cfg.kms.active_key_id, "dev-key-2");
    assert_eq!(
        cfg.kms.master_keys,
        vec![
            ("dev-key-1".to_owned(), b"first-key".to_vec()),
            ("dev-key-2".to_owned(), b"second-key".to_vec()),
        ]
    );
}

#[test]
fn missing_kms_active_key_is_rejected() {
    let _guard = env_lock().lock().expect("env lock");
    clear_certd_env();
    unsafe {
        std::env::set_var("CERTD_TRAILBASE_INTERNAL_SECRET", "kms-secret");
        std::env::set_var("CERTD_KMS_MASTER_KEYS", "dev-key-1:Zmlyc3Qta2V5");
    }

    let err = AppConfig::from_env().expect_err("missing active key should fail");
    assert!(err.to_string().contains("CERTD_KMS_ACTIVE_KEY_ID"));
}

#[test]
fn malformed_kms_master_keys_are_rejected() {
    let _guard = env_lock().lock().expect("env lock");
    clear_certd_env();
    unsafe {
        std::env::set_var("CERTD_TRAILBASE_INTERNAL_SECRET", "kms-secret");
        std::env::set_var("CERTD_KMS_ACTIVE_KEY_ID", "dev-key-1");
        std::env::set_var("CERTD_KMS_MASTER_KEYS", "broken-entry");
    }

    let err = AppConfig::from_env().expect_err("malformed keys should fail");
    assert!(err.to_string().contains("CERTD_KMS_MASTER_KEYS"));
}

#[test]
fn non_base64_kms_master_keys_are_rejected() {
    let _guard = env_lock().lock().expect("env lock");
    clear_certd_env();
    unsafe {
        std::env::set_var("CERTD_TRAILBASE_INTERNAL_SECRET", "kms-secret");
        std::env::set_var("CERTD_KMS_ACTIVE_KEY_ID", "dev-key-1");
        std::env::set_var("CERTD_KMS_MASTER_KEYS", "dev-key-1:not-base64");
    }

    let err = AppConfig::from_env().expect_err("invalid key material should fail");
    assert!(err.to_string().contains("CERTD_KMS_MASTER_KEYS"));
}

#[test]
fn env_kms_provider_wraps_and_unwraps_keys() {
    let _guard = env_lock().lock().expect("env lock");
    clear_certd_env();
    unsafe {
        std::env::set_var("CERTD_KMS_ACTIVE_KEY_ID", "dev-key-1");
        std::env::set_var("CERTD_KMS_MASTER_KEYS", "dev-key-1:Zm9vYmFy");
    }

    let provider = EnvKmsProvider::from_env().expect("kms provider");
    let wrapped = provider.wrap(b"dek-bytes").expect("wrap");
    let unwrapped = provider
        .unwrap(provider.active_key_id(), &wrapped)
        .expect("unwrap");

    assert_eq!(unwrapped, b"dek-bytes");
}

use std::path::PathBuf;

use infra::{
    config::KmsConfig,
    crypto::{
        CertificateBundle, decrypt_bundle_with_key, encrypt_bundle_with_key,
        generate_data_encryption_key,
    },
    kms::{EnvKmsProvider, KmsProvider},
    npki::{NpkiBundle, NpkiSigner},
};

#[test]
fn load_bundle_from_directory_reads_all_files() {
    let bundle = NpkiBundle::from_directory(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../secrets/cert"),
    )
    .expect("load bundle from secrets/cert");

    assert!(!bundle.sign_cert_der.is_empty());
    assert!(!bundle.sign_pri_key.is_empty());
    assert!(!bundle.km_cert_der.is_empty());
    assert!(!bundle.km_pri_key.is_empty());
}

#[test]
fn signer_loads_certificate_and_builds_payloads() {
    let password =
        std::env::var("RPS_TEST_CERT_PASSWORD").unwrap_or_else(|_| "jmztnti132!".to_owned());
    let bundle = NpkiBundle::from_directory(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../secrets/cert"),
    )
    .expect("load bundle from secrets/cert");

    let signer = NpkiSigner::new(&bundle, &password).expect("build signer");

    assert!(!signer.common_name().is_empty());
    assert!(!signer.vid_random_b64().is_empty());

    let signed = signer
        .sign_pkcs7_pem(b"2768603291")
        .expect("sign business number");
    assert!(signed.contains("BEGIN PKCS7"));

    let enveloped = signer
        .envelop_pkcs7_pem(&bundle.km_cert_der, b"2768603291")
        .expect("envelop business number");
    assert!(enveloped.contains("BEGIN PKCS7"));
}

#[test]
fn dek_encrypted_bundle_round_trips_after_kms_wrap_and_unwrap() {
    let bundle = CertificateBundle {
        business_no: "2768603291".to_owned(),
        password: "original-password".to_owned(),
        common_name: "Test Certificate".to_owned(),
        sign_cert_der: vec![1, 2, 3],
        sign_pri_key: vec![4, 5, 6],
        km_cert_der: vec![7, 8, 9],
        km_pri_key: vec![10, 11, 12],
    };
    let kms = EnvKmsProvider::from_config(&KmsConfig {
        active_key_id: "local-dev-v1".to_owned(),
        master_keys: vec![("local-dev-v1".to_owned(), b"master-key-material".to_vec())],
    })
    .expect("kms provider");

    let dek = generate_data_encryption_key();
    let encrypted = encrypt_bundle_with_key(&dek, &bundle).expect("encrypt bundle with dek");
    let wrapped = kms.wrap(&dek).expect("wrap dek");
    let unwrapped = kms
        .unwrap(kms.active_key_id(), &wrapped)
        .expect("unwrap dek");
    let decrypted =
        decrypt_bundle_with_key(&unwrapped, &encrypted).expect("decrypt bundle with dek");

    assert_eq!(decrypted.business_no, "2768603291");
    assert_eq!(decrypted.password, "original-password");
    assert_eq!(decrypted.common_name, "Test Certificate");
    assert_eq!(decrypted.sign_cert_der, vec![1, 2, 3]);
    assert_eq!(decrypted.km_pri_key, vec![10, 11, 12]);
}

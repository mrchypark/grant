use std::path::PathBuf;

use infra::npki::{NpkiBundle, NpkiSigner};

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

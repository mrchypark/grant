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
use openssl::{
    asn1::Asn1Time,
    hash::MessageDigest,
    pkey::{PKey, Private},
    rsa::Rsa,
    x509::{X509, X509NameBuilder},
};

#[test]
fn load_bundle_from_directory_reads_all_files() {
    let bundle = NpkiBundle::from_directory(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../secrets/cert"),
    )
    .expect("load bundle from secrets/cert");

    assert!(!bundle.sign_cert_der.is_empty());
    assert!(!bundle.sign_pri_key.is_empty());
    assert!(!bundle.km_cert_der.as_deref().unwrap_or_default().is_empty());
    assert!(!bundle.km_pri_key.as_deref().unwrap_or_default().is_empty());
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
        .envelop_pkcs7_pem(
            bundle.km_cert_der.as_deref().expect("km cert der present"),
            b"2768603291",
        )
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
        km_cert_der: Some(vec![7, 8, 9]),
        km_pri_key: Some(vec![10, 11, 12]),
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
    assert_eq!(decrypted.km_cert_der, Some(vec![7, 8, 9]));
    assert_eq!(decrypted.km_pri_key, Some(vec![10, 11, 12]));
}

#[test]
fn dek_encrypted_bundle_round_trips_without_km_material() {
    let bundle = CertificateBundle {
        business_no: "2768603291".to_owned(),
        password: "original-password".to_owned(),
        common_name: "Test Certificate".to_owned(),
        sign_cert_der: vec![1, 2, 3],
        sign_pri_key: vec![4, 5, 6],
        km_cert_der: None,
        km_pri_key: None,
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
    assert_eq!(decrypted.km_cert_der, None);
    assert_eq!(decrypted.km_pri_key, None);
}

#[test]
fn signer_accepts_direct_pkcs8_private_keys_without_vid_random() {
    let (certificate, private_key) = test_certificate_and_key();
    let signer = NpkiSigner::new(
        &NpkiBundle {
            sign_cert_der: certificate.to_der().expect("certificate der"),
            sign_pri_key: private_key
                .private_key_to_pkcs8()
                .expect("private key pkcs8"),
            km_cert_der: None,
            km_pri_key: None,
        },
        "unused-password",
    )
    .expect("build signer from direct pkcs8 key");

    assert!(!signer.common_name().is_empty());
    assert_eq!(signer.vid_random_b64(), "");
    let signed = signer
        .sign_pkcs7_pem(b"2768603291")
        .expect("sign business number");
    assert!(signed.contains("BEGIN PKCS7"));
}

#[test]
fn signer_rejects_unsupported_direct_private_key_algorithms() {
    let (certificate, _) = test_certificate_and_key();
    let private_key = PKey::generate_ed25519().expect("generate ed25519 key");
    let err = match NpkiSigner::new(
        &NpkiBundle {
            sign_cert_der: certificate.to_der().expect("certificate der"),
            sign_pri_key: private_key
                .private_key_to_pkcs8()
                .expect("private key pkcs8"),
            km_cert_der: None,
            km_pri_key: None,
        },
        "unused-password",
    ) {
        Ok(_) => panic!("ed25519 direct key should be rejected"),
        Err(err) => err,
    };

    let full_error = format!("{err:#}");
    assert!(
        full_error.contains("unsupported direct private key algorithm"),
        "error = {full_error}"
    );
}

fn test_certificate_and_key() -> (X509, PKey<Private>) {
    let rsa = Rsa::generate(2048).expect("generate rsa key");
    let private_key = PKey::from_rsa(rsa).expect("wrap rsa key");

    let mut name = X509NameBuilder::new().expect("x509 name");
    name.append_entry_by_text("CN", "Direct PKCS8 Test")
        .expect("append common name");
    let name = name.build();

    let mut builder = X509::builder().expect("x509 builder");
    builder.set_version(2).expect("set version");
    builder
        .set_subject_name(&name)
        .expect("set subject name");
    builder.set_issuer_name(&name).expect("set issuer name");
    builder.set_pubkey(&private_key).expect("set public key");
    let not_before = Asn1Time::days_from_now(0).expect("not before");
    builder
        .set_not_before(not_before.as_ref())
        .expect("set not before");
    let not_after = Asn1Time::days_from_now(365).expect("not after");
    builder
        .set_not_after(not_after.as_ref())
        .expect("set not after");
    builder
        .sign(&private_key, MessageDigest::sha256())
        .expect("sign certificate");
    (builder.build(), private_key)
}

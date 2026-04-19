use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, OsRng, rand_core::RngCore},
};
use anyhow::{Context, Result, anyhow, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use x509_parser::prelude::*;

const OID_X509_SERIAL_NUMBER: &str = "2.5.4.5";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CertificateBundle {
    pub business_no: String,
    pub password: String,
    pub common_name: String,
    pub sign_cert_der: Vec<u8>,
    pub sign_pri_key: Vec<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub km_cert_der: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub km_pri_key: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncryptedBundle {
    pub nonce_b64: String,
    pub ciphertext_b64: String,
}

pub fn normalize_business_no(value: &str) -> Result<String> {
    let digits: String = value.chars().filter(|ch| ch.is_ascii_digit()).collect();
    if digits.len() != 10 {
        bail!("businessNo must normalize to 10 digits");
    }
    Ok(digits)
}

pub fn decode_base64(value: &str) -> Result<Vec<u8>> {
    STANDARD
        .decode(value)
        .with_context(|| "decode base64 payload")
}

pub fn extract_common_name(sign_cert_der: &[u8]) -> Result<String> {
    let (_, cert) = X509Certificate::from_der(sign_cert_der)
        .map_err(|err| anyhow!("parse signCert.der as X509 DER: {err}"))?;
    for attr in cert.subject().iter_common_name() {
        let cn = attr
            .as_str()
            .map_err(|err| anyhow!("decode certificate common name: {err}"))?;
        if !cn.trim().is_empty() {
            return Ok(cn.trim().to_owned());
        }
    }
    bail!("certificate common name not found")
}

pub fn extract_business_no(sign_cert_der: &[u8]) -> Result<Option<String>> {
    let (_, cert) = X509Certificate::from_der(sign_cert_der)
        .map_err(|err| anyhow!("parse signCert.der as X509 DER: {err}"))?;

    for attr in cert.subject().iter_attributes() {
        if attr.attr_type().to_id_string() != OID_X509_SERIAL_NUMBER {
            continue;
        }
        let value = attr
            .as_str()
            .map_err(|err| anyhow!("decode certificate business number: {err}"))?;
        let digits: String = value.chars().filter(|ch| ch.is_ascii_digit()).collect();
        if digits.len() == 10 {
            return Ok(Some(digits));
        }
    }

    Ok(None)
}

pub fn encrypt_bundle(master_key: &str, bundle: &CertificateBundle) -> Result<String> {
    encrypt_bundle_with_key(master_key.as_bytes(), bundle)
}

pub fn encrypt_bundle_with_key(key_material: &[u8], bundle: &CertificateBundle) -> Result<String> {
    let key = derive_key(key_material);
    let cipher = Aes256Gcm::new_from_slice(&key).expect("32-byte AES key");

    let plaintext = serde_json::to_vec(bundle).context("serialize certificate bundle")?;
    let mut nonce = [0u8; 12];
    OsRng.fill_bytes(&mut nonce);

    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), plaintext.as_ref())
        .map_err(|err| anyhow!("encrypt certificate bundle: {err}"))?;

    let envelope = EncryptedBundle {
        nonce_b64: STANDARD.encode(nonce),
        ciphertext_b64: STANDARD.encode(ciphertext),
    };
    serde_json::to_string(&envelope).context("serialize encrypted bundle")
}

pub fn decrypt_bundle(master_key: &str, value: &str) -> Result<CertificateBundle> {
    decrypt_bundle_with_key(master_key.as_bytes(), value)
}

pub fn decrypt_bundle_with_key(key_material: &[u8], value: &str) -> Result<CertificateBundle> {
    let envelope: EncryptedBundle =
        serde_json::from_str(value).context("parse encrypted certificate bundle")?;
    let key = derive_key(key_material);
    let cipher = Aes256Gcm::new_from_slice(&key).expect("32-byte AES key");

    let nonce = STANDARD
        .decode(envelope.nonce_b64)
        .context("decode nonce")?;
    let ciphertext = STANDARD
        .decode(envelope.ciphertext_b64)
        .context("decode ciphertext")?;

    let plaintext = cipher
        .decrypt(Nonce::from_slice(&nonce), ciphertext.as_ref())
        .map_err(|err| anyhow!("decrypt certificate bundle: {err}"))?;
    serde_json::from_slice(&plaintext).context("decode certificate bundle")
}

pub fn generate_data_encryption_key() -> Vec<u8> {
    let mut dek = vec![0u8; 32];
    OsRng.fill_bytes(&mut dek);
    dek
}

fn derive_key(key_material: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(key_material);
    let digest = hasher.finalize();
    let mut key = [0u8; 32];
    key.copy_from_slice(&digest);
    key
}

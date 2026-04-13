use crate::config::KmsConfig;
use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use anyhow::{Context, Result, anyhow};
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

pub trait KmsProvider {
    fn active_key_id(&self) -> &str;
    fn wrap(&self, dek: &[u8]) -> Result<Vec<u8>>;
    fn unwrap(&self, key_id: &str, wrapped: &[u8]) -> Result<Vec<u8>>;
}

#[derive(Debug, Clone)]
pub struct EnvKmsProvider {
    active_key_id: String,
    master_keys: HashMap<String, Vec<u8>>,
}

impl EnvKmsProvider {
    pub fn from_env() -> Result<Self> {
        Self::from_config(&KmsConfig::from_env()?)
    }

    pub fn from_config(cfg: &KmsConfig) -> Result<Self> {
        let master_keys = cfg.master_keys.iter().cloned().collect::<HashMap<_, _>>();
        if !master_keys.contains_key(&cfg.active_key_id) {
            anyhow::bail!(
                "kms active key id {} is not present in configured master keys",
                cfg.active_key_id
            );
        }
        Ok(Self {
            active_key_id: cfg.active_key_id.clone(),
            master_keys,
        })
    }

    fn cipher_for_key(&self, key_id: &str) -> Result<Aes256Gcm> {
        let key_material = self
            .master_keys
            .get(key_id)
            .ok_or_else(|| anyhow!("unknown kms key id: {key_id}"))?;
        let key = derive_aead_key(key_material);
        Aes256Gcm::new_from_slice(&key).map_err(|_| anyhow!("invalid kms key material"))
    }
}

impl KmsProvider for EnvKmsProvider {
    fn active_key_id(&self) -> &str {
        &self.active_key_id
    }

    fn wrap(&self, dek: &[u8]) -> Result<Vec<u8>> {
        let cipher = self.cipher_for_key(&self.active_key_id)?;
        let mut nonce_bytes = [0u8; 12];
        rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
        let ciphertext = cipher
            .encrypt(Nonce::from_slice(&nonce_bytes), dek)
            .context("wrap dek with active kms key")?;
        let mut wrapped = nonce_bytes.to_vec();
        wrapped.extend_from_slice(&ciphertext);
        Ok(wrapped)
    }

    fn unwrap(&self, key_id: &str, wrapped: &[u8]) -> Result<Vec<u8>> {
        if wrapped.len() < 12 {
            anyhow::bail!("wrapped dek payload is too short");
        }

        let (nonce_bytes, ciphertext) = wrapped.split_at(12);
        let cipher = self.cipher_for_key(key_id)?;
        cipher
            .decrypt(Nonce::from_slice(nonce_bytes), ciphertext)
            .context("unwrap dek with kms key")
    }
}

fn derive_aead_key(key_material: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(key_material);
    hasher.finalize().into()
}

use crate::crypto::CertificateBundle;
use anyhow::{Context, Result, anyhow, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use openssl::{
    asn1::Asn1Time,
    hash::MessageDigest,
    nid::Nid,
    pkcs7::{Pkcs7, Pkcs7Flags},
    pkey::{Id, PKey, Private},
    provider::Provider,
    rsa::Rsa,
    sign::{Signer, Verifier},
    stack::Stack,
    symm::{Cipher, Crypter, Mode},
    x509::{
        X509, X509StoreContext,
        store::{X509StoreBuilder, X509StoreRef},
    },
};
use sha1::{Digest, Sha1};
use simple_asn1::{ASN1Block, ASN1Class, BigUint, OID, from_der};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::SystemTime,
};

const OID_NPKI_SEED_CBC: &[u64] = &[1, 2, 410, 200004, 1, 4];
const OID_NPKI_SEED_CBC_WITH_SHA1: &[u64] = &[1, 2, 410, 200004, 1, 15];

#[derive(Debug, Clone)]
pub struct NpkiBundle {
    pub sign_cert_der: Vec<u8>,
    pub sign_pri_key: Vec<u8>,
    pub km_cert_der: Option<Vec<u8>>,
    pub km_pri_key: Option<Vec<u8>>,
}

#[derive(Clone)]
pub struct NpkiSigner {
    certificate: X509,
    private_key: PKey<Private>,
    vid_random_b64: String,
}

impl NpkiBundle {
    pub fn from_directory(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        Ok(Self {
            sign_cert_der: std::fs::read(path.join("signCert.der"))
                .with_context(|| format!("read {}", path.join("signCert.der").display()))?,
            sign_pri_key: std::fs::read(path.join("signPri.key"))
                .with_context(|| format!("read {}", path.join("signPri.key").display()))?,
            km_cert_der: Some(
                std::fs::read(path.join("kmCert.der"))
                    .with_context(|| format!("read {}", path.join("kmCert.der").display()))?,
            ),
            km_pri_key: Some(
                std::fs::read(path.join("kmPri.key"))
                    .with_context(|| format!("read {}", path.join("kmPri.key").display()))?,
            ),
        })
    }
}

impl NpkiSigner {
    pub fn new(bundle: &NpkiBundle, password: &str) -> Result<Self> {
        ensure_openssl_legacy_provider()?;
        let certificate =
            X509::from_der(&bundle.sign_cert_der).context("parse signCert.der as X509")?;
        let (private_key, vid_random_b64) =
            load_signing_key(&bundle.sign_pri_key, password).context("load signing key")?;

        Ok(Self {
            certificate,
            private_key,
            vid_random_b64,
        })
    }

    pub fn common_name(&self) -> String {
        self.certificate
            .subject_name()
            .entries_by_nid(Nid::COMMONNAME)
            .find_map(|entry| entry.data().as_utf8().ok().map(|s| s.to_string()))
            .unwrap_or_default()
    }

    pub fn certificate_pem(&self) -> Result<String> {
        let pem = self
            .certificate
            .to_pem()
            .context("serialize certificate to PEM")?;
        String::from_utf8(pem).context("decode PEM as UTF-8")
    }

    pub fn vid_random_b64(&self) -> &str {
        &self.vid_random_b64
    }

    pub fn sign_pkcs7_pem(&self, message: &[u8]) -> Result<String> {
        let certs = Stack::new().context("create PKCS7 certificate stack")?;
        let pkcs7 = Pkcs7::sign(
            &self.certificate,
            &self.private_key,
            &certs,
            message,
            Pkcs7Flags::BINARY,
        )
        .context("build PKCS7 signedData")?;
        let pem = pkcs7.to_pem().context("encode PKCS7 signedData to PEM")?;
        String::from_utf8(pem).context("decode PKCS7 PEM as UTF-8")
    }

    pub fn envelop_pkcs7_pem(&self, recipient_cert_der: &[u8], plaintext: &[u8]) -> Result<String> {
        ensure_openssl_legacy_provider()?;
        let recipient = parse_certificate(recipient_cert_der)?;
        let mut certs = Stack::new().context("create recipient certificate stack")?;
        certs
            .push(recipient)
            .context("push recipient certificate")?;
        let pkcs7 = Pkcs7::encrypt(&certs, plaintext, Cipher::seed_cbc(), Pkcs7Flags::BINARY)
            .context("build PKCS7 envelopedData")?;
        let pem = pkcs7
            .to_pem()
            .context("encode PKCS7 envelopedData to PEM")?;
        String::from_utf8(pem).context("decode PKCS7 PEM as UTF-8")
    }
}

pub fn verify_challenge_signature(
    sign_cert_der: &[u8],
    challenge_payload: &str,
    challenge_signature_base64: &str,
) -> Result<()> {
    verify_message_signature(
        sign_cert_der,
        challenge_payload.as_bytes(),
        challenge_signature_base64,
        "challengeSignatureBase64",
    )
}

pub fn verify_message_signature(
    sign_cert_der: &[u8],
    message: &[u8],
    signature_base64: &str,
    signature_field: &str,
) -> Result<()> {
    ensure_openssl_legacy_provider()?;
    let certificate =
        X509::from_der(sign_cert_der).context("parse signCert.der as X509 for signature verify")?;
    let now = Asn1Time::days_from_now(0).context("build current ASN.1 time")?;
    if certificate
        .not_after()
        .compare(now.as_ref())
        .context("compare certificate expiry")?
        .is_lt()
    {
        bail!("certificate has expired");
    }
    if certificate
        .not_before()
        .compare(now.as_ref())
        .context("compare certificate not_before")?
        .is_gt()
    {
        bail!("certificate is not yet valid");
    }
    let public_key = certificate
        .public_key()
        .context("extract certificate public key")?;
    let signature = STANDARD
        .decode(signature_base64)
        .with_context(|| format!("decode {signature_field}"))?;
    let mut verifier =
        Verifier::new(MessageDigest::sha256(), &public_key).context("create signature verifier")?;
    verifier
        .update(message)
        .context("update signature verifier")?;
    let verified = verifier.verify(&signature).context("verify signature")?;
    if !verified {
        bail!("invalid cert gateway signature");
    }
    Ok(())
}

pub fn sign_message_base64(bundle: &CertificateBundle, message: &[u8]) -> Result<String> {
    ensure_openssl_legacy_provider()?;
    let private_key =
        load_private_key_material(&bundle.sign_pri_key, &bundle.password, "signPri.key")?
            .private_key;
    let mut signer =
        Signer::new(MessageDigest::sha256(), &private_key).context("create message signer")?;
    signer.update(message).context("update message signer")?;
    Ok(STANDARD.encode(signer.sign_to_vec().context("sign message")?))
}

pub fn validate_certificate_bundle(
    bundle: &CertificateBundle,
    allow_untrusted: bool,
    certificate_trust_bundle_path: Option<&Path>,
) -> Result<()> {
    validate_certificate_bundle_with_trust_bundle_path(
        bundle,
        allow_untrusted,
        certificate_trust_bundle_path,
    )
}

pub async fn validate_certificate_bundle_async(
    bundle: &CertificateBundle,
    allow_untrusted: bool,
    certificate_trust_bundle_path: Option<&Path>,
) -> Result<()> {
    let bundle = bundle.clone();
    let certificate_trust_bundle_path = certificate_trust_bundle_path.map(Path::to_path_buf);
    tokio::task::spawn_blocking(move || {
        validate_certificate_bundle_with_trust_bundle_path(
            &bundle,
            allow_untrusted,
            certificate_trust_bundle_path.as_deref(),
        )
    })
    .await
    .context("join certificate validation task")?
}

fn validate_certificate_bundle_with_trust_bundle_path(
    bundle: &CertificateBundle,
    allow_untrusted: bool,
    certificate_trust_bundle_path: Option<&Path>,
) -> Result<()> {
    validate_key_pair(
        &bundle.sign_cert_der,
        &bundle.sign_pri_key,
        &bundle.password,
        "signCert.der",
        "signPri.key",
    )?;
    match (&bundle.km_cert_der, &bundle.km_pri_key) {
        (Some(km_cert_der), Some(km_pri_key)) => validate_key_pair(
            km_cert_der,
            km_pri_key,
            &bundle.password,
            "kmCert.der",
            "kmPri.key",
        )?,
        (None, None) => {}
        _ => bail!("kmCert.der and kmPri.key must both be provided or both omitted"),
    }
    if !allow_untrusted {
        let explicit_store = certificate_trust_bundle_path
            .map(configured_trust_store)
            .transpose()?;
        let store = match explicit_store.as_ref() {
            Some(store) => store.as_ref().as_ref(),
            None => default_trust_store()?,
        };
        validate_certificate_trust_with_store(
            &bundle.sign_cert_der,
            "signCert.der",
            &bundle.certificate_chain_der,
            store,
        )?;
        if let Some(km_cert_der) = &bundle.km_cert_der {
            validate_certificate_trust_with_store(
                km_cert_der,
                "kmCert.der",
                &bundle.certificate_chain_der,
                store,
            )?;
        }
    }
    Ok(())
}

fn parse_certificate(input: &[u8]) -> Result<X509> {
    X509::from_pem(input)
        .or_else(|_| X509::from_der(input))
        .context("parse recipient certificate as PEM/DER")
}

fn ensure_openssl_legacy_provider() -> Result<()> {
    static INIT: OnceLock<Result<(), String>> = OnceLock::new();
    match INIT.get_or_init(|| {
        let default = Provider::load(None, "default")
            .map_err(|err| format!("load OpenSSL default provider: {err}"))?;
        let legacy = Provider::load(None, "legacy")
            .map_err(|err| format!("load OpenSSL legacy provider: {err}"))?;
        std::mem::forget(default);
        std::mem::forget(legacy);
        Ok(())
    }) {
        Ok(()) => Ok(()),
        Err(err) => Err(anyhow!(err.clone())),
    }
}

struct EncryptedPrivateKey {
    algorithm: OID,
    salt: Vec<u8>,
    iteration: usize,
    cipher_text: Vec<u8>,
}

struct SensitiveBytes(Vec<u8>);

impl SensitiveBytes {
    fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    fn as_slice(&self) -> &[u8] {
        &self.0
    }
}

impl Drop for SensitiveBytes {
    fn drop(&mut self) {
        for byte in &mut self.0 {
            // Use volatile writes so the compiler cannot elide clearing
            // sensitive buffers that are about to be dropped.
            unsafe { std::ptr::write_volatile(byte as *mut u8, 0) };
        }
        std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
    }
}

fn parse_encrypted_private_key(der: &[u8]) -> Result<EncryptedPrivateKey> {
    let blocks = from_der(der).context("parse signPri.key ASN.1")?;
    let sequence = expect_single_sequence(&blocks, "encrypted private key")?;
    if sequence.len() != 2 {
        bail!("encrypted private key must contain 2 fields");
    }

    let meta = expect_sequence(&sequence[0], "encrypted private key meta")?;
    if meta.len() != 2 {
        bail!("encrypted private key meta must contain algorithm and params");
    }
    let algorithm = expect_oid(&meta[0], "encrypted private key algorithm")?;
    let params = expect_sequence(&meta[1], "encrypted private key params")?;
    if params.len() != 2 {
        bail!("encrypted private key params must contain salt and iteration");
    }
    let salt = expect_octet_string(&params[0], "encrypted private key salt")?;
    let iteration = expect_integer_usize(&params[1], "encrypted private key iteration")?;
    let cipher_text = expect_octet_string(&sequence[1], "encrypted private key ciphertext")?;

    Ok(EncryptedPrivateKey {
        algorithm,
        salt,
        iteration,
        cipher_text,
    })
}

fn load_signing_key(sign_pri_key: &[u8], password: &str) -> Result<(PKey<Private>, String)> {
    let loaded = load_private_key_material(sign_pri_key, password, "signPri.key")?;
    Ok((
        loaded.private_key,
        loaded
            .vid_random
            .map(|vid_random| STANDARD.encode(vid_random))
            .unwrap_or_default(),
    ))
}

fn parse_private_key(plain_der: &[u8]) -> Result<PKey<Private>> {
    let blocks = from_der(plain_der).context("parse decrypted signPri.key ASN.1")?;
    let sequence = expect_single_sequence(&blocks, "plain private key")?;
    if sequence.len() < 3 {
        bail!("plain private key must contain at least 3 fields");
    }
    let private_key_der = expect_octet_string(&sequence[2], "plain private key octet")?;
    let rsa = Rsa::private_key_from_der(&private_key_der).context("parse PKCS#1 private key")?;
    PKey::from_rsa(rsa).context("wrap RSA private key")
}

fn parse_direct_private_key(der: &[u8]) -> Result<PKey<Private>> {
    let private_key = PKey::private_key_from_der(der)
        .or_else(|_| {
            let rsa = Rsa::private_key_from_der(der)?;
            PKey::from_rsa(rsa)
        })
        .context("parse direct private key as PKCS#8/PKCS#1 DER")?;
    match private_key.id() {
        Id::RSA | Id::EC => Ok(private_key),
        other => bail!("unsupported direct private key algorithm: {other:?}"),
    }
}

fn validate_key_pair(
    cert_der: &[u8],
    private_key_der: &[u8],
    password: &str,
    cert_name: &str,
    key_name: &str,
) -> Result<()> {
    ensure_openssl_legacy_provider()?;
    let certificate = X509::from_der(cert_der).with_context(|| format!("parse {cert_name}"))?;
    let certificate_public_key = certificate
        .public_key()
        .with_context(|| format!("extract public key from {cert_name}"))?;
    let private_key = load_private_key_material(private_key_der, password, key_name)?.private_key;
    if !private_key.public_eq(&certificate_public_key) {
        bail!("{key_name} does not match {cert_name}");
    }
    Ok(())
}

fn default_trust_store() -> Result<&'static openssl::x509::store::X509Store> {
    static STORE: OnceLock<Result<openssl::x509::store::X509Store, String>> = OnceLock::new();
    match STORE.get_or_init(|| {
        let mut store_builder =
            X509StoreBuilder::new().map_err(|err| format!("create X509 trust store: {err:#}"))?;
        store_builder
            .set_default_paths()
            .map_err(|err| format!("load default X509 trust paths: {err:#}"))?;
        Ok(store_builder.build())
    }) {
        Ok(store) => Ok(store),
        Err(message) => bail!("{message}"),
    }
}

fn trust_store_from_ca_bundle_pem(
    path: &Path,
    pem: &[u8],
) -> Result<openssl::x509::store::X509Store> {
    let certs = X509::stack_from_pem(pem)
        .with_context(|| format!("parse X509 trust bundle {}", path.display()))?;
    if certs.is_empty() {
        bail!(
            "X509 trust bundle {} contains no certificates",
            path.display()
        );
    }
    let mut store_builder =
        X509StoreBuilder::new().context("create configured X509 trust store")?;
    for cert in certs {
        store_builder.add_cert(cert).with_context(|| {
            format!("add certificate from X509 trust bundle {}", path.display())
        })?;
    }
    Ok(store_builder.build())
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct TrustStoreFileState {
    len: u64,
    modified: Option<SystemTime>,
}

struct CachedTrustStore {
    file_state: TrustStoreFileState,
    store: Arc<openssl::x509::store::X509Store>,
}

fn configured_trust_store(path: &Path) -> Result<Arc<openssl::x509::store::X509Store>> {
    static TRUST_STORES: OnceLock<Mutex<HashMap<PathBuf, CachedTrustStore>>> = OnceLock::new();

    let cache_key = path.to_path_buf();
    let metadata = std::fs::metadata(path)
        .with_context(|| format!("stat X509 trust bundle {}", path.display()))?;
    let file_state = TrustStoreFileState {
        len: metadata.len(),
        modified: metadata.modified().ok(),
    };
    let cache = TRUST_STORES.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(store) = cache
        .lock()
        .map_err(|_| anyhow!("X509 trust store cache lock poisoned"))?
        .get(&cache_key)
        .filter(|cached| cached.file_state == file_state)
        .map(|cached| cached.store.clone())
    {
        return Ok(store);
    }

    let pem = std::fs::read(path)
        .with_context(|| format!("read X509 trust bundle {}", path.display()))?;
    let store = Arc::new(trust_store_from_ca_bundle_pem(path, &pem)?);
    cache
        .lock()
        .map_err(|_| anyhow!("X509 trust store cache lock poisoned"))?
        .insert(
            cache_key,
            CachedTrustStore {
                file_state,
                store: store.clone(),
            },
        );
    Ok(store)
}

fn validate_certificate_trust_with_store(
    cert_der: &[u8],
    cert_name: &str,
    certificate_chain_der: &[Vec<u8>],
    store: &X509StoreRef,
) -> Result<()> {
    let certificate = X509::from_der(cert_der).with_context(|| format!("parse {cert_name}"))?;
    let mut chain = Stack::new().context("create certificate chain")?;
    for (idx, chain_cert_der) in certificate_chain_der.iter().enumerate() {
        chain
            .push(
                X509::from_der(chain_cert_der)
                    .with_context(|| format!("parse certificateChainDer[{idx}]"))?,
            )
            .with_context(|| format!("add certificateChainDer[{idx}] to chain"))?;
    }
    let mut context = X509StoreContext::new().context("create X509 store context")?;
    let trusted = context
        .init(&store, &certificate, &chain, |ctx| ctx.verify_cert())
        .with_context(|| format!("verify {cert_name} trust chain"))?;
    if !trusted {
        bail!("{cert_name} is not trusted: {}", context.error());
    }
    Ok(())
}

struct LoadedPrivateKey {
    private_key: PKey<Private>,
    vid_random: Option<Vec<u8>>,
}

fn load_private_key_material(
    private_key_der: &[u8],
    password: &str,
    key_name: &str,
) -> Result<LoadedPrivateKey> {
    if let Ok(encrypted) = parse_encrypted_private_key(private_key_der) {
        let (key, iv) = derive_npki_decryption_key(
            &encrypted.algorithm,
            &encrypted.salt,
            encrypted.iteration,
            password.as_bytes(),
        )?;
        let key = SensitiveBytes::new(key);
        let iv = SensitiveBytes::new(iv);
        let plain_der = SensitiveBytes::new(
            seed_cbc_decrypt(key.as_slice(), iv.as_slice(), &encrypted.cipher_text)
                .with_context(|| format!("decrypt {key_name}"))?,
        );
        let private_key =
            parse_private_key(plain_der.as_slice()).with_context(|| format!("parse {key_name}"))?;
        let vid_random = extract_vid_random(plain_der.as_slice()).ok();
        return Ok(LoadedPrivateKey {
            private_key,
            vid_random,
        });
    }

    let private_key =
        parse_direct_private_key(private_key_der).with_context(|| format!("parse {key_name}"))?;
    Ok(LoadedPrivateKey {
        private_key,
        vid_random: None,
    })
}

fn extract_vid_random(plain_der: &[u8]) -> Result<Vec<u8>> {
    let blocks = from_der(plain_der).context("parse plain private key for vidRandom")?;
    let sequence = expect_single_sequence(&blocks, "plain private key")?;
    let Some(rand_block) = sequence.get(3) else {
        bail!("vidRandom field missing");
    };

    let ASN1Block::Explicit(class, _, tag, inner) = rand_block else {
        bail!("vidRandom field is not an explicit ASN.1 block");
    };
    if *class != ASN1Class::ContextSpecific || biguint_to_u64(tag)? != 0 {
        bail!("vidRandom field does not use context-specific tag 0");
    }

    find_first_bit_string(inner).ok_or_else(|| anyhow!("decoded vidRandom bit string is empty"))
}

fn find_first_bit_string(block: &ASN1Block) -> Option<Vec<u8>> {
    match block {
        ASN1Block::BitString(_, _, bytes) if !bytes.is_empty() => Some(bytes.clone()),
        ASN1Block::Sequence(_, items) | ASN1Block::Set(_, items) => {
            items.iter().find_map(find_first_bit_string)
        }
        ASN1Block::Explicit(_, _, _, inner) => find_first_bit_string(inner),
        _ => None,
    }
}

fn derive_npki_decryption_key(
    algorithm: &OID,
    salt: &[u8],
    iteration: usize,
    password: &[u8],
) -> Result<(Vec<u8>, Vec<u8>)> {
    let digest = pbkdf1_sha1(password, salt, iteration, 20)?;
    let key = digest[..16].to_vec();

    if oid_matches(algorithm, OID_NPKI_SEED_CBC_WITH_SHA1) {
        let mut hasher = Sha1::new();
        hasher.update(&digest[16..20]);
        let sum = hasher.finalize();
        Ok((key, sum[..16].to_vec()))
    } else if oid_matches(algorithm, OID_NPKI_SEED_CBC) {
        Ok((key, b"0123456789012345".to_vec()))
    } else {
        bail!("unsupported NPKI private key algorithm")
    }
}

fn pbkdf1_sha1(password: &[u8], salt: &[u8], iterations: usize, key_len: usize) -> Result<Vec<u8>> {
    if salt.len() != 8 {
        bail!("pbkdf1 salt must be 8 bytes, got {}", salt.len());
    }
    if key_len > 20 {
        bail!("pbkdf1 key length {} exceeds SHA-1 output", key_len);
    }
    if iterations == 0 {
        bail!("pbkdf1 iterations must be positive");
    }

    let mut hasher = Sha1::new();
    hasher.update(password);
    hasher.update(salt);
    let mut digest = hasher.finalize().to_vec();
    for _ in 1..iterations {
        let mut hasher = Sha1::new();
        hasher.update(&digest);
        digest = hasher.finalize().to_vec();
    }
    Ok(digest[..key_len].to_vec())
}

fn seed_cbc_decrypt(key: &[u8], iv: &[u8], cipher_text: &[u8]) -> Result<Vec<u8>> {
    let cipher = Cipher::seed_cbc();
    if cipher_text.len() % cipher.block_size() != 0 {
        bail!(
            "ciphertext length {} is not a multiple of block size {}",
            cipher_text.len(),
            cipher.block_size()
        );
    }

    let mut crypter = Crypter::new(cipher, Mode::Decrypt, key, Some(iv))
        .context("initialize SEED-CBC crypter")?;
    crypter.pad(false);

    let mut out = vec![0u8; cipher_text.len() + cipher.block_size()];
    let mut written = crypter
        .update(cipher_text, &mut out)
        .context("decrypt signPri.key ciphertext")?;
    written += crypter
        .finalize(&mut out[written..])
        .context("finalize signPri.key decrypt")?;
    out.truncate(written);
    unpad_pkcs7(&out, cipher.block_size())
}

fn unpad_pkcs7(plain: &[u8], block_size: usize) -> Result<Vec<u8>> {
    if plain.is_empty() || plain.len() % block_size != 0 {
        bail!("invalid PKCS#7 padded data length {}", plain.len());
    }

    let padding = *plain.last().expect("non-empty") as usize;
    if padding == 0 || padding > block_size || padding > plain.len() {
        bail!("invalid PKCS#7 padding size {}", padding);
    }
    if plain[plain.len() - padding..]
        .iter()
        .any(|value| *value as usize != padding)
    {
        bail!("invalid PKCS#7 padding");
    }
    Ok(plain[..plain.len() - padding].to_vec())
}

fn oid_matches(oid: &OID, expected: &[u64]) -> bool {
    oid.as_vec::<u64>()
        .map(|values| values == expected)
        .unwrap_or(false)
}

fn expect_single_sequence<'a>(blocks: &'a [ASN1Block], label: &str) -> Result<&'a [ASN1Block]> {
    if blocks.len() != 1 {
        bail!("{label} must contain exactly one top-level ASN.1 block");
    }
    expect_sequence(&blocks[0], label)
}

fn expect_sequence<'a>(block: &'a ASN1Block, label: &str) -> Result<&'a [ASN1Block]> {
    match block {
        ASN1Block::Sequence(_, items) => Ok(items),
        _ => bail!("{label} is not an ASN.1 sequence"),
    }
}

fn expect_octet_string(block: &ASN1Block, label: &str) -> Result<Vec<u8>> {
    match block {
        ASN1Block::OctetString(_, bytes) => Ok(bytes.clone()),
        _ => bail!("{label} is not an ASN.1 octet string"),
    }
}

fn expect_oid(block: &ASN1Block, label: &str) -> Result<OID> {
    match block {
        ASN1Block::ObjectIdentifier(_, oid) => Ok(oid.clone()),
        _ => bail!("{label} is not an ASN.1 object identifier"),
    }
}

fn expect_integer_usize(block: &ASN1Block, label: &str) -> Result<usize> {
    match block {
        ASN1Block::Integer(_, value) => value
            .try_into()
            .map_err(|_| anyhow!("{label} does not fit into usize")),
        _ => bail!("{label} is not an ASN.1 integer"),
    }
}

fn biguint_to_u64(value: &BigUint) -> Result<u64> {
    value
        .try_into()
        .map_err(|_| anyhow!("ASN.1 tag does not fit into u64"))
}

#[cfg(test)]
mod tests {
    use super::{validate_certificate_trust_with_store, verify_challenge_signature};
    use base64::{Engine, engine::general_purpose::STANDARD};
    use openssl::{
        asn1::Asn1Time,
        bn::BigNum,
        hash::MessageDigest,
        pkey::{PKey, Private},
        rsa::Rsa,
        sign::Signer,
        x509::{
            X509, X509Builder, X509NameBuilder,
            extension::{BasicConstraints, KeyUsage},
            store::X509StoreBuilder,
        },
    };

    #[test]
    fn verify_challenge_signature_rejects_certificate_before_not_before() {
        let key = PKey::from_rsa(Rsa::generate(2048).expect("generate rsa key")).expect("wrap key");
        let cert_der = create_sign_cert_der_with_validity(&key, 1, 365);
        let payload = "cert-gateway:test:stored::nonce:2026-04-13T00:00:00Z";
        let signature = sign_challenge(&key, payload);

        let err = verify_challenge_signature(&cert_der, payload, &signature)
            .expect_err("future certificate should be rejected");
        assert!(err.to_string().contains("not yet valid"));
    }

    #[test]
    fn validate_certificate_trust_accepts_leaf_with_intermediate_chain() {
        let root_key = generate_key();
        let intermediate_key = generate_key();
        let leaf_key = generate_key();
        let root = create_ca_certificate("Root Test CA", &root_key, None, None, 1);
        let intermediate = create_ca_certificate(
            "Intermediate Test CA",
            &intermediate_key,
            Some(&root),
            Some(&root_key),
            2,
        );
        let leaf = create_leaf_certificate(
            "Leaf Test Certificate",
            &leaf_key,
            &intermediate,
            &intermediate_key,
            3,
        );
        let mut store_builder = X509StoreBuilder::new().expect("store builder");
        store_builder.add_cert(root).expect("add root");
        let store = store_builder.build();

        validate_certificate_trust_with_store(
            &leaf.to_der().expect("leaf der"),
            "signCert.der",
            &[intermediate.to_der().expect("intermediate der")],
            &store,
        )
        .expect("leaf should validate through supplied intermediate");
    }

    fn create_sign_cert_der_with_validity(
        key: &PKey<Private>,
        not_before_days_from_now: u32,
        not_after_days_from_now: u32,
    ) -> Vec<u8> {
        let mut name = X509NameBuilder::new().expect("name builder");
        name.append_entry_by_text("CN", "Future Certificate")
            .expect("append cn");
        let name = name.build();

        let mut builder = X509Builder::new().expect("x509 builder");
        builder.set_version(2).expect("set version");
        builder.set_subject_name(&name).expect("set subject");
        builder.set_issuer_name(&name).expect("set issuer");
        builder.set_pubkey(key).expect("set pubkey");

        let not_before = Asn1Time::days_from_now(not_before_days_from_now).expect("not before");
        let not_after = Asn1Time::days_from_now(not_after_days_from_now).expect("not after");
        builder
            .set_not_before(not_before.as_ref())
            .expect("set not before");
        builder
            .set_not_after(not_after.as_ref())
            .expect("set not after");
        builder
            .sign(key, MessageDigest::sha256())
            .expect("sign cert");
        builder.build().to_der().expect("cert der")
    }

    fn generate_key() -> PKey<Private> {
        PKey::from_rsa(Rsa::generate(2048).expect("generate rsa key")).expect("wrap key")
    }

    fn create_ca_certificate(
        common_name: &str,
        key: &PKey<Private>,
        issuer_cert: Option<&X509>,
        issuer_key: Option<&PKey<Private>>,
        serial: u32,
    ) -> X509 {
        let name = certificate_name(common_name);
        let signing_key = issuer_key.unwrap_or(key);
        let mut builder = base_certificate_builder(serial, key);
        builder.set_subject_name(&name).expect("set subject");
        builder
            .set_issuer_name(
                issuer_cert
                    .map(|cert| cert.subject_name())
                    .unwrap_or_else(|| name.as_ref()),
            )
            .expect("set issuer");
        builder
            .append_extension(
                BasicConstraints::new()
                    .critical()
                    .ca()
                    .build()
                    .expect("ca basic constraints"),
            )
            .expect("append ca basic constraints");
        builder
            .append_extension(
                KeyUsage::new()
                    .critical()
                    .key_cert_sign()
                    .crl_sign()
                    .build()
                    .expect("ca key usage"),
            )
            .expect("append ca key usage");
        builder
            .sign(signing_key, MessageDigest::sha256())
            .expect("sign ca cert");
        builder.build()
    }

    fn create_leaf_certificate(
        common_name: &str,
        key: &PKey<Private>,
        issuer_cert: &X509,
        issuer_key: &PKey<Private>,
        serial: u32,
    ) -> X509 {
        let name = certificate_name(common_name);
        let mut builder = base_certificate_builder(serial, key);
        builder.set_subject_name(&name).expect("set subject");
        builder
            .set_issuer_name(issuer_cert.subject_name())
            .expect("set issuer");
        builder
            .append_extension(
                BasicConstraints::new()
                    .critical()
                    .build()
                    .expect("leaf basic constraints"),
            )
            .expect("append leaf basic constraints");
        builder
            .append_extension(
                KeyUsage::new()
                    .critical()
                    .digital_signature()
                    .build()
                    .expect("leaf key usage"),
            )
            .expect("append leaf key usage");
        builder
            .sign(issuer_key, MessageDigest::sha256())
            .expect("sign leaf cert");
        builder.build()
    }

    fn base_certificate_builder(serial: u32, key: &PKey<Private>) -> X509Builder {
        let mut builder = X509Builder::new().expect("x509 builder");
        builder.set_version(2).expect("set version");
        let serial = BigNum::from_u32(serial)
            .expect("serial bignum")
            .to_asn1_integer()
            .expect("serial asn1");
        builder.set_serial_number(&serial).expect("set serial");
        builder.set_pubkey(key).expect("set pubkey");
        let not_before = Asn1Time::days_from_now(0).expect("not before");
        let not_after = Asn1Time::days_from_now(365).expect("not after");
        builder
            .set_not_before(not_before.as_ref())
            .expect("set not before");
        builder
            .set_not_after(not_after.as_ref())
            .expect("set not after");
        builder
    }

    fn certificate_name(common_name: &str) -> openssl::x509::X509Name {
        let mut name = X509NameBuilder::new().expect("name builder");
        name.append_entry_by_text("CN", common_name)
            .expect("append cn");
        name.build()
    }

    fn sign_challenge(key: &PKey<Private>, challenge_payload: &str) -> String {
        let mut signer = Signer::new(MessageDigest::sha256(), key).expect("create signer");
        signer
            .update(challenge_payload.as_bytes())
            .expect("sign payload");
        STANDARD.encode(signer.sign_to_vec().expect("signature bytes"))
    }
}

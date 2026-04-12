use anyhow::{Context, Result, anyhow, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use openssl::{
    asn1::Asn1Time,
    hash::MessageDigest,
    nid::Nid,
    pkcs7::{Pkcs7, Pkcs7Flags},
    pkey::{PKey, Private},
    provider::Provider,
    rsa::Rsa,
    sign::Verifier,
    stack::Stack,
    symm::{Cipher, Crypter, Mode},
    x509::X509,
};
use sha1::{Digest, Sha1};
use simple_asn1::{ASN1Block, ASN1Class, BigUint, OID, from_der};
use std::{path::Path, sync::OnceLock};

const OID_NPKI_SEED_CBC: &[u64] = &[1, 2, 410, 200004, 1, 4];
const OID_NPKI_SEED_CBC_WITH_SHA1: &[u64] = &[1, 2, 410, 200004, 1, 15];

#[derive(Debug, Clone)]
pub struct NpkiBundle {
    pub sign_cert_der: Vec<u8>,
    pub sign_pri_key: Vec<u8>,
    pub km_cert_der: Vec<u8>,
    pub km_pri_key: Vec<u8>,
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
            km_cert_der: std::fs::read(path.join("kmCert.der"))
                .with_context(|| format!("read {}", path.join("kmCert.der").display()))?,
            km_pri_key: std::fs::read(path.join("kmPri.key"))
                .with_context(|| format!("read {}", path.join("kmPri.key").display()))?,
        })
    }
}

impl NpkiSigner {
    pub fn new(bundle: &NpkiBundle, password: &str) -> Result<Self> {
        ensure_openssl_legacy_provider()?;
        let certificate =
            X509::from_der(&bundle.sign_cert_der).context("parse signCert.der as X509")?;

        let encrypted = parse_encrypted_private_key(&bundle.sign_pri_key)?;
        let (key, iv) = derive_npki_decryption_key(
            &encrypted.algorithm,
            &encrypted.salt,
            encrypted.iteration,
            password.as_bytes(),
        )?;
        let plain_der = seed_cbc_decrypt(&key, &iv, &encrypted.cipher_text)?;
        let private_key = parse_private_key(&plain_der)?;
        let vid_random = extract_vid_random(&plain_der)?;

        Ok(Self {
            certificate,
            private_key,
            vid_random_b64: STANDARD.encode(vid_random),
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
    ensure_openssl_legacy_provider()?;
    let certificate =
        X509::from_der(sign_cert_der).context("parse signCert.der as X509 for challenge verify")?;
    let now = Asn1Time::days_from_now(0).context("build current ASN.1 time")?;
    if certificate
        .not_after()
        .compare(now.as_ref())
        .context("compare certificate expiry")?
        .is_lt()
    {
        bail!("certificate has expired");
    }
    let public_key = certificate
        .public_key()
        .context("extract certificate public key")?;
    let signature = STANDARD
        .decode(challenge_signature_base64)
        .context("decode challengeSignatureBase64")?;
    let mut verifier = Verifier::new(MessageDigest::sha256(), &public_key)
        .context("create challenge signature verifier")?;
    verifier
        .update(challenge_payload.as_bytes())
        .context("update challenge signature verifier")?;
    let verified = verifier
        .verify(&signature)
        .context("verify challenge signature")?;
    if !verified {
        bail!("invalid cert gateway challenge signature");
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

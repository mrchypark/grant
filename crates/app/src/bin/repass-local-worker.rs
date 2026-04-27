use anyhow::{Context, Result, anyhow, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use infra::{
    bridge::{self, BridgeOptions, BridgeSnapshot},
    crypto::{CertificateBundle, normalize_business_no},
    npki::sign_message_base64,
    service::local_execution_result_signature_payload,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

const MAX_DOCUMENT_BYTES: u64 = 16 * 1024 * 1024;
const MAX_TOTAL_DOCUMENT_BYTES: u64 = 32 * 1024 * 1024;
const MAX_STDIN_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LocalEphemeralRequest {
    gateway_base_url: String,
    organization_id: String,
    business_no: Option<String>,
    password: String,
    bearer_token: Option<String>,
    active_organization_id: Option<String>,
    challenge_id: String,
    challenge_payload: String,
    challenge_signature_base64: String,
    facility_code: Option<String>,
    start_month: Option<String>,
    end_month: Option<String>,
    mode: Option<String>,
    bundle: ProofBundle,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProofBundle {
    business_no: String,
    common_name: String,
    sign_cert_der_base64: String,
    sign_pri_key_base64: String,
    km_cert_der_base64: String,
    km_pri_key_base64: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LocalExecutionResultRequest {
    challenge_id: String,
    challenge_payload: String,
    challenge_signature_base64: String,
    result_signature_base64: String,
    sign_cert_der_base64: String,
    snapshot: BridgeSnapshot,
    #[serde(skip_serializing_if = "Option::is_none")]
    mode: Option<String>,
}

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        let _ = writeln!(std::io::stderr(), "{err:#}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let raw = read_limited_stdin(std::io::stdin())?;
    let req: LocalEphemeralRequest =
        serde_json::from_str(&raw).context("decode local execution request")?;
    normalize_gateway_base_url(&req.gateway_base_url)?;
    let bundle = certificate_bundle(&req)?;
    let business_no = local_business_no(&req)?;

    let run = bridge::collect_snapshot(BridgeOptions {
        business_no,
        facility_code: req.facility_code.clone(),
        start_month: req.start_month.clone(),
        end_month: req.end_month.clone(),
        skip_documents: !should_collect_documents(req.mode.as_deref()),
        bundle: bundle.clone(),
    })
    .await
    .context("collect local RPS snapshot")?;

    let document_root = run.temp_dir.path().to_path_buf();
    let mut snapshot = run.snapshot;
    if should_collect_documents(req.mode.as_deref()) {
        embed_document_contents(&mut snapshot, &document_root)?;
    } else {
        snapshot.documents.clear();
    }
    let result_signature_base64 = sign_local_result(&req, &bundle, &snapshot)?;
    let result = post_local_result(
        &req,
        local_result_body(&req, snapshot, result_signature_base64),
    )
    .await
    .context("upload local execution result")?;
    std::io::stdout()
        .write_all(result.as_bytes())
        .context("write local execution response")?;
    Ok(())
}

fn read_limited_stdin<R: Read>(reader: R) -> Result<String> {
    let mut raw = String::new();
    let mut limited = reader.take(MAX_STDIN_BYTES + 1);
    limited
        .read_to_string(&mut raw)
        .context("read local execution request from stdin")?;
    if raw.len() as u64 > MAX_STDIN_BYTES {
        bail!("local execution request exceeds {} bytes", MAX_STDIN_BYTES);
    }
    Ok(raw)
}

fn should_collect_documents(mode: Option<&str>) -> bool {
    mode.map(str::trim)
        .map(|value| value.eq_ignore_ascii_case("full"))
        .unwrap_or(false)
}

fn local_business_no(req: &LocalEphemeralRequest) -> Result<String> {
    let business_no = req
        .business_no
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            let value = req.bundle.business_no.trim();
            (!value.is_empty()).then_some(value)
        })
        .ok_or_else(|| anyhow!("businessNo is required for local execution"))?;

    normalize_business_no(business_no).context("normalize local execution businessNo")
}

fn embed_document_contents(snapshot: &mut BridgeSnapshot, document_root: &Path) -> Result<()> {
    let mut total_bytes = 0_u64;
    let mut embedded_documents = Vec::with_capacity(snapshot.documents.len());
    for mut doc in std::mem::take(&mut snapshot.documents) {
        let source_path = doc.source_path.trim();
        if source_path.is_empty() {
            if doc.required {
                let err = anyhow!(
                    "required local document {} has no source path",
                    doc.file_name
                );
                return finish_with_document_error(snapshot, embedded_documents, doc, err);
            }
            continue;
        }
        let document_path = match resolve_local_document_path(document_root, source_path) {
            Ok(path) => path,
            Err(err) if !doc.required && is_not_found_error(&err) => {
                match is_missing_path_under_root(document_root, source_path) {
                    Ok(true) => continue,
                    Ok(false) => {
                        return finish_with_document_error(snapshot, embedded_documents, doc, err);
                    }
                    Err(err) => {
                        return finish_with_document_error(snapshot, embedded_documents, doc, err);
                    }
                }
            }
            Err(err) => return finish_with_document_error(snapshot, embedded_documents, doc, err),
        };
        let size = match std::fs::metadata(&document_path)
            .with_context(|| format!("stat local document {}", doc.source_path))
        {
            Ok(metadata) => metadata.len(),
            Err(err) if !doc.required && is_not_found_error(&err) => continue,
            Err(err) => return finish_with_document_error(snapshot, embedded_documents, doc, err),
        };
        if size > MAX_DOCUMENT_BYTES {
            let err = anyhow!(
                "local document {} exceeds {} bytes",
                doc.file_name,
                MAX_DOCUMENT_BYTES
            );
            return finish_with_document_error(snapshot, embedded_documents, doc, err);
        }
        let (content_base64, sha256) = match encode_document_base64_and_sha256(&document_path)
            .with_context(|| format!("read local document {}", doc.source_path))
        {
            Ok(encoded) => encoded,
            Err(err) if !doc.required && is_not_found_error(&err) => continue,
            Err(err) => return finish_with_document_error(snapshot, embedded_documents, doc, err),
        };
        total_bytes = total_bytes.saturating_add(size);
        if total_bytes > MAX_TOTAL_DOCUMENT_BYTES {
            return finish_with_document_error(
                snapshot,
                embedded_documents,
                doc,
                anyhow!(
                    "local execution documents exceed {} bytes",
                    MAX_TOTAL_DOCUMENT_BYTES
                ),
            );
        }
        if doc
            .source_sha256
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .is_empty()
        {
            doc.source_sha256 = Some(sha256.clone());
        }
        if doc
            .output_sha256
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .is_empty()
        {
            doc.output_sha256 = Some(sha256);
        }
        doc.content_base64 = Some(content_base64);
        doc.content_type = Some("application/pdf".to_owned());
        doc.source_path.clear();
        embedded_documents.push(doc);
    }
    snapshot.documents = embedded_documents;
    Ok(())
}

fn finish_with_document_error(
    snapshot: &mut BridgeSnapshot,
    mut embedded_documents: Vec<bridge::BridgeDocument>,
    doc: bridge::BridgeDocument,
    err: anyhow::Error,
) -> Result<()> {
    embedded_documents.push(doc);
    snapshot.documents = embedded_documents;
    Err(err)
}

fn encode_document_base64_and_sha256(path: &Path) -> Result<(String, String)> {
    let mut file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut encoded = Vec::new();
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    {
        let mut encoder = base64::write::EncoderWriter::new(&mut encoded, &STANDARD);
        loop {
            let read = file
                .read(&mut buffer)
                .with_context(|| format!("read {}", path.display()))?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
            encoder
                .write_all(&buffer[..read])
                .with_context(|| format!("encode {}", path.display()))?;
        }
        encoder
            .finish()
            .with_context(|| format!("finish base64 {}", path.display()))?;
    }
    let content_base64 =
        String::from_utf8(encoded).context("base64 encoder emitted non-utf8 output")?;
    Ok((content_base64, hex_lower(&hasher.finalize())))
}

fn resolve_local_document_path(document_root: &Path, source_path: &str) -> Result<PathBuf> {
    let root = std::fs::canonicalize(document_root)
        .with_context(|| format!("resolve local document root {}", document_root.display()))?;
    let path = std::fs::canonicalize(source_path).with_context(|| {
        format!("resolve local document {source_path}; file must exist under collector workspace")
    })?;
    if !path.starts_with(&root) {
        bail!("local document {source_path} is outside the collector workspace");
    }
    Ok(path)
}

fn is_not_found_error(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .downcast_ref::<io::Error>()
            .map(|io_err| io_err.kind() == io::ErrorKind::NotFound)
            .unwrap_or(false)
    })
}

fn is_missing_path_under_root(document_root: &Path, source_path: &str) -> Result<bool> {
    let root = std::fs::canonicalize(document_root)
        .with_context(|| format!("resolve local document root {}", document_root.display()))?;
    let path = Path::new(source_path);
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let parent = candidate.parent().ok_or_else(|| {
        anyhow!("local document {source_path} has no parent under collector workspace")
    })?;
    let parent = std::fs::canonicalize(parent).with_context(|| {
        format!("resolve local document parent {source_path}; parent must exist under collector workspace")
    })?;
    if !parent.starts_with(&root) {
        bail!("local document {source_path} is outside the collector workspace");
    }
    match std::fs::symlink_metadata(&candidate) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!("local document {source_path} is a dangling symlink")
        }
        Ok(_) => Ok(false),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(err) => {
            Err(err).with_context(|| format!("inspect local document candidate {source_path}"))
        }
    }
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

fn certificate_bundle(req: &LocalEphemeralRequest) -> Result<CertificateBundle> {
    Ok(CertificateBundle {
        business_no: req.bundle.business_no.clone(),
        password: req.password.clone(),
        common_name: req.bundle.common_name.clone(),
        sign_cert_der: decode_base64("bundle.signCertDerBase64", &req.bundle.sign_cert_der_base64)?,
        sign_pri_key: decode_base64("bundle.signPriKeyBase64", &req.bundle.sign_pri_key_base64)?,
        km_cert_der: decode_optional_base64(
            "bundle.kmCertDerBase64",
            &req.bundle.km_cert_der_base64,
        )?,
        km_pri_key: decode_optional_base64("bundle.kmPriKeyBase64", &req.bundle.km_pri_key_base64)?,
        certificate_chain_der: Vec::new(),
    })
}

fn decode_base64(field: &str, value: &str) -> Result<Vec<u8>> {
    STANDARD
        .decode(value.trim())
        .with_context(|| format!("decode {field}"))
}

fn decode_optional_base64(field: &str, value: &str) -> Result<Option<Vec<u8>>> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    decode_base64(field, value).map(Some)
}

async fn post_local_result(
    req: &LocalEphemeralRequest,
    body: LocalExecutionResultRequest,
) -> Result<String> {
    let url = local_result_url(req)?;
    let mut request = reqwest::Client::new().post(url).json(&body);
    if let Some(token) = req
        .bearer_token
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        request = request.bearer_auth(token);
    }
    if let Some(org_id) = req
        .active_organization_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        request = request.header("x-organization-id", org_id);
    }
    let response = request
        .send()
        .await
        .context("send local execution result")?;
    let status = response.status();
    let text = response.text().await.context("read gateway response")?;
    if !status.is_success() {
        bail!("gateway local execution result failed with status {status}: {text}");
    }
    Ok(text)
}

fn local_result_body(
    req: &LocalEphemeralRequest,
    snapshot: BridgeSnapshot,
    result_signature_base64: String,
) -> LocalExecutionResultRequest {
    LocalExecutionResultRequest {
        challenge_id: req.challenge_id.clone(),
        challenge_payload: req.challenge_payload.clone(),
        challenge_signature_base64: req.challenge_signature_base64.clone(),
        result_signature_base64,
        sign_cert_der_base64: req.bundle.sign_cert_der_base64.clone(),
        snapshot,
        mode: local_result_mode(req),
    }
}

fn local_result_mode(req: &LocalEphemeralRequest) -> Option<String> {
    req.mode
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
}

fn sign_local_result(
    req: &LocalEphemeralRequest,
    bundle: &CertificateBundle,
    snapshot: &BridgeSnapshot,
) -> Result<String> {
    let mode = local_result_mode(req).unwrap_or_else(|| "delta".to_owned());
    let payload = local_execution_result_signature_payload(
        &req.challenge_id,
        &req.challenge_payload,
        &mode,
        snapshot,
    )?;
    sign_message_base64(bundle, &payload).context("sign local execution result")
}

fn local_result_url(req: &LocalEphemeralRequest) -> Result<String> {
    let base_url = normalize_gateway_base_url(&req.gateway_base_url)?;
    Ok(format!(
        "{}/api/v1/organizations/{}/cert-gateway/local-execution-results",
        base_url,
        urlencoding::encode(&req.organization_id)
    ))
}

fn normalize_gateway_base_url(base_url: &str) -> Result<String> {
    let parsed = reqwest::Url::parse(base_url.trim()).context("parse gatewayBaseUrl")?;
    if parsed.username() != "" || parsed.password().is_some() {
        bail!("gatewayBaseUrl must not include userinfo");
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        bail!("gatewayBaseUrl must not include query or fragment");
    }
    match parsed.scheme() {
        "https" => {}
        "http" if is_loopback(parsed.host_str().unwrap_or_default()) => {}
        _ => bail!("gatewayBaseUrl must use https except for loopback http"),
    }
    let mut normalized = parsed.as_str().to_owned();
    if parsed.path() == "/" {
        normalized.pop();
    } else {
        while normalized.ends_with('/') {
            normalized.pop();
        }
    }
    Ok(normalized)
}

fn is_loopback(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, FixedOffset};
    use infra::bridge::{
        BridgeDocument, BridgeFacility, BridgeGenerationRecord, BridgeRateRecord, BridgeRecRecord,
    };

    #[test]
    fn local_result_body_excludes_certificate_secrets() {
        let req = fixture_request();
        let body = local_result_body(&req, fixture_snapshot(), "RESULT_SIGNATURE".to_owned());
        let json = serde_json::to_string(&body).expect("serialize result body");

        assert!(json.contains("PUBLIC_CERT"));
        for forbidden in [
            "secret-password",
            "PRIVATE_KEY",
            "KM_PRIVATE_KEY",
            "signPriKeyBase64",
            "kmPriKeyBase64",
            "\"bundle\"",
            "\"password\"",
        ] {
            assert!(
                !json.contains(forbidden),
                "local result body leaked {forbidden}: {json}"
            );
        }
    }

    #[test]
    fn local_result_body_omits_blank_mode_and_normalizes_nonblank_mode() {
        let mut req = fixture_request();
        req.mode = Some(" \t\n ".to_owned());
        let blank_body = local_result_body(&req, fixture_snapshot(), "RESULT_SIGNATURE".to_owned());
        assert_eq!(blank_body.mode, None);

        req.mode = Some(" FULL ".to_owned());
        let full_body = local_result_body(&req, fixture_snapshot(), "RESULT_SIGNATURE".to_owned());
        assert_eq!(full_body.mode.as_deref(), Some("full"));
    }

    #[test]
    fn read_limited_stdin_accepts_bounded_request() {
        let input = br#"{"ok":true}"#;
        let raw = read_limited_stdin(input.as_slice()).expect("bounded stdin");
        assert_eq!(raw, "{\"ok\":true}");
    }

    #[test]
    fn read_limited_stdin_rejects_oversized_request() {
        let input = vec![b'a'; (MAX_STDIN_BYTES + 1) as usize];
        let err = read_limited_stdin(input.as_slice()).expect_err("oversized stdin");
        assert!(err.to_string().contains("request exceeds"));
    }

    #[test]
    fn local_business_no_prefers_request_value_and_normalizes_digits() {
        let mut req = fixture_request();
        req.business_no = Some(" 276-86-03291 ".to_owned());
        req.bundle.business_no = "1111111111".to_owned();

        assert_eq!(local_business_no(&req).expect("businessNo"), "2768603291");
    }

    #[test]
    fn local_business_no_falls_back_to_bundle_business_no() {
        let mut req = fixture_request();
        req.business_no = Some("   ".to_owned());
        req.bundle.business_no = "276-86-03291".to_owned();

        assert_eq!(local_business_no(&req).expect("businessNo"), "2768603291");
    }

    #[test]
    fn local_business_no_rejects_missing_business_no_before_collecting() {
        let mut req = fixture_request();
        req.business_no = None;
        req.bundle.business_no.clear();

        let err = local_business_no(&req).expect_err("missing businessNo");
        assert!(err.to_string().contains("businessNo is required"));
    }

    #[test]
    fn local_business_no_rejects_malformed_business_no_before_collecting() {
        let mut req = fixture_request();
        req.business_no = None;
        req.bundle.business_no = "123".to_owned();

        let err = local_business_no(&req).expect_err("malformed businessNo");
        assert!(format!("{err:#}").contains("businessNo must normalize"));
    }

    #[test]
    fn local_result_url_normalizes_base_url_escapes_organization_and_rejects_untrusted_urls() {
        let mut req = fixture_request();
        req.gateway_base_url = "https://gateway.local/base/".to_owned();
        req.organization_id = "org/../evil".to_owned();
        assert_eq!(
            local_result_url(&req).expect("url"),
            "https://gateway.local/base/api/v1/organizations/org%2F..%2Fevil/cert-gateway/local-execution-results"
        );

        req.gateway_base_url = "https://gateway.local/base%2Ftenant/".to_owned();
        req.organization_id = "org-1".to_owned();
        assert_eq!(
            local_result_url(&req).expect("encoded base path url"),
            "https://gateway.local/base%2Ftenant/api/v1/organizations/org-1/cert-gateway/local-execution-results"
        );

        req.gateway_base_url = "http://gateway.local".to_owned();
        assert!(local_result_url(&req).is_err());

        req.gateway_base_url = "https://user:pass@gateway.local".to_owned();
        assert!(local_result_url(&req).is_err());

        req.gateway_base_url = "https://gateway.local?target=evil".to_owned();
        assert!(local_result_url(&req).is_err());

        req.gateway_base_url = "http://127.0.0.1:18080".to_owned();
        assert!(local_result_url(&req).is_ok());
    }

    #[test]
    fn embed_document_contents_replaces_local_paths_with_base64_payloads() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("permit.pdf");
        std::fs::write(&path, b"PDF-CONTENT").expect("write fixture document");

        let mut snapshot = fixture_snapshot();
        snapshot.documents.push(BridgeDocument {
            attachment_code: "ATT-1".to_owned(),
            file_class: "permit".to_owned(),
            file_sec_name: "permit".to_owned(),
            file_name: "permit.pdf".to_owned(),
            required: true,
            category: "permit".to_owned(),
            source_path: path.to_string_lossy().into_owned(),
            source_sha256: None,
            output_sha256: None,
            content_base64: None,
            content_type: None,
        });

        embed_document_contents(&mut snapshot, dir.path()).expect("embed document");
        let doc = &snapshot.documents[0];
        assert_eq!(doc.source_path, "");
        assert_eq!(doc.content_base64.as_deref(), Some("UERGLUNPTlRFTlQ="));
        assert_eq!(doc.content_type.as_deref(), Some("application/pdf"));
        assert!(doc.source_sha256.is_some());

        let json = serde_json::to_string(&local_result_body(
            &fixture_request(),
            snapshot,
            "RESULT_SIGNATURE".to_owned(),
        ))
        .expect("serialize result");
        assert!(!json.contains(path.to_string_lossy().as_ref()));
        assert!(json.contains("contentBase64"));
    }

    #[test]
    fn embed_document_contents_streams_multichunk_documents() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("permit.pdf");
        let bytes: Vec<u8> = (0..(70 * 1024)).map(|idx| (idx % 251) as u8).collect();
        std::fs::write(&path, &bytes).expect("write multichunk fixture document");
        let expected_sha256 = hex_lower(&Sha256::digest(&bytes));
        let expected_base64 = STANDARD.encode(&bytes);

        let mut snapshot = fixture_snapshot();
        snapshot.documents.push(BridgeDocument {
            attachment_code: "ATT-1".to_owned(),
            file_class: "permit".to_owned(),
            file_sec_name: "permit".to_owned(),
            file_name: "permit.pdf".to_owned(),
            required: true,
            category: "permit".to_owned(),
            source_path: path.to_string_lossy().into_owned(),
            source_sha256: Some(expected_sha256.clone()),
            output_sha256: None,
            content_base64: None,
            content_type: None,
        });

        embed_document_contents(&mut snapshot, dir.path()).expect("embed document");
        let doc = &snapshot.documents[0];
        assert_eq!(doc.source_path, "");
        assert_eq!(doc.source_sha256.as_deref(), Some(expected_sha256.as_str()));
        assert_eq!(doc.output_sha256.as_deref(), Some(expected_sha256.as_str()));
        assert_eq!(
            doc.content_base64.as_deref(),
            Some(expected_base64.as_str())
        );
    }

    #[test]
    fn embed_document_contents_rejects_oversized_documents_before_serialization() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("large.pdf");
        let file = std::fs::File::create(&path).expect("large fixture");
        file.set_len(MAX_DOCUMENT_BYTES + 1).expect("size fixture");

        let mut snapshot = fixture_snapshot();
        snapshot.documents.push(BridgeDocument {
            attachment_code: "ATT-LARGE".to_owned(),
            file_class: "permit".to_owned(),
            file_sec_name: "permit".to_owned(),
            file_name: "large.pdf".to_owned(),
            required: true,
            category: "permit".to_owned(),
            source_path: path.to_string_lossy().into_owned(),
            source_sha256: None,
            output_sha256: None,
            content_base64: None,
            content_type: None,
        });

        let err =
            embed_document_contents(&mut snapshot, dir.path()).expect_err("oversized document");
        assert!(err.to_string().contains("exceeds"));
        assert!(snapshot.documents[0].content_base64.is_none());
    }

    #[test]
    fn embed_document_contents_rejects_paths_outside_collector_workspace() {
        let collector_dir = tempfile::tempdir().expect("collector tempdir");
        let outside_dir = tempfile::tempdir().expect("outside tempdir");
        let outside_path = outside_dir.path().join("secret.pdf");
        std::fs::write(&outside_path, b"SECRET").expect("write outside fixture");

        let mut snapshot = fixture_snapshot();
        snapshot.documents.push(BridgeDocument {
            attachment_code: "ATT-OUTSIDE".to_owned(),
            file_class: "permit".to_owned(),
            file_sec_name: "permit".to_owned(),
            file_name: "secret.pdf".to_owned(),
            required: true,
            category: "permit".to_owned(),
            source_path: outside_path.to_string_lossy().into_owned(),
            source_sha256: None,
            output_sha256: None,
            content_base64: None,
            content_type: None,
        });

        let err = embed_document_contents(&mut snapshot, collector_dir.path())
            .expect_err("outside document path");
        assert!(err.to_string().contains("outside the collector workspace"));
        assert!(snapshot.documents[0].content_base64.is_none());
    }

    #[test]
    fn embed_document_contents_rejects_missing_optional_paths_outside_collector_workspace() {
        let collector_dir = tempfile::tempdir().expect("collector tempdir");
        let outside_dir = tempfile::tempdir().expect("outside tempdir");
        let outside_path = outside_dir.path().join("missing.pdf");

        let mut snapshot = fixture_snapshot();
        snapshot.documents.push(BridgeDocument {
            attachment_code: "ATT-OPTIONAL-OUTSIDE-MISSING".to_owned(),
            file_class: "permit".to_owned(),
            file_sec_name: "permit".to_owned(),
            file_name: "missing.pdf".to_owned(),
            required: false,
            category: "permit".to_owned(),
            source_path: outside_path.to_string_lossy().into_owned(),
            source_sha256: None,
            output_sha256: None,
            content_base64: None,
            content_type: None,
        });

        let err = embed_document_contents(&mut snapshot, collector_dir.path())
            .expect_err("missing outside optional document path");
        assert!(err.to_string().contains("outside the collector workspace"));
        assert!(snapshot.documents[0].content_base64.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn embed_document_contents_rejects_missing_optional_dangling_symlink() {
        let collector_dir = tempfile::tempdir().expect("collector tempdir");
        let outside_dir = tempfile::tempdir().expect("outside tempdir");
        let symlink_path = collector_dir.path().join("dangling.pdf");
        std::os::unix::fs::symlink(outside_dir.path().join("missing.pdf"), &symlink_path)
            .expect("dangling symlink fixture");

        let mut snapshot = fixture_snapshot();
        snapshot.documents.push(BridgeDocument {
            attachment_code: "ATT-OPTIONAL-DANGLING".to_owned(),
            file_class: "permit".to_owned(),
            file_sec_name: "permit".to_owned(),
            file_name: "dangling.pdf".to_owned(),
            required: false,
            category: "permit".to_owned(),
            source_path: symlink_path.to_string_lossy().into_owned(),
            source_sha256: None,
            output_sha256: None,
            content_base64: None,
            content_type: None,
        });

        let err = embed_document_contents(&mut snapshot, collector_dir.path())
            .expect_err("dangling optional symlink");
        assert!(err.to_string().contains("dangling symlink"));
        assert!(snapshot.documents[0].content_base64.is_none());
    }

    #[test]
    fn embed_document_contents_reports_missing_local_documents() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing_path = dir.path().join("missing.pdf");

        let mut snapshot = fixture_snapshot();
        snapshot.documents.push(BridgeDocument {
            attachment_code: "ATT-MISSING".to_owned(),
            file_class: "permit".to_owned(),
            file_sec_name: "permit".to_owned(),
            file_name: "missing.pdf".to_owned(),
            required: true,
            category: "permit".to_owned(),
            source_path: missing_path.to_string_lossy().into_owned(),
            source_sha256: None,
            output_sha256: None,
            content_base64: None,
            content_type: None,
        });

        let err =
            embed_document_contents(&mut snapshot, dir.path()).expect_err("missing document path");
        assert!(
            err.to_string()
                .contains("file must exist under collector workspace")
        );
        assert!(snapshot.documents[0].content_base64.is_none());
    }

    #[test]
    fn embed_document_contents_skips_missing_optional_local_documents() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing_path = dir.path().join("missing-optional.pdf");
        let disappearing_path = dir.path().join("disappearing-optional.pdf");

        let mut snapshot = fixture_snapshot();
        snapshot.documents.push(BridgeDocument {
            attachment_code: "ATT-OPTIONAL-MISSING".to_owned(),
            file_class: "permit".to_owned(),
            file_sec_name: "permit".to_owned(),
            file_name: "missing-optional.pdf".to_owned(),
            required: false,
            category: "permit".to_owned(),
            source_path: missing_path.to_string_lossy().into_owned(),
            source_sha256: None,
            output_sha256: None,
            content_base64: None,
            content_type: None,
        });
        snapshot.documents.push(BridgeDocument {
            attachment_code: "ATT-OPTIONAL-DISAPPEARING".to_owned(),
            file_class: "permit".to_owned(),
            file_sec_name: "permit".to_owned(),
            file_name: "disappearing-optional.pdf".to_owned(),
            required: false,
            category: "permit".to_owned(),
            source_path: disappearing_path.to_string_lossy().into_owned(),
            source_sha256: None,
            output_sha256: None,
            content_base64: None,
            content_type: None,
        });
        snapshot.documents.push(BridgeDocument {
            attachment_code: "ATT-OPTIONAL-BLANK".to_owned(),
            file_class: "permit".to_owned(),
            file_sec_name: "permit".to_owned(),
            file_name: "blank-optional.pdf".to_owned(),
            required: false,
            category: "permit".to_owned(),
            source_path: " \t\n ".to_owned(),
            source_sha256: None,
            output_sha256: None,
            content_base64: None,
            content_type: None,
        });

        embed_document_contents(&mut snapshot, dir.path()).expect("skip optional documents");
        assert!(snapshot.documents.is_empty());
    }

    #[test]
    fn embed_document_contents_rejects_oversized_document_sets_before_serialization() {
        let dir = tempfile::tempdir().expect("tempdir");
        let first_path = dir.path().join("first.pdf");
        let second_path = dir.path().join("second.pdf");
        let third_path = dir.path().join("third.pdf");
        let per_file_size = MAX_TOTAL_DOCUMENT_BYTES / 3 + 1;
        for path in [&first_path, &second_path, &third_path] {
            let file = std::fs::File::create(path).expect("document fixture");
            file.set_len(per_file_size).expect("document size fixture");
        }

        let mut snapshot = fixture_snapshot();
        snapshot.documents = vec![
            BridgeDocument {
                attachment_code: "ATT-1".to_owned(),
                file_class: "permit".to_owned(),
                file_sec_name: "permit".to_owned(),
                file_name: "first.pdf".to_owned(),
                required: true,
                category: "permit".to_owned(),
                source_path: first_path.to_string_lossy().into_owned(),
                source_sha256: None,
                output_sha256: None,
                content_base64: None,
                content_type: None,
            },
            BridgeDocument {
                attachment_code: "ATT-2".to_owned(),
                file_class: "permit".to_owned(),
                file_sec_name: "permit".to_owned(),
                file_name: "second.pdf".to_owned(),
                required: true,
                category: "permit".to_owned(),
                source_path: second_path.to_string_lossy().into_owned(),
                source_sha256: None,
                output_sha256: None,
                content_base64: None,
                content_type: None,
            },
            BridgeDocument {
                attachment_code: "ATT-3".to_owned(),
                file_class: "permit".to_owned(),
                file_sec_name: "permit".to_owned(),
                file_name: "third.pdf".to_owned(),
                required: true,
                category: "permit".to_owned(),
                source_path: third_path.to_string_lossy().into_owned(),
                source_sha256: None,
                output_sha256: None,
                content_base64: None,
                content_type: None,
            },
        ];

        let err =
            embed_document_contents(&mut snapshot, dir.path()).expect_err("oversized document set");
        assert!(err.to_string().contains("documents exceed"));
        assert!(snapshot.documents[2].content_base64.is_none());
    }

    fn fixture_request() -> LocalEphemeralRequest {
        LocalEphemeralRequest {
            gateway_base_url: "https://gateway.local".to_owned(),
            organization_id: "org-1".to_owned(),
            business_no: Some("2768603291".to_owned()),
            password: "secret-password".to_owned(),
            bearer_token: Some("bearer-token".to_owned()),
            active_organization_id: Some("org-1".to_owned()),
            challenge_id: "challenge-1".to_owned(),
            challenge_payload: "cert-gateway:test".to_owned(),
            challenge_signature_base64: "SIGNATURE".to_owned(),
            facility_code: Some("FAC-001".to_owned()),
            start_month: Some("2026-01".to_owned()),
            end_month: Some("2026-03".to_owned()),
            mode: Some("delta".to_owned()),
            bundle: ProofBundle {
                business_no: "2768603291".to_owned(),
                common_name: "Test Certificate".to_owned(),
                sign_cert_der_base64: "PUBLIC_CERT".to_owned(),
                sign_pri_key_base64: "PRIVATE_KEY".to_owned(),
                km_cert_der_base64: "KM_PUBLIC_CERT".to_owned(),
                km_pri_key_base64: "KM_PRIVATE_KEY".to_owned(),
            },
        }
    }

    fn fixture_snapshot() -> BridgeSnapshot {
        BridgeSnapshot {
            collected_at: DateTime::parse_from_rfc3339("2026-04-09T12:00:00+09:00")
                .expect("date")
                .with_timezone(&FixedOffset::east_opt(9 * 3600).expect("offset")),
            facility: BridgeFacility {
                code: "FAC-001".to_owned(),
                name: "Plant FAC-001".to_owned(),
                business_no: "2768603291".to_owned(),
                contract_no: "CONTRACT-1".to_owned(),
                address: "Seoul".to_owned(),
                installed_kw: 100.5,
                rec_start_date: "2026-01-01".to_owned(),
                confirm_issued: "Y".to_owned(),
            },
            generation: vec![BridgeGenerationRecord {
                month: "2026-03".to_owned(),
                generation_kwh: 1000.0,
                self_consumption_kwh: Some(100.0),
                supply_kwh: 900.0,
                application_deadline: Some("2026-04-10".to_owned()),
            }],
            rates: vec![BridgeRateRecord {
                month: "2026-03".to_owned(),
                self_rate: Some(12.5),
                national_average: Some(98.0),
                nearby_average: Some(97.0),
                nearby_highest: Some(110.0),
                published: true,
                source_control_flag: Some("local".to_owned()),
            }],
            rec: vec![BridgeRecRecord {
                month: "2026-03".to_owned(),
                application_no: "LOCAL-APP-001".to_owned(),
                supply_kwh: 900,
                rec_quantity: 1,
                fee_krw: 1000,
                issued_date: "2026-04-01".to_owned(),
                expiry_date: "2027-04-01".to_owned(),
                virtual_account: None,
                bank_name: None,
                status: Some("issued".to_owned()),
            }],
            documents: Vec::new(),
        }
    }
}

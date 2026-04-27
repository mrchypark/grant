mod support;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use chrono::Utc;
use domain::{
    Agreement, AgreementStatus, AgreementType, Plant, Role, RoleBinding, RoleBindingResourceType,
    RoleBindingState, RoleBindingSubjectType,
};
use hiqlite::{Node, NodeConfig, s3::EncKeys};
use http_body_util::BodyExt;
use infra::{
    artifacts::ArtifactStore,
    bridge::{
        BridgeFacility, BridgeGenerationRecord, BridgeRateRecord, BridgeRecRecord, BridgeRun,
        BridgeSnapshot,
    },
    config::{
        AppConfig, ArtifactBackendConfig, ArtifactConfig, HiqliteConfig, LocalNodeRole,
        ReadConsistency, RuntimeMode, SecurityConfig, ServerConfig,
    },
    database::Database,
    runtime::HiqliteRuntime,
    service::{AppService, local_execution_result_signature_payload},
};
use openssl::{
    asn1::Asn1Time,
    hash::MessageDigest,
    pkey::{PKey, Private},
    rsa::Rsa,
    sha::sha256,
    sign::Signer,
    x509::{X509, X509NameBuilder},
};
use serde_json::{Value, json};
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;
use uuid::Uuid;

fn free_local_addr() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    drop(listener);
    addr.to_string()
}

async fn test_runtime() -> (HiqliteRuntime, TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let api_addr = free_local_addr();
    let raft_addr = free_local_addr();
    let mut cfg = NodeConfig::default();
    cfg.node_id = 1;
    cfg.nodes = vec![Node {
        id: 1,
        addr_raft: raft_addr.clone(),
        addr_api: api_addr.clone(),
    }];
    cfg.listen_addr_api = "127.0.0.1".into();
    cfg.listen_addr_raft = "127.0.0.1".into();
    cfg.data_dir = dir.path().to_string_lossy().into_owned().into();
    cfg.filename_db = "test.sqlite".into();
    cfg.secret_api = "0123456789abcdef0123456789abcdef".to_owned();
    cfg.secret_raft = "fedcba9876543210fedcba9876543210".to_owned();
    cfg.enc_keys = EncKeys::generate().expect("enc keys");
    cfg.health_check_delay_secs = 0;
    cfg.raft_config = NodeConfig::default_raft_config(128);

    let client = hiqlite::start_node(cfg).await.expect("start hiqlite");
    (
        HiqliteRuntime::embedded(client, ReadConsistency::Local),
        dir,
    )
}

async fn build_app() -> (axum::Router, Database, TempDir, TempDir) {
    let trailbase_base_url = support::start_mock_trailbase().await;
    let (runtime, db_dir) = test_runtime().await;
    let artifacts_dir = tempfile::tempdir().expect("artifact tempdir");
    let cfg = AppConfig {
        server: ServerConfig {
            bind_addr: "127.0.0.1:0".to_owned(),
        },
        security: SecurityConfig {
            master_key: "dev-only-master-key".to_owned(),
            bootstrap_user_id: "bootstrap-user".to_owned(),
            bootstrap_user_name: "Bootstrap User".to_owned(),
        },
        auth: support::trailbase_auth_config(trailbase_base_url),
        kms: support::test_kms_config(),
        artifacts: ArtifactConfig {
            backend: ArtifactBackendConfig::FileSystem {
                root_dir: artifacts_dir.path().to_path_buf(),
            },
        },
        hiqlite: HiqliteConfig {
            runtime_mode: RuntimeMode::Embedded,
            local_node_role: LocalNodeRole::Voter,
            default_read_consistency: ReadConsistency::Local,
            remote_nodes: vec![],
            remote_tls: false,
            remote_tls_no_verify: false,
            remote_api_secret: None,
            remote_with_proxy: false,
        },
    };
    let db = Database::new(
        runtime,
        cfg.security.bootstrap_user_id.clone(),
        cfg.security.bootstrap_user_name.clone(),
    );
    let artifacts = ArtifactStore::new(&cfg.artifacts)
        .await
        .expect("artifact store");
    let service = Arc::new(AppService::new_with_collector(
        cfg,
        db.clone(),
        artifacts,
        mock_collect_snapshot,
    ));
    service.initialize().await.expect("initialize");
    db.upsert_organization_identity("bootstrap-user", Some("2768603291"), "Bootstrap User")
        .await
        .expect("seed bootstrap organization identity");
    (api::build_router(service), db, db_dir, artifacts_dir)
}

async fn mock_collect_snapshot(opts: infra::bridge::BridgeOptions) -> anyhow::Result<BridgeRun> {
    let facility_code = opts
        .facility_code
        .clone()
        .unwrap_or_else(|| "FAC-MOCK".to_owned());
    Ok(BridgeRun {
        temp_dir: tempfile::tempdir().expect("collector tempdir"),
        snapshot: BridgeSnapshot {
            collected_at: Utc::now().fixed_offset(),
            facility: BridgeFacility {
                code: facility_code,
                name: "Mock Facility".to_owned(),
                business_no: opts.business_no,
                contract_no: "CONTRACT-001".to_owned(),
                address: "Seoul".to_owned(),
                installed_kw: 123.4,
                rec_start_date: "2026-01-01".to_owned(),
                confirm_issued: "Y".to_owned(),
            },
            generation: vec![BridgeGenerationRecord {
                month: "2026-03".to_owned(),
                generation_kwh: 1000.0,
                self_consumption_kwh: Some(100.0),
                supply_kwh: 900.0,
                application_deadline: Some("2026-04-15".to_owned()),
            }],
            rates: vec![BridgeRateRecord {
                month: "2026-03".to_owned(),
                self_rate: Some(110.0),
                national_average: Some(120.0),
                nearby_average: Some(118.0),
                nearby_highest: Some(130.0),
                published: true,
                source_control_flag: Some("mock".to_owned()),
            }],
            rec: vec![BridgeRecRecord {
                month: "2026-03".to_owned(),
                application_no: "APP-001".to_owned(),
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
        },
    })
}

fn auth_request(
    method: &str,
    path: &str,
    token: &str,
    organization_id: &str,
    body: Value,
) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", support::bearer_header(token))
        .header("x-organization-id", organization_id)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request")
}

fn task_token_request(path: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .expect("request")
}

fn proof_bundle_request(challenge_payload: &str) -> Value {
    let (bundle, challenge_signature_base64) = signed_proof_bundle(challenge_payload);
    json!({
        "challengeSignatureBase64": challenge_signature_base64,
        "bundle": bundle,
    })
}

struct ProofMaterial {
    sign_key: PKey<Private>,
    sign_cert_der: Vec<u8>,
    sign_pri_key_der: Vec<u8>,
    km_cert_der: Vec<u8>,
    km_pri_key_der: Vec<u8>,
}

fn proof_material() -> ProofMaterial {
    let sign_key =
        PKey::from_rsa(Rsa::generate(2048).expect("generate sign rsa key")).expect("wrap sign key");
    let sign_cert_der = create_sign_cert_der(&sign_key);
    let sign_pri_key_der = sign_key
        .private_key_to_der()
        .expect("serialize sign private key");
    let km_key =
        PKey::from_rsa(Rsa::generate(2048).expect("generate km rsa key")).expect("wrap km key");
    let km_cert_der = create_sign_cert_der(&km_key);
    let km_pri_key_der = km_key
        .private_key_to_der()
        .expect("serialize km private key");
    ProofMaterial {
        sign_key,
        sign_cert_der,
        sign_pri_key_der,
        km_cert_der,
        km_pri_key_der,
    }
}

fn signed_proof_bundle_from_material(
    material: &ProofMaterial,
    challenge_payload: &str,
) -> (Value, String) {
    let challenge_signature_base64 = sign_challenge(&material.sign_key, challenge_payload);
    (
        json!({
            "businessNo": "1111111111",
            "password": "secret-password",
            "commonName": "Spoofed Client Name",
            "signCertDerBase64": STANDARD.encode(&material.sign_cert_der),
            "signPriKeyBase64": STANDARD.encode(&material.sign_pri_key_der),
            "kmCertDerBase64": STANDARD.encode(&material.km_cert_der),
            "kmPriKeyBase64": STANDARD.encode(&material.km_pri_key_der)
        }),
        challenge_signature_base64,
    )
}

fn signed_proof_bundle(challenge_payload: &str) -> (Value, String) {
    signed_proof_bundle_from_material(&proof_material(), challenge_payload)
}

fn sha256_hex(bytes: &[u8]) -> String {
    sha256(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn stable_document_id(
    organization_id: &str,
    plant_code: &str,
    attachment_code: &str,
    file_name: &str,
) -> String {
    let mut input = Vec::new();
    push_hash_component(&mut input, organization_id);
    push_hash_component(&mut input, plant_code);
    push_hash_component(&mut input, attachment_code);
    push_hash_component(&mut input, file_name);
    sha256(&input)[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn push_hash_component(input: &mut Vec<u8>, value: &str) {
    input.extend_from_slice(&(value.len() as u64).to_be_bytes());
    input.extend_from_slice(value.as_bytes());
}

fn signed_proof_bundle_without_km(challenge_payload: &str) -> (Value, String) {
    let (bundle, challenge_signature_base64) = signed_proof_bundle(challenge_payload);
    let mut bundle = bundle.as_object().expect("bundle object").clone();
    bundle.remove("kmCertDerBase64");
    bundle.remove("kmPriKeyBase64");
    (Value::Object(bundle), challenge_signature_base64)
}

fn signed_proof_bundle_with_empty_km(challenge_payload: &str) -> (Value, String) {
    let (bundle, challenge_signature_base64) = signed_proof_bundle(challenge_payload);
    let mut bundle = bundle.as_object().expect("bundle object").clone();
    bundle.insert("kmCertDerBase64".to_owned(), Value::String(String::new()));
    bundle.insert("kmPriKeyBase64".to_owned(), Value::String(String::new()));
    (Value::Object(bundle), challenge_signature_base64)
}

fn signed_proof_bundle_with_asymmetric_km(
    challenge_payload: &str,
    keep_cert: bool,
) -> (Value, String) {
    let (bundle, challenge_signature_base64) = signed_proof_bundle(challenge_payload);
    let mut bundle = bundle.as_object().expect("bundle object").clone();
    if keep_cert {
        bundle.remove("kmPriKeyBase64");
    } else {
        bundle.remove("kmCertDerBase64");
    }
    (Value::Object(bundle), challenge_signature_base64)
}

fn local_execution_snapshot(facility_code: &str) -> Value {
    json!({
        "collectedAt": "2026-04-09T12:00:00+09:00",
        "facility": {
            "code": facility_code,
            "name": format!("Plant {facility_code}"),
            "businessNo": "2768603291",
            "contractNo": "CONTRACT-LOCAL",
            "address": "Seoul",
            "installedKw": 100.5,
            "recStartDate": "2026-01-01",
            "confirmIssued": "Y"
        },
        "generation": [{
            "month": "2026-03",
            "generationKwh": 1000.0,
            "selfConsumptionKwh": 100.0,
            "supplyKwh": 900.0,
            "applicationDeadline": "2026-04-10"
        }],
        "rates": [{
            "month": "2026-03",
            "selfRate": 12.5,
            "nationalAverage": 98.0,
            "nearbyAverage": 97.0,
            "nearbyHighest": 110.0,
            "published": true,
            "sourceControlFlag": "local"
        }],
        "rec": [{
            "month": "2026-03",
            "applicationNo": "LOCAL-APP-001",
            "supplyKwh": 900,
            "recQuantity": 1,
            "feeKrw": 1000,
            "issuedDate": "2026-04-01",
            "expiryDate": "2027-04-01",
            "virtualAccount": null,
            "bankName": null,
            "status": "issued"
        }],
        "documents": []
    })
}

fn local_execution_snapshot_with_document(facility_code: &str) -> Value {
    let mut snapshot = local_execution_snapshot(facility_code);
    snapshot["documents"] = json!([{
        "attachmentCode": "ATT-LOCAL-1",
        "fileClass": "permit",
        "fileSecName": "permit",
        "fileName": "permit.pdf",
        "required": true,
        "category": "permit",
        "sourcePath": "",
        "sourceSha256": "sha-local",
        "outputSha256": "sha-local",
        "contentBase64": STANDARD.encode(b"PDF-CONTENT"),
        "contentType": "application/pdf"
    }]);
    snapshot
}

async fn grant_test_sync_access(
    db: &Database,
    organization_id: &str,
    user_id: &str,
    facility_code: &str,
) {
    db.upsert_role(&Role {
        id: "role-test-plant-sync-execute".to_owned(),
        organization_id: None,
        code: "plant.sync.execute".to_owned(),
        name: "Plant Sync Execute".to_owned(),
        description: None,
    })
    .await
    .expect("upsert test sync role");
    db.upsert_role_action("role-test-plant-sync-execute", "plant.sync.execute")
        .await
        .expect("upsert test sync action");
    db.save_role_binding(&RoleBinding {
        id: Uuid::new_v4().simple().to_string(),
        subject_type: RoleBindingSubjectType::Organization,
        subject_id: organization_id.to_owned(),
        subject_organization_id: organization_id.to_owned(),
        resource_type: RoleBindingResourceType::Plant,
        resource_id: facility_code.to_owned(),
        role_id: "role-test-plant-sync-execute".to_owned(),
        agreement_id: None,
        state: RoleBindingState::Active,
        created_by_user_id: user_id.to_owned(),
        created_at: "2026-04-03T00:00:00Z".to_owned(),
        updated_at: "2026-04-03T00:00:00Z".to_owned(),
    })
    .await
    .expect("save org sync binding");
    db.save_role_binding(&RoleBinding {
        id: Uuid::new_v4().simple().to_string(),
        subject_type: RoleBindingSubjectType::User,
        subject_id: user_id.to_owned(),
        subject_organization_id: organization_id.to_owned(),
        resource_type: RoleBindingResourceType::Plant,
        resource_id: facility_code.to_owned(),
        role_id: "role-test-plant-sync-execute".to_owned(),
        agreement_id: None,
        state: RoleBindingState::Active,
        created_by_user_id: user_id.to_owned(),
        created_at: "2026-04-03T00:00:00Z".to_owned(),
        updated_at: "2026-04-03T00:00:00Z".to_owned(),
    })
    .await
    .expect("save user sync binding");
}

async fn grant_test_delegated_sync_access(
    db: &Database,
    grantor_organization_id: &str,
    grantee_organization_id: &str,
    user_id: &str,
    facility_code: &str,
) {
    db.upsert_role(&Role {
        id: "role-test-plant-sync-execute".to_owned(),
        organization_id: None,
        code: "plant.sync.execute".to_owned(),
        name: "Plant Sync Execute".to_owned(),
        description: None,
    })
    .await
    .expect("upsert test sync role");
    db.upsert_role_action("role-test-plant-sync-execute", "plant.sync.execute")
        .await
        .expect("upsert test sync action");

    let agreement_id = Uuid::new_v4().simple().to_string();
    db.upsert_agreement(&Agreement {
        id: agreement_id.clone(),
        agreement_type: AgreementType::Delegation,
        grantor_organization_id: grantor_organization_id.to_owned(),
        grantee_organization_id: grantee_organization_id.to_owned(),
        status: AgreementStatus::Active,
        title: "Delegated Sync".to_owned(),
        reference_no: None,
        starts_at: None,
        ends_at: None,
    })
    .await
    .expect("upsert delegated sync agreement");

    db.save_role_binding(&RoleBinding {
        id: Uuid::new_v4().simple().to_string(),
        subject_type: RoleBindingSubjectType::Organization,
        subject_id: grantee_organization_id.to_owned(),
        subject_organization_id: grantee_organization_id.to_owned(),
        resource_type: RoleBindingResourceType::Plant,
        resource_id: facility_code.to_owned(),
        role_id: "role-test-plant-sync-execute".to_owned(),
        agreement_id: Some(agreement_id),
        state: RoleBindingState::Active,
        created_by_user_id: user_id.to_owned(),
        created_at: "2026-04-03T00:00:00Z".to_owned(),
        updated_at: "2026-04-03T00:00:00Z".to_owned(),
    })
    .await
    .expect("save delegated org sync binding");
    db.save_role_binding(&RoleBinding {
        id: Uuid::new_v4().simple().to_string(),
        subject_type: RoleBindingSubjectType::User,
        subject_id: user_id.to_owned(),
        subject_organization_id: grantee_organization_id.to_owned(),
        resource_type: RoleBindingResourceType::Plant,
        resource_id: facility_code.to_owned(),
        role_id: "role-test-plant-sync-execute".to_owned(),
        agreement_id: None,
        state: RoleBindingState::Active,
        created_by_user_id: user_id.to_owned(),
        created_at: "2026-04-03T00:00:00Z".to_owned(),
        updated_at: "2026-04-03T00:00:00Z".to_owned(),
    })
    .await
    .expect("save delegated user sync binding");
}

fn create_sign_cert_der(key: &PKey<Private>) -> Vec<u8> {
    let mut name = X509NameBuilder::new().expect("x509 name");
    name.append_entry_by_text("CN", "Test Certificate")
        .expect("append cn");
    name.append_entry_by_text("serialNumber", "2768603291")
        .expect("append business number");
    let name = name.build();

    let mut builder = X509::builder().expect("x509 builder");
    builder.set_version(2).expect("set version");
    builder.set_subject_name(&name).expect("set subject");
    builder.set_issuer_name(&name).expect("set issuer");
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
        .sign(key, MessageDigest::sha256())
        .expect("sign cert");
    builder.build().to_der().expect("cert der")
}

fn sign_challenge(key: &PKey<Private>, challenge_payload: &str) -> String {
    sign_bytes(key, challenge_payload.as_bytes())
}

fn sign_bytes(key: &PKey<Private>, message: &[u8]) -> String {
    let mut signer = Signer::new(MessageDigest::sha256(), key).expect("create signer");
    signer.update(message).expect("sign payload");
    STANDARD.encode(signer.sign_to_vec().expect("signature"))
}

fn sign_local_execution_result(
    key: &PKey<Private>,
    challenge_id: &Value,
    challenge_payload: &Value,
    mode: &Value,
    snapshot: &Value,
) -> String {
    let snapshot: BridgeSnapshot =
        serde_json::from_value(snapshot.clone()).expect("deserialize local snapshot");
    let payload = local_execution_result_signature_payload(
        challenge_id.as_str().expect("challengeId"),
        challenge_payload.as_str().expect("challengePayload"),
        mode.as_str().unwrap_or("delta"),
        &snapshot,
    )
    .expect("result signature payload");
    sign_bytes(key, &payload)
}

#[allow(clippy::too_many_arguments)]
fn assert_challenge_payload_binds_context(
    challenge_payload: &str,
    organization_id: &str,
    mode: &str,
    origin: &str,
    business_no: &str,
    facility_code: &str,
    start_month: &str,
    end_month: &str,
    sign_cert_sha256_hex: &str,
) {
    let encoded = challenge_payload
        .strip_prefix("cert-gateway:")
        .expect("challenge payload prefix");
    let raw = URL_SAFE_NO_PAD
        .decode(encoded)
        .expect("decode challenge payload");
    let payload: Value = serde_json::from_slice(&raw).expect("challenge payload json");
    assert_eq!(payload["organizationId"], organization_id);
    assert_eq!(payload["mode"], mode);
    assert_eq!(payload["origin"], origin);
    assert_eq!(payload["businessNo"], business_no);
    assert_eq!(payload["facilityCode"], facility_code);
    assert_eq!(payload["startMonth"], start_month);
    assert_eq!(payload["endMonth"], end_month);
    assert_eq!(payload["signCertSha256Hex"], sign_cert_sha256_hex);
}

async fn json_body(response: axum::response::Response) -> Value {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("json body")
}

fn decode_challenge_payload_json(challenge_payload: &str) -> Value {
    let encoded = challenge_payload
        .strip_prefix("cert-gateway:")
        .expect("challenge payload prefix");
    let raw = URL_SAFE_NO_PAD
        .decode(encoded)
        .expect("decode challenge payload");
    serde_json::from_slice(&raw).expect("challenge payload json")
}

#[tokio::test]
async fn cert_gateway_routes_cover_stored_and_ephemeral_flows() {
    let (app, db, _db_dir, artifacts_dir) = build_app().await;
    let stored_material = proof_material();
    let stored_sign_cert_sha256_hex = sha256_hex(&stored_material.sign_cert_der);

    let challenge_response = app
        .clone()
        .oneshot(auth_request(
            "POST",
            "/api/v1/organizations/bootstrap-user/cert-gateway/challenges",
            "bootstrap-user",
            "bootstrap-user",
            json!({
                "mode": "stored",
                "origin": "http://127.0.0.1:4173",
                "businessNo": "276-86-03291",
                "facilityCode": "FAC-001",
                "startMonth": "2026-01",
                "endMonth": "2026-03",
                "signCertSha256Hex": stored_sign_cert_sha256_hex
            }),
        ))
        .await
        .expect("challenge response");
    assert_eq!(challenge_response.status(), StatusCode::CREATED);
    let challenge_json = json_body(challenge_response).await;
    let challenge_id = challenge_json["challengeId"]
        .as_str()
        .expect("challenge id")
        .to_owned();
    let challenge_payload = challenge_json["payload"]
        .as_str()
        .expect("challenge payload")
        .to_owned();
    assert_eq!(challenge_json["facilityCode"], "FAC-001");
    assert_eq!(challenge_json["businessNo"], "2768603291");
    assert_challenge_payload_binds_context(
        &challenge_payload,
        "bootstrap-user",
        "stored",
        "http://127.0.0.1:4173",
        "2768603291",
        "FAC-001",
        "2026-01",
        "2026-03",
        &stored_sign_cert_sha256_hex,
    );

    let register_response = app
        .clone()
        .oneshot(auth_request(
            "POST",
            "/api/v1/organizations/bootstrap-user/certificates",
            "bootstrap-user",
            "bootstrap-user",
            {
                let (bundle, challenge_signature_base64) =
                    signed_proof_bundle_from_material(&stored_material, &challenge_payload);
                let mut request = json!({
                    "challengeSignatureBase64": challenge_signature_base64,
                    "bundle": bundle,
                });
                request["challengeId"] = Value::String(challenge_id.clone());
                request["challengePayload"] = Value::String(challenge_payload.clone());
                request
            },
        ))
        .await
        .expect("register response");
    let register_status = register_response.status();
    let register_json = json_body(register_response).await;
    assert_eq!(
        register_status,
        StatusCode::CREATED,
        "register response body: {register_json}"
    );
    assert_eq!(register_json["organizationId"], "bootstrap-user");
    assert_eq!(register_json["partyId"], "party-2768603291");
    assert_eq!(register_json["commonName"], "Test Certificate");
    assert_eq!(register_json["businessNo"], "2768603291");
    let certificate_id = register_json["id"]
        .as_str()
        .expect("certificate id")
        .to_owned();

    let list_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/organizations/bootstrap-user/certificates")
                .header("authorization", support::bearer_header("bootstrap-user"))
                .header("x-organization-id", "bootstrap-user")
                .body(Body::empty())
                .expect("list request"),
        )
        .await
        .expect("list response");
    assert_eq!(list_response.status(), StatusCode::OK);
    let list_json = json_body(list_response).await;
    assert_eq!(list_json.as_array().expect("list array").len(), 1);
    assert_eq!(list_json[0]["organizationId"], "bootstrap-user");
    assert_eq!(list_json[0]["partyId"], "party-2768603291");
    assert_eq!(list_json[0]["commonName"], "Test Certificate");
    assert_eq!(list_json[0]["businessNo"], "2768603291");

    let issue_response = app
        .clone()
        .oneshot(auth_request(
            "POST",
            &format!(
                "/api/v1/organizations/bootstrap-user/certificates/{certificate_id}/crawl-tokens"
            ),
            "bootstrap-user",
            "bootstrap-user",
            json!({
                "facilityCode": "FAC-003",
                "startMonth": "2026-01",
                "endMonth": "2026-04",
                "mode": "full"
            }),
        ))
        .await
        .expect("issue response");
    assert_eq!(issue_response.status(), StatusCode::CREATED);
    let issue_json = json_body(issue_response).await;
    let crawl_task_token = issue_json["crawlTaskToken"]
        .as_str()
        .expect("crawl task token")
        .to_owned();

    let start_response = app
        .clone()
        .oneshot(task_token_request(
            "/api/v1/organizations/bootstrap-user/crawls",
            &crawl_task_token,
        ))
        .await
        .expect("start response");
    assert_eq!(start_response.status(), StatusCode::ACCEPTED);
    let start_json = json_body(start_response).await;
    assert_eq!(start_json["status"], "completed");
    assert_eq!(start_json["plantId"], "FAC-003");

    let reuse_response = app
        .clone()
        .oneshot(task_token_request(
            "/api/v1/organizations/bootstrap-user/crawls",
            &crawl_task_token,
        ))
        .await
        .expect("reuse response");
    assert_eq!(reuse_response.status(), StatusCode::BAD_REQUEST);

    let ephemeral_challenge = app
        .clone()
        .oneshot(auth_request(
            "POST",
            "/api/v1/organizations/bootstrap-user/cert-gateway/challenges",
            "bootstrap-user",
            "bootstrap-user",
            json!({ "mode": "ephemeral", "facilityCode": "FAC-002" }),
        ))
        .await
        .expect("ephemeral challenge");
    assert_eq!(ephemeral_challenge.status(), StatusCode::CREATED);
    let ephemeral_challenge_json = json_body(ephemeral_challenge).await;
    assert_eq!(ephemeral_challenge_json["syncMode"], "full");
    assert_eq!(ephemeral_challenge_json["facilityCode"], "FAC-002");
    assert_eq!(ephemeral_challenge_json["businessNo"], "2768603291");
    let ephemeral_payload = decode_challenge_payload_json(
        ephemeral_challenge_json["payload"]
            .as_str()
            .expect("ephemeral challenge payload"),
    );
    assert_eq!(ephemeral_payload["businessNo"], "2768603291");

    let ephemeral_response = app
        .clone()
        .oneshot(auth_request(
            "POST",
            "/api/v1/organizations/bootstrap-user/cert-gateway/ephemeral-executions",
            "bootstrap-user",
            "bootstrap-user",
            {
                let payload = ephemeral_challenge_json["payload"]
                    .as_str()
                    .expect("ephemeral payload");
                let mut request = proof_bundle_request(payload);
                request["challengeId"] = ephemeral_challenge_json["challengeId"].clone();
                request["challengePayload"] = ephemeral_challenge_json["payload"].clone();
                request["facilityCode"] = Value::String("FAC-002".to_owned());
                request["mode"] = ephemeral_challenge_json["syncMode"].clone();
                request
            },
        ))
        .await
        .expect("ephemeral response");
    assert_eq!(ephemeral_response.status(), StatusCode::ACCEPTED);
    let ephemeral_json = json_body(ephemeral_response).await;
    assert_eq!(ephemeral_json["status"], "completed");
    assert_eq!(ephemeral_json["plantId"], "FAC-002");

    let local_challenge = app
        .clone()
        .oneshot(auth_request(
            "POST",
            "/api/v1/organizations/bootstrap-user/cert-gateway/challenges",
            "bootstrap-user",
            "bootstrap-user",
            json!({
                "mode": "ephemeral",
                "origin": "http://127.0.0.1:4173",
                "facilityCode": "FAC-004"
            }),
        ))
        .await
        .expect("local result challenge");
    assert_eq!(local_challenge.status(), StatusCode::CREATED);
    let local_challenge_json = json_body(local_challenge).await;
    assert_eq!(local_challenge_json["syncMode"], "full");
    let local_payload = local_challenge_json["payload"]
        .as_str()
        .expect("local challenge payload");
    let local_material = proof_material();
    let (bundle, signature) = signed_proof_bundle_from_material(&local_material, local_payload);
    let local_snapshot = local_execution_snapshot("FAC-004");
    let result_signature = sign_local_execution_result(
        &local_material.sign_key,
        &local_challenge_json["challengeId"],
        &local_challenge_json["payload"],
        &local_challenge_json["syncMode"],
        &local_snapshot,
    );
    let local_result_response = app
        .clone()
        .oneshot(auth_request(
            "POST",
            "/api/v1/organizations/bootstrap-user/cert-gateway/local-execution-results",
            "bootstrap-user",
            "bootstrap-user",
            json!({
                "challengeId": local_challenge_json["challengeId"],
                "challengePayload": local_challenge_json["payload"],
                "challengeSignatureBase64": signature,
                "resultSignatureBase64": result_signature,
                "signCertDerBase64": bundle["signCertDerBase64"],
                "mode": local_challenge_json["syncMode"],
                "snapshot": local_snapshot
            }),
        ))
        .await
        .expect("local result response");
    assert_eq!(local_result_response.status(), StatusCode::ACCEPTED);
    let local_result_json = json_body(local_result_response).await;
    assert_eq!(local_result_json["status"], "completed");
    assert_eq!(local_result_json["plantId"], "FAC-004");
    grant_test_sync_access(&db, "bootstrap-user", "bootstrap-user", "FAC-004").await;

    let local_delta_challenge = app
        .clone()
        .oneshot(auth_request(
            "POST",
            "/api/v1/organizations/bootstrap-user/cert-gateway/challenges",
            "bootstrap-user",
            "bootstrap-user",
            json!({
                "mode": "ephemeral",
                "syncMode": "delta",
                "origin": "http://127.0.0.1:4173",
                "facilityCode": "FAC-004"
            }),
        ))
        .await
        .expect("local delta challenge");
    assert_eq!(local_delta_challenge.status(), StatusCode::CREATED);
    let local_delta_challenge_json = json_body(local_delta_challenge).await;
    assert_eq!(local_delta_challenge_json["syncMode"], "delta");
    assert_eq!(local_delta_challenge_json["startMonth"], "2026-03");
    assert!(
        local_delta_challenge_json["endMonth"]
            .as_str()
            .expect("endMonth")
            .len()
            == 7
    );

    let local_doc_challenge = app
        .clone()
        .oneshot(auth_request(
            "POST",
            "/api/v1/organizations/bootstrap-user/cert-gateway/challenges",
            "bootstrap-user",
            "bootstrap-user",
            json!({
                "mode": "ephemeral",
                "origin": "http://127.0.0.1:4173",
                "facilityCode": "FAC-DOC"
            }),
        ))
        .await
        .expect("local document challenge");
    assert_eq!(local_doc_challenge.status(), StatusCode::CREATED);
    let local_doc_challenge_json = json_body(local_doc_challenge).await;
    let local_doc_payload = local_doc_challenge_json["payload"]
        .as_str()
        .expect("local document challenge payload");
    let doc_material = proof_material();
    let (doc_bundle, doc_signature) =
        signed_proof_bundle_from_material(&doc_material, local_doc_payload);
    let doc_snapshot = local_execution_snapshot_with_document("FAC-DOC");
    let doc_result_signature = sign_local_execution_result(
        &doc_material.sign_key,
        &local_doc_challenge_json["challengeId"],
        &local_doc_challenge_json["payload"],
        &Value::String("full".to_owned()),
        &doc_snapshot,
    );
    let local_doc_result = app
        .clone()
        .oneshot(auth_request(
            "POST",
            "/api/v1/organizations/bootstrap-user/cert-gateway/local-execution-results",
            "bootstrap-user",
            "bootstrap-user",
            json!({
                "challengeId": local_doc_challenge_json["challengeId"],
                "challengePayload": local_doc_challenge_json["payload"],
                "challengeSignatureBase64": doc_signature,
                "resultSignatureBase64": doc_result_signature,
                "signCertDerBase64": doc_bundle["signCertDerBase64"],
                "mode": "full",
                "snapshot": doc_snapshot
            }),
        ))
        .await
        .expect("local document result response");
    assert_eq!(local_doc_result.status(), StatusCode::ACCEPTED);

    let doc_id = stable_document_id("bootstrap-user", "FAC-DOC", "ATT-LOCAL-1", "permit.pdf");
    let stored_bytes = std::fs::read(
        artifacts_dir
            .path()
            .join(format!("bootstrap-user/FAC-DOC/{doc_id}.pdf")),
    )
    .expect("local execution document artifact");
    assert_eq!(stored_bytes.as_slice(), b"PDF-CONTENT");
}

#[tokio::test]
async fn local_execution_result_rejects_document_source_path_fallback() {
    let (app, _db, _db_dir, _artifacts_dir) = build_app().await;
    let secret_dir = tempfile::tempdir().expect("secret tempdir");
    let secret_path = secret_dir.path().join("server-secret.txt");
    std::fs::write(&secret_path, b"SERVER-SECRET").expect("secret fixture");

    let challenge = app
        .clone()
        .oneshot(auth_request(
            "POST",
            "/api/v1/organizations/bootstrap-user/cert-gateway/challenges",
            "bootstrap-user",
            "bootstrap-user",
            json!({
                "mode": "ephemeral",
                "origin": "http://127.0.0.1:4173",
                "facilityCode": "FAC-SOURCE"
            }),
        ))
        .await
        .expect("local document challenge");
    assert_eq!(challenge.status(), StatusCode::CREATED);
    let challenge_json = json_body(challenge).await;
    let payload = challenge_json["payload"]
        .as_str()
        .expect("local document challenge payload");
    let material = proof_material();
    let (bundle, signature) = signed_proof_bundle_from_material(&material, payload);

    let mut snapshot = local_execution_snapshot_with_document("FAC-SOURCE");
    snapshot["documents"][0]["sourcePath"] =
        Value::String(secret_path.to_string_lossy().into_owned());
    snapshot["documents"][0]
        .as_object_mut()
        .expect("document object")
        .remove("contentBase64");
    let result_signature = sign_local_execution_result(
        &material.sign_key,
        &challenge_json["challengeId"],
        &challenge_json["payload"],
        &Value::String("full".to_owned()),
        &snapshot,
    );

    let response = app
        .oneshot(auth_request(
            "POST",
            "/api/v1/organizations/bootstrap-user/cert-gateway/local-execution-results",
            "bootstrap-user",
            "bootstrap-user",
            json!({
                "challengeId": challenge_json["challengeId"],
                "challengePayload": challenge_json["payload"],
                "challengeSignatureBase64": signature,
                "resultSignatureBase64": result_signature,
                "signCertDerBase64": bundle["signCertDerBase64"],
                "mode": "full",
                "snapshot": snapshot
            }),
        ))
        .await
        .expect("local document result response");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = json_body(response).await;
    assert!(
        body["error"]
            .as_str()
            .expect("error message")
            .contains("contentBase64"),
        "body = {body}"
    );
}

#[tokio::test]
async fn local_execution_result_rejects_unlinked_organization_business_number_override() {
    let (app, db, _db_dir, _artifacts_dir) = build_app().await;
    db.upsert_organization_with_owner("unlinked-org", "Unlinked Org", None, "Unlinked Org")
        .await
        .expect("seed unlinked organization");
    db.upsert_plant(&Plant {
        id: "FAC-UNLINKED".to_owned(),
        facility_code: "FAC-UNLINKED".to_owned(),
        name: "Unlinked Plant".to_owned(),
        owner_organization_id: "unlinked-org".to_owned(),
        created_by_organization_id: Some("unlinked-org".to_owned()),
        contract_no: "CONTRACT-UNLINKED".to_owned(),
        address: "Seoul".to_owned(),
        installed_kw: 100.5,
        rec_start_date: "2026-01-01".to_owned(),
        confirm_issued: "Y".to_owned(),
    })
    .await
    .expect("seed unlinked plant");
    grant_test_sync_access(&db, "unlinked-org", "bootstrap-user", "FAC-UNLINKED").await;

    let challenge = app
        .clone()
        .oneshot(auth_request(
            "POST",
            "/api/v1/organizations/unlinked-org/cert-gateway/challenges",
            "bootstrap-user",
            "unlinked-org",
            json!({
                "mode": "ephemeral",
                "origin": "http://127.0.0.1:4173",
                "businessNo": "2768603291",
                "facilityCode": "FAC-UNLINKED"
            }),
        ))
        .await
        .expect("local challenge");
    assert_eq!(challenge.status(), StatusCode::CREATED);
    let challenge_json = json_body(challenge).await;
    let payload = challenge_json["payload"]
        .as_str()
        .expect("local challenge payload");
    let material = proof_material();
    let (bundle, signature) = signed_proof_bundle_from_material(&material, payload);
    let snapshot = local_execution_snapshot("FAC-UNLINKED");
    let result_signature = sign_local_execution_result(
        &material.sign_key,
        &challenge_json["challengeId"],
        &challenge_json["payload"],
        &challenge_json["syncMode"],
        &snapshot,
    );

    let response = app
        .oneshot(auth_request(
            "POST",
            "/api/v1/organizations/unlinked-org/cert-gateway/local-execution-results",
            "bootstrap-user",
            "unlinked-org",
            json!({
                "challengeId": challenge_json["challengeId"],
                "challengePayload": challenge_json["payload"],
                "challengeSignatureBase64": signature,
                "resultSignatureBase64": result_signature,
                "signCertDerBase64": bundle["signCertDerBase64"],
                "mode": challenge_json["syncMode"],
                "snapshot": snapshot
            }),
        ))
        .await
        .expect("local result response");

    let status = response.status();
    let body = json_body(response).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body = {body}");
    assert!(
        body["error"]
            .as_str()
            .expect("error message")
            .contains("business number"),
        "body = {body}"
    );
}

#[tokio::test]
async fn local_execution_result_preserves_existing_plant_owner_for_authorized_non_owner_sync() {
    let (app, db, _db_dir, _artifacts_dir) = build_app().await;
    db.upsert_organization_with_owner("owner-org", "Owner Org", Some("1234567890"), "Owner Org")
        .await
        .expect("seed owner organization");
    db.upsert_organization_with_owner(
        "operator-org",
        "Operator Org",
        Some("2768603291"),
        "Operator Org",
    )
    .await
    .expect("seed operator organization");
    db.upsert_plant(&Plant {
        id: "FAC-CROSS".to_owned(),
        facility_code: "FAC-CROSS".to_owned(),
        name: "Original Owner Plant".to_owned(),
        owner_organization_id: "owner-org".to_owned(),
        created_by_organization_id: Some("bootstrap-user".to_owned()),
        contract_no: "CONTRACT-ORIGINAL".to_owned(),
        address: "Busan".to_owned(),
        installed_kw: 80.0,
        rec_start_date: "2025-01-01".to_owned(),
        confirm_issued: "Y".to_owned(),
    })
    .await
    .expect("seed owner plant");
    grant_test_delegated_sync_access(
        &db,
        "owner-org",
        "operator-org",
        "bootstrap-user",
        "FAC-CROSS",
    )
    .await;

    let challenge = app
        .clone()
        .oneshot(auth_request(
            "POST",
            "/api/v1/organizations/operator-org/cert-gateway/challenges",
            "bootstrap-user",
            "operator-org",
            json!({
                "mode": "ephemeral",
                "origin": "http://127.0.0.1:4173",
                "facilityCode": "FAC-CROSS"
            }),
        ))
        .await
        .expect("local challenge");
    assert_eq!(challenge.status(), StatusCode::CREATED);
    let challenge_json = json_body(challenge).await;
    let payload = challenge_json["payload"]
        .as_str()
        .expect("local challenge payload");
    let material = proof_material();
    let (bundle, signature) = signed_proof_bundle_from_material(&material, payload);
    let snapshot = local_execution_snapshot("FAC-CROSS");
    let result_signature = sign_local_execution_result(
        &material.sign_key,
        &challenge_json["challengeId"],
        &challenge_json["payload"],
        &challenge_json["syncMode"],
        &snapshot,
    );

    let response = app
        .oneshot(auth_request(
            "POST",
            "/api/v1/organizations/operator-org/cert-gateway/local-execution-results",
            "bootstrap-user",
            "operator-org",
            json!({
                "challengeId": challenge_json["challengeId"],
                "challengePayload": challenge_json["payload"],
                "challengeSignatureBase64": signature,
                "resultSignatureBase64": result_signature,
                "signCertDerBase64": bundle["signCertDerBase64"],
                "mode": challenge_json["syncMode"],
                "snapshot": snapshot
            }),
        ))
        .await
        .expect("local result response");

    let status = response.status();
    let body = json_body(response).await;
    assert_eq!(status, StatusCode::ACCEPTED, "body = {body}");

    let plant = db
        .get_plant_by_facility_code("FAC-CROSS")
        .await
        .expect("plant after authorized non-owner sync");
    assert_eq!(plant.owner_organization_id, "owner-org");
    assert_eq!(
        plant.created_by_organization_id.as_deref(),
        Some("bootstrap-user")
    );
    assert_eq!(plant.name, "Plant FAC-CROSS");
    assert_eq!(plant.contract_no, "CONTRACT-LOCAL");
}

#[tokio::test]
async fn local_execution_result_rejects_untrusted_document_content_type() {
    let (app, _db, _db_dir, _artifacts_dir) = build_app().await;

    let challenge = app
        .clone()
        .oneshot(auth_request(
            "POST",
            "/api/v1/organizations/bootstrap-user/cert-gateway/challenges",
            "bootstrap-user",
            "bootstrap-user",
            json!({
                "mode": "ephemeral",
                "origin": "http://127.0.0.1:4173",
                "facilityCode": "FAC-BAD-CTYPE"
            }),
        ))
        .await
        .expect("local document challenge");
    assert_eq!(challenge.status(), StatusCode::CREATED);
    let challenge_json = json_body(challenge).await;
    let payload = challenge_json["payload"]
        .as_str()
        .expect("local document challenge payload");
    let material = proof_material();
    let (bundle, signature) = signed_proof_bundle_from_material(&material, payload);
    let mut snapshot = local_execution_snapshot_with_document("FAC-BAD-CTYPE");
    snapshot["documents"][0]["contentType"] =
        Value::String("application/pdf\r\nx-evil: 1".to_owned());
    let result_signature = sign_local_execution_result(
        &material.sign_key,
        &challenge_json["challengeId"],
        &challenge_json["payload"],
        &Value::String("full".to_owned()),
        &snapshot,
    );

    let response = app
        .oneshot(auth_request(
            "POST",
            "/api/v1/organizations/bootstrap-user/cert-gateway/local-execution-results",
            "bootstrap-user",
            "bootstrap-user",
            json!({
                "challengeId": challenge_json["challengeId"],
                "challengePayload": challenge_json["payload"],
                "challengeSignatureBase64": signature,
                "resultSignatureBase64": result_signature,
                "signCertDerBase64": bundle["signCertDerBase64"],
                "mode": "full",
                "snapshot": snapshot
            }),
        ))
        .await
        .expect("local document result response");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = json_body(response).await;
    assert!(
        body["error"]
            .as_str()
            .expect("error message")
            .contains("unsupported contentType"),
        "body = {body}"
    );
}

#[tokio::test]
async fn local_execution_result_requires_origin_bound_challenge_payload() {
    let (app, _db, _db_dir, _artifacts_dir) = build_app().await;

    let challenge = app
        .clone()
        .oneshot(auth_request(
            "POST",
            "/api/v1/organizations/bootstrap-user/cert-gateway/challenges",
            "bootstrap-user",
            "bootstrap-user",
            json!({ "mode": "ephemeral", "facilityCode": "FAC-NO-ORIGIN" }),
        ))
        .await
        .expect("local challenge");
    assert_eq!(challenge.status(), StatusCode::CREATED);
    let challenge_json = json_body(challenge).await;
    let payload = challenge_json["payload"]
        .as_str()
        .expect("local challenge payload");
    let material = proof_material();
    let (bundle, signature) = signed_proof_bundle_from_material(&material, payload);
    let snapshot = local_execution_snapshot("FAC-NO-ORIGIN");
    let result_signature = sign_local_execution_result(
        &material.sign_key,
        &challenge_json["challengeId"],
        &challenge_json["payload"],
        &challenge_json["syncMode"],
        &snapshot,
    );

    let response = app
        .oneshot(auth_request(
            "POST",
            "/api/v1/organizations/bootstrap-user/cert-gateway/local-execution-results",
            "bootstrap-user",
            "bootstrap-user",
            json!({
                "challengeId": challenge_json["challengeId"],
                "challengePayload": challenge_json["payload"],
                "challengeSignatureBase64": signature,
                "resultSignatureBase64": result_signature,
                "signCertDerBase64": bundle["signCertDerBase64"],
                "mode": challenge_json["syncMode"],
                "snapshot": snapshot
            }),
        ))
        .await
        .expect("local result response");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = json_body(response).await;
    assert!(
        body["error"]
            .as_str()
            .expect("error message")
            .contains("origin"),
        "body = {body}"
    );
}

#[tokio::test]
async fn cert_gateway_challenge_rejects_unsafe_facility_code_segments() {
    let (app, _db, _db_dir, _artifacts_dir) = build_app().await;

    let response = app
        .oneshot(auth_request(
            "POST",
            "/api/v1/organizations/bootstrap-user/cert-gateway/challenges",
            "bootstrap-user",
            "bootstrap-user",
            json!({
                "mode": "ephemeral",
                "origin": "http://127.0.0.1:4173",
                "facilityCode": "../FAC-ESCAPE"
            }),
        ))
        .await
        .expect("challenge response");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = json_body(response).await;
    assert!(
        body["error"]
            .as_str()
            .expect("error message")
            .contains("facilityCode"),
        "body = {body}"
    );
}

#[tokio::test]
async fn local_execution_result_rejects_challenge_certificate_fingerprint_mismatch() {
    let (app, _db, _db_dir, _artifacts_dir) = build_app().await;
    let material = proof_material();

    let challenge = app
        .clone()
        .oneshot(auth_request(
            "POST",
            "/api/v1/organizations/bootstrap-user/cert-gateway/challenges",
            "bootstrap-user",
            "bootstrap-user",
            json!({
                "mode": "ephemeral",
                "origin": "http://127.0.0.1:4173",
                "facilityCode": "FAC-004",
                "signCertSha256Hex": "0000000000000000000000000000000000000000000000000000000000000000"
            }),
        ))
        .await
        .expect("challenge");
    assert_eq!(challenge.status(), StatusCode::CREATED);
    let challenge_json = json_body(challenge).await;
    let payload = challenge_json["payload"]
        .as_str()
        .expect("challenge payload");
    let (bundle, signature) = signed_proof_bundle_from_material(&material, payload);
    let snapshot = local_execution_snapshot("FAC-004");
    let result_signature = sign_local_execution_result(
        &material.sign_key,
        &challenge_json["challengeId"],
        &challenge_json["payload"],
        &challenge_json["syncMode"],
        &snapshot,
    );

    let response = app
        .oneshot(auth_request(
            "POST",
            "/api/v1/organizations/bootstrap-user/cert-gateway/local-execution-results",
            "bootstrap-user",
            "bootstrap-user",
            json!({
                "challengeId": challenge_json["challengeId"],
                "challengePayload": challenge_json["payload"],
                "challengeSignatureBase64": signature,
                "resultSignatureBase64": result_signature,
                "signCertDerBase64": bundle["signCertDerBase64"],
                "mode": challenge_json["syncMode"],
                "snapshot": snapshot
            }),
        ))
        .await
        .expect("local result response");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn local_execution_result_persists_normalized_facility_code() {
    let (app, _db, _db_dir, _artifacts_dir) = build_app().await;

    let challenge = app
        .clone()
        .oneshot(auth_request(
            "POST",
            "/api/v1/organizations/bootstrap-user/cert-gateway/challenges",
            "bootstrap-user",
            "bootstrap-user",
            json!({
                "mode": "ephemeral",
                "origin": "http://127.0.0.1:4173",
                "facilityCode": "FAC-TRIM"
            }),
        ))
        .await
        .expect("challenge");
    assert_eq!(challenge.status(), StatusCode::CREATED);
    let challenge_json = json_body(challenge).await;
    let payload = challenge_json["payload"]
        .as_str()
        .expect("challenge payload");
    let material = proof_material();
    let (bundle, signature) = signed_proof_bundle_from_material(&material, payload);
    let snapshot = local_execution_snapshot(" FAC-TRIM ");
    let result_signature = sign_local_execution_result(
        &material.sign_key,
        &challenge_json["challengeId"],
        &challenge_json["payload"],
        &challenge_json["syncMode"],
        &snapshot,
    );

    let response = app
        .oneshot(auth_request(
            "POST",
            "/api/v1/organizations/bootstrap-user/cert-gateway/local-execution-results",
            "bootstrap-user",
            "bootstrap-user",
            json!({
                "challengeId": challenge_json["challengeId"],
                "challengePayload": challenge_json["payload"],
                "challengeSignatureBase64": signature,
                "resultSignatureBase64": result_signature,
                "signCertDerBase64": bundle["signCertDerBase64"],
                "mode": challenge_json["syncMode"],
                "snapshot": snapshot
            }),
        ))
        .await
        .expect("local result response");

    let status = response.status();
    let body = json_body(response).await;
    assert_eq!(status, StatusCode::ACCEPTED, "body = {body}");
    assert_eq!(body["plantId"], "FAC-TRIM");
}

#[tokio::test]
async fn local_execution_result_rejects_snapshot_not_bound_to_result_signature() {
    let (app, _db, _db_dir, _artifacts_dir) = build_app().await;
    let material = proof_material();

    let challenge = app
        .clone()
        .oneshot(auth_request(
            "POST",
            "/api/v1/organizations/bootstrap-user/cert-gateway/challenges",
            "bootstrap-user",
            "bootstrap-user",
            json!({
                "mode": "ephemeral",
                "origin": "http://127.0.0.1:4173",
                "facilityCode": "FAC-SIGNED"
            }),
        ))
        .await
        .expect("challenge");
    assert_eq!(challenge.status(), StatusCode::CREATED);
    let challenge_json = json_body(challenge).await;
    let payload = challenge_json["payload"]
        .as_str()
        .expect("challenge payload");
    let (bundle, signature) = signed_proof_bundle_from_material(&material, payload);
    let signed_snapshot = local_execution_snapshot("FAC-SIGNED");
    let result_signature = sign_local_execution_result(
        &material.sign_key,
        &challenge_json["challengeId"],
        &challenge_json["payload"],
        &challenge_json["syncMode"],
        &signed_snapshot,
    );
    let mut tampered_snapshot = signed_snapshot;
    tampered_snapshot["generation"][0]["generationKwh"] = json!(999999.0);

    let response = app
        .oneshot(auth_request(
            "POST",
            "/api/v1/organizations/bootstrap-user/cert-gateway/local-execution-results",
            "bootstrap-user",
            "bootstrap-user",
            json!({
                "challengeId": challenge_json["challengeId"],
                "challengePayload": challenge_json["payload"],
                "challengeSignatureBase64": signature,
                "resultSignatureBase64": result_signature,
                "signCertDerBase64": bundle["signCertDerBase64"],
                "mode": challenge_json["syncMode"],
                "snapshot": tampered_snapshot
            }),
        ))
        .await
        .expect("local result response");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = json_body(response).await;
    assert!(
        body["error"]
            .as_str()
            .expect("error message")
            .contains("signature"),
        "body = {body}"
    );
}

#[tokio::test]
async fn crawl_start_rejects_organization_path_mismatch() {
    let (app, _db, _db_dir, _artifacts_dir) = build_app().await;

    let challenge_response = app
        .clone()
        .oneshot(auth_request(
            "POST",
            "/api/v1/organizations/bootstrap-user/cert-gateway/challenges",
            "bootstrap-user",
            "bootstrap-user",
            json!({ "mode": "stored", "facilityCode": "FAC-001" }),
        ))
        .await
        .expect("challenge response");
    assert_eq!(challenge_response.status(), StatusCode::CREATED);
    let challenge_json = json_body(challenge_response).await;

    let register_response = app
        .clone()
        .oneshot(auth_request(
            "POST",
            "/api/v1/organizations/bootstrap-user/certificates",
            "bootstrap-user",
            "bootstrap-user",
            {
                let payload = challenge_json["payload"]
                    .as_str()
                    .expect("challenge payload");
                let mut request = proof_bundle_request(payload);
                request["challengeId"] = challenge_json["challengeId"].clone();
                request["challengePayload"] = challenge_json["payload"].clone();
                request
            },
        ))
        .await
        .expect("register response");
    assert_eq!(register_response.status(), StatusCode::CREATED);
    let register_json = json_body(register_response).await;
    let certificate_id = register_json["id"]
        .as_str()
        .expect("certificate id")
        .to_owned();

    let issue_response = app
        .clone()
        .oneshot(auth_request(
            "POST",
            &format!(
                "/api/v1/organizations/bootstrap-user/certificates/{certificate_id}/crawl-tokens"
            ),
            "bootstrap-user",
            "bootstrap-user",
            json!({
                "facilityCode": "FAC-003",
                "mode": "full"
            }),
        ))
        .await
        .expect("issue response");
    assert_eq!(issue_response.status(), StatusCode::CREATED);
    let issue_json = json_body(issue_response).await;
    let crawl_task_token = issue_json["crawlTaskToken"]
        .as_str()
        .expect("crawl task token")
        .to_owned();

    let mismatch_response = app
        .clone()
        .oneshot(task_token_request(
            "/api/v1/organizations/other-org/crawls",
            &crawl_task_token,
        ))
        .await
        .expect("mismatch response");
    assert_eq!(mismatch_response.status(), StatusCode::FORBIDDEN);

    let retry_response = app
        .oneshot(task_token_request(
            "/api/v1/organizations/bootstrap-user/crawls",
            &crawl_task_token,
        ))
        .await
        .expect("retry response");
    assert_eq!(retry_response.status(), StatusCode::ACCEPTED);
}

#[tokio::test]
async fn crawl_start_returns_not_found_for_unknown_token() {
    let (app, _db, _db_dir, _artifacts_dir) = build_app().await;

    let response = app
        .oneshot(task_token_request(
            "/api/v1/organizations/bootstrap-user/crawls",
            "missing-crawl-task-token",
        ))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let body = json_body(response).await;
    assert_eq!(body["error"], "crawl task token not found");
}

#[tokio::test]
async fn issue_crawl_task_token_returns_not_found_for_unknown_certificate() {
    let (app, _db, _db_dir, _artifacts_dir) = build_app().await;

    let response = app
        .oneshot(auth_request(
            "POST",
            "/api/v1/organizations/bootstrap-user/certificates/missing-certificate/crawl-tokens",
            "bootstrap-user",
            "bootstrap-user",
            json!({
                "facilityCode": "FAC-001",
                "startMonth": "2026-01",
                "endMonth": "2026-02",
                "mode": "delta"
            }),
        ))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let body = json_body(response).await;
    assert_eq!(body["error"], "certificate not found");
}

#[tokio::test]
async fn create_cert_gateway_challenge_rejects_unsupported_mode() {
    let (app, _db, _db_dir, _artifacts_dir) = build_app().await;

    let response = app
        .oneshot(auth_request(
            "POST",
            "/api/v1/organizations/bootstrap-user/cert-gateway/challenges",
            "bootstrap-user",
            "bootstrap-user",
            json!({ "mode": "store" }),
        ))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let body = json_body(response).await;
    assert_eq!(body["error"], "unsupported cert gateway mode: store");
}

#[tokio::test]
async fn create_cert_gateway_challenge_rejects_malformed_business_number() {
    let (app, _db, _db_dir, _artifacts_dir) = build_app().await;

    let response = app
        .oneshot(auth_request(
            "POST",
            "/api/v1/organizations/bootstrap-user/cert-gateway/challenges",
            "bootstrap-user",
            "bootstrap-user",
            json!({ "mode": "stored", "businessNo": "not-a-business-number" }),
        ))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let body = json_body(response).await;
    assert_eq!(body["error"], "businessNo must normalize to 10 digits");
}

#[tokio::test]
async fn create_cert_gateway_challenge_rejects_mismatched_organization_business_number() {
    let (app, _db, _db_dir, _artifacts_dir) = build_app().await;

    let response = app
        .oneshot(auth_request(
            "POST",
            "/api/v1/organizations/bootstrap-user/cert-gateway/challenges",
            "bootstrap-user",
            "bootstrap-user",
            json!({ "mode": "ephemeral", "businessNo": "1111111111" }),
        ))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let body = json_body(response).await;
    assert_eq!(
        body["error"],
        "cert gateway challenge business number mismatch"
    );
}

#[tokio::test]
async fn stored_registration_requires_challenge_signature() {
    let (app, _db, _db_dir, _artifacts_dir) = build_app().await;

    let challenge_response = app
        .clone()
        .oneshot(auth_request(
            "POST",
            "/api/v1/organizations/bootstrap-user/cert-gateway/challenges",
            "bootstrap-user",
            "bootstrap-user",
            json!({ "mode": "stored", "facilityCode": "FAC-001" }),
        ))
        .await
        .expect("challenge response");
    assert_eq!(challenge_response.status(), StatusCode::CREATED);
    let challenge_json = json_body(challenge_response).await;

    let register_response = app
        .oneshot(auth_request(
            "POST",
            "/api/v1/organizations/bootstrap-user/certificates",
            "bootstrap-user",
            "bootstrap-user",
            {
                let (bundle, _) = signed_proof_bundle(
                    challenge_json["payload"]
                        .as_str()
                        .expect("challenge payload"),
                );
                json!({
                    "challengeId": challenge_json["challengeId"],
                    "challengePayload": challenge_json["payload"],
                    "challengeSignatureBase64": "",
                    "bundle": bundle
                })
            },
        ))
        .await
        .expect("register response");
    assert_eq!(register_response.status(), StatusCode::BAD_REQUEST);
    let register_json = json_body(register_response).await;
    assert_eq!(
        register_json["error"],
        "challengeSignatureBase64 must not be empty"
    );
}

#[tokio::test]
async fn stored_registration_accepts_omitted_or_empty_km_material() {
    let (app, _db, _db_dir, _artifacts_dir) = build_app().await;

    for bundle_builder in [
        signed_proof_bundle_without_km as fn(&str) -> (Value, String),
        signed_proof_bundle_with_empty_km as fn(&str) -> (Value, String),
    ] {
        let challenge_response = app
            .clone()
            .oneshot(auth_request(
                "POST",
                "/api/v1/organizations/bootstrap-user/cert-gateway/challenges",
                "bootstrap-user",
                "bootstrap-user",
                json!({ "mode": "stored", "facilityCode": "FAC-001" }),
            ))
            .await
            .expect("challenge response");
        assert_eq!(challenge_response.status(), StatusCode::CREATED);
        let challenge_json = json_body(challenge_response).await;

        let (bundle, challenge_signature_base64) = bundle_builder(
            challenge_json["payload"]
                .as_str()
                .expect("challenge payload"),
        );
        let register_response = app
            .clone()
            .oneshot(auth_request(
                "POST",
                "/api/v1/organizations/bootstrap-user/certificates",
                "bootstrap-user",
                "bootstrap-user",
                json!({
                    "challengeId": challenge_json["challengeId"],
                    "challengePayload": challenge_json["payload"],
                    "challengeSignatureBase64": challenge_signature_base64,
                    "bundle": bundle
                }),
            ))
            .await
            .expect("register response");
        let register_status = register_response.status();
        let register_json = json_body(register_response).await;
        assert_eq!(
            register_status,
            StatusCode::CREATED,
            "register response body: {register_json}"
        );
    }
}

#[tokio::test]
async fn stored_registration_rejects_asymmetric_km_material() {
    let (app, _db, _db_dir, _artifacts_dir) = build_app().await;

    for keep_cert in [true, false] {
        let challenge_response = app
            .clone()
            .oneshot(auth_request(
                "POST",
                "/api/v1/organizations/bootstrap-user/cert-gateway/challenges",
                "bootstrap-user",
                "bootstrap-user",
                json!({ "mode": "stored", "facilityCode": "FAC-001" }),
            ))
            .await
            .expect("challenge response");
        assert_eq!(challenge_response.status(), StatusCode::CREATED);
        let challenge_json = json_body(challenge_response).await;

        let (bundle, challenge_signature_base64) = signed_proof_bundle_with_asymmetric_km(
            challenge_json["payload"]
                .as_str()
                .expect("challenge payload"),
            keep_cert,
        );
        let register_response = app
            .clone()
            .oneshot(auth_request(
                "POST",
                "/api/v1/organizations/bootstrap-user/certificates",
                "bootstrap-user",
                "bootstrap-user",
                json!({
                    "challengeId": challenge_json["challengeId"],
                    "challengePayload": challenge_json["payload"],
                    "challengeSignatureBase64": challenge_signature_base64,
                    "bundle": bundle
                }),
            ))
            .await
            .expect("register response");
        assert_eq!(register_response.status(), StatusCode::BAD_REQUEST);
        let register_json = json_body(register_response).await;
        assert!(
            register_json["error"]
                .as_str()
                .expect("error string")
                .contains("kmCertDerBase64 and kmPriKeyBase64"),
            "register response body: {register_json}"
        );
    }
}

#[tokio::test]
async fn stored_registration_returns_not_found_for_unknown_challenge_id() {
    let (app, _db, _db_dir, _artifacts_dir) = build_app().await;

    let register_response = app
        .oneshot(auth_request(
            "POST",
            "/api/v1/organizations/bootstrap-user/certificates",
            "bootstrap-user",
            "bootstrap-user",
            {
                let challenge_payload = "stale-challenge-payload";
                let (bundle, challenge_signature_base64) = signed_proof_bundle(challenge_payload);
                json!({
                    "challengeId": "missing-challenge-id",
                    "challengePayload": challenge_payload,
                    "challengeSignatureBase64": challenge_signature_base64,
                    "bundle": bundle
                })
            },
        ))
        .await
        .expect("register response");
    assert_eq!(register_response.status(), StatusCode::NOT_FOUND);
    let register_json = json_body(register_response).await;
    assert_eq!(register_json["error"], "cert gateway challenge not found");
}

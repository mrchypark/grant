use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::Utc;
use domain::{
    Agreement, AgreementStatus, AgreementType, GenerationRecord, Role, RoleBinding,
    RoleBindingResourceType, RoleBindingState, RoleBindingSubjectType, SyncMode,
};
use hiqlite::{Node, NodeConfig, Row, s3::EncKeys};
use infra::{
    artifacts::ArtifactStore,
    bridge::{
        BridgeFacility, BridgeGenerationRecord, BridgeOptions, BridgeRateRecord, BridgeRecRecord,
        BridgeRun, BridgeSnapshot,
    },
    config::{
        AppConfig, ArtifactBackendConfig, ArtifactConfig, AuthConfig, HiqliteConfig, KmsConfig,
        LocalNodeRole, ReadConsistency, RuntimeMode, SecurityConfig, ServerConfig,
        TrailbaseAuthConfig,
    },
    crypto::CertificateBundle,
    database::{Database, DatabaseLookupError},
    runtime::HiqliteRuntime,
    service::{
        AppService, CreateCertGatewayChallengeInput, ExecuteEphemeralCrawlInput,
        IssueCrawlTaskTokenInput, RegisterCertificateFromProofInput, ServiceError,
        ServiceErrorKind, StartCrawlWithTaskTokenInput,
    },
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
use serde_json::Value;
use tempfile::TempDir;
use uuid::Uuid;

fn challenge_sync_mode(sync_mode: Option<&str>) -> SyncMode {
    match sync_mode {
        Some("full") => SyncMode::Full,
        Some("delta") => SyncMode::Delta,
        other => panic!("unexpected challenge sync mode: {other:?}"),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct OrganizationPlantActionRow {
    organization_id: String,
    plant_id: String,
    action_id: String,
}

impl From<&mut Row<'_>> for OrganizationPlantActionRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            organization_id: row.get("organization_id"),
            plant_id: row.get("plant_id"),
            action_id: row.get("action_id"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct UserPlantActionRow {
    user_id: String,
    organization_id: String,
    plant_id: String,
    action_id: String,
}

impl From<&mut Row<'_>> for UserPlantActionRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            user_id: row.get("user_id"),
            organization_id: row.get("organization_id"),
            plant_id: row.get("plant_id"),
            action_id: row.get("action_id"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct UserPlantVisibilityRow {
    user_id: String,
    organization_id: String,
    plant_id: String,
}

impl From<&mut Row<'_>> for UserPlantVisibilityRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            user_id: row.get("user_id"),
            organization_id: row.get("organization_id"),
            plant_id: row.get("plant_id"),
        }
    }
}

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

fn test_config(artifacts_root: &Path) -> AppConfig {
    AppConfig {
        server: ServerConfig {
            bind_addr: "127.0.0.1:0".to_owned(),
        },
        security: SecurityConfig {
            master_key: "dev-only-master-key".to_owned(),
            bootstrap_user_id: "bootstrap-user".to_owned(),
            bootstrap_user_name: "Bootstrap User".to_owned(),
            allow_untrusted_certificates: true,
        },
        auth: AuthConfig {
            trailbase: TrailbaseAuthConfig {
                base_url: "http://127.0.0.1:4000".to_owned(),
                introspect_path: "/api/certd/introspect".to_owned(),
                internal_secret: "test-trailbase-secret".to_owned(),
                cache_ttl_seconds: 30,
            },
        },
        kms: KmsConfig {
            active_key_id: "local-dev-v1".to_owned(),
            master_keys: vec![("local-dev-v1".to_owned(), b"dev-only-master-key".to_vec())],
        },
        artifacts: ArtifactConfig {
            backend: ArtifactBackendConfig::FileSystem {
                root_dir: artifacts_root.to_path_buf(),
            },
        },
        hiqlite: HiqliteConfig {
            runtime_mode: RuntimeMode::Embedded,
            local_node_role: LocalNodeRole::Voter,
            default_read_consistency: ReadConsistency::Local,
            remote_nodes: Vec::new(),
            remote_tls: false,
            remote_tls_no_verify: false,
            remote_api_secret: None,
            remote_with_proxy: false,
        },
    }
}

async fn build_service() -> (AppService, Database, TempDir, TempDir) {
    build_service_with_collector(mock_collect_snapshot).await
}

async fn build_service_with_collector<F, Fut>(
    collector: F,
) -> (AppService, Database, TempDir, TempDir)
where
    F: Fn(BridgeOptions) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = anyhow::Result<BridgeRun>> + Send + 'static,
{
    let (runtime, db_dir) = test_runtime().await;
    let artifacts_dir = tempfile::tempdir().expect("artifact tempdir");
    let cfg = test_config(artifacts_dir.path());
    let bootstrap_user_id = cfg.security.bootstrap_user_id.clone();
    let bootstrap_user_name = cfg.security.bootstrap_user_name.clone();
    let db = Database::new(
        runtime.clone(),
        bootstrap_user_id.clone(),
        bootstrap_user_name.clone(),
    );
    let artifacts = ArtifactStore::new(&cfg.artifacts)
        .await
        .expect("artifact store");
    let service = AppService::new_with_collector(cfg, db.clone(), artifacts, collector);
    service.initialize().await.expect("initialize service");
    db.upsert_organization_identity(&bootstrap_user_id, Some("2768603291"), &bootstrap_user_name)
        .await
        .expect("seed bootstrap organization identity");
    (service, db, db_dir, artifacts_dir)
}

async fn mock_collect_snapshot(opts: infra::bridge::BridgeOptions) -> anyhow::Result<BridgeRun> {
    let facility_code = opts
        .facility_code
        .clone()
        .unwrap_or_else(|| "FAC-MOCK".to_owned());
    let month = opts
        .start_month
        .clone()
        .unwrap_or_else(|| "2026-03".to_owned());
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
                month: month.clone(),
                generation_kwh: 1000.0,
                self_consumption_kwh: Some(100.0),
                supply_kwh: 900.0,
                application_deadline: Some("2026-04-15".to_owned()),
            }],
            rates: vec![BridgeRateRecord {
                month: month.clone(),
                self_rate: Some(110.0),
                national_average: Some(120.0),
                nearby_average: Some(118.0),
                nearby_highest: Some(130.0),
                published: true,
                source_control_flag: Some("mock".to_owned()),
            }],
            rec: vec![BridgeRecRecord {
                month,
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

async fn mismatched_collect_snapshot(
    opts: infra::bridge::BridgeOptions,
) -> anyhow::Result<BridgeRun> {
    let mut run = mock_collect_snapshot(opts).await?;
    run.snapshot.facility.code = "FAC-RETURNED".to_owned();
    Ok(run)
}

fn fake_bundle_and_signature(challenge_payload: &str) -> (CertificateBundle, String) {
    let key = PKey::from_rsa(Rsa::generate(2048).expect("generate rsa key")).expect("wrap key");
    let cert_der = create_sign_cert_der(&key);
    let bundle = fake_bundle_with_sign_material(&key, cert_der);
    let challenge_signature_base64 = sign_challenge(&key, challenge_payload);
    (bundle, challenge_signature_base64)
}

fn fake_bundle_with_sign_material(key: &PKey<Private>, cert_der: Vec<u8>) -> CertificateBundle {
    let km_key =
        PKey::from_rsa(Rsa::generate(2048).expect("generate km rsa key")).expect("wrap km key");
    let km_cert_der = create_sign_cert_der_for_common_name(&km_key, "KM Test Certificate");
    CertificateBundle {
        business_no: "2768603291".to_owned(),
        password: "secret-password".to_owned(),
        common_name: "Test Certificate".to_owned(),
        sign_cert_der: cert_der,
        sign_pri_key: direct_private_key_der(key),
        km_cert_der: Some(km_cert_der),
        km_pri_key: Some(direct_private_key_der(&km_key)),
        certificate_chain_der: Vec::new(),
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    sha256(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn create_sign_cert_der(key: &PKey<Private>) -> Vec<u8> {
    create_sign_cert_der_for_common_name(key, "Test Certificate")
}

fn create_sign_cert_der_for_common_name(key: &PKey<Private>, common_name: &str) -> Vec<u8> {
    create_sign_cert_der_with_subject(key, common_name, None, 0, 365)
}

fn create_sign_cert_der_for_common_name_and_business_no(
    key: &PKey<Private>,
    common_name: &str,
    business_no: &str,
) -> Vec<u8> {
    create_sign_cert_der_with_subject(key, common_name, Some(business_no), 0, 365)
}

fn create_expired_sign_cert_der(key: &PKey<Private>, common_name: &str) -> Vec<u8> {
    create_sign_cert_der_with_subject(key, common_name, None, -365, -1)
}

fn create_sign_cert_der_with_subject(
    key: &PKey<Private>,
    common_name: &str,
    business_no: Option<&str>,
    not_before_days_from_now: i32,
    not_after_days_from_now: i32,
) -> Vec<u8> {
    let mut name = X509NameBuilder::new().expect("x509 name");
    name.append_entry_by_text("CN", common_name)
        .expect("append cn");
    if let Some(business_no) = business_no {
        name.append_entry_by_text("serialNumber", business_no)
            .expect("append serial number");
    }
    let name = name.build();

    let mut builder = X509::builder().expect("x509 builder");
    builder.set_version(2).expect("set version");
    builder.set_subject_name(&name).expect("set subject");
    builder.set_issuer_name(&name).expect("set issuer");
    builder.set_pubkey(key).expect("set pubkey");
    let not_before = if not_before_days_from_now >= 0 {
        Asn1Time::days_from_now(not_before_days_from_now as u32).expect("not before")
    } else {
        Asn1Time::from_unix(
            (std::time::SystemTime::now()
                - std::time::Duration::from_secs((-not_before_days_from_now) as u64 * 24 * 60 * 60))
            .duration_since(std::time::UNIX_EPOCH)
            .expect("not before duration")
            .as_secs() as i64,
        )
        .expect("not before")
    };
    let not_after = if not_after_days_from_now >= 0 {
        Asn1Time::days_from_now(not_after_days_from_now as u32).expect("not after")
    } else {
        Asn1Time::from_unix(
            (std::time::SystemTime::now()
                - std::time::Duration::from_secs((-not_after_days_from_now) as u64 * 24 * 60 * 60))
            .duration_since(std::time::UNIX_EPOCH)
            .expect("not after duration")
            .as_secs() as i64,
        )
        .expect("not after")
    };
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

fn direct_private_key_der(key: &PKey<Private>) -> Vec<u8> {
    key.private_key_to_pkcs8().expect("private key pkcs8")
}

fn sign_challenge(key: &PKey<Private>, challenge_payload: &str) -> String {
    let mut signer = Signer::new(MessageDigest::sha256(), key).expect("create signer");
    signer
        .update(challenge_payload.as_bytes())
        .expect("sign challenge payload");
    STANDARD.encode(signer.sign_to_vec().expect("signature"))
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
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
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

fn decode_challenge_payload_json(challenge_payload: &str) -> Value {
    let encoded = challenge_payload
        .strip_prefix("cert-gateway:")
        .expect("challenge payload prefix");
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .expect("decode challenge payload");
    serde_json::from_slice(&raw).expect("challenge payload json")
}

async fn seed_gateway_scope_fixture(db: &Database) {
    db.upsert_organization_with_owner("owner-org", "Owner Org", None, "Owner Org")
        .await
        .expect("owner org");
    db.upsert_organization_with_owner("operator-org", "Operator Org", None, "Operator Org")
        .await
        .expect("operator org");
    db.upsert_organization_identity("owner-org", Some("1234567890"), "Owner Org")
        .await
        .expect("owner business no");
    db.upsert_organization_identity("operator-org", Some("2768603291"), "Operator Org")
        .await
        .expect("operator business no");

    db.upsert_role(&Role {
        id: "role-plant-sync-execute".to_owned(),
        organization_id: None,
        code: "plant.sync.execute".to_owned(),
        name: "Plant Sync Execute".to_owned(),
        description: None,
    })
    .await
    .expect("upsert sync role");
    db.upsert_role_action("role-plant-sync-execute", "plant.sync.execute")
        .await
        .expect("upsert sync action");
    db.upsert_role(&Role {
        id: "role-plant-profile-reader".to_owned(),
        organization_id: None,
        code: "plant.profile.read".to_owned(),
        name: "Plant Profile Read".to_owned(),
        description: None,
    })
    .await
    .expect("upsert profile role");
    db.upsert_role_action("role-plant-profile-reader", "plant.profile.read")
        .await
        .expect("upsert profile role action");

    db.upsert_agreement(&Agreement {
        id: "agreement-sync".to_owned(),
        agreement_type: AgreementType::Delegation,
        grantor_organization_id: "owner-org".to_owned(),
        grantee_organization_id: "operator-org".to_owned(),
        status: AgreementStatus::Active,
        title: "Sync Delegation".to_owned(),
        reference_no: None,
        starts_at: None,
        ends_at: None,
    })
    .await
    .expect("upsert agreement");

    db.upsert_plant(&domain::Plant {
        id: "FAC-SYNC-001".to_owned(),
        facility_code: "FAC-SYNC-001".to_owned(),
        name: "Sync Plant".to_owned(),
        owner_organization_id: "owner-org".to_owned(),
        created_by_organization_id: Some("operator-org".to_owned()),
        contract_no: "CN-SYNC-001".to_owned(),
        address: "Seoul".to_owned(),
        installed_kw: 100.0,
        rec_start_date: "2026-01-01".to_owned(),
        confirm_issued: "Y".to_owned(),
    })
    .await
    .expect("upsert plant");
}

#[tokio::test]
async fn sync_rejects_collector_facility_mismatch_before_writing_rows() {
    let (service, db, _db_dir, _artifacts_dir) =
        build_service_with_collector(mismatched_collect_snapshot).await;

    let challenge = service
        .create_cert_gateway_challenge(CreateCertGatewayChallengeInput {
            organization_id: "bootstrap-user".to_owned(),
            mode: "ephemeral".to_owned(),
            sync_mode: None,
            origin: None,
            business_no: None,
            facility_code: Some("FAC-REQUESTED".to_owned()),
            start_month: None,
            end_month: None,
            sign_cert_sha256_hex: None,
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect("create challenge");
    let (bundle, signature) = fake_bundle_and_signature(&challenge.payload);

    let err = service
        .execute_ephemeral_crawl(ExecuteEphemeralCrawlInput {
            organization_id: "bootstrap-user".to_owned(),
            requested_by_user_id: Some("bootstrap-user".to_owned()),
            challenge_id: challenge.id,
            challenge_payload: challenge.payload,
            challenge_signature_base64: signature,
            bundle,
            facility_code: Some("FAC-REQUESTED".to_owned()),
            start_month: challenge.start_month.clone(),
            end_month: challenge.end_month.clone(),
            mode: challenge_sync_mode(challenge.sync_mode.as_deref()),
        })
        .await
        .expect_err("collector returning another facility must be rejected");

    assert!(err.to_string().contains("facility code mismatch"));
    db.get_plant_by_facility_code("FAC-RETURNED")
        .await
        .expect_err("mismatched facility must not be materialized");
    let audit = db
        .get_latest_sync_run_audit_for_facility("FAC-REQUESTED")
        .await
        .expect("load failed sync audit");
    assert_eq!(audit.run_status, "failed");
    assert_eq!(audit.item_status.as_deref(), Some("failed"));
    assert!(
        audit
            .item_error_message
            .as_deref()
            .unwrap_or_default()
            .contains("facility code mismatch")
    );
}

#[tokio::test]
async fn delegated_sync_preserves_existing_asset_and_plant_owner() {
    let (service, db, _db_dir, _artifacts_dir) = build_service().await;

    db.upsert_organization_with_owner("owner-org", "Owner Org", Some("1234567890"), "Owner Org")
        .await
        .expect("owner org");
    db.upsert_organization_with_owner(
        "operator-org",
        "Operator Org",
        Some("2768603291"),
        "Operator Org",
    )
    .await
    .expect("operator org");

    let existing_plant = domain::Plant {
        id: "FAC-OWNER-001".to_owned(),
        facility_code: "FAC-OWNER-001".to_owned(),
        name: "Owner Plant".to_owned(),
        owner_organization_id: "owner-org".to_owned(),
        created_by_organization_id: Some("operator-org".to_owned()),
        contract_no: "CN-OWNER-001".to_owned(),
        address: "Seoul".to_owned(),
        installed_kw: 100.0,
        rec_start_date: "2026-01-01".to_owned(),
        confirm_issued: "Y".to_owned(),
    };
    db.upsert_plant(&existing_plant)
        .await
        .expect("upsert existing plant");
    db.upsert_kr_rps_asset_projection(
        &existing_plant,
        Some("owner-org"),
        Some("1234567890"),
        &[],
        &[],
    )
    .await
    .expect("upsert existing projection");
    db.upsert_role(&Role {
        id: "role-delegated-sync-execute".to_owned(),
        organization_id: None,
        code: "delegated.sync.execute".to_owned(),
        name: "Delegated Sync Execute".to_owned(),
        description: None,
    })
    .await
    .expect("upsert sync role");
    db.upsert_role_action("role-delegated-sync-execute", "plant.sync.execute")
        .await
        .expect("upsert sync action");
    db.upsert_agreement(&Agreement {
        id: "agreement-owner-operator-sync".to_owned(),
        agreement_type: AgreementType::Delegation,
        grantor_organization_id: "owner-org".to_owned(),
        grantee_organization_id: "operator-org".to_owned(),
        status: AgreementStatus::Active,
        title: "Owner to Operator Sync".to_owned(),
        reference_no: None,
        starts_at: None,
        ends_at: None,
    })
    .await
    .expect("upsert agreement");
    db.save_role_binding(&RoleBinding {
        id: Uuid::new_v4().simple().to_string(),
        subject_type: RoleBindingSubjectType::Organization,
        subject_id: "operator-org".to_owned(),
        subject_organization_id: "operator-org".to_owned(),
        resource_type: RoleBindingResourceType::Plant,
        resource_id: "FAC-OWNER-001".to_owned(),
        role_id: "role-delegated-sync-execute".to_owned(),
        agreement_id: Some("agreement-owner-operator-sync".to_owned()),
        state: RoleBindingState::Active,
        created_by_user_id: "bootstrap-user".to_owned(),
        created_at: "2026-04-25T00:00:00Z".to_owned(),
        updated_at: "2026-04-25T00:00:00Z".to_owned(),
    })
    .await
    .expect("save org sync binding");
    db.save_role_binding(&RoleBinding {
        id: Uuid::new_v4().simple().to_string(),
        subject_type: RoleBindingSubjectType::User,
        subject_id: "bootstrap-user".to_owned(),
        subject_organization_id: "operator-org".to_owned(),
        resource_type: RoleBindingResourceType::Plant,
        resource_id: "FAC-OWNER-001".to_owned(),
        role_id: "role-delegated-sync-execute".to_owned(),
        agreement_id: None,
        state: RoleBindingState::Active,
        created_by_user_id: "bootstrap-user".to_owned(),
        created_at: "2026-04-25T00:00:00Z".to_owned(),
        updated_at: "2026-04-25T00:00:00Z".to_owned(),
    })
    .await
    .expect("save user sync binding");

    let challenge = service
        .create_cert_gateway_challenge(CreateCertGatewayChallengeInput {
            organization_id: "operator-org".to_owned(),
            mode: "ephemeral".to_owned(),
            sync_mode: None,
            origin: None,
            business_no: None,
            facility_code: Some("FAC-OWNER-001".to_owned()),
            start_month: None,
            end_month: None,
            sign_cert_sha256_hex: None,
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect("create challenge");
    let (bundle, signature) = fake_bundle_and_signature(&challenge.payload);

    service
        .execute_ephemeral_crawl(ExecuteEphemeralCrawlInput {
            organization_id: "operator-org".to_owned(),
            requested_by_user_id: Some("bootstrap-user".to_owned()),
            challenge_id: challenge.id,
            challenge_payload: challenge.payload,
            challenge_signature_base64: signature,
            bundle,
            facility_code: Some("FAC-OWNER-001".to_owned()),
            start_month: challenge.start_month.clone(),
            end_month: challenge.end_month.clone(),
            mode: challenge_sync_mode(challenge.sync_mode.as_deref()),
        })
        .await
        .expect("execute delegated sync");

    let plant = db
        .get_plant_by_facility_code("FAC-OWNER-001")
        .await
        .expect("reload plant");
    assert_eq!(plant.owner_organization_id, "owner-org");
    let binding = db
        .get_kr_rps_registry_project_binding("FAC-OWNER-001")
        .await
        .expect("load registry project binding");
    assert_eq!(binding.asset_owner_organization_id, "owner-org");
    assert_eq!(
        binding.registry_account_organization_id.as_deref(),
        Some("owner-org")
    );
    assert_eq!(
        binding.registry_account_external_account_id.as_deref(),
        Some("1234567890")
    );
}

#[tokio::test]
async fn delegated_first_sync_does_not_attach_operator_registry_account_to_owner_asset() {
    let (service, db, _db_dir, _artifacts_dir) = build_service().await;

    db.upsert_organization_with_owner("owner-org", "Owner Org", Some("1234567890"), "Owner Org")
        .await
        .expect("owner org");
    db.upsert_organization_with_owner(
        "operator-org",
        "Operator Org",
        Some("2768603291"),
        "Operator Org",
    )
    .await
    .expect("operator org");
    db.upsert_plant(&domain::Plant {
        id: "FAC-FIRST-DELEGATED".to_owned(),
        facility_code: "FAC-FIRST-DELEGATED".to_owned(),
        name: "First Delegated Plant".to_owned(),
        owner_organization_id: "owner-org".to_owned(),
        created_by_organization_id: Some("operator-org".to_owned()),
        contract_no: "CN-FIRST-DELEGATED".to_owned(),
        address: "Seoul".to_owned(),
        installed_kw: 100.0,
        rec_start_date: "2026-01-01".to_owned(),
        confirm_issued: "Y".to_owned(),
    })
    .await
    .expect("upsert existing owner plant");
    db.upsert_role(&Role {
        id: "role-first-delegated-sync-execute".to_owned(),
        organization_id: None,
        code: "first.delegated.sync.execute".to_owned(),
        name: "First Delegated Sync Execute".to_owned(),
        description: None,
    })
    .await
    .expect("upsert sync role");
    db.upsert_role_action("role-first-delegated-sync-execute", "plant.sync.execute")
        .await
        .expect("upsert sync action");
    db.upsert_agreement(&Agreement {
        id: "agreement-first-delegated-sync".to_owned(),
        agreement_type: AgreementType::Delegation,
        grantor_organization_id: "owner-org".to_owned(),
        grantee_organization_id: "operator-org".to_owned(),
        status: AgreementStatus::Active,
        title: "First Delegated Sync".to_owned(),
        reference_no: None,
        starts_at: None,
        ends_at: None,
    })
    .await
    .expect("upsert agreement");
    for (subject_type, subject_id, agreement_id) in [
        (
            RoleBindingSubjectType::Organization,
            "operator-org",
            Some("agreement-first-delegated-sync"),
        ),
        (RoleBindingSubjectType::User, "bootstrap-user", None),
    ] {
        db.save_role_binding(&RoleBinding {
            id: Uuid::new_v4().simple().to_string(),
            subject_type,
            subject_id: subject_id.to_owned(),
            subject_organization_id: "operator-org".to_owned(),
            resource_type: RoleBindingResourceType::Plant,
            resource_id: "FAC-FIRST-DELEGATED".to_owned(),
            role_id: "role-first-delegated-sync-execute".to_owned(),
            agreement_id: agreement_id.map(ToOwned::to_owned),
            state: RoleBindingState::Active,
            created_by_user_id: "bootstrap-user".to_owned(),
            created_at: "2026-04-25T00:00:00Z".to_owned(),
            updated_at: "2026-04-25T00:00:00Z".to_owned(),
        })
        .await
        .expect("save sync binding");
    }

    let challenge = service
        .create_cert_gateway_challenge(CreateCertGatewayChallengeInput {
            organization_id: "operator-org".to_owned(),
            mode: "ephemeral".to_owned(),
            sync_mode: None,
            origin: None,
            business_no: None,
            facility_code: Some("FAC-FIRST-DELEGATED".to_owned()),
            start_month: None,
            end_month: None,
            sign_cert_sha256_hex: None,
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect("create challenge");
    let (bundle, signature) = fake_bundle_and_signature(&challenge.payload);

    service
        .execute_ephemeral_crawl(ExecuteEphemeralCrawlInput {
            organization_id: "operator-org".to_owned(),
            requested_by_user_id: Some("bootstrap-user".to_owned()),
            challenge_id: challenge.id,
            challenge_payload: challenge.payload,
            challenge_signature_base64: signature,
            bundle,
            facility_code: Some("FAC-FIRST-DELEGATED".to_owned()),
            start_month: challenge.start_month.clone(),
            end_month: challenge.end_month.clone(),
            mode: challenge_sync_mode(challenge.sync_mode.as_deref()),
        })
        .await
        .expect("execute delegated sync");

    let binding = db
        .get_kr_rps_registry_project_binding("FAC-FIRST-DELEGATED")
        .await
        .expect("load registry project binding");
    assert_eq!(binding.asset_owner_organization_id, "owner-org");
    assert_eq!(binding.registry_account_organization_id, None);
    assert_eq!(binding.registry_account_external_account_id, None);
}

#[tokio::test]
async fn sync_created_plant_bootstraps_creator_asset_permissions() {
    let (service, db, _db_dir, _artifacts_dir) = build_service().await;

    let challenge = service
        .create_cert_gateway_challenge(CreateCertGatewayChallengeInput {
            organization_id: "bootstrap-user".to_owned(),
            mode: "ephemeral".to_owned(),
            sync_mode: None,
            origin: None,
            business_no: None,
            facility_code: Some("FAC-SYNC-CREATED".to_owned()),
            start_month: None,
            end_month: None,
            sign_cert_sha256_hex: None,
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect("create challenge");
    let (bundle, signature) = fake_bundle_and_signature(&challenge.payload);
    service
        .execute_ephemeral_crawl(ExecuteEphemeralCrawlInput {
            organization_id: "bootstrap-user".to_owned(),
            requested_by_user_id: Some("bootstrap-user".to_owned()),
            challenge_id: challenge.id,
            challenge_payload: challenge.payload,
            challenge_signature_base64: signature,
            bundle,
            facility_code: Some("FAC-SYNC-CREATED".to_owned()),
            start_month: challenge.start_month.clone(),
            end_month: challenge.end_month.clone(),
            mode: challenge_sync_mode(challenge.sync_mode.as_deref()),
        })
        .await
        .expect("execute first sync");

    service
        .get_plant_profile(
            "bootstrap-user",
            "bootstrap-user",
            "FAC-SYNC-CREATED",
            Some(ReadConsistency::Strong),
        )
        .await
        .expect("creator can read sync-created plant");
    let plant = db
        .get_plant_by_facility_code("FAC-SYNC-CREATED")
        .await
        .expect("sync-created plant");
    assert_ne!(plant.id, plant.facility_code);
    let summary = db
        .get_plant_summary_by_id_with_consistency(&plant.id, Some(ReadConsistency::Strong))
        .await
        .expect("sync-created summary");
    assert_eq!(summary.generation_count, 1);
    assert_eq!(summary.rates_count, 1);
    assert_eq!(summary.rec_count, 1);
    service
        .authorize_gateway_sync_scope("bootstrap-user", "bootstrap-user", Some("FAC-SYNC-CREATED"))
        .await
        .expect("creator can resync sync-created plant");
}

#[tokio::test]
async fn delta_sync_uses_internal_plant_id_for_latest_generation_baseline() {
    let captured_requests = Arc::new(Mutex::new(Vec::new()));
    let collector_requests = Arc::clone(&captured_requests);
    let (service, db, _db_dir, _artifacts_dir) = build_service_with_collector(move |opts| {
        let collector_requests = Arc::clone(&collector_requests);
        async move {
            collector_requests.lock().expect("capture request").push((
                opts.facility_code.clone(),
                opts.start_month.clone(),
                opts.end_month.clone(),
                opts.skip_documents,
            ));
            mock_collect_snapshot(opts).await
        }
    })
    .await;

    let first_challenge = service
        .create_cert_gateway_challenge(CreateCertGatewayChallengeInput {
            organization_id: "bootstrap-user".to_owned(),
            mode: "ephemeral".to_owned(),
            sync_mode: None,
            origin: None,
            business_no: None,
            facility_code: Some("FAC-DELTA-ID".to_owned()),
            start_month: None,
            end_month: None,
            sign_cert_sha256_hex: None,
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect("create first challenge");
    let (first_bundle, first_signature) = fake_bundle_and_signature(&first_challenge.payload);
    service
        .execute_ephemeral_crawl(ExecuteEphemeralCrawlInput {
            organization_id: "bootstrap-user".to_owned(),
            requested_by_user_id: Some("bootstrap-user".to_owned()),
            challenge_id: first_challenge.id,
            challenge_payload: first_challenge.payload,
            challenge_signature_base64: first_signature,
            bundle: first_bundle,
            facility_code: Some("FAC-DELTA-ID".to_owned()),
            start_month: first_challenge.start_month.clone(),
            end_month: first_challenge.end_month.clone(),
            mode: challenge_sync_mode(first_challenge.sync_mode.as_deref()),
        })
        .await
        .expect("first delta falls back to full sync");

    let plant = db
        .get_plant_by_facility_code("FAC-DELTA-ID")
        .await
        .expect("materialized plant");
    assert_ne!(plant.id, plant.facility_code);

    let second_challenge = service
        .create_cert_gateway_challenge(CreateCertGatewayChallengeInput {
            organization_id: "bootstrap-user".to_owned(),
            mode: "ephemeral".to_owned(),
            sync_mode: None,
            origin: None,
            business_no: None,
            facility_code: Some("FAC-DELTA-ID".to_owned()),
            start_month: None,
            end_month: None,
            sign_cert_sha256_hex: None,
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect("create second challenge");
    let (second_bundle, second_signature) = fake_bundle_and_signature(&second_challenge.payload);
    service
        .execute_ephemeral_crawl(ExecuteEphemeralCrawlInput {
            organization_id: "bootstrap-user".to_owned(),
            requested_by_user_id: Some("bootstrap-user".to_owned()),
            challenge_id: second_challenge.id,
            challenge_payload: second_challenge.payload,
            challenge_signature_base64: second_signature,
            bundle: second_bundle,
            facility_code: Some("FAC-DELTA-ID".to_owned()),
            start_month: second_challenge.start_month.clone(),
            end_month: second_challenge.end_month.clone(),
            mode: challenge_sync_mode(second_challenge.sync_mode.as_deref()),
        })
        .await
        .expect("second delta uses existing generation baseline");

    let requests = captured_requests.lock().expect("captured requests");
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].1, None);
    assert!(!requests[0].3);
    assert_eq!(requests[1].0.as_deref(), Some("FAC-DELTA-ID"));
    assert_eq!(requests[1].1.as_deref(), Some("2026-03"));
    assert!(requests[1].2.is_some());
    assert!(requests[1].3);
}

#[tokio::test]
async fn stored_certificate_registration_rejects_requester_different_from_challenge_creator() {
    let (service, db, _db_dir, _artifacts_dir) = build_service().await;

    let challenge = service
        .create_cert_gateway_challenge(CreateCertGatewayChallengeInput {
            organization_id: "bootstrap-user".to_owned(),
            mode: "stored".to_owned(),
            sync_mode: None,
            origin: None,
            business_no: None,
            facility_code: None,
            start_month: None,
            end_month: None,
            sign_cert_sha256_hex: None,
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect("create stored challenge");
    let (bundle, signature) = fake_bundle_and_signature(&challenge.payload);
    let err = service
        .register_certificate_from_proof(RegisterCertificateFromProofInput {
            organization_id: "bootstrap-user".to_owned(),
            requested_by_user_id: Some("other-user".to_owned()),
            challenge_id: challenge.id.clone(),
            challenge_payload: challenge.payload.clone(),
            challenge_signature_base64: signature,
            bundle,
        })
        .await
        .expect_err("challenge requester mismatch must be rejected");
    assert!(err.to_string().contains("challenge creator"));
    let challenge = db
        .get_cert_gateway_challenge(&challenge.id)
        .await
        .expect("challenge after requester mismatch");
    assert!(challenge.consumed_at.is_none());
}

#[tokio::test]
async fn stored_certificate_challenge_binds_facility_scope_at_creation() {
    let (service, db, _db_dir, _artifacts_dir) = build_service().await;

    let challenge = service
        .create_cert_gateway_challenge(CreateCertGatewayChallengeInput {
            organization_id: "bootstrap-user".to_owned(),
            mode: "stored".to_owned(),
            sync_mode: None,
            origin: None,
            business_no: None,
            facility_code: Some("FAC-STORED-SCOPE".to_owned()),
            start_month: None,
            end_month: None,
            sign_cert_sha256_hex: None,
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect("stored challenge accepts facility scope");

    let stored = db
        .get_cert_gateway_challenge(&challenge.id)
        .await
        .expect("stored challenge");
    assert_eq!(stored.facility_code.as_deref(), Some("FAC-STORED-SCOPE"));
}

#[tokio::test]
async fn stored_certificate_challenge_rejects_blank_facility_scope_at_creation() {
    let (service, _db, _db_dir, _artifacts_dir) = build_service().await;

    let err = service
        .create_cert_gateway_challenge(CreateCertGatewayChallengeInput {
            organization_id: "bootstrap-user".to_owned(),
            mode: "stored".to_owned(),
            sync_mode: None,
            origin: None,
            business_no: None,
            facility_code: Some("   ".to_owned()),
            start_month: None,
            end_month: None,
            sign_cert_sha256_hex: None,
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect_err("blank facility scope should be rejected");

    assert!(err.to_string().contains("facilityCode must not be empty"));
    assert_eq!(
        err.downcast_ref::<ServiceError>().map(ServiceError::kind),
        Some(ServiceErrorKind::BadRequest)
    );
}

#[tokio::test]
async fn full_sync_rejects_bounded_month_range() {
    let (service, db, _db_dir, _artifacts_dir) = build_service().await;

    let challenge = service
        .create_cert_gateway_challenge(CreateCertGatewayChallengeInput {
            organization_id: "bootstrap-user".to_owned(),
            mode: "ephemeral".to_owned(),
            sync_mode: None,
            origin: None,
            business_no: None,
            facility_code: Some("FAC-FULL-RANGE".to_owned()),
            start_month: None,
            end_month: None,
            sign_cert_sha256_hex: None,
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect("create challenge");
    let (bundle, signature) = fake_bundle_and_signature(&challenge.payload);

    let err = service
        .execute_ephemeral_crawl(ExecuteEphemeralCrawlInput {
            organization_id: "bootstrap-user".to_owned(),
            requested_by_user_id: Some("bootstrap-user".to_owned()),
            challenge_id: challenge.id.clone(),
            challenge_payload: challenge.payload.clone(),
            challenge_signature_base64: signature,
            bundle,
            facility_code: Some("FAC-FULL-RANGE".to_owned()),
            start_month: Some("2026-01".to_owned()),
            end_month: Some("2026-03".to_owned()),
            mode: SyncMode::Full,
        })
        .await
        .expect_err("full sync range must be rejected");
    assert!(
        err.to_string()
            .contains("full sync cannot be bounded by startMonth or endMonth")
    );
    let challenge = db
        .get_cert_gateway_challenge(&challenge.id)
        .await
        .expect("challenge after invalid full sync request");
    assert!(challenge.consumed_at.is_none());
}

#[tokio::test]
async fn issue_crawl_task_token_requires_created_by_user_scope_authorization() {
    let (service, db, _db_dir, _artifacts_dir) = build_service().await;

    let challenge = service
        .create_cert_gateway_challenge(CreateCertGatewayChallengeInput {
            organization_id: "bootstrap-user".to_owned(),
            mode: "stored".to_owned(),
            sync_mode: None,
            origin: None,
            business_no: None,
            facility_code: None,
            start_month: None,
            end_month: None,
            sign_cert_sha256_hex: None,
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect("create challenge");
    let (bundle, signature) = fake_bundle_and_signature(&challenge.payload);
    let cert = service
        .register_certificate_from_proof(RegisterCertificateFromProofInput {
            organization_id: "bootstrap-user".to_owned(),
            requested_by_user_id: Some("bootstrap-user".to_owned()),
            challenge_id: challenge.id,
            challenge_payload: challenge.payload,
            challenge_signature_base64: signature,
            bundle,
        })
        .await
        .expect("register certificate");

    db.upsert_plant(&domain::Plant {
        id: "FAC-AUTHZ-TOKEN".to_owned(),
        facility_code: "FAC-AUTHZ-TOKEN".to_owned(),
        name: "Token Authz Plant".to_owned(),
        owner_organization_id: "bootstrap-user".to_owned(),
        created_by_organization_id: Some("bootstrap-user".to_owned()),
        contract_no: "CN-AUTHZ-TOKEN".to_owned(),
        address: "Seoul".to_owned(),
        installed_kw: 100.0,
        rec_start_date: "2026-01-01".to_owned(),
        confirm_issued: "Y".to_owned(),
    })
    .await
    .expect("upsert plant");

    let err = service
        .issue_crawl_task_token(IssueCrawlTaskTokenInput {
            organization_id: "bootstrap-user".to_owned(),
            certificate_id: cert.id,
            facility_code: Some("FAC-AUTHZ-TOKEN".to_owned()),
            start_month: Some("2026-01".to_owned()),
            end_month: Some("2026-02".to_owned()),
            mode: SyncMode::Delta,
            created_by_user_id: "missing-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect_err("unauthorized token creator must not mint crawl token");
    assert!(err.to_string().contains("forbidden"));
}

#[tokio::test]
async fn ephemeral_crawl_rejects_facility_different_from_signed_challenge_scope() {
    let (service, db, _db_dir, _artifacts_dir) = build_service().await;

    let challenge = service
        .create_cert_gateway_challenge(CreateCertGatewayChallengeInput {
            organization_id: "bootstrap-user".to_owned(),
            mode: "ephemeral".to_owned(),
            sync_mode: None,
            origin: None,
            business_no: None,
            facility_code: Some("FAC-SIGNED".to_owned()),
            start_month: None,
            end_month: None,
            sign_cert_sha256_hex: None,
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect("create challenge");
    let (bundle, signature) = fake_bundle_and_signature(&challenge.payload);

    let err = service
        .execute_ephemeral_crawl(ExecuteEphemeralCrawlInput {
            organization_id: "bootstrap-user".to_owned(),
            requested_by_user_id: Some("bootstrap-user".to_owned()),
            challenge_id: challenge.id.clone(),
            challenge_payload: challenge.payload.clone(),
            challenge_signature_base64: signature,
            bundle,
            facility_code: Some("FAC-OTHER".to_owned()),
            start_month: challenge.start_month.clone(),
            end_month: challenge.end_month.clone(),
            mode: challenge_sync_mode(challenge.sync_mode.as_deref()),
        })
        .await
        .expect_err("challenge scope must be authoritative");
    assert!(err.to_string().contains("challenge facility code mismatch"));
    let challenge = db
        .get_cert_gateway_challenge(&challenge.id)
        .await
        .expect("challenge after facility mismatch");
    assert!(challenge.consumed_at.is_none());
}

#[tokio::test]
async fn ephemeral_crawl_normalizes_blank_requested_facility_scope() {
    let (service, _db, _db_dir, _artifacts_dir) = build_service().await;

    let challenge = service
        .create_cert_gateway_challenge(CreateCertGatewayChallengeInput {
            organization_id: "bootstrap-user".to_owned(),
            mode: "ephemeral".to_owned(),
            sync_mode: None,
            origin: None,
            business_no: None,
            facility_code: None,
            start_month: None,
            end_month: None,
            sign_cert_sha256_hex: None,
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect("create challenge");
    let (bundle, signature) = fake_bundle_and_signature(&challenge.payload);

    let run = service
        .execute_ephemeral_crawl(ExecuteEphemeralCrawlInput {
            organization_id: "bootstrap-user".to_owned(),
            requested_by_user_id: Some("bootstrap-user".to_owned()),
            challenge_id: challenge.id,
            challenge_payload: challenge.payload,
            challenge_signature_base64: signature,
            bundle,
            facility_code: Some("   ".to_owned()),
            start_month: challenge.start_month.clone(),
            end_month: challenge.end_month.clone(),
            mode: challenge_sync_mode(challenge.sync_mode.as_deref()),
        })
        .await
        .expect("blank requested facility scope should be treated as unscoped");

    assert_eq!(run.status, "completed");
}

#[tokio::test]
async fn ephemeral_crawl_rejects_binding_unscoped_challenge_to_request_facility() {
    let (service, db, _db_dir, _artifacts_dir) = build_service().await;

    let challenge = service
        .create_cert_gateway_challenge(CreateCertGatewayChallengeInput {
            organization_id: "bootstrap-user".to_owned(),
            mode: "ephemeral".to_owned(),
            sync_mode: None,
            origin: None,
            business_no: None,
            facility_code: None,
            start_month: None,
            end_month: None,
            sign_cert_sha256_hex: None,
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect("create challenge");
    let (bundle, signature) = fake_bundle_and_signature(&challenge.payload);

    let err = service
        .execute_ephemeral_crawl(ExecuteEphemeralCrawlInput {
            organization_id: "bootstrap-user".to_owned(),
            requested_by_user_id: Some("bootstrap-user".to_owned()),
            challenge_id: challenge.id.clone(),
            challenge_payload: challenge.payload.clone(),
            challenge_signature_base64: signature,
            bundle,
            facility_code: Some("FAC-OTHER".to_owned()),
            start_month: challenge.start_month.clone(),
            end_month: challenge.end_month.clone(),
            mode: challenge_sync_mode(challenge.sync_mode.as_deref()),
        })
        .await
        .expect_err("unscoped challenge must not be rebound to a facility");
    assert!(err.to_string().contains("challenge facilityCode mismatch"));
    let challenge = db
        .get_cert_gateway_challenge(&challenge.id)
        .await
        .expect("challenge after unscoped facility binding attempt");
    assert!(challenge.consumed_at.is_none());
}

#[tokio::test]
async fn ephemeral_crawl_rejects_requester_different_from_challenge_creator() {
    let (service, db, _db_dir, _artifacts_dir) = build_service().await;

    let challenge = service
        .create_cert_gateway_challenge(CreateCertGatewayChallengeInput {
            organization_id: "bootstrap-user".to_owned(),
            mode: "ephemeral".to_owned(),
            sync_mode: None,
            origin: None,
            business_no: None,
            facility_code: Some("FAC-REQUESTER-MISMATCH".to_owned()),
            start_month: None,
            end_month: None,
            sign_cert_sha256_hex: None,
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect("create challenge");
    let (bundle, signature) = fake_bundle_and_signature(&challenge.payload);

    let err = service
        .execute_ephemeral_crawl(ExecuteEphemeralCrawlInput {
            organization_id: "bootstrap-user".to_owned(),
            requested_by_user_id: Some("other-user".to_owned()),
            challenge_id: challenge.id.clone(),
            challenge_payload: challenge.payload.clone(),
            challenge_signature_base64: signature,
            bundle,
            facility_code: Some("FAC-REQUESTER-MISMATCH".to_owned()),
            start_month: challenge.start_month.clone(),
            end_month: challenge.end_month.clone(),
            mode: challenge_sync_mode(challenge.sync_mode.as_deref()),
        })
        .await
        .expect_err("requester must match challenge creator");
    assert!(err.to_string().contains("challenge creator"));
    let challenge = db
        .get_cert_gateway_challenge(&challenge.id)
        .await
        .expect("challenge after requester mismatch");
    assert!(challenge.consumed_at.is_none());
}

#[tokio::test]
async fn requires_explicit_contract_backed_access_for_each_organization() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "bootstrap-user".to_owned(),
        "Bootstrap User".to_owned(),
    );
    db.ensure_schema().await.expect("ensure schema");

    db.upsert_organization_with_owner("generator-org", "Generator Org", None, "Generator Org")
        .await
        .expect("upsert generator org");
    db.upsert_organization_with_owner("dealer-org", "Dealer Org", None, "Dealer Org")
        .await
        .expect("upsert dealer org");
    db.upsert_organization_with_owner(
        "distributor-org",
        "Distributor Org",
        None,
        "Distributor Org",
    )
    .await
    .expect("upsert distributor org");

    runtime
        .execute(
            "INSERT INTO users (id, email, display_name, active, created_at)
             VALUES ($1, $2, $3, $4, $5)",
            vec![
                "dealer-user".into(),
                "dist@example.com".into(),
                "Dealer User".into(),
                1_i64.into(),
                "2026-04-03T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert user");
    runtime
        .execute(
            "INSERT INTO organization_memberships (id, organization_id, user_id, membership_role, status, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            vec![
                "membership-dealer-user".into(),
                "dealer-org".into(),
                "dealer-user".into(),
                "member".into(),
                "active".into(),
                "2026-04-03T00:00:00Z".into(),
                "2026-04-03T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert membership");
    runtime
        .execute(
            "INSERT INTO users (id, email, display_name, active, created_at)
             VALUES ($1, $2, $3, $4, $5)",
            vec![
                "dist-user".into(),
                "dist-user@example.com".into(),
                "Distributor User".into(),
                1_i64.into(),
                "2026-04-03T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert distributor user");
    runtime
        .execute(
            "INSERT INTO organization_memberships (id, organization_id, user_id, membership_role, status, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            vec![
                "membership-dist-user".into(),
                "distributor-org".into(),
                "dist-user".into(),
                "member".into(),
                "active".into(),
                "2026-04-03T00:00:00Z".into(),
                "2026-04-03T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert distributor membership");
    runtime
        .execute(
            "INSERT INTO groups (id, organization_id, name, description, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6)",
            vec![
                "dealer-ops".into(),
                "dealer-org".into(),
                "Dealer Ops".into(),
                Option::<String>::None.into(),
                "2026-04-03T00:00:00Z".into(),
                "2026-04-03T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert group");
    runtime
        .execute(
            "INSERT INTO group_memberships (group_id, organization_membership_id)
             VALUES ($1, $2)",
            vec!["dealer-ops".into(), "membership-dealer-user".into()],
        )
        .await
        .expect("insert group membership");

    runtime
        .execute(
            "INSERT INTO plants (id, facility_code, owner_organization_id, created_by_organization_id, name, contract_no, address, installed_kw, rec_start_date, confirm_issued, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
            vec![
                "plant-1".into(),
                "FAC-001".into(),
                "generator-org".into(),
                Option::<String>::Some("dealer-org".to_owned()).into(),
                "Plant 1".into(),
                "CN-001".into(),
                "Seoul".into(),
                100.0_f64.into(),
                "2026-01-01".into(),
                "Y".into(),
                "2026-04-03T00:00:00Z".into(),
                "2026-04-03T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert plant");

    db.upsert_role(&Role {
        id: "role-plant-profile-reader".to_owned(),
        organization_id: None,
        code: "plant.profile.reader".to_owned(),
        name: "Plant Profile Reader".to_owned(),
        description: None,
    })
    .await
    .expect("upsert role");
    db.upsert_role_action("role-plant-profile-reader", "plant.profile.read")
        .await
        .expect("upsert role action");

    let agreement = Agreement {
        id: "agreement-1".to_owned(),
        agreement_type: AgreementType::Delegation,
        grantor_organization_id: "generator-org".to_owned(),
        grantee_organization_id: "dealer-org".to_owned(),
        status: AgreementStatus::Active,
        title: "Generator to Dealer".to_owned(),
        reference_no: None,
        starts_at: None,
        ends_at: None,
    };
    db.upsert_agreement(&agreement)
        .await
        .expect("upsert agreement");

    db.save_role_binding(&RoleBinding {
        id: Uuid::new_v4().simple().to_string(),
        subject_type: RoleBindingSubjectType::Organization,
        subject_id: "dealer-org".to_owned(),
        subject_organization_id: "dealer-org".to_owned(),
        resource_type: RoleBindingResourceType::Plant,
        resource_id: "plant-1".to_owned(),
        role_id: "role-plant-profile-reader".to_owned(),
        agreement_id: Some("agreement-1".to_owned()),
        state: RoleBindingState::Active,
        created_by_user_id: "bootstrap-user".to_owned(),
        created_at: "2026-04-03T00:00:00Z".to_owned(),
        updated_at: "2026-04-03T00:00:00Z".to_owned(),
    })
    .await
    .expect("save org role binding");

    let distributor_agreement = Agreement {
        id: "agreement-2".to_owned(),
        agreement_type: AgreementType::Delegation,
        grantor_organization_id: "generator-org".to_owned(),
        grantee_organization_id: "distributor-org".to_owned(),
        status: AgreementStatus::Active,
        title: "Generator to Distributor".to_owned(),
        reference_no: None,
        starts_at: None,
        ends_at: None,
    };
    db.upsert_agreement(&distributor_agreement)
        .await
        .expect("upsert distributor agreement");

    db.save_role_binding(&RoleBinding {
        id: Uuid::new_v4().simple().to_string(),
        subject_type: RoleBindingSubjectType::Organization,
        subject_id: "distributor-org".to_owned(),
        subject_organization_id: "distributor-org".to_owned(),
        resource_type: RoleBindingResourceType::Plant,
        resource_id: "plant-1".to_owned(),
        role_id: "role-plant-profile-reader".to_owned(),
        agreement_id: Some("agreement-2".to_owned()),
        state: RoleBindingState::Active,
        created_by_user_id: "bootstrap-user".to_owned(),
        created_at: "2026-04-03T00:00:00Z".to_owned(),
        updated_at: "2026-04-03T00:00:00Z".to_owned(),
    })
    .await
    .expect("save distributor org role binding");
    db.save_role_binding(&RoleBinding {
        id: Uuid::new_v4().simple().to_string(),
        subject_type: RoleBindingSubjectType::User,
        subject_id: "dist-user".to_owned(),
        subject_organization_id: "distributor-org".to_owned(),
        resource_type: RoleBindingResourceType::Plant,
        resource_id: "plant-1".to_owned(),
        role_id: "role-plant-profile-reader".to_owned(),
        agreement_id: None,
        state: RoleBindingState::Active,
        created_by_user_id: "bootstrap-user".to_owned(),
        created_at: "2026-04-03T00:00:00Z".to_owned(),
        updated_at: "2026-04-03T00:00:00Z".to_owned(),
    })
    .await
    .expect("save distributor user role binding");

    db.save_role_binding(&RoleBinding {
        id: Uuid::new_v4().simple().to_string(),
        subject_type: RoleBindingSubjectType::User,
        subject_id: "dist-user".to_owned(),
        subject_organization_id: "distributor-org".to_owned(),
        resource_type: RoleBindingResourceType::Plant,
        resource_id: "plant-1".to_owned(),
        role_id: "role-plant-profile-reader".to_owned(),
        agreement_id: None,
        state: RoleBindingState::Active,
        created_by_user_id: "bootstrap-user".to_owned(),
        created_at: "2026-04-03T00:00:00Z".to_owned(),
        updated_at: "2026-04-03T00:00:00Z".to_owned(),
    })
    .await
    .expect("save distributor user role binding");

    db.save_role_binding(&RoleBinding {
        id: Uuid::new_v4().simple().to_string(),
        subject_type: RoleBindingSubjectType::Group,
        subject_id: "dealer-ops".to_owned(),
        subject_organization_id: "dealer-org".to_owned(),
        resource_type: RoleBindingResourceType::Plant,
        resource_id: "plant-1".to_owned(),
        role_id: "role-plant-profile-reader".to_owned(),
        agreement_id: None,
        state: RoleBindingState::Active,
        created_by_user_id: "bootstrap-user".to_owned(),
        created_at: "2026-04-03T00:00:00Z".to_owned(),
        updated_at: "2026-04-03T00:00:00Z".to_owned(),
    })
    .await
    .expect("save group role binding");

    let org_rows: HashSet<OrganizationPlantActionRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT organization_id, plant_id, action_id
             FROM effective_organization_plant_actions
             ORDER BY organization_id ASC, plant_id ASC, action_id ASC",
            vec![],
        )
        .await
        .expect("organization action rows")
        .into_iter()
        .collect();
    assert!(org_rows.contains(&OrganizationPlantActionRow {
        organization_id: "dealer-org".to_owned(),
        plant_id: "plant-1".to_owned(),
        action_id: "plant.profile.read".to_owned(),
    }));
    let user_rows: HashSet<UserPlantActionRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT user_id, organization_id, plant_id, action_id
             FROM effective_user_plant_actions
             ORDER BY user_id ASC, organization_id ASC, plant_id ASC, action_id ASC",
            vec![],
        )
        .await
        .expect("user action rows")
        .into_iter()
        .collect();
    assert!(user_rows.contains(&UserPlantActionRow {
        user_id: "dealer-user".to_owned(),
        organization_id: "dealer-org".to_owned(),
        plant_id: "plant-1".to_owned(),
        action_id: "plant.profile.read".to_owned(),
    }));

    let visibility_rows: HashSet<UserPlantVisibilityRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT user_id, organization_id, plant_id
             FROM effective_user_plant_visibility
             ORDER BY user_id ASC, organization_id ASC, plant_id ASC",
            vec![],
        )
        .await
        .expect("visibility rows")
        .into_iter()
        .collect();
    assert!(visibility_rows.contains(&UserPlantVisibilityRow {
        user_id: "dealer-user".to_owned(),
        organization_id: "dealer-org".to_owned(),
        plant_id: "plant-1".to_owned(),
    }));

    db.upsert_agreement(&Agreement {
        status: AgreementStatus::Terminated,
        ..agreement
    })
    .await
    .expect("terminate dealer agreement");

    let org_rows_after: HashSet<OrganizationPlantActionRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT organization_id, plant_id, action_id
             FROM effective_organization_plant_actions
             WHERE plant_id = $1",
            vec!["plant-1".into()],
        )
        .await
        .expect("organization action rows after dealer termination")
        .into_iter()
        .collect();
    assert!(
        !org_rows_after.contains(&OrganizationPlantActionRow {
            organization_id: "dealer-org".to_owned(),
            plant_id: "plant-1".to_owned(),
            action_id: "plant.profile.read".to_owned(),
        }),
        "dealer access should disappear when its agreement terminates"
    );
    assert!(
        org_rows_after.contains(&OrganizationPlantActionRow {
            organization_id: "distributor-org".to_owned(),
            plant_id: "plant-1".to_owned(),
            action_id: "plant.profile.read".to_owned(),
        }),
        "distributor access should remain while its own agreement is still active"
    );

    let user_rows_after: HashSet<UserPlantActionRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT user_id, organization_id, plant_id, action_id
             FROM effective_user_plant_actions
             WHERE plant_id = $1",
            vec!["plant-1".into()],
        )
        .await
        .expect("user action rows after dealer termination")
        .into_iter()
        .collect();
    assert!(
        user_rows_after.contains(&UserPlantActionRow {
            user_id: "dist-user".to_owned(),
            organization_id: "distributor-org".to_owned(),
            plant_id: "plant-1".to_owned(),
            action_id: "plant.profile.read".to_owned(),
        }),
        "distributor user action should remain while distributor agreement is active"
    );

    let visibility_rows_after: HashSet<UserPlantVisibilityRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT user_id, organization_id, plant_id
             FROM effective_user_plant_visibility
             WHERE plant_id = $1",
            vec!["plant-1".into()],
        )
        .await
        .expect("visibility rows after dealer termination")
        .into_iter()
        .collect();
    assert!(
        visibility_rows_after.contains(&UserPlantVisibilityRow {
            user_id: "dist-user".to_owned(),
            organization_id: "distributor-org".to_owned(),
            plant_id: "plant-1".to_owned(),
        }),
        "visibility should remain while distributor agreement is active"
    );

    db.upsert_agreement(&Agreement {
        status: AgreementStatus::Terminated,
        ..distributor_agreement
    })
    .await
    .expect("terminate distributor agreement");

    let org_rows_final: Vec<OrganizationPlantActionRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT organization_id, plant_id, action_id
             FROM effective_organization_plant_actions
             WHERE plant_id = $1",
            vec!["plant-1".into()],
        )
        .await
        .expect("organization action rows after all terminations");
    assert!(
        org_rows_final.is_empty(),
        "all explicit organization access should disappear after both agreements terminate"
    );

    let user_rows_final: Vec<UserPlantActionRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT user_id, organization_id, plant_id, action_id
             FROM effective_user_plant_actions
             WHERE plant_id = $1",
            vec!["plant-1".into()],
        )
        .await
        .expect("user action rows after all terminations");
    assert!(
        user_rows_final.is_empty(),
        "group and user actions should disappear after the final explicit organization agreement terminates"
    );

    let visibility_rows_final: Vec<UserPlantVisibilityRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT user_id, organization_id, plant_id
             FROM effective_user_plant_visibility
             WHERE plant_id = $1",
            vec!["plant-1".into()],
        )
        .await
        .expect("visibility rows after all terminations");
    assert!(
        visibility_rows_final.is_empty(),
        "visibility should disappear after the final explicit organization agreement terminates"
    );
}

#[tokio::test]
async fn certificate_gateway_service_flows_enforce_expiry_reuse_and_single_use_tokens() {
    let (service, db, _db_dir, _artifacts_dir) = build_service().await;

    let expired = service
        .create_cert_gateway_challenge(CreateCertGatewayChallengeInput {
            organization_id: "bootstrap-user".to_owned(),
            mode: "stored".to_owned(),
            sync_mode: None,
            origin: None,
            business_no: None,
            facility_code: None,
            start_month: None,
            end_month: None,
            sign_cert_sha256_hex: None,
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: -60,
        })
        .await
        .expect("create expired challenge");
    let (expired_bundle, expired_signature) = fake_bundle_and_signature(&expired.payload);
    let err = service
        .register_certificate_from_proof(RegisterCertificateFromProofInput {
            organization_id: "bootstrap-user".to_owned(),
            requested_by_user_id: Some("bootstrap-user".to_owned()),
            challenge_id: expired.id.clone(),
            challenge_payload: expired.payload.clone(),
            challenge_signature_base64: expired_signature,
            bundle: expired_bundle,
        })
        .await
        .expect_err("expired challenge should be rejected");
    assert!(err.to_string().contains("expired"));

    let stored_key =
        PKey::from_rsa(Rsa::generate(2048).expect("generate stored rsa key")).expect("wrap key");
    let stored_cert_der = create_sign_cert_der(&stored_key);
    let stored_sign_cert_sha256_hex = sha256_hex(&stored_cert_der);
    let stored_bundle = fake_bundle_with_sign_material(&stored_key, stored_cert_der);
    let stored = service
        .create_cert_gateway_challenge(CreateCertGatewayChallengeInput {
            organization_id: "bootstrap-user".to_owned(),
            mode: "stored".to_owned(),
            sync_mode: None,
            origin: Some("http://127.0.0.1:4173".to_owned()),
            business_no: Some("2768603291".to_owned()),
            facility_code: Some("FAC-001".to_owned()),
            start_month: Some("2026-01".to_owned()),
            end_month: Some("2026-03".to_owned()),
            sign_cert_sha256_hex: Some(stored_sign_cert_sha256_hex.clone()),
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect("create stored challenge");
    assert_challenge_payload_binds_context(
        &stored.payload,
        "bootstrap-user",
        "stored",
        "http://127.0.0.1:4173",
        "2768603291",
        "FAC-001",
        "2026-01",
        "2026-03",
        &stored_sign_cert_sha256_hex,
    );
    let stored_signature = sign_challenge(&stored_key, &stored.payload);
    let cert = service
        .register_certificate_from_proof(RegisterCertificateFromProofInput {
            organization_id: "bootstrap-user".to_owned(),
            requested_by_user_id: Some("bootstrap-user".to_owned()),
            challenge_id: stored.id.clone(),
            challenge_payload: stored.payload.clone(),
            challenge_signature_base64: stored_signature.clone(),
            bundle: stored_bundle.clone(),
        })
        .await
        .expect("register certificate");
    assert_eq!(cert.kms_key_id, "local-dev-v1");
    assert!(!cert.wrapped_dek.is_empty());

    let reuse_err = service
        .register_certificate_from_proof(RegisterCertificateFromProofInput {
            organization_id: "bootstrap-user".to_owned(),
            requested_by_user_id: Some("bootstrap-user".to_owned()),
            challenge_id: stored.id.clone(),
            challenge_payload: stored.payload.clone(),
            challenge_signature_base64: stored_signature,
            bundle: stored_bundle,
        })
        .await
        .expect_err("reused challenge should fail");
    assert!(reuse_err.to_string().contains("consumed"));

    let ephemeral = service
        .create_cert_gateway_challenge(CreateCertGatewayChallengeInput {
            organization_id: "bootstrap-user".to_owned(),
            mode: "ephemeral".to_owned(),
            sync_mode: None,
            origin: None,
            business_no: None,
            facility_code: Some("FAC-002".to_owned()),
            start_month: None,
            end_month: None,
            sign_cert_sha256_hex: None,
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect("create ephemeral challenge");
    let (ephemeral_bundle, ephemeral_signature) = fake_bundle_and_signature(&ephemeral.payload);
    let run = service
        .execute_ephemeral_crawl(ExecuteEphemeralCrawlInput {
            organization_id: "bootstrap-user".to_owned(),
            requested_by_user_id: Some("bootstrap-user".to_owned()),
            challenge_id: ephemeral.id.clone(),
            challenge_payload: ephemeral.payload.clone(),
            challenge_signature_base64: ephemeral_signature,
            bundle: ephemeral_bundle,
            facility_code: Some("FAC-002".to_owned()),
            start_month: None,
            end_month: None,
            mode: SyncMode::Full,
        })
        .await
        .expect("execute ephemeral crawl");
    assert_eq!(run.status, "completed");
    assert!(
        run.plant_id
            .as_deref()
            .is_some_and(|plant_id| plant_id.starts_with("plant-"))
    );
    assert_eq!(
        db.get_plant_by_facility_code("FAC-002")
            .await
            .expect("ephemeral plant")
            .facility_code,
        "FAC-002"
    );
    let asset_id = db
        .get_kr_rps_asset_id_by_facility_code_with_consistency("FAC-002", None)
        .await
        .expect("ephemeral KR RPS asset profile");
    assert_ne!(asset_id, "FAC-002");
    let projection_counts = db
        .kr_rps_projection_counts("FAC-002")
        .await
        .expect("ephemeral KR RPS projection counts");
    assert_eq!(projection_counts.asset_profiles, 1);
    assert_eq!(projection_counts.registry_projects, 1);
    assert_eq!(projection_counts.data_sources, 2);
    assert_eq!(projection_counts.generation_data, 1);
    assert_eq!(projection_counts.rate_data, 4);
    assert_eq!(projection_counts.energy_certificates, 1);

    let implicit_ephemeral = service
        .create_cert_gateway_challenge(CreateCertGatewayChallengeInput {
            organization_id: "bootstrap-user".to_owned(),
            mode: "ephemeral".to_owned(),
            sync_mode: None,
            origin: None,
            business_no: None,
            facility_code: None,
            start_month: None,
            end_month: None,
            sign_cert_sha256_hex: None,
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect("create implicit ephemeral challenge");
    let (implicit_bundle, implicit_signature) =
        fake_bundle_and_signature(&implicit_ephemeral.payload);
    let implicit_run = service
        .execute_ephemeral_crawl(ExecuteEphemeralCrawlInput {
            organization_id: "bootstrap-user".to_owned(),
            requested_by_user_id: Some("bootstrap-user".to_owned()),
            challenge_id: implicit_ephemeral.id,
            challenge_payload: implicit_ephemeral.payload,
            challenge_signature_base64: implicit_signature,
            bundle: implicit_bundle,
            facility_code: implicit_ephemeral.facility_code.clone(),
            start_month: implicit_ephemeral.start_month.clone(),
            end_month: implicit_ephemeral.end_month.clone(),
            mode: challenge_sync_mode(implicit_ephemeral.sync_mode.as_deref()),
        })
        .await
        .expect("execute implicit facility ephemeral crawl");
    let implicit_plant_id = implicit_run
        .plant_id
        .as_deref()
        .expect("implicit ephemeral plant id");
    assert_eq!(
        db.get_plant_by_id_with_consistency(implicit_plant_id, Some(ReadConsistency::Strong))
            .await
            .expect("implicit ephemeral plant")
            .facility_code,
        "FAC-002"
    );

    let implicit_token = service
        .issue_crawl_task_token(IssueCrawlTaskTokenInput {
            organization_id: "bootstrap-user".to_owned(),
            certificate_id: cert.id.clone(),
            facility_code: None,
            start_month: Some("2026-01".to_owned()),
            end_month: Some("2026-03".to_owned()),
            mode: SyncMode::Delta,
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect("issue implicit facility crawl task token");
    let implicit_started = service
        .consume_crawl_task_token_and_start_crawl(StartCrawlWithTaskTokenInput {
            organization_id: "bootstrap-user".to_owned(),
            token: implicit_token.token,
        })
        .await
        .expect("consume implicit facility crawl task token");
    let implicit_started_plant_id = implicit_started
        .plant_id
        .as_deref()
        .expect("implicit token plant id");
    assert_eq!(
        db.get_plant_by_id_with_consistency(
            implicit_started_plant_id,
            Some(ReadConsistency::Strong)
        )
        .await
        .expect("implicit token plant")
        .facility_code,
        "FAC-002"
    );

    let invalid_full_token = service
        .issue_crawl_task_token(IssueCrawlTaskTokenInput {
            organization_id: "bootstrap-user".to_owned(),
            certificate_id: cert.id.clone(),
            facility_code: Some("FAC-002".to_owned()),
            start_month: Some("2026-01".to_owned()),
            end_month: Some("2026-03".to_owned()),
            mode: SyncMode::Full,
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect_err("bounded full sync crawl token must be rejected before issue");
    assert!(
        invalid_full_token
            .to_string()
            .contains("full sync cannot be bounded by startMonth or endMonth")
    );

    let issued = service
        .issue_crawl_task_token(IssueCrawlTaskTokenInput {
            organization_id: "bootstrap-user".to_owned(),
            certificate_id: cert.id.clone(),
            facility_code: Some("FAC-003".to_owned()),
            start_month: None,
            end_month: None,
            mode: SyncMode::Full,
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect("issue crawl task token");
    let started = service
        .consume_crawl_task_token_and_start_crawl(StartCrawlWithTaskTokenInput {
            organization_id: "bootstrap-user".to_owned(),
            token: issued.token.clone(),
        })
        .await
        .expect("consume crawl task token");
    assert_eq!(started.status, "completed");
    assert_eq!(started.mode, SyncMode::Full);
    assert!(
        started
            .plant_id
            .as_deref()
            .is_some_and(|plant_id| plant_id.starts_with("plant-"))
    );
    let full_projection_counts = db
        .kr_rps_projection_counts("FAC-003")
        .await
        .expect("stored KR RPS projection counts");
    assert_eq!(full_projection_counts.asset_profiles, 1);
    assert_eq!(full_projection_counts.registry_projects, 1);
    assert_eq!(full_projection_counts.data_sources, 2);
    assert_eq!(full_projection_counts.generation_data, 1);
    assert_eq!(full_projection_counts.rate_data, 4);
    assert_eq!(full_projection_counts.energy_certificates, 1);
    let used_certificate = db
        .get_certificate_by_id(&cert.id)
        .await
        .expect("reload used certificate");
    assert!(
        used_certificate.last_used_at.is_some(),
        "crawl task token flow should stamp certificate usage"
    );

    let token_reuse = service
        .consume_crawl_task_token_and_start_crawl(StartCrawlWithTaskTokenInput {
            organization_id: "bootstrap-user".to_owned(),
            token: issued.token,
        })
        .await
        .expect_err("reused crawl task token should fail");
    assert!(token_reuse.to_string().contains("already used"));
}

#[tokio::test]
async fn certificate_gateway_rejects_expired_sign_certificate_and_extracts_common_name() {
    let (service, _db, _db_dir, _artifacts_dir) = build_service().await;

    let expired = service
        .create_cert_gateway_challenge(CreateCertGatewayChallengeInput {
            organization_id: "bootstrap-user".to_owned(),
            mode: "stored".to_owned(),
            sync_mode: None,
            origin: None,
            business_no: None,
            facility_code: None,
            start_month: None,
            end_month: None,
            sign_cert_sha256_hex: None,
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect("create challenge");
    let expired_key =
        PKey::from_rsa(Rsa::generate(2048).expect("generate expired rsa key")).expect("wrap key");
    let expired_cert_der = create_expired_sign_cert_der(&expired_key, "Expired Certificate");
    let expired_signature = sign_challenge(&expired_key, &expired.payload);
    let expired_err = service
        .register_certificate_from_proof(RegisterCertificateFromProofInput {
            organization_id: "bootstrap-user".to_owned(),
            requested_by_user_id: Some("bootstrap-user".to_owned()),
            challenge_id: expired.id.clone(),
            challenge_payload: expired.payload.clone(),
            challenge_signature_base64: expired_signature,
            bundle: CertificateBundle {
                business_no: "2768603291".to_owned(),
                password: "secret-password".to_owned(),
                common_name: "Client Spoofed Name".to_owned(),
                sign_cert_der: expired_cert_der,
                sign_pri_key: direct_private_key_der(&expired_key),
                km_cert_der: None,
                km_pri_key: None,
                certificate_chain_der: Vec::new(),
            },
        })
        .await
        .expect_err("expired certificate should be rejected");
    assert!(expired_err.to_string().contains("expired"));

    let stored = service
        .create_cert_gateway_challenge(CreateCertGatewayChallengeInput {
            organization_id: "bootstrap-user".to_owned(),
            mode: "stored".to_owned(),
            sync_mode: None,
            origin: None,
            business_no: None,
            facility_code: None,
            start_month: None,
            end_month: None,
            sign_cert_sha256_hex: None,
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect("create challenge");
    let valid_key =
        PKey::from_rsa(Rsa::generate(2048).expect("generate valid rsa key")).expect("wrap key");
    let valid_cert_der = create_sign_cert_der_for_common_name(&valid_key, "CN From Certificate");
    let valid_signature = sign_challenge(&valid_key, &stored.payload);
    let stored_cert = service
        .register_certificate_from_proof(RegisterCertificateFromProofInput {
            organization_id: "bootstrap-user".to_owned(),
            requested_by_user_id: Some("bootstrap-user".to_owned()),
            challenge_id: stored.id.clone(),
            challenge_payload: stored.payload.clone(),
            challenge_signature_base64: valid_signature,
            bundle: CertificateBundle {
                business_no: "2768603291".to_owned(),
                password: "secret-password".to_owned(),
                common_name: "Client Spoofed Name".to_owned(),
                sign_cert_der: valid_cert_der,
                sign_pri_key: direct_private_key_der(&valid_key),
                km_cert_der: None,
                km_pri_key: None,
                certificate_chain_der: Vec::new(),
            },
        })
        .await
        .expect("register certificate");
    assert_eq!(stored_cert.common_name, "CN From Certificate");
}

#[tokio::test]
async fn certificate_gateway_accepts_proof_bundles_without_km_material() {
    let (service, _db, _db_dir, _artifacts_dir) = build_service().await;

    let stored = service
        .create_cert_gateway_challenge(CreateCertGatewayChallengeInput {
            organization_id: "bootstrap-user".to_owned(),
            mode: "stored".to_owned(),
            sync_mode: None,
            origin: None,
            business_no: None,
            facility_code: Some("FAC-010".to_owned()),
            start_month: None,
            end_month: None,
            sign_cert_sha256_hex: None,
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect("create stored challenge");
    let stored_key =
        PKey::from_rsa(Rsa::generate(2048).expect("generate stored rsa key")).expect("wrap key");
    let stored_cert_der = create_sign_cert_der_for_common_name(&stored_key, "Stored Certificate");
    let stored_signature = sign_challenge(&stored_key, &stored.payload);
    let stored_cert = service
        .register_certificate_from_proof(RegisterCertificateFromProofInput {
            organization_id: "bootstrap-user".to_owned(),
            requested_by_user_id: Some("bootstrap-user".to_owned()),
            challenge_id: stored.id.clone(),
            challenge_payload: stored.payload.clone(),
            challenge_signature_base64: stored_signature,
            bundle: CertificateBundle {
                business_no: "2768603291".to_owned(),
                password: "secret-password".to_owned(),
                common_name: "Client Spoofed Name".to_owned(),
                sign_cert_der: stored_cert_der,
                sign_pri_key: direct_private_key_der(&stored_key),
                km_cert_der: None,
                km_pri_key: None,
                certificate_chain_der: Vec::new(),
            },
        })
        .await
        .expect("register certificate without km");
    assert_eq!(stored_cert.common_name, "Stored Certificate");

    let ephemeral = service
        .create_cert_gateway_challenge(CreateCertGatewayChallengeInput {
            organization_id: "bootstrap-user".to_owned(),
            mode: "ephemeral".to_owned(),
            sync_mode: None,
            origin: None,
            business_no: None,
            facility_code: Some("FAC-011".to_owned()),
            start_month: None,
            end_month: None,
            sign_cert_sha256_hex: None,
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect("create ephemeral challenge");
    let ephemeral_key =
        PKey::from_rsa(Rsa::generate(2048).expect("generate ephemeral rsa key")).expect("wrap key");
    let ephemeral_cert_der =
        create_sign_cert_der_for_common_name(&ephemeral_key, "Ephemeral Certificate");
    let ephemeral_signature = sign_challenge(&ephemeral_key, &ephemeral.payload);
    let run = service
        .execute_ephemeral_crawl(ExecuteEphemeralCrawlInput {
            organization_id: "bootstrap-user".to_owned(),
            requested_by_user_id: Some("bootstrap-user".to_owned()),
            challenge_id: ephemeral.id.clone(),
            challenge_payload: ephemeral.payload.clone(),
            challenge_signature_base64: ephemeral_signature,
            bundle: CertificateBundle {
                business_no: "2768603291".to_owned(),
                password: "secret-password".to_owned(),
                common_name: "Client Spoofed Name".to_owned(),
                sign_cert_der: ephemeral_cert_der,
                sign_pri_key: direct_private_key_der(&ephemeral_key),
                km_cert_der: None,
                km_pri_key: None,
                certificate_chain_der: Vec::new(),
            },
            facility_code: Some("FAC-011".to_owned()),
            start_month: None,
            end_month: None,
            mode: SyncMode::Full,
        })
        .await
        .expect("execute ephemeral crawl without km");
    assert_eq!(run.status, "completed");
    assert!(
        run.plant_id
            .as_deref()
            .is_some_and(|plant_id| plant_id.starts_with("plant-"))
    );
}

#[tokio::test]
async fn certificate_gateway_accepts_certificate_without_subject_business_no() {
    let (service, _db, _db_dir, _artifacts_dir) = build_service().await;

    let stored = service
        .create_cert_gateway_challenge(CreateCertGatewayChallengeInput {
            organization_id: "bootstrap-user".to_owned(),
            mode: "stored".to_owned(),
            sync_mode: None,
            origin: None,
            business_no: None,
            facility_code: None,
            start_month: None,
            end_month: None,
            sign_cert_sha256_hex: None,
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect("create stored challenge");
    let key =
        PKey::from_rsa(Rsa::generate(2048).expect("generate stored rsa key")).expect("wrap key");
    let cert_der = create_sign_cert_der_for_common_name(&key, "No Serial Number Certificate");
    let signature = sign_challenge(&key, &stored.payload);

    let stored_cert = service
        .register_certificate_from_proof(RegisterCertificateFromProofInput {
            organization_id: "bootstrap-user".to_owned(),
            requested_by_user_id: Some("bootstrap-user".to_owned()),
            challenge_id: stored.id,
            challenge_payload: stored.payload,
            challenge_signature_base64: signature,
            bundle: CertificateBundle {
                business_no: "9999999999".to_owned(),
                password: "secret-password".to_owned(),
                common_name: "Client Spoofed Name".to_owned(),
                sign_cert_der: cert_der,
                sign_pri_key: direct_private_key_der(&key),
                km_cert_der: None,
                km_pri_key: None,
                certificate_chain_der: Vec::new(),
            },
        })
        .await
        .expect("register certificate without subject business number");

    assert_eq!(stored_cert.business_no.as_deref(), Some("2768603291"));
    assert_eq!(stored_cert.common_name, "No Serial Number Certificate");
}

#[tokio::test]
async fn certificate_gateway_rejects_no_subject_business_number_for_unlinked_organization() {
    let (service, db, _db_dir, _artifacts_dir) = build_service().await;
    db.upsert_organization_with_owner("unlinked-org", "Unlinked Org", None, "Unlinked Org")
        .await
        .expect("unlinked organization");

    let stored = service
        .create_cert_gateway_challenge(CreateCertGatewayChallengeInput {
            organization_id: "unlinked-org".to_owned(),
            mode: "stored".to_owned(),
            sync_mode: None,
            origin: None,
            business_no: None,
            facility_code: None,
            start_month: None,
            end_month: None,
            sign_cert_sha256_hex: None,
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect("create stored challenge");
    let key =
        PKey::from_rsa(Rsa::generate(2048).expect("generate stored rsa key")).expect("wrap key");
    let cert_der = create_sign_cert_der_for_common_name(&key, "No Serial Number Certificate");
    let signature = sign_challenge(&key, &stored.payload);

    let err = service
        .register_certificate_from_proof(RegisterCertificateFromProofInput {
            organization_id: "unlinked-org".to_owned(),
            requested_by_user_id: Some("bootstrap-user".to_owned()),
            challenge_id: stored.id,
            challenge_payload: stored.payload,
            challenge_signature_base64: signature,
            bundle: CertificateBundle {
                business_no: "9999999999".to_owned(),
                password: "secret-password".to_owned(),
                common_name: "Client Spoofed Name".to_owned(),
                sign_cert_der: cert_der,
                sign_pri_key: direct_private_key_der(&key),
                km_cert_der: None,
                km_pri_key: None,
                certificate_chain_der: Vec::new(),
            },
        })
        .await
        .expect_err("client-supplied business number must not bootstrap legal identity");
    assert!(
        err.to_string()
            .contains("organization missing linked legal party business number")
    );
}

#[tokio::test]
async fn certificate_gateway_rejects_stored_proof_when_certificate_business_no_mismatches() {
    let (service, _db, _db_dir, _artifacts_dir) = build_service().await;

    let stored = service
        .create_cert_gateway_challenge(CreateCertGatewayChallengeInput {
            organization_id: "bootstrap-user".to_owned(),
            mode: "stored".to_owned(),
            sync_mode: None,
            origin: None,
            business_no: None,
            facility_code: None,
            start_month: None,
            end_month: None,
            sign_cert_sha256_hex: None,
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect("create stored challenge");
    let key =
        PKey::from_rsa(Rsa::generate(2048).expect("generate stored rsa key")).expect("wrap key");
    let cert_der = create_sign_cert_der_for_common_name_and_business_no(
        &key,
        "Mismatched Stored Certificate",
        "1234567890",
    );
    let signature = sign_challenge(&key, &stored.payload);

    let err = service
        .register_certificate_from_proof(RegisterCertificateFromProofInput {
            organization_id: "bootstrap-user".to_owned(),
            requested_by_user_id: Some("bootstrap-user".to_owned()),
            challenge_id: stored.id,
            challenge_payload: stored.payload,
            challenge_signature_base64: signature,
            bundle: CertificateBundle {
                business_no: "2768603291".to_owned(),
                password: "secret-password".to_owned(),
                common_name: "Client Spoofed Name".to_owned(),
                sign_cert_der: cert_der,
                sign_pri_key: direct_private_key_der(&key),
                km_cert_der: None,
                km_pri_key: None,
                certificate_chain_der: Vec::new(),
            },
        })
        .await
        .expect_err("mismatched business number should be rejected");

    assert!(
        err.to_string()
            .contains("certificate business number mismatch")
    );
}

#[tokio::test]
async fn certificate_gateway_rejects_ephemeral_proof_when_certificate_business_no_mismatches() {
    let (service, _db, _db_dir, _artifacts_dir) = build_service().await;

    let ephemeral = service
        .create_cert_gateway_challenge(CreateCertGatewayChallengeInput {
            organization_id: "bootstrap-user".to_owned(),
            mode: "ephemeral".to_owned(),
            sync_mode: None,
            origin: None,
            business_no: None,
            facility_code: Some("FAC-013".to_owned()),
            start_month: None,
            end_month: None,
            sign_cert_sha256_hex: None,
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect("create ephemeral challenge");
    let key =
        PKey::from_rsa(Rsa::generate(2048).expect("generate ephemeral rsa key")).expect("wrap key");
    let cert_der = create_sign_cert_der_for_common_name_and_business_no(
        &key,
        "Mismatched Ephemeral Certificate",
        "1234567890",
    );
    let signature = sign_challenge(&key, &ephemeral.payload);

    let err = service
        .execute_ephemeral_crawl(ExecuteEphemeralCrawlInput {
            organization_id: "bootstrap-user".to_owned(),
            requested_by_user_id: Some("bootstrap-user".to_owned()),
            challenge_id: ephemeral.id,
            challenge_payload: ephemeral.payload,
            challenge_signature_base64: signature,
            bundle: CertificateBundle {
                business_no: "2768603291".to_owned(),
                password: "secret-password".to_owned(),
                common_name: "Client Spoofed Name".to_owned(),
                sign_cert_der: cert_der,
                sign_pri_key: direct_private_key_der(&key),
                km_cert_der: None,
                km_pri_key: None,
                certificate_chain_der: Vec::new(),
            },
            facility_code: Some("FAC-013".to_owned()),
            start_month: ephemeral.start_month.clone(),
            end_month: ephemeral.end_month.clone(),
            mode: challenge_sync_mode(ephemeral.sync_mode.as_deref()),
        })
        .await
        .expect_err("mismatched business number should be rejected");

    assert!(
        err.to_string()
            .contains("certificate business number mismatch")
    );
}

#[tokio::test]
async fn certificate_gateway_rejects_subject_business_number_for_unlinked_organization() {
    let (service, db, _db_dir, _artifacts_dir) = build_service().await;
    db.upsert_organization_identity("pending-org", None, "Pending Org")
        .await
        .expect("seed pending organization identity");

    let challenge = service
        .create_cert_gateway_challenge(CreateCertGatewayChallengeInput {
            organization_id: "pending-org".to_owned(),
            mode: "stored".to_owned(),
            sync_mode: None,
            origin: None,
            business_no: None,
            facility_code: Some("FAC-012".to_owned()),
            start_month: None,
            end_month: None,
            sign_cert_sha256_hex: None,
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect("create stored challenge");
    let key = PKey::from_rsa(Rsa::generate(2048).expect("generate rsa key")).expect("wrap key");
    let cert_der = create_sign_cert_der_for_common_name_and_business_no(
        &key,
        "Pending Org Certificate",
        "2768603291",
    );
    let signature = sign_challenge(&key, &challenge.payload);

    let err = service
        .register_certificate_from_proof(RegisterCertificateFromProofInput {
            organization_id: "pending-org".to_owned(),
            requested_by_user_id: Some("bootstrap-user".to_owned()),
            challenge_id: challenge.id.clone(),
            challenge_payload: challenge.payload.clone(),
            challenge_signature_base64: signature,
            bundle: CertificateBundle {
                business_no: "2768603291".to_owned(),
                password: "secret-password".to_owned(),
                common_name: "Client Spoofed Name".to_owned(),
                sign_cert_der: cert_der,
                sign_pri_key: direct_private_key_der(&key),
                km_cert_der: None,
                km_pri_key: None,
                certificate_chain_der: Vec::new(),
            },
        })
        .await
        .expect_err("certificate subject business number must not bootstrap legal identity");
    assert!(
        err.to_string()
            .contains("organization missing linked legal party business number")
    );
}

#[tokio::test]
async fn crawl_task_token_start_rejects_inactive_organization_without_consuming_token() {
    let (service, db, _db_dir, _artifacts_dir) = build_service().await;

    let challenge = service
        .create_cert_gateway_challenge(CreateCertGatewayChallengeInput {
            organization_id: "bootstrap-user".to_owned(),
            mode: "stored".to_owned(),
            sync_mode: None,
            origin: None,
            business_no: None,
            facility_code: Some("FAC-005".to_owned()),
            start_month: None,
            end_month: None,
            sign_cert_sha256_hex: None,
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect("create challenge");
    let (bundle, signature) = fake_bundle_and_signature(&challenge.payload);
    let cert = service
        .register_certificate_from_proof(RegisterCertificateFromProofInput {
            organization_id: "bootstrap-user".to_owned(),
            requested_by_user_id: Some("bootstrap-user".to_owned()),
            challenge_id: challenge.id.clone(),
            challenge_payload: challenge.payload.clone(),
            challenge_signature_base64: signature,
            bundle,
        })
        .await
        .expect("register certificate");

    let issued = service
        .issue_crawl_task_token(IssueCrawlTaskTokenInput {
            organization_id: "bootstrap-user".to_owned(),
            certificate_id: cert.id,
            facility_code: Some("FAC-005".to_owned()),
            start_month: Some("2026-01".to_owned()),
            end_month: Some("2026-02".to_owned()),
            mode: SyncMode::Delta,
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect("issue crawl task token");

    db.deactivate_organization("bootstrap-user")
        .await
        .expect("deactivate organization");

    let inactive_err = service
        .consume_crawl_task_token_and_start_crawl(StartCrawlWithTaskTokenInput {
            organization_id: "bootstrap-user".to_owned(),
            token: issued.token.clone(),
        })
        .await
        .expect_err("inactive organization should not start crawl");
    assert!(inactive_err.to_string().contains("inactive"));

    db.reactivate_organization("bootstrap-user")
        .await
        .expect("reactivate organization");

    let run = service
        .consume_crawl_task_token_and_start_crawl(StartCrawlWithTaskTokenInput {
            organization_id: "bootstrap-user".to_owned(),
            token: issued.token,
        })
        .await
        .expect("token should remain usable after reactivation");
    assert_eq!(run.status, "completed");
    assert!(
        run.plant_id
            .as_deref()
            .is_some_and(|plant_id| plant_id.starts_with("plant-"))
    );
}

#[tokio::test]
async fn issue_crawl_task_token_rejects_inactive_certificate() {
    let (service, _db, _db_dir, _artifacts_dir) = build_service().await;

    let first_challenge = service
        .create_cert_gateway_challenge(CreateCertGatewayChallengeInput {
            organization_id: "bootstrap-user".to_owned(),
            mode: "stored".to_owned(),
            sync_mode: None,
            origin: None,
            business_no: None,
            facility_code: Some("FAC-006".to_owned()),
            start_month: None,
            end_month: None,
            sign_cert_sha256_hex: None,
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect("create first challenge");
    let (first_bundle, first_signature) = fake_bundle_and_signature(&first_challenge.payload);
    let first_cert = service
        .register_certificate_from_proof(RegisterCertificateFromProofInput {
            organization_id: "bootstrap-user".to_owned(),
            requested_by_user_id: Some("bootstrap-user".to_owned()),
            challenge_id: first_challenge.id.clone(),
            challenge_payload: first_challenge.payload.clone(),
            challenge_signature_base64: first_signature,
            bundle: first_bundle,
        })
        .await
        .expect("register first certificate");

    let second_challenge = service
        .create_cert_gateway_challenge(CreateCertGatewayChallengeInput {
            organization_id: "bootstrap-user".to_owned(),
            mode: "stored".to_owned(),
            sync_mode: None,
            origin: None,
            business_no: None,
            facility_code: Some("FAC-006".to_owned()),
            start_month: None,
            end_month: None,
            sign_cert_sha256_hex: None,
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect("create second challenge");
    let (second_bundle, second_signature) = fake_bundle_and_signature(&second_challenge.payload);
    service
        .register_certificate_from_proof(RegisterCertificateFromProofInput {
            organization_id: "bootstrap-user".to_owned(),
            requested_by_user_id: Some("bootstrap-user".to_owned()),
            challenge_id: second_challenge.id.clone(),
            challenge_payload: second_challenge.payload.clone(),
            challenge_signature_base64: second_signature,
            bundle: second_bundle,
        })
        .await
        .expect("register second certificate");

    let err = service
        .issue_crawl_task_token(IssueCrawlTaskTokenInput {
            organization_id: "bootstrap-user".to_owned(),
            certificate_id: first_cert.id,
            facility_code: Some("FAC-006".to_owned()),
            start_month: Some("2026-01".to_owned()),
            end_month: Some("2026-02".to_owned()),
            mode: SyncMode::Delta,
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect_err("inactive certificate should not mint crawl tokens");
    assert!(err.to_string().contains("certificate is inactive"));
}

#[tokio::test]
async fn crawl_task_token_start_rejects_inactive_certificate_without_consuming_token() {
    let (service, _db, _db_dir, _artifacts_dir) = build_service().await;

    let first_challenge = service
        .create_cert_gateway_challenge(CreateCertGatewayChallengeInput {
            organization_id: "bootstrap-user".to_owned(),
            mode: "stored".to_owned(),
            sync_mode: None,
            origin: None,
            business_no: None,
            facility_code: Some("FAC-007".to_owned()),
            start_month: None,
            end_month: None,
            sign_cert_sha256_hex: None,
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect("create first challenge");
    let (first_bundle, first_signature) = fake_bundle_and_signature(&first_challenge.payload);
    let first_cert = service
        .register_certificate_from_proof(RegisterCertificateFromProofInput {
            organization_id: "bootstrap-user".to_owned(),
            requested_by_user_id: Some("bootstrap-user".to_owned()),
            challenge_id: first_challenge.id.clone(),
            challenge_payload: first_challenge.payload.clone(),
            challenge_signature_base64: first_signature,
            bundle: first_bundle,
        })
        .await
        .expect("register first certificate");

    let issued = service
        .issue_crawl_task_token(IssueCrawlTaskTokenInput {
            organization_id: "bootstrap-user".to_owned(),
            certificate_id: first_cert.id.clone(),
            facility_code: Some("FAC-007".to_owned()),
            start_month: Some("2026-01".to_owned()),
            end_month: Some("2026-02".to_owned()),
            mode: SyncMode::Delta,
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect("issue crawl task token");

    let second_challenge = service
        .create_cert_gateway_challenge(CreateCertGatewayChallengeInput {
            organization_id: "bootstrap-user".to_owned(),
            mode: "stored".to_owned(),
            sync_mode: None,
            origin: None,
            business_no: None,
            facility_code: Some("FAC-007".to_owned()),
            start_month: None,
            end_month: None,
            sign_cert_sha256_hex: None,
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect("create second challenge");
    let (second_bundle, second_signature) = fake_bundle_and_signature(&second_challenge.payload);
    service
        .register_certificate_from_proof(RegisterCertificateFromProofInput {
            organization_id: "bootstrap-user".to_owned(),
            requested_by_user_id: Some("bootstrap-user".to_owned()),
            challenge_id: second_challenge.id.clone(),
            challenge_payload: second_challenge.payload.clone(),
            challenge_signature_base64: second_signature,
            bundle: second_bundle,
        })
        .await
        .expect("register second certificate");

    let first_err = service
        .consume_crawl_task_token_and_start_crawl(StartCrawlWithTaskTokenInput {
            organization_id: "bootstrap-user".to_owned(),
            token: issued.token.clone(),
        })
        .await
        .expect_err("inactive certificate should not start crawl");
    assert!(first_err.to_string().contains("certificate is inactive"));

    let second_err = service
        .consume_crawl_task_token_and_start_crawl(StartCrawlWithTaskTokenInput {
            organization_id: "bootstrap-user".to_owned(),
            token: issued.token,
        })
        .await
        .expect_err("inactive certificate should not consume the token");
    assert!(second_err.to_string().contains("certificate is inactive"));
}

#[tokio::test]
async fn gateway_sync_scope_requires_permission_for_existing_plant() {
    let (service, db, _db_dir, _artifacts_dir) = build_service().await;
    seed_gateway_scope_fixture(&db).await;

    let err = service
        .authorize_gateway_sync_scope("bootstrap-user", "operator-org", Some("FAC-SYNC-001"))
        .await
        .expect_err("existing plant should require plant sync execute permission");
    assert!(err.to_string().contains("cannot execute sync"));
}

#[tokio::test]
async fn asset_permissions_authorize_kr_rps_sync_listing_and_generation() {
    let (service, db, _db_dir, _artifacts_dir) = build_service().await;
    seed_gateway_scope_fixture(&db).await;
    let plant = db
        .get_plant_by_facility_code("FAC-SYNC-001")
        .await
        .expect("load plant");
    let generation = vec![GenerationRecord {
        month: "2026-03".to_owned(),
        generation_kwh: 1000.0,
        self_consumption_kwh: Some(100.0),
        supply_kwh: 900.0,
        application_deadline: Some("2026-04-15".to_owned()),
    }];
    db.upsert_generation_records(&plant.id, &generation)
        .await
        .expect("upsert legacy generation");
    let asset_id = db
        .upsert_kr_rps_asset_projection(
            &plant,
            Some("operator-org"),
            Some("2768603291"),
            &generation,
            &[],
        )
        .await
        .expect("upsert KR RPS asset projection");

    db.upsert_role(&Role {
        id: "role-asset-sync-and-generation".to_owned(),
        organization_id: None,
        code: "asset.sync_and_generation".to_owned(),
        name: "Asset Sync And Generation".to_owned(),
        description: None,
    })
    .await
    .expect("upsert asset role");
    for action in [
        "asset.profile.read",
        "asset.sync.execute",
        "asset.generation.read",
    ] {
        db.upsert_role_action("role-asset-sync-and-generation", action)
            .await
            .expect("upsert asset role action");
    }

    for (binding_id, subject_type, subject_id, agreement_id) in [
        (
            "binding-asset-org-sync-generation",
            RoleBindingSubjectType::Organization,
            "operator-org",
            Some("agreement-sync"),
        ),
        (
            "binding-asset-user-sync-generation",
            RoleBindingSubjectType::User,
            "bootstrap-user",
            None,
        ),
    ] {
        db.save_role_binding(&RoleBinding {
            id: binding_id.to_owned(),
            subject_type,
            subject_id: subject_id.to_owned(),
            subject_organization_id: "operator-org".to_owned(),
            resource_type: RoleBindingResourceType::Asset,
            resource_id: asset_id.clone(),
            role_id: "role-asset-sync-and-generation".to_owned(),
            agreement_id: agreement_id.map(ToOwned::to_owned),
            state: RoleBindingState::Active,
            created_by_user_id: "bootstrap-user".to_owned(),
            created_at: "2026-04-03T00:00:00Z".to_owned(),
            updated_at: "2026-04-03T00:00:00Z".to_owned(),
        })
        .await
        .expect("save asset binding");
    }

    service
        .authorize_gateway_sync_scope("bootstrap-user", "operator-org", Some("FAC-SYNC-001"))
        .await
        .expect("asset sync permission should authorize existing KR RPS plant");

    let facilities = db
        .list_user_plant_action_facility_codes_with_consistency(
            "bootstrap-user",
            "operator-org",
            "plant.sync.execute",
            Some(ReadConsistency::Strong),
        )
        .await
        .expect("list executable facilities through asset permission");
    assert_eq!(facilities, vec!["FAC-SYNC-001".to_owned()]);

    let visible = service
        .list_visible_plants("bootstrap-user", "operator-org", None)
        .await
        .expect("asset profile permission should make KR RPS plant visible");
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].facility_code, "FAC-SYNC-001");

    let generation_rows = service
        .list_generation_authorized(
            "bootstrap-user",
            "operator-org",
            "FAC-SYNC-001",
            Some(ReadConsistency::Strong),
        )
        .await
        .expect("asset generation permission should authorize legacy generation read");
    assert_eq!(generation_rows, generation);
}

#[tokio::test]
async fn cert_gateway_challenge_requires_scope_authorization_before_sync_planning() {
    let (service, db, _db_dir, _artifacts_dir) = build_service().await;
    seed_gateway_scope_fixture(&db).await;

    let err = service
        .create_cert_gateway_challenge(CreateCertGatewayChallengeInput {
            organization_id: "operator-org".to_owned(),
            mode: "ephemeral".to_owned(),
            sync_mode: Some("delta".to_owned()),
            origin: Some("https://app.example".to_owned()),
            business_no: Some("2768603291".to_owned()),
            facility_code: Some(" FAC-SYNC-001 ".to_owned()),
            start_month: None,
            end_month: None,
            sign_cert_sha256_hex: None,
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect_err("unauthorized facility challenge should fail before planning");

    assert!(err.to_string().contains("cannot execute sync"));
}

#[tokio::test]
async fn ephemeral_execution_uses_challenge_bound_month_window() {
    let observed = Arc::new(Mutex::new(Vec::<BridgeOptions>::new()));
    let observed_collector = observed.clone();
    let (service, db, _db_dir, _artifacts_dir) =
        build_service_with_collector(move |opts: BridgeOptions| {
            let observed = observed_collector.clone();
            async move {
                observed.lock().expect("collector log").push(opts.clone());
                mock_collect_snapshot(opts).await
            }
        })
        .await;

    let seed_challenge = service
        .create_cert_gateway_challenge(CreateCertGatewayChallengeInput {
            organization_id: "bootstrap-user".to_owned(),
            mode: "ephemeral".to_owned(),
            sync_mode: Some("delta".to_owned()),
            origin: None,
            business_no: Some("2768603291".to_owned()),
            facility_code: Some("FAC-BOUND".to_owned()),
            start_month: None,
            end_month: None,
            sign_cert_sha256_hex: None,
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect("seed challenge");
    let (seed_bundle, seed_signature) = fake_bundle_and_signature(&seed_challenge.payload);
    service
        .execute_ephemeral_crawl(ExecuteEphemeralCrawlInput {
            organization_id: "bootstrap-user".to_owned(),
            requested_by_user_id: None,
            challenge_id: seed_challenge.id,
            challenge_payload: seed_challenge.payload,
            challenge_signature_base64: seed_signature,
            bundle: seed_bundle,
            facility_code: Some("FAC-BOUND".to_owned()),
            start_month: seed_challenge.start_month,
            end_month: seed_challenge.end_month,
            mode: SyncMode::Full,
        })
        .await
        .expect("seed execution");
    let seeded_plant = db
        .get_plant_by_facility_code("FAC-BOUND")
        .await
        .expect("seeded plant");
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
        .expect("upsert test sync role action");
    db.upsert_role(&Role {
        id: "role-test-plant-profile-read".to_owned(),
        organization_id: None,
        code: "plant.profile.read".to_owned(),
        name: "Plant Profile Read".to_owned(),
        description: None,
    })
    .await
    .expect("upsert test profile role");
    db.upsert_role_action("role-test-plant-profile-read", "plant.profile.read")
        .await
        .expect("upsert test profile role action");
    db.save_role_binding(&RoleBinding {
        id: Uuid::new_v4().simple().to_string(),
        subject_type: RoleBindingSubjectType::Organization,
        subject_id: "bootstrap-user".to_owned(),
        subject_organization_id: "bootstrap-user".to_owned(),
        resource_type: RoleBindingResourceType::Plant,
        resource_id: seeded_plant.id.clone(),
        role_id: "role-test-plant-sync-execute".to_owned(),
        agreement_id: None,
        state: RoleBindingState::Active,
        created_by_user_id: "bootstrap-user".to_owned(),
        created_at: "2026-04-03T00:00:00Z".to_owned(),
        updated_at: "2026-04-03T00:00:00Z".to_owned(),
    })
    .await
    .expect("grant organization sync access for seeded plant");
    db.save_role_binding(&RoleBinding {
        id: Uuid::new_v4().simple().to_string(),
        subject_type: RoleBindingSubjectType::Organization,
        subject_id: "bootstrap-user".to_owned(),
        subject_organization_id: "bootstrap-user".to_owned(),
        resource_type: RoleBindingResourceType::Plant,
        resource_id: seeded_plant.id.clone(),
        role_id: "role-test-plant-profile-read".to_owned(),
        agreement_id: None,
        state: RoleBindingState::Active,
        created_by_user_id: "bootstrap-user".to_owned(),
        created_at: "2026-04-03T00:00:00Z".to_owned(),
        updated_at: "2026-04-03T00:00:00Z".to_owned(),
    })
    .await
    .expect("grant organization profile access for seeded plant");
    db.save_role_binding(&RoleBinding {
        id: Uuid::new_v4().simple().to_string(),
        subject_type: RoleBindingSubjectType::User,
        subject_id: "bootstrap-user".to_owned(),
        subject_organization_id: "bootstrap-user".to_owned(),
        resource_type: RoleBindingResourceType::Plant,
        resource_id: seeded_plant.id.clone(),
        role_id: "role-test-plant-sync-execute".to_owned(),
        agreement_id: None,
        state: RoleBindingState::Active,
        created_by_user_id: "bootstrap-user".to_owned(),
        created_at: "2026-04-03T00:00:00Z".to_owned(),
        updated_at: "2026-04-03T00:00:00Z".to_owned(),
    })
    .await
    .expect("grant user sync access for seeded plant");
    db.save_role_binding(&RoleBinding {
        id: Uuid::new_v4().simple().to_string(),
        subject_type: RoleBindingSubjectType::User,
        subject_id: "bootstrap-user".to_owned(),
        subject_organization_id: "bootstrap-user".to_owned(),
        resource_type: RoleBindingResourceType::Plant,
        resource_id: seeded_plant.id.clone(),
        role_id: "role-test-plant-profile-read".to_owned(),
        agreement_id: None,
        state: RoleBindingState::Active,
        created_by_user_id: "bootstrap-user".to_owned(),
        created_at: "2026-04-03T00:00:00Z".to_owned(),
        updated_at: "2026-04-03T00:00:00Z".to_owned(),
    })
    .await
    .expect("grant user profile access for seeded plant");

    let auto_scoped_challenge = service
        .create_cert_gateway_challenge(CreateCertGatewayChallengeInput {
            organization_id: "bootstrap-user".to_owned(),
            mode: "ephemeral".to_owned(),
            sync_mode: Some("delta".to_owned()),
            origin: None,
            business_no: Some("2768603291".to_owned()),
            facility_code: None,
            start_month: None,
            end_month: None,
            sign_cert_sha256_hex: None,
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect("auto scoped delta challenge");
    assert_eq!(auto_scoped_challenge.sync_mode.as_deref(), Some("delta"));
    assert_eq!(
        auto_scoped_challenge.start_month.as_deref(),
        Some("2026-03")
    );
    let auto_payload = decode_challenge_payload_json(&auto_scoped_challenge.payload);
    assert_eq!(auto_payload["facilityCode"], "FAC-BOUND");

    let delta_challenge = service
        .create_cert_gateway_challenge(CreateCertGatewayChallengeInput {
            organization_id: "bootstrap-user".to_owned(),
            mode: "ephemeral".to_owned(),
            sync_mode: Some("delta".to_owned()),
            origin: None,
            business_no: Some("2768603291".to_owned()),
            facility_code: Some("FAC-BOUND".to_owned()),
            start_month: None,
            end_month: None,
            sign_cert_sha256_hex: None,
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect("delta challenge");
    assert_eq!(delta_challenge.sync_mode.as_deref(), Some("delta"));
    assert_eq!(delta_challenge.start_month.as_deref(), Some("2026-03"));

    let newer_challenge = service
        .create_cert_gateway_challenge(CreateCertGatewayChallengeInput {
            organization_id: "bootstrap-user".to_owned(),
            mode: "ephemeral".to_owned(),
            sync_mode: Some("full".to_owned()),
            origin: None,
            business_no: Some("2768603291".to_owned()),
            facility_code: Some("FAC-BOUND".to_owned()),
            start_month: None,
            end_month: None,
            sign_cert_sha256_hex: None,
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect("newer challenge");
    let (newer_bundle, newer_signature) = fake_bundle_and_signature(&newer_challenge.payload);
    service
        .execute_ephemeral_crawl(ExecuteEphemeralCrawlInput {
            organization_id: "bootstrap-user".to_owned(),
            requested_by_user_id: None,
            challenge_id: newer_challenge.id,
            challenge_payload: newer_challenge.payload,
            challenge_signature_base64: newer_signature,
            bundle: newer_bundle,
            facility_code: Some("FAC-BOUND".to_owned()),
            start_month: newer_challenge.start_month,
            end_month: newer_challenge.end_month,
            mode: SyncMode::Full,
        })
        .await
        .expect("newer execution");

    let (delta_bundle, delta_signature) = fake_bundle_and_signature(&delta_challenge.payload);
    service
        .execute_ephemeral_crawl(ExecuteEphemeralCrawlInput {
            organization_id: "bootstrap-user".to_owned(),
            requested_by_user_id: None,
            challenge_id: delta_challenge.id,
            challenge_payload: delta_challenge.payload,
            challenge_signature_base64: delta_signature,
            bundle: delta_bundle,
            facility_code: Some("FAC-BOUND".to_owned()),
            start_month: delta_challenge.start_month,
            end_month: delta_challenge.end_month,
            mode: SyncMode::Delta,
        })
        .await
        .expect("delta execution");

    let calls = observed.lock().expect("collector log");
    let final_bound_call = calls
        .iter()
        .rev()
        .find(|call| call.facility_code.as_deref() == Some("FAC-BOUND"))
        .expect("FAC-BOUND collector call");
    assert_eq!(final_bound_call.start_month.as_deref(), Some("2026-03"));
    assert!(final_bound_call.skip_documents);
}

#[tokio::test]
async fn get_plant_by_facility_code_returns_structured_not_found() {
    let (_service, db, _db_dir, _artifacts_dir) = build_service().await;

    let err = db
        .get_plant_by_facility_code("FAC-MISSING")
        .await
        .expect_err("missing facility should fail");
    let lookup_err = err
        .downcast_ref::<DatabaseLookupError>()
        .expect("plant lookup should expose structured database lookup error");
    assert_eq!(*lookup_err, DatabaseLookupError::PlantNotFound);
    assert_eq!(lookup_err.to_string(), "plant not found");
}

#[tokio::test]
async fn sync_run_audit_resolves_facility_through_internal_plant_id() {
    let (_service, db, _db_dir, _artifacts_dir) = build_service().await;
    let plant = domain::Plant {
        id: "plant-audit-internal".to_owned(),
        facility_code: "FAC-AUDIT-INTERNAL".to_owned(),
        name: "Audit Plant".to_owned(),
        owner_organization_id: "bootstrap-user".to_owned(),
        created_by_organization_id: Some("bootstrap-user".to_owned()),
        contract_no: "CN-AUDIT-INTERNAL".to_owned(),
        address: "Seoul".to_owned(),
        installed_kw: 100.0,
        rec_start_date: "2026-01-01".to_owned(),
        confirm_issued: "Y".to_owned(),
    };
    db.upsert_plant(&plant).await.expect("upsert audit plant");
    let run = db
        .start_sync_run("bootstrap-user", Some(&plant.id), SyncMode::Full)
        .await
        .expect("start sync run with internal plant id");
    db.finish_sync_run(
        &run.id,
        "failed",
        42,
        Some("partial materialization failure"),
    )
    .await
    .expect("finish sync run without sync item");

    let audit = db
        .get_latest_sync_run_audit_for_facility("FAC-AUDIT-INTERNAL")
        .await
        .expect("load audit through facility code");

    assert_eq!(audit.run_status, "failed");
    assert_eq!(
        audit.run_error_message.as_deref(),
        Some("partial materialization failure")
    );
    assert!(audit.item_status.is_none());
}

#[tokio::test]
async fn gateway_sync_scope_lists_executable_facilities_in_one_query_shape() {
    let (_service, db, _db_dir, _artifacts_dir) = build_service().await;
    seed_gateway_scope_fixture(&db).await;

    db.upsert_plant(&domain::Plant {
        id: "FAC-SYNC-002".to_owned(),
        facility_code: "FAC-SYNC-002".to_owned(),
        name: "Sync Plant 2".to_owned(),
        owner_organization_id: "owner-org".to_owned(),
        created_by_organization_id: Some("operator-org".to_owned()),
        contract_no: "CN-SYNC-002".to_owned(),
        address: "Busan".to_owned(),
        installed_kw: 50.0,
        rec_start_date: "2026-02-01".to_owned(),
        confirm_issued: "Y".to_owned(),
    })
    .await
    .expect("upsert second plant");
    for (binding_id, resource_id, role_id) in [
        (
            "binding-org-sync-execute-1",
            "FAC-SYNC-001",
            "role-plant-sync-execute",
        ),
        (
            "binding-org-sync-execute-2",
            "FAC-SYNC-002",
            "role-plant-sync-execute",
        ),
        (
            "binding-org-profile-read-1",
            "FAC-SYNC-001",
            "role-plant-profile-reader",
        ),
        (
            "binding-org-profile-read-2",
            "FAC-SYNC-002",
            "role-plant-profile-reader",
        ),
    ] {
        db.save_role_binding(&RoleBinding {
            id: binding_id.to_owned(),
            subject_type: RoleBindingSubjectType::Organization,
            subject_id: "operator-org".to_owned(),
            subject_organization_id: "operator-org".to_owned(),
            resource_type: RoleBindingResourceType::Plant,
            resource_id: resource_id.to_owned(),
            role_id: role_id.to_owned(),
            agreement_id: Some("agreement-sync".to_owned()),
            state: RoleBindingState::Active,
            created_by_user_id: "bootstrap-user".to_owned(),
            created_at: "2026-04-03T00:00:00Z".to_owned(),
            updated_at: "2026-04-03T00:00:00Z".to_owned(),
        })
        .await
        .expect("grant plant binding");
    }
    for (binding_id, resource_id, role_id) in [
        (
            "binding-user-sync-execute-1",
            "FAC-SYNC-001",
            "role-plant-sync-execute",
        ),
        (
            "binding-user-sync-execute-2",
            "FAC-SYNC-002",
            "role-plant-sync-execute",
        ),
        (
            "binding-user-profile-read-1",
            "FAC-SYNC-001",
            "role-plant-profile-reader",
        ),
        (
            "binding-user-profile-read-2",
            "FAC-SYNC-002",
            "role-plant-profile-reader",
        ),
    ] {
        db.save_role_binding(&RoleBinding {
            id: binding_id.to_owned(),
            subject_type: RoleBindingSubjectType::User,
            subject_id: "bootstrap-user".to_owned(),
            subject_organization_id: "operator-org".to_owned(),
            resource_type: RoleBindingResourceType::Plant,
            resource_id: resource_id.to_owned(),
            role_id: role_id.to_owned(),
            agreement_id: None,
            state: RoleBindingState::Active,
            created_by_user_id: "bootstrap-user".to_owned(),
            created_at: "2026-04-03T00:00:00Z".to_owned(),
            updated_at: "2026-04-03T00:00:00Z".to_owned(),
        })
        .await
        .expect("grant user plant binding");
    }

    let facilities = db
        .list_user_plant_action_facility_codes_with_consistency(
            "bootstrap-user",
            "operator-org",
            "plant.sync.execute",
            Some(ReadConsistency::Strong),
        )
        .await
        .expect("list executable facility codes");
    assert_eq!(
        facilities,
        vec!["FAC-SYNC-001".to_owned(), "FAC-SYNC-002".to_owned()]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rebuild_keeps_existing_visibility_readable_during_save_role_binding() {
    let (runtime, dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "bootstrap-user".to_owned(),
        "Bootstrap User".to_owned(),
    );
    db.ensure_schema().await.expect("ensure schema");

    db.upsert_organization_with_owner("owner-org", "Owner Org", None, "Owner Org")
        .await
        .expect("upsert owner org");
    db.upsert_organization_with_owner("worker-org", "Worker Org", None, "Worker Org")
        .await
        .expect("upsert worker org");

    runtime
        .execute(
            "INSERT INTO users (id, email, display_name, active, created_at)
             VALUES ($1, $2, $3, $4, $5)",
            vec![
                "worker-user".into(),
                "worker@example.com".into(),
                "Worker User".into(),
                1_i64.into(),
                "2026-04-03T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert user");
    runtime
        .execute(
            "INSERT INTO organization_memberships (id, organization_id, user_id, membership_role, status, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            vec![
                "membership-worker-user".into(),
                "worker-org".into(),
                "worker-user".into(),
                "member".into(),
                "active".into(),
                "2026-04-03T00:00:00Z".into(),
                "2026-04-03T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert membership");

    db.upsert_role(&Role {
        id: "role-plant-profile-reader".to_owned(),
        organization_id: None,
        code: "plant.profile.reader".to_owned(),
        name: "Plant Profile Reader".to_owned(),
        description: None,
    })
    .await
    .expect("upsert role");
    db.upsert_role_action("role-plant-profile-reader", "plant.profile.read")
        .await
        .expect("upsert role action");

    runtime
        .execute(
            "INSERT INTO plants (id, facility_code, owner_organization_id, created_by_organization_id, name, contract_no, address, installed_kw, rec_start_date, confirm_issued, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
            vec![
                "plant-baseline".into(),
                "FAC-BASELINE".into(),
                "worker-org".into(),
                Option::<String>::Some("owner-org".to_owned()).into(),
                "Baseline Plant".into(),
                "CN-BASELINE".into(),
                "Seoul".into(),
                50.0_f64.into(),
                "2026-01-01".into(),
                "Y".into(),
                "2026-04-03T00:00:00Z".into(),
                "2026-04-03T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert baseline plant");

    db.save_role_binding(&RoleBinding {
        id: Uuid::new_v4().simple().to_string(),
        subject_type: RoleBindingSubjectType::Organization,
        subject_id: "worker-org".to_owned(),
        subject_organization_id: "worker-org".to_owned(),
        resource_type: RoleBindingResourceType::Plant,
        resource_id: "plant-baseline".to_owned(),
        role_id: "role-plant-profile-reader".to_owned(),
        agreement_id: None,
        state: RoleBindingState::Active,
        created_by_user_id: "bootstrap-user".to_owned(),
        created_at: "2026-04-03T00:00:00Z".to_owned(),
        updated_at: "2026-04-03T00:00:00Z".to_owned(),
    })
    .await
    .expect("save org role binding");
    db.save_role_binding(&RoleBinding {
        id: Uuid::new_v4().simple().to_string(),
        subject_type: RoleBindingSubjectType::User,
        subject_id: "worker-user".to_owned(),
        subject_organization_id: "worker-org".to_owned(),
        resource_type: RoleBindingResourceType::Plant,
        resource_id: "plant-baseline".to_owned(),
        role_id: "role-plant-profile-reader".to_owned(),
        agreement_id: None,
        state: RoleBindingState::Active,
        created_by_user_id: "bootstrap-user".to_owned(),
        created_at: "2026-04-03T00:00:00Z".to_owned(),
        updated_at: "2026-04-03T00:00:00Z".to_owned(),
    })
    .await
    .expect("save baseline user role binding");

    for idx in 0..250 {
        let plant_id = format!("plant-bulk-{idx:04}");
        runtime
            .execute(
                "INSERT INTO plants (id, facility_code, owner_organization_id, created_by_organization_id, name, contract_no, address, installed_kw, rec_start_date, confirm_issued, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
                vec![
                    plant_id.clone().into(),
                    format!("FAC-BULK-{idx:04}").into(),
                    "worker-org".into(),
                    Option::<String>::Some("owner-org".to_owned()).into(),
                    format!("Bulk Plant {idx}").into(),
                    format!("CN-BULK-{idx:04}").into(),
                    "Busan".into(),
                    75.0_f64.into(),
                    "2026-01-01".into(),
                    "Y".into(),
                    "2026-04-03T00:00:00Z".into(),
                    "2026-04-03T00:00:00Z".into(),
                ],
            )
            .await
            .expect("insert bulk plant");

        runtime
            .execute(
                "INSERT INTO role_bindings (
                    id, subject_type, subject_id, subject_organization_id, resource_type, resource_id,
                    role_id, agreement_id, state, created_by_user_id, created_at, updated_at
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
                vec![
                    format!("org-bulk-binding-{idx:04}").into(),
                    "organization".into(),
                    "worker-org".into(),
                    "worker-org".into(),
                    "plant".into(),
                    plant_id.clone().into(),
                    "role-plant-profile-reader".into(),
                    Option::<String>::None.into(),
                    "active".into(),
                    "bootstrap-user".into(),
                    "2026-04-03T00:00:00Z".into(),
                    "2026-04-03T00:00:00Z".into(),
                ],
            )
            .await
            .expect("insert bulk org binding");
        runtime
            .execute(
                "INSERT INTO role_bindings (
                    id, subject_type, subject_id, subject_organization_id, resource_type, resource_id,
                    role_id, agreement_id, state, created_by_user_id, created_at, updated_at
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
                vec![
                    format!("user-bulk-binding-{idx:04}").into(),
                    "user".into(),
                    "worker-user".into(),
                    "worker-org".into(),
                    "plant".into(),
                    plant_id.into(),
                    "role-plant-profile-reader".into(),
                    Option::<String>::None.into(),
                    "active".into(),
                    "bootstrap-user".into(),
                    "2026-04-03T00:00:00Z".into(),
                    "2026-04-03T00:00:00Z".into(),
                ],
            )
            .await
            .expect("insert bulk user binding");
    }

    runtime
        .execute(
            "INSERT INTO plants (id, facility_code, owner_organization_id, created_by_organization_id, name, contract_no, address, installed_kw, rec_start_date, confirm_issued, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
            vec![
                "plant-trigger".into(),
                "FAC-TRIGGER".into(),
                "worker-org".into(),
                Option::<String>::Some("owner-org".to_owned()).into(),
                "Trigger Plant".into(),
                "CN-TRIGGER".into(),
                "Daegu".into(),
                80.0_f64.into(),
                "2026-01-01".into(),
                "Y".into(),
                "2026-04-03T00:00:00Z".into(),
                "2026-04-03T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert trigger plant");
    runtime
        .execute(
            "INSERT INTO role_bindings (
                id, subject_type, subject_id, subject_organization_id, resource_type, resource_id,
                role_id, agreement_id, state, created_by_user_id, created_at, updated_at
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
            vec![
                "org-trigger-binding".into(),
                "organization".into(),
                "worker-org".into(),
                "worker-org".into(),
                "plant".into(),
                "plant-trigger".into(),
                "role-plant-profile-reader".into(),
                Option::<String>::None.into(),
                "active".into(),
                "bootstrap-user".into(),
                "2026-04-03T00:00:00Z".into(),
                "2026-04-03T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert trigger org binding");

    let saw_gap = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));
    let db_path = sqlite_db_path(dir.path());
    let saw_gap_reader = saw_gap.clone();
    let stop_reader = stop.clone();
    let reader = thread::spawn(move || {
        let conn = rusqlite::Connection::open_with_flags(
            db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .expect("open sqlite reader");
        let started = Instant::now();
        while !stop_reader.load(Ordering::Relaxed) && started.elapsed() < Duration::from_secs(10) {
            let visible = conn
                .query_row(
                    "SELECT COUNT(*) FROM effective_user_plant_visibility
                     WHERE user_id = ?1 AND organization_id = ?2 AND plant_id = ?3",
                    rusqlite::params!["worker-user", "worker-org", "plant-baseline"],
                    |row| row.get::<_, i64>(0),
                )
                .expect("read baseline visibility");
            if visible == 0 {
                saw_gap_reader.store(true, Ordering::Relaxed);
                break;
            }
        }
    });

    db.save_role_binding(&RoleBinding {
        id: Uuid::new_v4().simple().to_string(),
        subject_type: RoleBindingSubjectType::User,
        subject_id: "worker-user".to_owned(),
        subject_organization_id: "worker-org".to_owned(),
        resource_type: RoleBindingResourceType::Plant,
        resource_id: "plant-trigger".to_owned(),
        role_id: "role-plant-profile-reader".to_owned(),
        agreement_id: None,
        state: RoleBindingState::Active,
        created_by_user_id: "bootstrap-user".to_owned(),
        created_at: "2026-04-03T00:00:00Z".to_owned(),
        updated_at: "2026-04-03T00:00:00Z".to_owned(),
    })
    .await
    .expect("save trigger role binding");

    stop.store(true, Ordering::Relaxed);
    reader.join().expect("join visibility reader");

    assert!(
        !saw_gap.load(Ordering::Relaxed),
        "rebuild should not expose missing baseline visibility to concurrent readers"
    );
}

#[tokio::test]
async fn save_role_binding_is_idempotent_for_logical_duplicate() {
    let (_service, db, _db_dir, _artifacts_dir) = build_service().await;

    db.upsert_plant(&domain::Plant {
        id: "plant-logical-binding".to_owned(),
        facility_code: "FAC-LOGICAL-BINDING".to_owned(),
        name: "Logical Binding Plant".to_owned(),
        owner_organization_id: "bootstrap-user".to_owned(),
        created_by_organization_id: Some("bootstrap-user".to_owned()),
        contract_no: "CN-LOGICAL-BINDING".to_owned(),
        address: "Seoul".to_owned(),
        installed_kw: 100.0,
        rec_start_date: "2026-01-01".to_owned(),
        confirm_issued: "Y".to_owned(),
    })
    .await
    .expect("upsert plant");
    db.upsert_role(&Role {
        id: "role-logical-binding-reader".to_owned(),
        organization_id: None,
        code: "logical.binding.reader".to_owned(),
        name: "Logical Binding Reader".to_owned(),
        description: None,
    })
    .await
    .expect("upsert role");
    db.upsert_role_action("role-logical-binding-reader", "plant.profile.read")
        .await
        .expect("upsert role action");

    let mut binding = RoleBinding {
        id: "binding-logical-first".to_owned(),
        subject_type: RoleBindingSubjectType::User,
        subject_id: "bootstrap-user".to_owned(),
        subject_organization_id: "bootstrap-user".to_owned(),
        resource_type: RoleBindingResourceType::Plant,
        resource_id: "plant-logical-binding".to_owned(),
        role_id: "role-logical-binding-reader".to_owned(),
        agreement_id: None,
        state: RoleBindingState::Active,
        created_by_user_id: "bootstrap-user".to_owned(),
        created_at: "2026-04-03T00:00:00Z".to_owned(),
        updated_at: "2026-04-03T00:00:00Z".to_owned(),
    };
    db.save_role_binding(&binding)
        .await
        .expect("save initial logical binding");

    binding.id = "binding-logical-second".to_owned();
    binding.updated_at = "2026-04-03T00:01:00Z".to_owned();
    db.save_role_binding(&binding)
        .await
        .expect("resave duplicate logical binding");

    let bindings = db
        .list_role_bindings_for_resource(RoleBindingResourceType::Plant, "plant-logical-binding")
        .await
        .expect("list plant bindings");
    let matching: Vec<_> = bindings
        .iter()
        .filter(|binding| {
            binding.subject_type == RoleBindingSubjectType::User
                && binding.subject_id == "bootstrap-user"
                && binding.subject_organization_id == "bootstrap-user"
                && binding.role_id == "role-logical-binding-reader"
        })
        .collect();
    assert_eq!(matching.len(), 1);
    assert_eq!(matching[0].id, "binding-logical-first");
    assert_eq!(matching[0].updated_at, "2026-04-03T00:01:00Z");
}

fn sqlite_db_path(root: &std::path::Path) -> PathBuf {
    root.join("state_machine").join("db").join("test.sqlite")
}

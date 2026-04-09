use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use domain::{
    Agreement, AgreementStatus, AgreementType, Role, RoleBinding, RoleBindingResourceType,
    RoleBindingState, RoleBindingSubjectType, SyncMode,
};
use hiqlite::{Node, NodeConfig, Row, s3::EncKeys};
use infra::{
    artifacts::ArtifactStore,
    config::{
        AppConfig, ArtifactBackendConfig, ArtifactConfig, AuthConfig, HiqliteConfig, KmsConfig,
        LocalNodeRole, ReadConsistency, RuntimeMode, SecurityConfig, ServerConfig,
        TrailbaseAuthConfig,
    },
    crypto::CertificateBundle,
    database::Database,
    runtime::HiqliteRuntime,
    service::{
        AppService, CreateCertGatewayChallengeInput, ExecuteEphemeralCrawlInput,
        IssueCrawlTaskTokenInput, RegisterCertificateFromProofInput, StartCrawlWithTaskTokenInput,
    },
};
use tempfile::TempDir;
use uuid::Uuid;

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
    let (runtime, db_dir) = test_runtime().await;
    let artifacts_dir = tempfile::tempdir().expect("artifact tempdir");
    let cfg = test_config(artifacts_dir.path());
    let db = Database::new(
        runtime.clone(),
        cfg.security.bootstrap_user_id.clone(),
        cfg.security.bootstrap_user_name.clone(),
    );
    let artifacts = ArtifactStore::new(&cfg.artifacts)
        .await
        .expect("artifact store");
    let service = AppService::new(cfg, db.clone(), artifacts);
    service.initialize().await.expect("initialize service");
    (service, db, db_dir, artifacts_dir)
}

fn fake_bundle() -> CertificateBundle {
    CertificateBundle {
        business_no: "2768603291".to_owned(),
        password: "secret-password".to_owned(),
        common_name: "Test Certificate".to_owned(),
        sign_cert_der: vec![1, 2, 3],
        sign_pri_key: vec![4, 5, 6],
        km_cert_der: vec![7, 8, 9],
        km_pri_key: vec![10, 11, 12],
    }
}

#[tokio::test]
async fn expands_hierarchy_and_drops_access_when_agreement_terminates() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "bootstrap-user".to_owned(),
        "Bootstrap User".to_owned(),
    );
    db.ensure_schema().await.expect("ensure schema");

    db.upsert_organization_with_owner("generator-org", "Generator Org", "Generator Org")
        .await
        .expect("upsert generator org");
    db.upsert_organization_with_owner("distributor-org", "Distributor Org", "Distributor Org")
        .await
        .expect("upsert distributor org");
    db.upsert_organization_with_owner("dealer-org", "Dealer Org", "Dealer Org")
        .await
        .expect("upsert dealer org");

    runtime
        .execute(
            "INSERT INTO organization_edges (parent_organization_id, child_organization_id, created_at)
             VALUES ($1, $2, $3)",
            vec![
                "distributor-org".into(),
                "dealer-org".into(),
                "2026-04-03T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert organization edge");

    runtime
        .execute(
            "INSERT INTO users (id, email, display_name, active, created_at)
             VALUES ($1, $2, $3, $4, $5)",
            vec![
                "dist-user".into(),
                "dist@example.com".into(),
                "Distributor User".into(),
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
        .expect("insert membership");
    runtime
        .execute(
            "INSERT INTO groups (id, organization_id, name, description, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6)",
            vec![
                "dist-ops".into(),
                "distributor-org".into(),
                "Distributor Ops".into(),
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
            vec!["dist-ops".into(), "membership-dist-user".into()],
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

    db.save_role_binding(&RoleBinding {
        id: Uuid::new_v4().simple().to_string(),
        subject_type: RoleBindingSubjectType::Group,
        subject_id: "dist-ops".to_owned(),
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
    assert!(org_rows.contains(&OrganizationPlantActionRow {
        organization_id: "distributor-org".to_owned(),
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
        user_id: "dist-user".to_owned(),
        organization_id: "distributor-org".to_owned(),
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
        user_id: "dist-user".to_owned(),
        organization_id: "distributor-org".to_owned(),
        plant_id: "plant-1".to_owned(),
    }));

    db.upsert_agreement(&Agreement {
        status: AgreementStatus::Terminated,
        ..agreement
    })
    .await
    .expect("terminate agreement");

    let org_rows_after: Vec<OrganizationPlantActionRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT organization_id, plant_id, action_id
             FROM effective_organization_plant_actions
             WHERE plant_id = $1",
            vec!["plant-1".into()],
        )
        .await
        .expect("organization action rows after termination");
    assert!(
        org_rows_after.is_empty(),
        "agreement termination should remove inherited organization access"
    );

    let user_rows_after: Vec<UserPlantActionRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT user_id, organization_id, plant_id, action_id
             FROM effective_user_plant_actions
             WHERE plant_id = $1",
            vec!["plant-1".into()],
        )
        .await
        .expect("user action rows after termination");
    assert!(
        user_rows_after.is_empty(),
        "agreement termination should remove user actions"
    );

    let visibility_rows_after: Vec<UserPlantVisibilityRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT user_id, organization_id, plant_id
             FROM effective_user_plant_visibility
             WHERE plant_id = $1",
            vec!["plant-1".into()],
        )
        .await
        .expect("visibility rows after termination");
    assert!(
        visibility_rows_after.is_empty(),
        "agreement termination should remove plant visibility"
    );
}

#[tokio::test]
async fn certificate_gateway_service_flows_enforce_expiry_reuse_and_single_use_tokens() {
    let (service, _db, _db_dir, _artifacts_dir) = build_service().await;

    let expired = service
        .create_cert_gateway_challenge(CreateCertGatewayChallengeInput {
            organization_id: "bootstrap-user".to_owned(),
            mode: "stored".to_owned(),
            facility_code: None,
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: -60,
        })
        .await
        .expect("create expired challenge");
    let err = service
        .register_certificate_from_proof(RegisterCertificateFromProofInput {
            organization_id: "bootstrap-user".to_owned(),
            challenge_id: expired.id.clone(),
            challenge_payload: expired.payload.clone(),
            bundle: fake_bundle(),
        })
        .await
        .expect_err("expired challenge should be rejected");
    assert!(err.to_string().contains("expired"));

    let stored = service
        .create_cert_gateway_challenge(CreateCertGatewayChallengeInput {
            organization_id: "bootstrap-user".to_owned(),
            mode: "stored".to_owned(),
            facility_code: Some("FAC-001".to_owned()),
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect("create stored challenge");
    let cert = service
        .register_certificate_from_proof(RegisterCertificateFromProofInput {
            organization_id: "bootstrap-user".to_owned(),
            challenge_id: stored.id.clone(),
            challenge_payload: stored.payload.clone(),
            bundle: fake_bundle(),
        })
        .await
        .expect("register certificate");
    assert_eq!(cert.kms_key_id, "local-dev-v1");
    assert!(!cert.wrapped_dek.is_empty());

    let reuse_err = service
        .register_certificate_from_proof(RegisterCertificateFromProofInput {
            organization_id: "bootstrap-user".to_owned(),
            challenge_id: stored.id.clone(),
            challenge_payload: stored.payload.clone(),
            bundle: fake_bundle(),
        })
        .await
        .expect_err("reused challenge should fail");
    assert!(reuse_err.to_string().contains("consumed"));

    let ephemeral = service
        .create_cert_gateway_challenge(CreateCertGatewayChallengeInput {
            organization_id: "bootstrap-user".to_owned(),
            mode: "ephemeral".to_owned(),
            facility_code: Some("FAC-002".to_owned()),
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect("create ephemeral challenge");
    let run = service
        .execute_ephemeral_crawl(ExecuteEphemeralCrawlInput {
            organization_id: "bootstrap-user".to_owned(),
            challenge_id: ephemeral.id.clone(),
            challenge_payload: ephemeral.payload.clone(),
            bundle: fake_bundle(),
            facility_code: Some("FAC-002".to_owned()),
            start_month: Some("2026-01".to_owned()),
            end_month: Some("2026-03".to_owned()),
            mode: SyncMode::Delta,
        })
        .await
        .expect("execute ephemeral crawl");
    assert_eq!(run.status, "accepted");
    assert_eq!(run.plant_id.as_deref(), Some("FAC-002"));

    let issued = service
        .issue_crawl_task_token(IssueCrawlTaskTokenInput {
            organization_id: "bootstrap-user".to_owned(),
            certificate_id: cert.id.clone(),
            facility_code: Some("FAC-003".to_owned()),
            start_month: Some("2026-01".to_owned()),
            end_month: Some("2026-04".to_owned()),
            mode: SyncMode::Full,
            created_by_user_id: "bootstrap-user".to_owned(),
            ttl_seconds: 120,
        })
        .await
        .expect("issue crawl task token");
    let started = service
        .consume_crawl_task_token_and_start_crawl(StartCrawlWithTaskTokenInput {
            token: issued.token.clone(),
        })
        .await
        .expect("consume crawl task token");
    assert_eq!(started.status, "accepted");
    assert_eq!(started.mode, SyncMode::Full);
    assert_eq!(started.plant_id.as_deref(), Some("FAC-003"));

    let token_reuse = service
        .consume_crawl_task_token_and_start_crawl(StartCrawlWithTaskTokenInput {
            token: issued.token,
        })
        .await
        .expect_err("reused crawl task token should fail");
    assert!(token_reuse.to_string().contains("already used"));
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

    db.upsert_organization_with_owner("owner-org", "Owner Org", "Owner Org")
        .await
        .expect("upsert owner org");
    db.upsert_organization_with_owner("worker-org", "Worker Org", "Worker Org")
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

fn sqlite_db_path(root: &std::path::Path) -> PathBuf {
    root.join("state_machine").join("db").join("test.sqlite")
}

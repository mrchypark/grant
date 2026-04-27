#[path = "support/auth.rs"]
mod auth_support;
mod support;

use std::sync::Arc;
use std::{fs, path::Path};

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use domain::{
    DocumentCategory, GenerationRecord, RateRecord, RecRecord, RoleBinding,
    RoleBindingResourceType, RoleBindingState, RoleBindingSubjectType,
};
use hiqlite::{Node, NodeConfig, s3::EncKeys};
use infra::{
    artifacts::ArtifactStore,
    config::{
        AppConfig, ArtifactBackendConfig, ArtifactConfig, HiqliteConfig, LocalNodeRole,
        ReadConsistency, RuntimeMode, SecurityConfig, ServerConfig,
    },
    database::Database,
    runtime::HiqliteRuntime,
    service::AppService,
};
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

fn test_config(artifacts_root: &std::path::Path) -> AppConfig {
    let trailbase_base_url = "http://127.0.0.1:0".to_owned();
    AppConfig {
        server: ServerConfig {
            bind_addr: "127.0.0.1:0".to_owned(),
        },
        security: SecurityConfig {
            master_key: "dev-only-master-key".to_owned(),
            bootstrap_user_id: "bootstrap-user".to_owned(),
            bootstrap_user_name: "Bootstrap User".to_owned(),
            allow_untrusted_certificates: true,
            certificate_trust_bundle_path: None,
        },
        auth: support::trailbase_auth_config(trailbase_base_url),
        kms: support::test_kms_config(),
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

async fn build_service() -> (Arc<AppService>, Database, HiqliteRuntime, TempDir, TempDir) {
    let trailbase_base_url = support::start_mock_trailbase().await;
    let (runtime, db_dir) = test_runtime().await;
    let artifacts_dir = tempfile::tempdir().expect("artifact tempdir");
    let mut cfg = test_config(artifacts_dir.path());
    cfg.auth = support::trailbase_auth_config(trailbase_base_url);
    let db = Database::new(
        runtime.clone(),
        cfg.security.bootstrap_user_id.clone(),
        cfg.security.bootstrap_user_name.clone(),
    );
    let artifacts = ArtifactStore::new(&cfg.artifacts)
        .await
        .expect("artifact store");
    let service = Arc::new(AppService::new(cfg, db.clone(), artifacts));
    service.initialize().await.expect("initialize service");
    (service, db, runtime, db_dir, artifacts_dir)
}

async fn seed_authorization_fixture(
    db: &Database,
    runtime: &HiqliteRuntime,
    artifacts_root: &Path,
) {
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
                "group-user".into(),
                "group@example.com".into(),
                "Group User".into(),
                1_i64.into(),
                "2026-04-03T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert group user");
    runtime
        .execute(
            "INSERT INTO users (id, email, display_name, active, created_at)
             VALUES ($1, $2, $3, $4, $5)",
            vec![
                "direct-user".into(),
                "direct@example.com".into(),
                "Direct User".into(),
                1_i64.into(),
                "2026-04-03T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert direct user");
    db.bind_identity_to_user(
        "group-user",
        "trailbase",
        "group-user",
        Some("group@example.com"),
        "Group User",
        true,
        true,
    )
    .await
    .expect("bind group identity");
    db.bind_identity_to_user(
        "direct-user",
        "trailbase",
        "direct-user",
        Some("direct@example.com"),
        "Direct User",
        true,
        true,
    )
    .await
    .expect("bind direct identity");
    runtime
        .execute(
            "INSERT INTO organization_memberships (id, organization_id, user_id, membership_role, status, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            vec![
                "membership-group-user".into(),
                "worker-org".into(),
                "group-user".into(),
                "member".into(),
                "active".into(),
                "2026-04-03T00:00:00Z".into(),
                "2026-04-03T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert group user membership");
    runtime
        .execute(
            "INSERT INTO organization_memberships (id, organization_id, user_id, membership_role, status, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            vec![
                "membership-direct-user".into(),
                "worker-org".into(),
                "direct-user".into(),
                "member".into(),
                "active".into(),
                "2026-04-03T00:00:00Z".into(),
                "2026-04-03T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert direct user membership");

    runtime
        .execute(
            "INSERT INTO groups (id, organization_id, name, description, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6)",
            vec![
                "field-team".into(),
                "worker-org".into(),
                "Field Team".into(),
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
            vec!["field-team".into(), "membership-group-user".into()],
        )
        .await
        .expect("insert group membership");

    runtime
        .execute(
            "INSERT INTO plants (id, facility_code, owner_organization_id, created_by_organization_id, name, contract_no, address, installed_kw, rec_start_date, confirm_issued, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
            vec![
                "plant-profile".into(),
                "FAC-PROFILE".into(),
                "owner-org".into(),
                Option::<String>::Some("worker-org".to_owned()).into(),
                "Plant Profile".into(),
                "CN-100".into(),
                "Seoul".into(),
                50.0_f64.into(),
                "2026-01-01".into(),
                "Y".into(),
                "2026-04-03T00:00:00Z".into(),
                "2026-04-03T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert profile plant");
    runtime
        .execute(
            "INSERT INTO plants (id, facility_code, owner_organization_id, created_by_organization_id, name, contract_no, address, installed_kw, rec_start_date, confirm_issued, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
            vec![
                "plant-document".into(),
                "FAC-DOCUMENT".into(),
                "owner-org".into(),
                Option::<String>::Some("worker-org".to_owned()).into(),
                "Plant Document".into(),
                "CN-200".into(),
                "Busan".into(),
                75.0_f64.into(),
                "2026-01-01".into(),
                "Y".into(),
                "2026-04-03T00:00:00Z".into(),
                "2026-04-03T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert document plant");

    runtime
        .execute(
            "INSERT INTO roles (id, organization_id, code, name, description, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            vec![
                "role-org-profile".into(),
                Option::<String>::None.into(),
                "org.profile".into(),
                "Org Profile".into(),
                Option::<String>::None.into(),
                "2026-04-03T00:00:00Z".into(),
                "2026-04-03T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert org profile role");
    runtime
        .execute(
            "INSERT INTO roles (id, organization_id, code, name, description, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            vec![
                "role-org-document".into(),
                Option::<String>::None.into(),
                "org.document".into(),
                "Org Document".into(),
                Option::<String>::None.into(),
                "2026-04-03T00:00:00Z".into(),
                "2026-04-03T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert org document role");
    runtime
        .execute(
            "INSERT INTO roles (id, organization_id, code, name, description, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            vec![
                "role-group-profile".into(),
                Option::<String>::Some("worker-org".to_owned()).into(),
                "group.profile".into(),
                "Group Profile".into(),
                Option::<String>::None.into(),
                "2026-04-03T00:00:00Z".into(),
                "2026-04-03T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert group profile role");
    runtime
        .execute(
            "INSERT INTO roles (id, organization_id, code, name, description, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            vec![
                "role-user-document".into(),
                Option::<String>::Some("worker-org".to_owned()).into(),
                "user.document".into(),
                "User Document".into(),
                Option::<String>::None.into(),
                "2026-04-03T00:00:00Z".into(),
                "2026-04-03T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert user document role");

    runtime
        .execute(
            "INSERT INTO role_actions (role_id, action_id) VALUES ($1, $2)",
            vec!["role-org-profile".into(), "plant.profile.read".into()],
        )
        .await
        .expect("insert org profile action");
    runtime
        .execute(
            "INSERT INTO role_actions (role_id, action_id) VALUES ($1, $2)",
            vec!["role-org-profile".into(), "plant.telemetry.read".into()],
        )
        .await
        .expect("insert org telemetry action");
    runtime
        .execute(
            "INSERT INTO role_actions (role_id, action_id) VALUES ($1, $2)",
            vec!["role-org-profile".into(), "plant.rate.read".into()],
        )
        .await
        .expect("insert org rate action");
    runtime
        .execute(
            "INSERT INTO role_actions (role_id, action_id) VALUES ($1, $2)",
            vec!["role-org-profile".into(), "plant.rec.read".into()],
        )
        .await
        .expect("insert org rec action");
    runtime
        .execute(
            "INSERT INTO role_actions (role_id, action_id) VALUES ($1, $2)",
            vec!["role-org-document".into(), "plant.document.read".into()],
        )
        .await
        .expect("insert org document action");
    runtime
        .execute(
            "INSERT INTO role_actions (role_id, action_id) VALUES ($1, $2)",
            vec!["role-group-profile".into(), "plant.profile.read".into()],
        )
        .await
        .expect("insert group profile action");
    runtime
        .execute(
            "INSERT INTO role_actions (role_id, action_id) VALUES ($1, $2)",
            vec!["role-group-profile".into(), "plant.telemetry.read".into()],
        )
        .await
        .expect("insert group telemetry action");
    runtime
        .execute(
            "INSERT INTO role_actions (role_id, action_id) VALUES ($1, $2)",
            vec!["role-group-profile".into(), "plant.rate.read".into()],
        )
        .await
        .expect("insert group rate action");
    runtime
        .execute(
            "INSERT INTO role_actions (role_id, action_id) VALUES ($1, $2)",
            vec!["role-group-profile".into(), "plant.rec.read".into()],
        )
        .await
        .expect("insert group rec action");
    runtime
        .execute(
            "INSERT INTO role_actions (role_id, action_id) VALUES ($1, $2)",
            vec!["role-user-document".into(), "plant.document.read".into()],
        )
        .await
        .expect("insert user document action");

    runtime
        .execute(
            "INSERT INTO agreements (id, agreement_type, grantor_organization_id, grantee_organization_id, status, title, reference_no, starts_at, ends_at, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
            vec![
                "agreement-profile".into(),
                "delegation".into(),
                "owner-org".into(),
                "worker-org".into(),
                "active".into(),
                "Profile Agreement".into(),
                Option::<String>::None.into(),
                Option::<String>::None.into(),
                Option::<String>::None.into(),
                "2026-04-03T00:00:00Z".into(),
                "2026-04-03T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert profile agreement");
    runtime
        .execute(
            "INSERT INTO agreements (id, agreement_type, grantor_organization_id, grantee_organization_id, status, title, reference_no, starts_at, ends_at, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
            vec![
                "agreement-document".into(),
                "delegation".into(),
                "owner-org".into(),
                "worker-org".into(),
                "active".into(),
                "Document Agreement".into(),
                Option::<String>::None.into(),
                Option::<String>::None.into(),
                Option::<String>::None.into(),
                "2026-04-03T00:00:00Z".into(),
                "2026-04-03T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert document agreement");

    db.save_role_binding(&RoleBinding {
        id: Uuid::new_v4().simple().to_string(),
        subject_type: RoleBindingSubjectType::Organization,
        subject_id: "worker-org".to_owned(),
        subject_organization_id: "worker-org".to_owned(),
        resource_type: RoleBindingResourceType::Plant,
        resource_id: "plant-profile".to_owned(),
        role_id: "role-org-profile".to_owned(),
        agreement_id: Some("agreement-profile".to_owned()),
        state: RoleBindingState::Active,
        created_by_user_id: "bootstrap-user".to_owned(),
        created_at: "2026-04-03T00:00:00Z".to_owned(),
        updated_at: "2026-04-03T00:00:00Z".to_owned(),
    })
    .await
    .expect("save org profile binding");
    db.save_role_binding(&RoleBinding {
        id: Uuid::new_v4().simple().to_string(),
        subject_type: RoleBindingSubjectType::Organization,
        subject_id: "worker-org".to_owned(),
        subject_organization_id: "worker-org".to_owned(),
        resource_type: RoleBindingResourceType::Plant,
        resource_id: "plant-document".to_owned(),
        role_id: "role-org-document".to_owned(),
        agreement_id: Some("agreement-document".to_owned()),
        state: RoleBindingState::Active,
        created_by_user_id: "bootstrap-user".to_owned(),
        created_at: "2026-04-03T00:00:00Z".to_owned(),
        updated_at: "2026-04-03T00:00:00Z".to_owned(),
    })
    .await
    .expect("save org document binding");
    db.save_role_binding(&RoleBinding {
        id: Uuid::new_v4().simple().to_string(),
        subject_type: RoleBindingSubjectType::Group,
        subject_id: "field-team".to_owned(),
        subject_organization_id: "worker-org".to_owned(),
        resource_type: RoleBindingResourceType::Plant,
        resource_id: "plant-profile".to_owned(),
        role_id: "role-group-profile".to_owned(),
        agreement_id: None,
        state: RoleBindingState::Active,
        created_by_user_id: "bootstrap-user".to_owned(),
        created_at: "2026-04-03T00:00:00Z".to_owned(),
        updated_at: "2026-04-03T00:00:00Z".to_owned(),
    })
    .await
    .expect("save group profile binding");
    db.save_role_binding(&RoleBinding {
        id: Uuid::new_v4().simple().to_string(),
        subject_type: RoleBindingSubjectType::User,
        subject_id: "direct-user".to_owned(),
        subject_organization_id: "worker-org".to_owned(),
        resource_type: RoleBindingResourceType::Plant,
        resource_id: "plant-document".to_owned(),
        role_id: "role-user-document".to_owned(),
        agreement_id: None,
        state: RoleBindingState::Active,
        created_by_user_id: "bootstrap-user".to_owned(),
        created_at: "2026-04-03T00:00:00Z".to_owned(),
        updated_at: "2026-04-03T00:00:00Z".to_owned(),
    })
    .await
    .expect("save direct user document binding");

    db.upsert_generation_records(
        "plant-profile",
        &[GenerationRecord {
            month: "2026-03".to_owned(),
            generation_kwh: 1234.5,
            self_consumption_kwh: Some(12.3),
            supply_kwh: 1222.2,
            application_deadline: Some("2026-04-25".to_owned()),
        }],
    )
    .await
    .expect("insert generation records");
    db.upsert_rate_records(
        "plant-profile",
        &[infra::database::StoredRateRecord {
            record: RateRecord {
                month: "2026-03".to_owned(),
                self_rate: Some(11.1),
                national_average: Some(13.2),
                nearby_average: Some(12.4),
                nearby_highest: Some(15.8),
            },
            published: true,
            source_control_flag: Some("N".to_owned()),
        }],
    )
    .await
    .expect("insert rate records");
    db.upsert_rec_records(
        "plant-profile",
        &[RecRecord {
            month: "2026-03".to_owned(),
            application_no: "APP-001".to_owned(),
            supply_kwh: 1000,
            rec_quantity: 10,
            fee_krw: 100,
            issued_date: "2026-03-31".to_owned(),
            expiry_date: "2026-04-30".to_owned(),
            virtual_account: Some("123-456".to_owned()),
            bank_name: Some("Bank".to_owned()),
            status: Some("issued".to_owned()),
        }],
    )
    .await
    .expect("insert rec records");

    let object_key = "docs/plant-document/sample.pdf";
    let artifact_path = artifacts_root.join(object_key);
    fs::create_dir_all(artifact_path.parent().expect("artifact parent")).expect("create parent");
    fs::write(&artifact_path, b"test-pdf-bytes").expect("write artifact");
    db.replace_documents(
        "plant-document",
        &[infra::database::PersistedDocument {
            id: "doc-1".to_owned(),
            category: DocumentCategory::Drawings,
            attachment_code: "ATT-001".to_owned(),
            file_class: "drawing".to_owned(),
            file_sec_name: "site".to_owned(),
            file_name: "layout.pdf".to_owned(),
            required: true,
            source_sha256: None,
            output_sha256: None,
            object_key: object_key.to_owned(),
            content_type: "application/pdf".to_owned(),
        }],
    )
    .await
    .expect("insert documents");
}

#[tokio::test]
async fn plant_routes_require_context_and_apply_visibility() {
    let (service, db, runtime, _db_dir, _artifacts_dir) = build_service().await;
    seed_authorization_fixture(&db, &runtime, _artifacts_dir.path()).await;
    let router = api::build_router(service);

    let list_response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/plants")
                .header("authorization", auth_support::bearer_header("group-user"))
                .header("x-organization-id", "worker-org")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("list response");
    assert_eq!(list_response.status(), StatusCode::OK);
    let body = to_bytes(list_response.into_body(), usize::MAX)
        .await
        .expect("list body");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("list json");
    assert_eq!(json.as_array().expect("array").len(), 1);
    assert_eq!(json[0]["facilityCode"], "FAC-PROFILE");

    let allowed = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/plants/FAC-PROFILE")
                .header("authorization", auth_support::bearer_header("group-user"))
                .header("x-organization-id", "worker-org")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("allowed detail response");
    assert_eq!(allowed.status(), StatusCode::OK);

    let forbidden = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/plants/FAC-DOCUMENT")
                .header("authorization", auth_support::bearer_header("direct-user"))
                .header("x-organization-id", "worker-org")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("forbidden response");
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);

    let missing_headers = router
        .oneshot(
            Request::builder()
                .uri("/api/v1/plants/FAC-PROFILE")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("missing headers response");
    assert_eq!(missing_headers.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn plant_subresource_routes_require_context_and_authorization() {
    let (service, db, runtime, _db_dir, artifacts_dir) = build_service().await;
    seed_authorization_fixture(&db, &runtime, artifacts_dir.path()).await;
    let router = api::build_router(service);

    let missing_generation_headers = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/plants/FAC-PROFILE/generation")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("generation response");
    assert_eq!(missing_generation_headers.status(), StatusCode::BAD_REQUEST);

    let forbidden_generation = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/plants/FAC-PROFILE/generation")
                .header("authorization", auth_support::bearer_header("direct-user"))
                .header("x-organization-id", "worker-org")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("forbidden generation response");
    assert_eq!(forbidden_generation.status(), StatusCode::FORBIDDEN);

    let allowed_generation = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/plants/FAC-PROFILE/generation")
                .header("authorization", auth_support::bearer_header("group-user"))
                .header("x-organization-id", "worker-org")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("allowed generation response");
    assert_eq!(allowed_generation.status(), StatusCode::OK);

    let allowed_rates = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/plants/FAC-PROFILE/rates")
                .header("authorization", auth_support::bearer_header("group-user"))
                .header("x-organization-id", "worker-org")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("allowed rates response");
    assert_eq!(allowed_rates.status(), StatusCode::OK);

    let allowed_rec = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/plants/FAC-PROFILE/rec")
                .header("authorization", auth_support::bearer_header("group-user"))
                .header("x-organization-id", "worker-org")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("allowed rec response");
    assert_eq!(allowed_rec.status(), StatusCode::OK);

    let forbidden_documents = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/plants/FAC-PROFILE/documents")
                .header("authorization", auth_support::bearer_header("group-user"))
                .header("x-organization-id", "worker-org")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("forbidden documents response");
    assert_eq!(forbidden_documents.status(), StatusCode::FORBIDDEN);

    let allowed_documents = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/plants/FAC-DOCUMENT/documents")
                .header("authorization", auth_support::bearer_header("direct-user"))
                .header("x-organization-id", "worker-org")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("allowed documents response");
    assert_eq!(allowed_documents.status(), StatusCode::OK);
    let documents_body = to_bytes(allowed_documents.into_body(), usize::MAX)
        .await
        .expect("documents body");
    let documents_json: serde_json::Value =
        serde_json::from_slice(&documents_body).expect("documents json");
    assert_eq!(documents_json.as_array().expect("documents array").len(), 1);
    assert_eq!(
        documents_json[0]["downloadUrl"],
        "/api/v1/documents/doc-1/download"
    );

    let missing_download_headers = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/documents/doc-1/download")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("download response");
    assert_eq!(missing_download_headers.status(), StatusCode::BAD_REQUEST);

    let forbidden_download = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/documents/doc-1/download")
                .header("authorization", auth_support::bearer_header("group-user"))
                .header("x-organization-id", "worker-org")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("forbidden download response");
    assert_eq!(forbidden_download.status(), StatusCode::FORBIDDEN);

    let allowed_download = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/documents/doc-1/download")
                .header("authorization", auth_support::bearer_header("direct-user"))
                .header("x-organization-id", "worker-org")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("allowed download response");
    assert_eq!(allowed_download.status(), StatusCode::OK);
    let bytes = to_bytes(allowed_download.into_body(), usize::MAX)
        .await
        .expect("download bytes");
    assert_eq!(&bytes[..], b"test-pdf-bytes");

    let plant = db
        .get_plant_by_facility_code("FAC-DOCUMENT")
        .await
        .expect("load document plant");
    let asset_id = db
        .upsert_kr_rps_asset_projection(&plant, None, None, &[], &[])
        .await
        .expect("upsert asset projection");
    runtime
        .execute(
            "INSERT INTO roles (id, organization_id, code, name, description, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            vec![
                "role-asset-document".into(),
                Option::<String>::Some("worker-org".to_owned()).into(),
                "asset.document".into(),
                "Asset Document".into(),
                Option::<String>::None.into(),
                "2026-04-03T00:00:00Z".into(),
                "2026-04-03T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert asset document role");
    runtime
        .execute(
            "INSERT INTO role_actions (role_id, action_id) VALUES ($1, $2)",
            vec!["role-asset-document".into(), "asset.document.read".into()],
        )
        .await
        .expect("insert asset document action");
    db.save_role_binding(&RoleBinding {
        id: Uuid::new_v4().simple().to_string(),
        subject_type: RoleBindingSubjectType::Organization,
        subject_id: "worker-org".to_owned(),
        subject_organization_id: "worker-org".to_owned(),
        resource_type: RoleBindingResourceType::Asset,
        resource_id: asset_id.clone(),
        role_id: "role-asset-document".to_owned(),
        agreement_id: Some("agreement-document".to_owned()),
        state: RoleBindingState::Active,
        created_by_user_id: "bootstrap-user".to_owned(),
        created_at: "2026-04-03T00:00:00Z".to_owned(),
        updated_at: "2026-04-03T00:00:00Z".to_owned(),
    })
    .await
    .expect("save asset organization document binding");
    db.save_role_binding(&RoleBinding {
        id: Uuid::new_v4().simple().to_string(),
        subject_type: RoleBindingSubjectType::User,
        subject_id: "group-user".to_owned(),
        subject_organization_id: "worker-org".to_owned(),
        resource_type: RoleBindingResourceType::Asset,
        resource_id: asset_id,
        role_id: "role-asset-document".to_owned(),
        agreement_id: None,
        state: RoleBindingState::Active,
        created_by_user_id: "bootstrap-user".to_owned(),
        created_at: "2026-04-03T00:00:00Z".to_owned(),
        updated_at: "2026-04-03T00:00:00Z".to_owned(),
    })
    .await
    .expect("save asset user document binding");

    let asset_allowed_download = router
        .oneshot(
            Request::builder()
                .uri("/api/v1/documents/doc-1/download")
                .header("authorization", auth_support::bearer_header("group-user"))
                .header("x-organization-id", "worker-org")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("asset allowed download response");
    assert_eq!(asset_allowed_download.status(), StatusCode::OK);
}

#[tokio::test]
async fn plant_detail_handles_integer_generation_aggregates() {
    let (service, db, runtime, _db_dir, artifacts_dir) = build_service().await;
    seed_authorization_fixture(&db, &runtime, artifacts_dir.path()).await;

    runtime
        .execute(
            "INSERT INTO plants (id, facility_code, owner_organization_id, created_by_organization_id, name, contract_no, address, installed_kw, rec_start_date, confirm_issued, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
            vec![
                "plant-integer".into(),
                "FAC-INTEGER".into(),
                "owner-org".into(),
                Option::<String>::Some("worker-org".to_owned()).into(),
                "Plant Integer".into(),
                "CN-300".into(),
                "Incheon".into(),
                10.0_f64.into(),
                "2026-01-01".into(),
                "Y".into(),
                "2026-04-03T00:00:00Z".into(),
                "2026-04-03T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert integer plant");

    db.save_role_binding(&RoleBinding {
        id: Uuid::new_v4().simple().to_string(),
        subject_type: RoleBindingSubjectType::Organization,
        subject_id: "worker-org".to_owned(),
        subject_organization_id: "worker-org".to_owned(),
        resource_type: RoleBindingResourceType::Plant,
        resource_id: "plant-integer".to_owned(),
        role_id: "role-org-profile".to_owned(),
        agreement_id: Some("agreement-profile".to_owned()),
        state: RoleBindingState::Active,
        created_by_user_id: "bootstrap-user".to_owned(),
        created_at: "2026-04-03T00:00:00Z".to_owned(),
        updated_at: "2026-04-03T00:00:00Z".to_owned(),
    })
    .await
    .expect("save integer org binding");
    db.save_role_binding(&RoleBinding {
        id: Uuid::new_v4().simple().to_string(),
        subject_type: RoleBindingSubjectType::Group,
        subject_id: "field-team".to_owned(),
        subject_organization_id: "worker-org".to_owned(),
        resource_type: RoleBindingResourceType::Plant,
        resource_id: "plant-integer".to_owned(),
        role_id: "role-group-profile".to_owned(),
        agreement_id: None,
        state: RoleBindingState::Active,
        created_by_user_id: "bootstrap-user".to_owned(),
        created_at: "2026-04-03T00:00:00Z".to_owned(),
        updated_at: "2026-04-03T00:00:00Z".to_owned(),
    })
    .await
    .expect("save integer group binding");

    runtime
        .execute(
            "INSERT INTO generation_records (plant_id, month, generation_kwh, self_consumption_kwh, supply_kwh, application_deadline, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            vec![
                "plant-integer".into(),
                "2026-03".into(),
                1000_i64.into(),
                Option::<f64>::None.into(),
                900_i64.into(),
                Option::<String>::None.into(),
                "2026-04-03T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert integer generation");

    let router = api::build_router(service);
    let response = router
        .oneshot(
            Request::builder()
                .uri("/api/v1/plants/FAC-INTEGER")
                .header("authorization", auth_support::bearer_header("group-user"))
                .header("x-organization-id", "worker-org")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("integer detail response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("detail body");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("detail json");
    assert_eq!(json["generation"]["count"], 1);
    assert_eq!(json["generation"]["totalKwh"], 1000.0);
}

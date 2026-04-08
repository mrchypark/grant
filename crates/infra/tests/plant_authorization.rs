use domain::{RoleBinding, RoleBindingResourceType, RoleBindingState, RoleBindingSubjectType};
use hiqlite::{Node, NodeConfig, s3::EncKeys};
use infra::{
    artifacts::ArtifactStore,
    config::{
        AppConfig, ArtifactBackendConfig, ArtifactConfig, AuthConfig, HiqliteConfig, LocalNodeRole,
        ReadConsistency, RuntimeMode, SecurityConfig, ServerConfig, TrailbaseAuthConfig,
    },
    database::Database,
    runtime::HiqliteRuntime,
    service::AppService,
};
use tempfile::TempDir;
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

async fn build_service() -> (AppService, Database, HiqliteRuntime, TempDir, TempDir) {
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
    (service, db, runtime, db_dir, artifacts_dir)
}

async fn seed_authorization_fixture(db: &Database, runtime: &HiqliteRuntime) {
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
}

#[tokio::test]
async fn lists_only_visible_plants_and_requires_profile_read_for_detail() {
    let (service, db, runtime, _db_dir, _artifacts_dir) = build_service().await;
    seed_authorization_fixture(&db, &runtime).await;

    let visible = service
        .list_visible_plants("group-user", "worker-org", None)
        .await
        .expect("visible plants");
    assert_eq!(
        visible.len(),
        1,
        "only profile-visible plants should be listed"
    );
    assert_eq!(visible[0].id, "plant-profile");
    assert_eq!(visible[0].facility_code, "FAC-PROFILE");

    let direct_visible = service
        .list_visible_plants("direct-user", "worker-org", None)
        .await
        .expect("direct visible plants");
    assert!(
        direct_visible.is_empty(),
        "document.read alone should not make a plant visible"
    );

    let profile = service
        .get_plant_profile("group-user", "worker-org", "FAC-PROFILE", None)
        .await
        .expect("authorized profile");
    assert_eq!(profile.plant.id, "plant-profile");

    let err = service
        .get_plant_profile("direct-user", "worker-org", "FAC-DOCUMENT", None)
        .await
        .expect_err("document-only access should be rejected for profile read");
    assert!(
        err.to_string().contains("forbidden"),
        "expected forbidden error, got {err:#}"
    );
}

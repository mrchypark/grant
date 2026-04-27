use domain::{
    Agreement, AgreementDocument, AgreementDocumentType, AgreementStatus, AgreementType, Client,
    ClientAction, ClientAppType, ClientConsentPolicy, ClientGrant, ClientGrantAction,
    ClientGrantStatus, ClientType, GenerationRecord, Plant, RateRecord, RecRecord, Role,
    RoleBinding, RoleBindingResourceType, RoleBindingState, RoleBindingSubjectType, SyncMode,
};
use hiqlite::{Node, NodeConfig, Row, s3::EncKeys};
use infra::{
    config::ReadConsistency,
    database::{Database, KrRpsSyncMaterialization, PersistedDocument, StoredRateRecord},
    runtime::HiqliteRuntime,
};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

#[derive(Debug)]
struct OrganizationRow {
    id: String,
    party_id: Option<String>,
    name: String,
}

impl From<&mut Row<'_>> for OrganizationRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            id: row.get("id"),
            party_id: row.get("party_id"),
            name: row.get("name"),
        }
    }
}

#[derive(Debug)]
struct PartyRow {
    id: String,
    #[allow(dead_code)]
    business_no: String,
    legal_name: String,
}

impl From<&mut Row<'_>> for PartyRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            id: row.get("id"),
            business_no: row.get("business_no"),
            legal_name: row.get("legal_name"),
        }
    }
}

#[derive(Debug)]
struct MembershipRow {
    organization_id: String,
    user_id: String,
    membership_role: String,
    status: String,
}

impl From<&mut Row<'_>> for MembershipRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            organization_id: row.get("organization_id"),
            user_id: row.get("user_id"),
            membership_role: row.get("membership_role"),
            status: row.get("status"),
        }
    }
}

#[derive(Debug)]
struct ActionRow {
    code: String,
    visibility_granting: i64,
}

impl From<&mut Row<'_>> for ActionRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            code: row.get("code"),
            visibility_granting: row.get("visibility_granting"),
        }
    }
}

#[derive(Debug)]
struct CountRow {
    count: i64,
}

impl From<&mut Row<'_>> for CountRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            count: row.get("count"),
        }
    }
}

#[derive(Debug)]
struct OptionalTextRow {
    value: Option<String>,
}

impl From<&mut Row<'_>> for OptionalTextRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            value: row.get("value"),
        }
    }
}

#[derive(Debug)]
struct RequiredTextRow {
    value: String,
}

impl From<&mut Row<'_>> for RequiredTextRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            value: row.get("value"),
        }
    }
}

#[derive(Debug)]
struct GenerationScopeRow {
    asset_id: String,
    source_id: String,
}

impl From<&mut Row<'_>> for GenerationScopeRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            asset_id: row.get("asset_id"),
            source_id: row.get("source_id"),
        }
    }
}

fn stable_scoped_id(scope: &str, value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(scope.as_bytes());
    hasher.update(b":");
    hasher.update(value.as_bytes());
    format!(
        "{}-{}",
        scope.replace(':', "-"),
        hex::encode(&hasher.finalize()[..8])
    )
}

#[derive(Debug)]
struct RoleRow {
    id: String,
    code: String,
}

impl From<&mut Row<'_>> for RoleRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            id: row.get("id"),
            code: row.get("code"),
        }
    }
}

#[derive(Debug)]
struct RoleActionRow {
    role_id: String,
    action_id: String,
}

impl From<&mut Row<'_>> for RoleActionRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            role_id: row.get("role_id"),
            action_id: row.get("action_id"),
        }
    }
}

#[derive(Debug)]
struct RoleBindingRow {
    subject_type: String,
    subject_id: String,
    subject_organization_id: String,
    resource_type: String,
    resource_id: String,
    role_id: String,
    state: String,
}

#[derive(Debug)]
struct ClientRow {
    id: String,
    organization_id: String,
    slug: String,
    name: String,
    client_type: String,
    app_type: String,
    first_party: i64,
    consent_policy: String,
    active: i64,
    created_by_user_id: String,
}

impl From<&mut Row<'_>> for ClientRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            id: row.get("id"),
            organization_id: row.get("organization_id"),
            slug: row.get("slug"),
            name: row.get("name"),
            client_type: row.get("client_type"),
            app_type: row.get("app_type"),
            first_party: row.get("first_party"),
            consent_policy: row.get("consent_policy"),
            active: row.get("active"),
            created_by_user_id: row.get("created_by_user_id"),
        }
    }
}

#[derive(Debug)]
struct ClientGrantRow {
    id: String,
    user_id: String,
    client_id: String,
    organization_id: String,
    status: String,
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct UserAssetVisibilityRow {
    user_id: String,
    organization_id: String,
    asset_id: String,
}

impl From<&mut Row<'_>> for UserAssetVisibilityRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            user_id: row.get("user_id"),
            organization_id: row.get("organization_id"),
            asset_id: row.get("asset_id"),
        }
    }
}

impl From<&mut Row<'_>> for ClientGrantRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            id: row.get("id"),
            user_id: row.get("user_id"),
            client_id: row.get("client_id"),
            organization_id: row.get("organization_id"),
            status: row.get("status"),
        }
    }
}

impl From<&mut Row<'_>> for RoleBindingRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            subject_type: row.get("subject_type"),
            subject_id: row.get("subject_id"),
            subject_organization_id: row.get("subject_organization_id"),
            resource_type: row.get("resource_type"),
            resource_id: row.get("resource_id"),
            role_id: row.get("role_id"),
            state: row.get("state"),
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

#[tokio::test]
async fn plant_upsert_rejects_cross_organization_facility_takeover() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime,
        "bootstrap-user".to_owned(),
        "Bootstrap User".to_owned(),
    );
    db.ensure_schema().await.expect("ensure schema");

    let mut plant = Plant {
        id: "FAC-SHARED".to_owned(),
        facility_code: "FAC-SHARED".to_owned(),
        owner_organization_id: "ops-seoul".to_owned(),
        created_by_organization_id: None,
        name: "Seoul Plant".to_owned(),
        contract_no: "CONTRACT-1".to_owned(),
        address: "Seoul".to_owned(),
        installed_kw: 100.0,
        rec_start_date: "2026-01-01".to_owned(),
        confirm_issued: "Y".to_owned(),
    };
    db.upsert_plant(&plant).await.expect("initial plant");

    plant.owner_organization_id = "ops-busan".to_owned();
    plant.name = "Busan Plant".to_owned();
    let err = db
        .upsert_plant(&plant)
        .await
        .expect_err("cross-organization takeover");
    assert!(
        err.to_string()
            .contains("plant already belongs to another organization")
    );

    let stored = db
        .get_plant_by_facility_code("FAC-SHARED")
        .await
        .expect("stored plant");
    assert_eq!(stored.owner_organization_id, "ops-seoul");
    assert_eq!(stored.name, "Seoul Plant");
}

#[tokio::test]
async fn asset_role_bindings_expand_to_asset_visibility() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "conalog-account".to_owned(),
        "Conalog Account".to_owned(),
    );

    db.ensure_schema().await.expect("ensure schema");
    runtime
        .execute(
            "INSERT INTO assets (
                id, owner_organization_id, created_by_organization_id, name, country_code,
                address, capacity_kw, timezone, created_at, updated_at
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
            vec![
                "asset-1".into(),
                "conalog-account".into(),
                "conalog-account".into(),
                "Asset One".into(),
                "KR".into(),
                "Busan".into(),
                100.0.into(),
                "Asia/Seoul".into(),
                "2026-04-25T00:00:00Z".into(),
                "2026-04-25T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert asset");
    db.upsert_role(&Role {
        id: "role-asset-profile-reader".to_owned(),
        organization_id: Some("conalog-account".to_owned()),
        code: "asset_profile_reader".to_owned(),
        name: "Asset Profile Reader".to_owned(),
        description: None,
    })
    .await
    .expect("upsert asset role");
    db.upsert_role_action("role-asset-profile-reader", "asset.profile.read")
        .await
        .expect("bind asset action");

    let now = "2026-04-25T00:00:00Z".to_owned();
    db.save_role_binding(&RoleBinding {
        id: "binding-org-asset-reader".to_owned(),
        subject_type: RoleBindingSubjectType::Organization,
        subject_id: "conalog-account".to_owned(),
        subject_organization_id: "conalog-account".to_owned(),
        resource_type: RoleBindingResourceType::Asset,
        resource_id: "asset-1".to_owned(),
        role_id: "role-asset-profile-reader".to_owned(),
        agreement_id: None,
        state: RoleBindingState::Active,
        created_by_user_id: "conalog-account".to_owned(),
        created_at: now.clone(),
        updated_at: now.clone(),
    })
    .await
    .expect("save organization asset binding");
    db.save_role_binding(&RoleBinding {
        id: "binding-user-asset-reader".to_owned(),
        subject_type: RoleBindingSubjectType::User,
        subject_id: "conalog-account".to_owned(),
        subject_organization_id: "conalog-account".to_owned(),
        resource_type: RoleBindingResourceType::Asset,
        resource_id: "asset-1".to_owned(),
        role_id: "role-asset-profile-reader".to_owned(),
        agreement_id: None,
        state: RoleBindingState::Active,
        created_by_user_id: "conalog-account".to_owned(),
        created_at: now.clone(),
        updated_at: now,
    })
    .await
    .expect("save user asset binding");

    let rows: Vec<UserAssetVisibilityRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT user_id, organization_id, asset_id FROM effective_user_asset_visibility",
            vec![],
        )
        .await
        .expect("asset visibility rows");

    assert_eq!(
        rows,
        vec![UserAssetVisibilityRow {
            user_id: "conalog-account".to_owned(),
            organization_id: "conalog-account".to_owned(),
            asset_id: "asset-1".to_owned(),
        }]
    );
}

#[tokio::test]
async fn ensure_schema_does_not_backfill_kr_rps_asset_projection_from_existing_plants() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "conalog-account".to_owned(),
        "Conalog Account".to_owned(),
    );

    db.ensure_schema_only().await.expect("ensure schema only");
    db.upsert_plant(&Plant {
        id: "legacy-plant-1".to_owned(),
        facility_code: "FAC-BACKFILL-001".to_owned(),
        name: "Legacy Plant".to_owned(),
        owner_organization_id: "legacy-owner-org".to_owned(),
        created_by_organization_id: Some("legacy-creator-org".to_owned()),
        contract_no: "CN-BACKFILL-001".to_owned(),
        address: "Seoul".to_owned(),
        installed_kw: 120.0,
        rec_start_date: "2026-01-01".to_owned(),
        confirm_issued: "Y".to_owned(),
    })
    .await
    .expect("upsert preexisting plant");

    db.ensure_schema().await.expect("ensure schema");

    db.get_kr_rps_asset_id_by_facility_code_with_consistency("FAC-BACKFILL-001", None)
        .await
        .expect_err("ensure_schema must not create a KR RPS asset projection for old plant rows");

    let projection_counts: Vec<CountRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT (
                 SELECT COUNT(*)
                 FROM asset_profiles
                 WHERE provider = 'korea_energy_rps'
                   AND profile_type = 'kr_rps'
                   AND external_id = $1
             ) + (
                 SELECT COUNT(*)
                 FROM registry_projects
                 WHERE registry_id = 'registry-kr-rps'
                   AND external_project_id = $1
             ) + (
                 SELECT COUNT(*)
                 FROM data_sources
                 WHERE provider = 'korea_energy_rps'
                   AND external_source_id = $1
             ) AS count",
            vec!["FAC-BACKFILL-001".into()],
        )
        .await
        .expect("KR RPS projection count");
    assert_eq!(projection_counts[0].count, 0);
}

#[tokio::test]
async fn kr_rps_asset_projection_without_business_no_does_not_fabricate_registry_account() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "conalog-account".to_owned(),
        "Conalog Account".to_owned(),
    );

    db.ensure_schema().await.expect("ensure schema");
    let plant = Plant {
        id: "plant-no-registry-account".to_owned(),
        facility_code: "FAC-NO-REGISTRY-ACCOUNT".to_owned(),
        name: "No Registry Account Plant".to_owned(),
        owner_organization_id: "conalog-account".to_owned(),
        created_by_organization_id: Some("conalog-account".to_owned()),
        contract_no: "CN-NO-REGISTRY-ACCOUNT".to_owned(),
        address: "Seoul".to_owned(),
        installed_kw: 120.0,
        rec_start_date: "2026-01-01".to_owned(),
        confirm_issued: "Y".to_owned(),
    };
    db.upsert_plant(&plant).await.expect("upsert plant");
    let asset_id = db
        .upsert_kr_rps_asset_projection(&plant, None, None, &[], &[])
        .await
        .expect("upsert KR RPS asset projection");
    assert!(
        asset_id.starts_with("asset-") && !asset_id.starts_with("asset-kr_rps-"),
        "asset_id must be an internal canonical id, not a KR RPS facility-code derived id"
    );

    let registry_accounts: Vec<CountRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT COUNT(*) AS count FROM registry_accounts WHERE registry_id = $1",
            vec!["registry-kr-rps".into()],
        )
        .await
        .expect("registry account count");
    assert_eq!(
        registry_accounts[0].count, 0,
        "projection must not fabricate a registry account without a real external account"
    );

    let registry_projects_without_account: Vec<CountRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT COUNT(*) AS count
             FROM registry_projects
             WHERE external_project_id = $1 AND registry_account_id IS NULL",
            vec!["FAC-NO-REGISTRY-ACCOUNT".into()],
        )
        .await
        .expect("registry project without account count");
    assert_eq!(registry_projects_without_account[0].count, 1);
}

#[tokio::test]
async fn kr_rps_projection_without_business_no_preserves_existing_registry_account_link() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "conalog-account".to_owned(),
        "Conalog Account".to_owned(),
    );

    db.ensure_schema().await.expect("ensure schema");
    db.upsert_organization_identity("conalog-account", Some("2768603291"), "Conalog Account")
        .await
        .expect("organization identity");
    db.save_certificate(
        "conalog-account",
        Some("2768603291"),
        "Conalog Account Certificate",
        "encrypted-bundle",
        b"wrapped-dek",
        "local-dev-v1",
    )
    .await
    .expect("save certificate");
    let plant = Plant {
        id: "plant-preserve-registry-account".to_owned(),
        facility_code: "FAC-PRESERVE-REGISTRY-ACCOUNT".to_owned(),
        name: "Preserve Registry Account Plant".to_owned(),
        owner_organization_id: "conalog-account".to_owned(),
        created_by_organization_id: Some("conalog-account".to_owned()),
        contract_no: "CN-PRESERVE-REGISTRY-ACCOUNT".to_owned(),
        address: "Seoul".to_owned(),
        installed_kw: 120.0,
        rec_start_date: "2026-01-01".to_owned(),
        confirm_issued: "Y".to_owned(),
    };
    db.upsert_plant(&plant).await.expect("upsert plant");
    db.upsert_kr_rps_asset_projection(
        &plant,
        Some("conalog-account"),
        Some("2768603291"),
        &[],
        &[],
    )
    .await
    .expect("upsert KR RPS asset projection with account");

    let before: Vec<OptionalTextRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT registry_account_id AS value
             FROM registry_projects
             WHERE registry_id = 'registry-kr-rps' AND external_project_id = $1",
            vec!["FAC-PRESERVE-REGISTRY-ACCOUNT".into()],
        )
        .await
        .expect("registry project account before");
    assert!(before[0].value.is_some());
    let credential_bindings: Vec<CountRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT COUNT(*) AS count
             FROM credential_bindings
             WHERE registry_account_id = $1",
            vec![before[0].value.clone().expect("registry account id").into()],
        )
        .await
        .expect("credential binding count");
    assert_eq!(credential_bindings[0].count, 1);

    db.save_certificate(
        "conalog-account",
        Some("2768603291"),
        "Replacement Conalog Account Certificate",
        "replacement-encrypted-bundle",
        b"replacement-wrapped-dek",
        "local-dev-v1",
    )
    .await
    .expect("replace certificate");
    let active_bound_credentials: Vec<CountRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT COUNT(*) AS count
             FROM credential_bindings cb
             JOIN auth_credentials ac ON ac.id = cb.credential_id
             WHERE cb.registry_account_id = $1
               AND ac.active = 1
               AND ac.provider = 'korea_energy_rps'
               AND ac.credential_type = 'certificate'",
            vec![before[0].value.clone().expect("registry account id").into()],
        )
        .await
        .expect("active bound credential count");
    assert_eq!(active_bound_credentials[0].count, 1);

    db.upsert_kr_rps_asset_projection(&plant, None, None, &[], &[])
        .await
        .expect("upsert KR RPS asset projection without account");

    let after: Vec<OptionalTextRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT registry_account_id AS value
             FROM registry_projects
             WHERE registry_id = 'registry-kr-rps' AND external_project_id = $1",
            vec!["FAC-PRESERVE-REGISTRY-ACCOUNT".into()],
        )
        .await
        .expect("registry project account after");
    assert_eq!(after[0].value, before[0].value);
}

#[tokio::test]
async fn kr_rps_projection_conflict_preserves_first_claimed_owner_and_account() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "conalog-account".to_owned(),
        "Conalog Account".to_owned(),
    );

    db.ensure_schema().await.expect("ensure schema");
    db.upsert_organization_identity("owner-org", Some("1234567890"), "Owner Org")
        .await
        .expect("owner identity");
    db.upsert_organization_identity("operator-org", Some("2768603291"), "Operator Org")
        .await
        .expect("operator identity");
    let owner_plant = Plant {
        id: "plant-first-claim".to_owned(),
        facility_code: "FAC-FIRST-CLAIM".to_owned(),
        name: "First Claim Plant".to_owned(),
        owner_organization_id: "owner-org".to_owned(),
        created_by_organization_id: Some("owner-org".to_owned()),
        contract_no: "CN-FIRST-CLAIM".to_owned(),
        address: "Seoul".to_owned(),
        installed_kw: 120.0,
        rec_start_date: "2026-01-01".to_owned(),
        confirm_issued: "Y".to_owned(),
    };
    db.upsert_plant(&owner_plant).await.expect("owner plant");
    db.upsert_kr_rps_asset_projection(
        &owner_plant,
        Some("owner-org"),
        Some("1234567890"),
        &[],
        &[],
    )
    .await
    .expect("owner projection");

    let operator_plant = Plant {
        owner_organization_id: "operator-org".to_owned(),
        created_by_organization_id: Some("operator-org".to_owned()),
        ..owner_plant.clone()
    };
    let err = db
        .upsert_plant(&operator_plant)
        .await
        .expect_err("operator conflicting plant must not overwrite plant owner");
    assert!(
        err.to_string()
            .contains("plant already belongs to another organization")
    );
    db.upsert_kr_rps_asset_projection(
        &operator_plant,
        Some("operator-org"),
        Some("2768603291"),
        &[],
        &[],
    )
    .await
    .expect("operator conflicting projection");

    let plant = db
        .get_plant_by_facility_code("FAC-FIRST-CLAIM")
        .await
        .expect("plant after conflict");
    assert_eq!(plant.owner_organization_id, "owner-org");

    let asset_owners: Vec<RequiredTextRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT owner_organization_id AS value
             FROM assets a
             JOIN asset_profiles ap ON ap.asset_id = a.id
             WHERE ap.external_id = $1",
            vec!["FAC-FIRST-CLAIM".into()],
        )
        .await
        .expect("asset owner");
    assert_eq!(asset_owners[0].value, "owner-org");

    let registry_accounts: Vec<RequiredTextRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT ra.organization_id AS value
             FROM registry_projects rp
             JOIN registry_accounts ra ON ra.id = rp.registry_account_id
             WHERE rp.registry_id = 'registry-kr-rps' AND rp.external_project_id = $1",
            vec!["FAC-FIRST-CLAIM".into()],
        )
        .await
        .expect("registry account owner");
    assert_eq!(registry_accounts[0].value, "owner-org");
}

#[tokio::test]
async fn kr_rps_stored_credential_use_requires_binding_to_project_registry_account() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "conalog-account".to_owned(),
        "Conalog Account".to_owned(),
    );

    db.ensure_schema().await.expect("ensure schema");
    db.upsert_organization_identity("owner-org", Some("1234567890"), "Owner Org")
        .await
        .expect("owner identity");
    db.upsert_organization_identity("operator-org", Some("2768603291"), "Operator Org")
        .await
        .expect("operator identity");
    db.save_certificate(
        "owner-org",
        Some("1234567890"),
        "Owner Certificate",
        "owner-encrypted-bundle",
        b"owner-wrapped-dek",
        "local-dev-v1",
    )
    .await
    .expect("save owner certificate");
    db.save_certificate(
        "operator-org",
        Some("2768603291"),
        "Operator Certificate",
        "operator-encrypted-bundle",
        b"operator-wrapped-dek",
        "local-dev-v1",
    )
    .await
    .expect("save operator certificate");

    let plant = Plant {
        id: "plant-bound-registry-account".to_owned(),
        facility_code: "FAC-BOUND-REGISTRY-ACCOUNT".to_owned(),
        name: "Bound Registry Account Plant".to_owned(),
        owner_organization_id: "owner-org".to_owned(),
        created_by_organization_id: Some("operator-org".to_owned()),
        contract_no: "CN-BOUND-REGISTRY-ACCOUNT".to_owned(),
        address: "Seoul".to_owned(),
        installed_kw: 120.0,
        rec_start_date: "2026-01-01".to_owned(),
        confirm_issued: "Y".to_owned(),
    };
    db.upsert_plant(&plant).await.expect("upsert plant");
    db.upsert_kr_rps_asset_projection(&plant, Some("owner-org"), Some("1234567890"), &[], &[])
        .await
        .expect("upsert owner registry projection");

    assert!(
        db.has_usable_kr_rps_certificate_credential_with_consistency(
            "owner-org",
            "FAC-BOUND-REGISTRY-ACCOUNT",
            Some(ReadConsistency::Strong),
        )
        .await
        .expect("owner credential usability")
    );
    assert!(
        !db.has_usable_kr_rps_certificate_credential_with_consistency(
            "operator-org",
            "FAC-BOUND-REGISTRY-ACCOUNT",
            Some(ReadConsistency::Strong),
        )
        .await
        .expect("operator credential usability"),
        "operator's unrelated active certificate must not satisfy owner registry project binding"
    );
}

#[tokio::test]
async fn kr_rps_stored_credential_null_registry_account_fallback_is_owner_only() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime,
        "conalog-account".to_owned(),
        "Conalog Account".to_owned(),
    );

    db.ensure_schema().await.expect("ensure schema");
    db.upsert_organization_identity("owner-org", Some("1234567890"), "Owner Org")
        .await
        .expect("owner identity");
    db.upsert_organization_identity("operator-org", Some("2768603291"), "Operator Org")
        .await
        .expect("operator identity");
    db.save_certificate(
        "owner-org",
        Some("1234567890"),
        "Owner Certificate",
        "owner-encrypted-bundle",
        b"owner-wrapped-dek",
        "local-dev-v1",
    )
    .await
    .expect("save owner certificate");
    db.save_certificate(
        "operator-org",
        Some("2768603291"),
        "Operator Certificate",
        "operator-encrypted-bundle",
        b"operator-wrapped-dek",
        "local-dev-v1",
    )
    .await
    .expect("save operator certificate");

    let plant = Plant {
        id: "plant-null-registry-account".to_owned(),
        facility_code: "FAC-NULL-REGISTRY-ACCOUNT".to_owned(),
        name: "Null Registry Account Plant".to_owned(),
        owner_organization_id: "owner-org".to_owned(),
        created_by_organization_id: Some("operator-org".to_owned()),
        contract_no: "CN-NULL-REGISTRY-ACCOUNT".to_owned(),
        address: "Seoul".to_owned(),
        installed_kw: 120.0,
        rec_start_date: "2026-01-01".to_owned(),
        confirm_issued: "Y".to_owned(),
    };
    db.upsert_plant(&plant).await.expect("upsert plant");
    db.upsert_kr_rps_asset_projection(&plant, None, None, &[], &[])
        .await
        .expect("upsert projection without registry account");

    assert!(
        db.has_usable_kr_rps_certificate_credential_with_consistency(
            "owner-org",
            "FAC-NULL-REGISTRY-ACCOUNT",
            Some(ReadConsistency::Strong),
        )
        .await
        .expect("owner credential usability")
    );
    assert!(
        !db.has_usable_kr_rps_certificate_credential_with_consistency(
            "operator-org",
            "FAC-NULL-REGISTRY-ACCOUNT",
            Some(ReadConsistency::Strong),
        )
        .await
        .expect("operator credential usability")
    );
}

#[tokio::test]
async fn failed_kr_rps_full_projection_preserves_existing_canonical_rows() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime,
        "conalog-account".to_owned(),
        "Conalog Account".to_owned(),
    );

    db.ensure_schema().await.expect("ensure schema");
    let plant = Plant {
        id: "plant-failed-full-preserve".to_owned(),
        facility_code: "FAC-FAILED-FULL-PRESERVE".to_owned(),
        name: "Failed Full Preserve Plant".to_owned(),
        owner_organization_id: "conalog-account".to_owned(),
        created_by_organization_id: Some("conalog-account".to_owned()),
        contract_no: "CN-FAILED-FULL-PRESERVE".to_owned(),
        address: "Seoul".to_owned(),
        installed_kw: 100.0,
        rec_start_date: "2026-01-01".to_owned(),
        confirm_issued: "Y".to_owned(),
    };
    db.upsert_plant(&plant).await.expect("upsert plant");
    db.upsert_kr_rps_asset_projection(
        &plant,
        Some("conalog-account"),
        Some("2768603291"),
        &[GenerationRecord {
            month: "2026-03".to_owned(),
            generation_kwh: 1000.0,
            self_consumption_kwh: None,
            supply_kwh: 1000.0,
            application_deadline: None,
        }],
        &[],
    )
    .await
    .expect("seed canonical rows");

    db.replace_kr_rps_asset_projection(
        &plant,
        Some("conalog-account"),
        Some("2768603291"),
        &[GenerationRecord {
            month: "not-a-month".to_owned(),
            generation_kwh: 2000.0,
            self_consumption_kwh: None,
            supply_kwh: 2000.0,
            application_deadline: None,
        }],
        &[],
    )
    .await
    .expect_err("invalid full replacement should fail");

    let counts = db
        .kr_rps_projection_counts("FAC-FAILED-FULL-PRESERVE")
        .await
        .expect("projection counts");
    assert_eq!(counts.generation_data, 1);
}

#[tokio::test]
async fn kr_rps_energy_certificates_do_not_collide_on_application_no_across_plants() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "conalog-account".to_owned(),
        "Conalog Account".to_owned(),
    );

    db.ensure_schema().await.expect("ensure schema");
    let rec = [RecRecord {
        month: "2026-02".to_owned(),
        application_no: "APP-SHARED".to_owned(),
        supply_kwh: 1000,
        rec_quantity: 1,
        fee_krw: 1000,
        issued_date: "2026-03-01".to_owned(),
        expiry_date: "2027-03-01".to_owned(),
        virtual_account: None,
        bank_name: None,
        status: Some("issued".to_owned()),
    }];
    for facility_code in ["FAC-CERT-001", "FAC-CERT-002"] {
        let plant = Plant {
            id: format!("plant-{facility_code}"),
            facility_code: facility_code.to_owned(),
            name: format!("Certificate Plant {facility_code}"),
            owner_organization_id: "conalog-account".to_owned(),
            created_by_organization_id: Some("conalog-account".to_owned()),
            contract_no: format!("CN-{facility_code}"),
            address: "Seoul".to_owned(),
            installed_kw: 100.0,
            rec_start_date: "2026-01-01".to_owned(),
            confirm_issued: "Y".to_owned(),
        };
        db.upsert_plant(&plant).await.expect("upsert plant");
        db.upsert_kr_rps_asset_projection(
            &plant,
            Some("conalog-account"),
            Some("2768603291"),
            &[],
            &rec,
        )
        .await
        .expect("upsert KR RPS asset projection");
    }

    let certificate_count: Vec<CountRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT COUNT(*) AS count
             FROM energy_certificates
             WHERE registry_id = 'registry-kr-rps'
               AND serial_number IN ('FAC-CERT-001:APP-SHARED', 'FAC-CERT-002:APP-SHARED')",
            vec![],
        )
        .await
        .expect("energy certificate count");
    assert_eq!(certificate_count[0].count, 2);
}

#[tokio::test]
async fn plant_role_bindings_do_not_project_to_kr_rps_asset_actions() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime,
        "conalog-account".to_owned(),
        "Conalog Account".to_owned(),
    );

    db.ensure_schema().await.expect("ensure schema");
    let plant = Plant {
        id: "legacy-plant-permission".to_owned(),
        facility_code: "FAC-LEGACY-PERMISSION".to_owned(),
        name: "Legacy Permission Plant".to_owned(),
        owner_organization_id: "conalog-account".to_owned(),
        created_by_organization_id: Some("conalog-account".to_owned()),
        contract_no: "CN-LEGACY-PERMISSION".to_owned(),
        address: "Seoul".to_owned(),
        installed_kw: 100.0,
        rec_start_date: "2026-01-01".to_owned(),
        confirm_issued: "Y".to_owned(),
    };
    db.upsert_plant(&plant).await.expect("upsert plant");
    let asset_id = db
        .upsert_kr_rps_asset_projection(&plant, None, None, &[], &[])
        .await
        .expect("upsert asset projection");

    db.upsert_role(&Role {
        id: "role-legacy-plant-operator".to_owned(),
        organization_id: Some("conalog-account".to_owned()),
        code: "plant_operator".to_owned(),
        name: "Plant Operator".to_owned(),
        description: None,
    })
    .await
    .expect("upsert legacy role");
    for action in ["plant.profile.read", "plant.sync.execute"] {
        db.upsert_role_action("role-legacy-plant-operator", action)
            .await
            .expect("bind legacy action");
    }

    let now = "2026-04-25T00:00:00Z".to_owned();
    for (binding_id, subject_type, subject_id) in [
        (
            "binding-org-plant-operator",
            RoleBindingSubjectType::Organization,
            "conalog-account",
        ),
        (
            "binding-user-plant-operator",
            RoleBindingSubjectType::User,
            "conalog-account",
        ),
    ] {
        db.save_role_binding(&RoleBinding {
            id: binding_id.to_owned(),
            subject_type,
            subject_id: subject_id.to_owned(),
            subject_organization_id: "conalog-account".to_owned(),
            resource_type: RoleBindingResourceType::Plant,
            resource_id: plant.id.clone(),
            role_id: "role-legacy-plant-operator".to_owned(),
            agreement_id: None,
            state: RoleBindingState::Active,
            created_by_user_id: "conalog-account".to_owned(),
            created_at: now.clone(),
            updated_at: now.clone(),
        })
        .await
        .expect("save plant binding");
    }

    assert!(
        !db.has_user_asset_action_with_consistency(
            "conalog-account",
            "conalog-account",
            &asset_id,
            "asset.profile.read",
            Some(ReadConsistency::Local),
        )
        .await
        .expect("asset profile action")
    );
    assert!(
        !db.has_user_asset_action_with_consistency(
            "conalog-account",
            "conalog-account",
            &asset_id,
            "asset.sync.execute",
            Some(ReadConsistency::Local),
        )
        .await
        .expect("asset sync action")
    );
}

#[tokio::test]
async fn kr_rps_full_projection_replace_removes_stale_canonical_rows() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "conalog-account".to_owned(),
        "Conalog Account".to_owned(),
    );

    db.ensure_schema().await.expect("ensure schema");
    let plant = Plant {
        id: "FAC-FULL-001".to_owned(),
        facility_code: "FAC-FULL-001".to_owned(),
        name: "Full Sync Plant".to_owned(),
        owner_organization_id: "conalog-account".to_owned(),
        created_by_organization_id: Some("conalog-account".to_owned()),
        contract_no: "CN-FULL-001".to_owned(),
        address: "Seoul".to_owned(),
        installed_kw: 100.0,
        rec_start_date: "2026-01-01".to_owned(),
        confirm_issued: "Y".to_owned(),
    };
    db.upsert_plant(&plant).await.expect("upsert plant");
    db.upsert_kr_rps_asset_projection(
        &plant,
        Some("conalog-account"),
        Some("2768603291"),
        &[
            GenerationRecord {
                month: "2026-02".to_owned(),
                generation_kwh: 500.0,
                self_consumption_kwh: None,
                supply_kwh: 500.0,
                application_deadline: None,
            },
            GenerationRecord {
                month: "2026-03".to_owned(),
                generation_kwh: 1000.0,
                self_consumption_kwh: None,
                supply_kwh: 1000.0,
                application_deadline: None,
            },
        ],
        &[
            RecRecord {
                month: "2026-02".to_owned(),
                application_no: "APP-OLD".to_owned(),
                supply_kwh: 500,
                rec_quantity: 1,
                fee_krw: 1000,
                issued_date: "2026-03-01".to_owned(),
                expiry_date: "2027-03-01".to_owned(),
                virtual_account: None,
                bank_name: None,
                status: Some("issued".to_owned()),
            },
            RecRecord {
                month: "2026-03".to_owned(),
                application_no: "APP-KEEP".to_owned(),
                supply_kwh: 1000,
                rec_quantity: 1,
                fee_krw: 1000,
                issued_date: "2026-04-01".to_owned(),
                expiry_date: "2027-04-01".to_owned(),
                virtual_account: None,
                bank_name: None,
                status: Some("issued".to_owned()),
            },
        ],
    )
    .await
    .expect("initial canonical projection");

    let generation_scope: Vec<GenerationScopeRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT asset_id, source_id FROM generation_data LIMIT 1",
            vec![],
        )
        .await
        .expect("generation scope");
    let generation_scope = generation_scope
        .into_iter()
        .next()
        .expect("generation scope row");
    runtime
        .execute(
            "DELETE FROM generation_data
             WHERE asset_id = $1
               AND source_id = $2
               AND period_start = $3
               AND period_end = $4
               AND metric_type = $5",
            vec![
                generation_scope.asset_id.clone().into(),
                generation_scope.source_id.clone().into(),
                "2026-03-01".into(),
                "2026-04-01".into(),
                "gross_generation_kwh".into(),
            ],
        )
        .await
        .expect("delete current generation row before stale id regression");
    runtime
        .execute(
            "INSERT INTO generation_data (
                id, asset_id, source_id, period_start, period_end, metric_type, value,
                unit, quality, metadata_json, recorded_at, updated_at
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
            vec![
                "stale-generation-metric".into(),
                generation_scope.asset_id.clone().into(),
                generation_scope.source_id.clone().into(),
                "2026-03-01".into(),
                "2026-04-01".into(),
                "net_export_kwh".into(),
                900.0.into(),
                "kWh".into(),
                "settled".into(),
                "{}".into(),
                "2026-04-01T00:00:00Z".into(),
                "2026-04-01T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert stale generation metric");
    runtime
        .execute(
            "INSERT INTO generation_data (
                id, asset_id, source_id, period_start, period_end, metric_type, value,
                unit, quality, metadata_json, recorded_at, updated_at
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
            vec![
                stable_scoped_id(
                    "generation_data:kr_rps",
                    "FAC-FULL-001:2026-03:gross_generation_kwh",
                )
                .into(),
                generation_scope.asset_id.into(),
                generation_scope.source_id.into(),
                "2026-03-01".into(),
                "2026-05-01".into(),
                "gross_generation_kwh".into(),
                900.0.into(),
                "kWh".into(),
                "settled".into(),
                "{}".into(),
                "2026-04-01T00:00:00Z".into(),
                "2026-04-01T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert stale generation period end");

    db.replace_kr_rps_asset_projection(
        &plant,
        Some("conalog-account"),
        Some("2768603291"),
        &[GenerationRecord {
            month: "2026-03".to_owned(),
            generation_kwh: 1100.0,
            self_consumption_kwh: None,
            supply_kwh: 1100.0,
            application_deadline: None,
        }],
        &[RecRecord {
            month: "2026-03".to_owned(),
            application_no: "APP-KEEP".to_owned(),
            supply_kwh: 1100,
            rec_quantity: 1,
            fee_krw: 1200,
            issued_date: "2026-04-01".to_owned(),
            expiry_date: "2027-04-01".to_owned(),
            virtual_account: None,
            bank_name: None,
            status: Some("issued".to_owned()),
        }],
    )
    .await
    .expect("replace canonical projection");

    let counts = db
        .kr_rps_projection_counts("FAC-FULL-001")
        .await
        .expect("projection counts");
    assert_eq!(counts.generation_data, 1);
    assert_eq!(counts.energy_certificates, 1);

    let generation_metrics: Vec<RequiredTextRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT metric_type AS value FROM generation_data ORDER BY metric_type ASC",
            vec![],
        )
        .await
        .expect("generation metric semantics");
    assert_eq!(
        generation_metrics
            .into_iter()
            .map(|row| row.value)
            .collect::<Vec<_>>(),
        vec!["gross_generation_kwh"]
    );
}

#[tokio::test]
async fn kr_rps_full_sync_replaces_canonical_rate_data() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "conalog-account".to_owned(),
        "Conalog Account".to_owned(),
    );

    db.ensure_schema().await.expect("ensure schema");
    db.upsert_organization_identity("conalog-account", Some("2768603291"), "Conalog Account")
        .await
        .expect("organization identity");
    let plant = Plant {
        id: "plant-rate-canonical".to_owned(),
        facility_code: "FAC-RATE-CANONICAL".to_owned(),
        name: "Rate Canonical Plant".to_owned(),
        owner_organization_id: "conalog-account".to_owned(),
        created_by_organization_id: Some("conalog-account".to_owned()),
        contract_no: "CN-RATE-CANONICAL".to_owned(),
        address: "Seoul".to_owned(),
        installed_kw: 120.0,
        rec_start_date: "2026-01-01".to_owned(),
        confirm_issued: "Y".to_owned(),
    };
    let first_rates = vec![StoredRateRecord {
        record: RateRecord {
            month: "2026-03".to_owned(),
            self_rate: Some(110.0),
            national_average: Some(120.0),
            nearby_average: None,
            nearby_highest: Some(130.0),
        },
        published: true,
        source_control_flag: Some("first".to_owned()),
    }];
    let docs: Vec<PersistedDocument> = Vec::new();
    let run = db
        .start_sync_run("conalog-account", Some(&plant.id), SyncMode::Full)
        .await
        .expect("start first sync run");
    db.materialize_kr_rps_sync(KrRpsSyncMaterialization {
        run_id: &run.id,
        plant: &plant,
        registry_account_organization_id: Some("conalog-account"),
        business_no: Some("2768603291"),
        mode: &SyncMode::Full,
        generation: &[],
        rates: &first_rates,
        rec: &[],
        documents: &docs,
        duration_ms: 1,
    })
    .await
    .expect("materialize first full sync");

    let first_count: Vec<CountRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT COUNT(*) AS count FROM rate_data",
            vec![],
        )
        .await
        .expect("first rate data count");
    assert_eq!(first_count[0].count, 3);

    let rate_scope: Vec<GenerationScopeRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT asset_id, source_id FROM rate_data LIMIT 1",
            vec![],
        )
        .await
        .expect("rate scope");
    let rate_scope = rate_scope.into_iter().next().expect("rate scope row");
    runtime
        .execute(
            "DELETE FROM rate_data
             WHERE asset_id = $1
               AND source_id = $2
               AND period_start = $3
               AND period_end = $4
               AND metric_type = $5",
            vec![
                rate_scope.asset_id.clone().into(),
                rate_scope.source_id.clone().into(),
                "2026-03-01".into(),
                "2026-04-01".into(),
                "self_rate".into(),
            ],
        )
        .await
        .expect("delete current rate row before stale id regression");
    runtime
        .execute(
            "INSERT INTO rate_data (
                id, asset_id, source_id, period_start, period_end, metric_type, value,
                unit, currency, quality, metadata_json, recorded_at, updated_at
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
            vec![
                stable_scoped_id("rate_data:kr_rps", "FAC-RATE-CANONICAL:2026-03:self_rate").into(),
                rate_scope.asset_id.into(),
                rate_scope.source_id.into(),
                "2026-03-01".into(),
                "2026-05-01".into(),
                "self_rate".into(),
                109.0.into(),
                "KRW/kWh".into(),
                "KRW".into(),
                "published".into(),
                "{}".into(),
                "2026-04-01T00:00:00Z".into(),
                "2026-04-01T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert stale rate period end");

    let second_rates = vec![StoredRateRecord {
        record: RateRecord {
            month: "2026-03".to_owned(),
            self_rate: Some(111.0),
            national_average: None,
            nearby_average: None,
            nearby_highest: None,
        },
        published: true,
        source_control_flag: Some("second".to_owned()),
    }];
    let run = db
        .start_sync_run("conalog-account", Some(&plant.id), SyncMode::Full)
        .await
        .expect("start second sync run");
    db.materialize_kr_rps_sync(KrRpsSyncMaterialization {
        run_id: &run.id,
        plant: &plant,
        registry_account_organization_id: Some("conalog-account"),
        business_no: Some("2768603291"),
        mode: &SyncMode::Full,
        generation: &[],
        rates: &second_rates,
        rec: &[],
        documents: &docs,
        duration_ms: 1,
    })
    .await
    .expect("materialize second full sync");

    let remaining: Vec<RequiredTextRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT metric_type AS value FROM rate_data ORDER BY metric_type ASC",
            vec![],
        )
        .await
        .expect("remaining rate metrics");
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].value, "self_rate");
}

#[tokio::test]
async fn ensure_schema_seeds_bootstrap_organization_and_actions() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "conalog-account".to_owned(),
        "Conalog Account".to_owned(),
    );

    db.ensure_schema().await.expect("ensure schema");

    let organizations: Vec<OrganizationRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT o.id, opl.party_id, o.name
             FROM organizations o
             LEFT JOIN organization_party_links opl
               ON opl.organization_id = o.id
              AND opl.relation_type = 'legal'
             ORDER BY o.id ASC",
            vec![],
        )
        .await
        .expect("organizations");
    assert_eq!(organizations.len(), 1);
    assert_eq!(organizations[0].id, "conalog-account");
    assert_eq!(organizations[0].party_id, None);
    assert_eq!(organizations[0].name, "Conalog Account");

    let memberships: Vec<MembershipRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT organization_id, user_id, membership_role, status FROM organization_memberships ORDER BY organization_id ASC",
            vec![],
        )
        .await
        .expect("memberships");
    assert_eq!(memberships.len(), 1);
    assert_eq!(memberships[0].organization_id, "conalog-account");
    assert_eq!(memberships[0].user_id, "conalog-account");
    assert_eq!(memberships[0].membership_role, "owner");
    assert_eq!(memberships[0].status, "active");

    let actions: Vec<ActionRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT code, visibility_granting FROM actions ORDER BY code ASC",
            vec![],
        )
        .await
        .expect("actions");
    assert!(!actions.is_empty(), "action catalog must be seeded");
    let profile_read = actions
        .iter()
        .find(|action| action.code == "plant.profile.read")
        .expect("plant.profile.read action");
    assert_eq!(profile_read.visibility_granting, 1);
    let asset_profile_read = actions
        .iter()
        .find(|action| action.code == "asset.profile.read")
        .expect("asset.profile.read action");
    assert_eq!(asset_profile_read.visibility_granting, 1);
    assert!(
        actions
            .iter()
            .any(|action| action.code == "organization.plant.create"),
        "organization.plant.create action must exist"
    );

    let roles: Vec<RoleRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT id, code FROM roles ORDER BY code ASC",
            vec![],
        )
        .await
        .expect("roles");
    assert!(
        roles
            .iter()
            .any(|role| role.id == "role-system-bootstrap-owner"
                && role.code == "system.bootstrap.owner"),
        "bootstrap system role must be seeded"
    );

    let role_actions: Vec<RoleActionRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT role_id, action_id FROM role_actions WHERE role_id = 'role-system-bootstrap-owner' ORDER BY action_id ASC",
            vec![],
        )
        .await
        .expect("role actions");
    let seeded_actions: Vec<String> = role_actions
        .into_iter()
        .map(|row| {
            assert_eq!(row.role_id, "role-system-bootstrap-owner");
            row.action_id
        })
        .collect();
    assert!(seeded_actions.contains(&"organization.plant.create".to_owned()));

    let bindings: Vec<RoleBindingRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT subject_type, subject_id, subject_organization_id, resource_type, resource_id, role_id, state
             FROM role_bindings
             WHERE id = 'binding-system-bootstrap-owner'",
            vec![],
        )
        .await
        .expect("role bindings");
    assert_eq!(bindings.len(), 1);
    let binding = &bindings[0];
    assert_eq!(binding.subject_type, "user");
    assert_eq!(binding.subject_id, "conalog-account");
    assert_eq!(binding.subject_organization_id, "conalog-account");
    assert_eq!(binding.resource_type, "organization");
    assert_eq!(binding.resource_id, "conalog-account");
    assert_eq!(binding.role_id, "role-system-bootstrap-owner");
    assert_eq!(binding.state, "active");
}

#[tokio::test]
async fn organization_identity_links_party_without_overwriting_org_name() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "bootstrap-user".to_owned(),
        "Bootstrap User".to_owned(),
    );
    db.ensure_schema().await.expect("ensure schema");

    db.upsert_organization_with_owner("ops-seoul", "서울사업팀", None, "서울사업팀")
        .await
        .expect("organization");
    db.upsert_organization_identity("ops-seoul", Some("2768603291"), "주식회사 커널로그에너지")
        .await
        .expect("organization identity");

    let organizations: Vec<OrganizationRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT o.id, opl.party_id, o.name
             FROM organizations o
             LEFT JOIN organization_party_links opl
               ON opl.organization_id = o.id
              AND opl.relation_type = 'legal'
             WHERE o.id = 'ops-seoul'",
            vec![],
        )
        .await
        .expect("organizations");

    let parties: Vec<PartyRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT id, business_no, legal_name FROM parties WHERE business_no = '2768603291'",
            vec![],
        )
        .await
        .expect("parties");
    assert_eq!(organizations.len(), 1);
    assert_eq!(organizations[0].name, "서울사업팀");
    assert!(organizations[0].party_id.is_some());
    assert_eq!(parties.len(), 1);
    assert_eq!(parties[0].id, organizations[0].party_id.clone().unwrap());
    assert_eq!(parties[0].legal_name, "주식회사 커널로그에너지");
}

#[tokio::test]
async fn bootstrap_principals_preserve_existing_party_link() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "conalog-account".to_owned(),
        "Conalog Account".to_owned(),
    );
    db.ensure_schema().await.expect("ensure schema");
    db.upsert_organization_identity(
        "conalog-account",
        Some("2768603291"),
        "주식회사 커널로그에너지",
    )
    .await
    .expect("bootstrap identity");

    db.ensure_bootstrap_principals()
        .await
        .expect("reseed bootstrap principals");

    let organizations: Vec<OrganizationRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT o.id, opl.party_id, o.name
             FROM organizations o
             LEFT JOIN organization_party_links opl
               ON opl.organization_id = o.id
              AND opl.relation_type = 'legal'
             WHERE o.id = 'conalog-account'",
            vec![],
        )
        .await
        .expect("organizations");
    assert_eq!(organizations.len(), 1);
    assert_eq!(
        organizations[0].party_id.as_deref(),
        Some("party-2768603291")
    );
}

#[tokio::test]
async fn organization_identity_without_business_number_returns_existing_party_link() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "bootstrap-user".to_owned(),
        "Bootstrap User".to_owned(),
    );
    db.ensure_schema().await.expect("ensure schema");

    db.upsert_organization_with_owner("ops-seoul", "서울사업팀", None, "서울사업팀")
        .await
        .expect("organization");
    db.upsert_organization_identity("ops-seoul", Some("2768603291"), "주식회사 커널로그에너지")
        .await
        .expect("organization identity");
    let organization = db
        .upsert_organization_identity("ops-seoul", None, "ignored")
        .await
        .expect("organization identity without business number");

    assert_eq!(organization.party_id.as_deref(), Some("party-2768603291"));
}

#[tokio::test]
async fn organization_identity_reuses_existing_party_id_for_business_number() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "bootstrap-user".to_owned(),
        "Bootstrap User".to_owned(),
    );
    db.ensure_schema().await.expect("ensure schema");
    db.upsert_organization_with_owner("ops-seoul", "서울사업팀", None, "서울사업팀")
        .await
        .expect("organization");

    runtime
        .execute(
            "INSERT INTO parties (id, business_no, legal_name, active, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6)",
            vec![
                "party-custom".into(),
                "2768603291".into(),
                "주식회사 커널로그에너지".into(),
                1_i64.into(),
                "2026-04-11T00:00:00Z".into(),
                "2026-04-11T00:00:00Z".into(),
            ],
        )
        .await
        .expect("seed custom party");

    let organization = db
        .upsert_organization_identity(
            "ops-seoul",
            Some("2768603291"),
            "주식회사 커널로그에너지(변경)",
        )
        .await
        .expect("organization identity");

    assert_eq!(organization.party_id.as_deref(), Some("party-custom"));
    let parties: Vec<PartyRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT id, business_no, legal_name FROM parties WHERE business_no = '2768603291'",
            vec![],
        )
        .await
        .expect("parties");
    assert_eq!(parties.len(), 1);
    assert_eq!(parties[0].legal_name, "주식회사 커널로그에너지(변경)");
}

#[tokio::test]
async fn organization_creation_with_business_number_creates_legal_party_link() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "bootstrap-user".to_owned(),
        "Bootstrap User".to_owned(),
    );
    db.ensure_schema().await.expect("ensure schema");

    let organization = db
        .upsert_organization_with_owner(
            "ops-seoul",
            "서울사업팀",
            Some("2768603291"),
            "주식회사 커널로그에너지",
        )
        .await
        .expect("organization");

    assert_eq!(organization.party_id.as_deref(), Some("party-2768603291"));

    let parties: Vec<PartyRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT id, business_no, legal_name FROM parties WHERE business_no = '2768603291'",
            vec![],
        )
        .await
        .expect("parties");
    assert_eq!(parties.len(), 1);
    assert_eq!(parties[0].legal_name, "주식회사 커널로그에너지");
}

#[tokio::test]
async fn agreement_documents_round_trip_through_database() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "bootstrap-user".to_owned(),
        "Bootstrap User".to_owned(),
    );
    db.ensure_schema().await.expect("ensure schema");

    db.upsert_agreement(&Agreement {
        id: "agreement-1".to_owned(),
        agreement_type: AgreementType::Om,
        grantor_organization_id: "grantor-org".to_owned(),
        grantee_organization_id: "grantee-org".to_owned(),
        status: AgreementStatus::Active,
        title: "운영 위탁 계약".to_owned(),
        reference_no: Some("OM-2026-001".to_owned()),
        starts_at: Some("2026-01-01T00:00:00Z".to_owned()),
        ends_at: None,
    })
    .await
    .expect("agreement");

    db.upsert_agreement_document(&AgreementDocument {
        id: "agreement-doc-1".to_owned(),
        agreement_id: "agreement-1".to_owned(),
        document_type: AgreementDocumentType::MasterContract,
        file_name: "om-2026-001.pdf".to_owned(),
        object_key: "agreements/agreement-1/master.pdf".to_owned(),
        content_type: "application/pdf".to_owned(),
        document_no: Some("DOC-001".to_owned()),
        uploaded_at: "2026-04-11T00:00:00Z".to_owned(),
    })
    .await
    .expect("agreement document");

    let listed = db
        .list_agreement_documents("agreement-1")
        .await
        .expect("list agreement documents");
    assert_eq!(listed.len(), 1);
    assert_eq!(
        listed[0].document_type,
        AgreementDocumentType::MasterContract
    );
    assert_eq!(listed[0].document_no.as_deref(), Some("DOC-001"));
    assert_eq!(listed[0].uploaded_at, "2026-04-11T00:00:00Z");

    let fetched = db
        .get_agreement_document("agreement-doc-1")
        .await
        .expect("get agreement document");
    assert_eq!(fetched.agreement_id, "agreement-1");
    assert_eq!(fetched.object_key, "agreements/agreement-1/master.pdf");
    assert_eq!(fetched.uploaded_at, "2026-04-11T00:00:00Z");
}

#[tokio::test]
async fn client_grants_round_trip_through_database() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "bootstrap-user".to_owned(),
        "Bootstrap User".to_owned(),
    );
    db.ensure_schema().await.expect("ensure schema");

    db.upsert_organization_with_owner("ops-seoul", "서울사업팀", None, "서울사업팀")
        .await
        .expect("organization");

    db.upsert_client(&Client {
        id: "client-awesome-dashboard".to_owned(),
        organization_id: "ops-seoul".to_owned(),
        slug: "awesome-dashboard".to_owned(),
        name: "Awesome Dashboard".to_owned(),
        description: Some("외부 발전소 대시보드".to_owned()),
        client_type: ClientType::Confidential,
        app_type: ClientAppType::Web,
        first_party: false,
        consent_policy: ClientConsentPolicy::Explicit,
        active: true,
        created_by_user_id: "bootstrap-user".to_owned(),
    })
    .await
    .expect("client");
    db.upsert_client_action(&ClientAction {
        client_id: "client-awesome-dashboard".to_owned(),
        action_id: "plant.profile.read".to_owned(),
    })
    .await
    .expect("client action");
    let persisted_grant_id = db
        .upsert_client_grant(&ClientGrant {
            id: "grant-awesome-dashboard".to_owned(),
            user_id: "bootstrap-user".to_owned(),
            client_id: "client-awesome-dashboard".to_owned(),
            organization_id: "ops-seoul".to_owned(),
            status: ClientGrantStatus::Active,
            consented_at: "2026-04-12T00:00:00Z".to_owned(),
            revoked_at: None,
            revoked_by_user_id: None,
            created_at: "2026-04-12T00:00:00Z".to_owned(),
            updated_at: "2026-04-12T00:00:00Z".to_owned(),
        })
        .await
        .expect("client grant");
    assert_eq!(persisted_grant_id, "grant-awesome-dashboard");
    let pre_client_actions: Vec<RoleActionRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT client_id AS role_id, action_id
             FROM client_actions
             WHERE client_id = 'client-awesome-dashboard'",
            vec![],
        )
        .await
        .expect("pre client actions");
    assert_eq!(pre_client_actions.len(), 1);
    let pre_grants: Vec<ClientGrantRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT id, user_id, client_id, organization_id, status
             FROM client_grants
             WHERE id = 'grant-awesome-dashboard'",
            vec![],
        )
        .await
        .expect("pre client grants");
    assert_eq!(pre_grants.len(), 1);
    db.upsert_client_grant_action(&ClientGrantAction {
        client_grant_id: persisted_grant_id.clone(),
        action_id: "plant.profile.read".to_owned(),
    })
    .await
    .expect("client grant action");

    let clients: Vec<ClientRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT id, organization_id, slug, name, client_type, app_type, first_party, consent_policy, active, created_by_user_id
             FROM clients
             WHERE id = 'client-awesome-dashboard'",
            vec![],
        )
        .await
        .expect("clients");
    assert_eq!(clients.len(), 1);
    assert_eq!(clients[0].id, "client-awesome-dashboard");
    assert_eq!(clients[0].organization_id, "ops-seoul");
    assert_eq!(clients[0].slug, "awesome-dashboard");
    assert_eq!(clients[0].name, "Awesome Dashboard");
    assert_eq!(clients[0].client_type, "confidential");
    assert_eq!(clients[0].app_type, "web");
    assert_eq!(clients[0].first_party, 0);
    assert_eq!(clients[0].consent_policy, "explicit");
    assert_eq!(clients[0].active, 1);
    assert_eq!(clients[0].created_by_user_id, "bootstrap-user");

    let grants: Vec<ClientGrantRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT id, user_id, client_id, organization_id, status
             FROM client_grants
             WHERE id = 'grant-awesome-dashboard'",
            vec![],
        )
        .await
        .expect("client grants");
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].id, "grant-awesome-dashboard");
    assert_eq!(grants[0].user_id, "bootstrap-user");
    assert_eq!(grants[0].client_id, "client-awesome-dashboard");
    assert_eq!(grants[0].organization_id, "ops-seoul");
    assert_eq!(grants[0].status, "active");

    let client_actions: Vec<RoleActionRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT client_id AS role_id, action_id
             FROM client_actions
             WHERE client_id = 'client-awesome-dashboard'",
            vec![],
        )
        .await
        .expect("client actions");
    assert_eq!(client_actions.len(), 1);
    assert_eq!(client_actions[0].role_id, "client-awesome-dashboard");
    assert_eq!(client_actions[0].action_id, "plant.profile.read");

    let grant_actions: Vec<RoleActionRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT client_grant_id AS role_id, action_id
             FROM client_grant_actions
             WHERE client_grant_id = 'grant-awesome-dashboard'",
            vec![],
        )
        .await
        .expect("client grant actions");
    assert_eq!(grant_actions.len(), 1);
    assert_eq!(grant_actions[0].role_id, "grant-awesome-dashboard");
    assert_eq!(grant_actions[0].action_id, "plant.profile.read");
}

#[tokio::test]
async fn client_actions_reject_non_grantable_actions() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "bootstrap-user".to_owned(),
        "Bootstrap User".to_owned(),
    );
    db.ensure_schema().await.expect("ensure schema");

    db.upsert_organization_with_owner("ops-seoul", "서울사업팀", None, "서울사업팀")
        .await
        .expect("organization");
    db.upsert_client(&Client {
        id: "client-awesome-dashboard".to_owned(),
        organization_id: "ops-seoul".to_owned(),
        slug: "awesome-dashboard".to_owned(),
        name: "Awesome Dashboard".to_owned(),
        description: None,
        client_type: ClientType::Confidential,
        app_type: ClientAppType::Web,
        first_party: false,
        consent_policy: ClientConsentPolicy::Explicit,
        active: true,
        created_by_user_id: "bootstrap-user".to_owned(),
    })
    .await
    .expect("client");

    let err = db
        .upsert_client_action(&ClientAction {
            client_id: "client-awesome-dashboard".to_owned(),
            action_id: "plant.certificate.use".to_owned(),
        })
        .await
        .expect_err("non-grantable action must fail");
    assert!(err.to_string().contains("not grantable to clients"));
}

#[tokio::test]
async fn client_grant_actions_must_be_subset_of_client_actions() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "bootstrap-user".to_owned(),
        "Bootstrap User".to_owned(),
    );
    db.ensure_schema().await.expect("ensure schema");

    db.upsert_organization_with_owner("ops-seoul", "서울사업팀", None, "서울사업팀")
        .await
        .expect("organization");
    db.upsert_client(&Client {
        id: "client-awesome-dashboard".to_owned(),
        organization_id: "ops-seoul".to_owned(),
        slug: "awesome-dashboard".to_owned(),
        name: "Awesome Dashboard".to_owned(),
        description: None,
        client_type: ClientType::Confidential,
        app_type: ClientAppType::Web,
        first_party: false,
        consent_policy: ClientConsentPolicy::Explicit,
        active: true,
        created_by_user_id: "bootstrap-user".to_owned(),
    })
    .await
    .expect("client");
    let grant_id = db
        .upsert_client_grant(&ClientGrant {
            id: "grant-awesome-dashboard".to_owned(),
            user_id: "bootstrap-user".to_owned(),
            client_id: "client-awesome-dashboard".to_owned(),
            organization_id: "ops-seoul".to_owned(),
            status: ClientGrantStatus::Active,
            consented_at: "2026-04-12T00:00:00Z".to_owned(),
            revoked_at: None,
            revoked_by_user_id: None,
            created_at: "2026-04-12T00:00:00Z".to_owned(),
            updated_at: "2026-04-12T00:00:00Z".to_owned(),
        })
        .await
        .expect("client grant");

    let err = db
        .upsert_client_grant_action(&ClientGrantAction {
            client_grant_id: grant_id,
            action_id: "plant.report.read".to_owned(),
        })
        .await
        .expect_err("grant action outside client action subset must fail");
    assert!(
        err.to_string()
            .contains("client grant action must be included in client_actions")
    );
}

#[tokio::test]
async fn active_client_grants_are_upserted_by_logical_key() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "bootstrap-user".to_owned(),
        "Bootstrap User".to_owned(),
    );
    db.ensure_schema().await.expect("ensure schema");

    db.upsert_organization_with_owner("ops-seoul", "서울사업팀", None, "서울사업팀")
        .await
        .expect("organization");
    db.upsert_client(&Client {
        id: "client-awesome-dashboard".to_owned(),
        organization_id: "ops-seoul".to_owned(),
        slug: "awesome-dashboard".to_owned(),
        name: "Awesome Dashboard".to_owned(),
        description: None,
        client_type: ClientType::Confidential,
        app_type: ClientAppType::Web,
        first_party: false,
        consent_policy: ClientConsentPolicy::Explicit,
        active: true,
        created_by_user_id: "bootstrap-user".to_owned(),
    })
    .await
    .expect("client");

    let first_id = db
        .upsert_client_grant(&ClientGrant {
            id: "grant-1".to_owned(),
            user_id: "bootstrap-user".to_owned(),
            client_id: "client-awesome-dashboard".to_owned(),
            organization_id: "ops-seoul".to_owned(),
            status: ClientGrantStatus::Active,
            consented_at: "2026-04-12T00:00:00Z".to_owned(),
            revoked_at: None,
            revoked_by_user_id: None,
            created_at: "2026-04-12T00:00:00Z".to_owned(),
            updated_at: "2026-04-12T00:00:00Z".to_owned(),
        })
        .await
        .expect("first grant");
    let second_id = db
        .upsert_client_grant(&ClientGrant {
            id: "grant-2".to_owned(),
            user_id: "bootstrap-user".to_owned(),
            client_id: "client-awesome-dashboard".to_owned(),
            organization_id: "ops-seoul".to_owned(),
            status: ClientGrantStatus::Active,
            consented_at: "2026-04-13T00:00:00Z".to_owned(),
            revoked_at: None,
            revoked_by_user_id: None,
            created_at: "2026-04-13T00:00:00Z".to_owned(),
            updated_at: "2026-04-13T00:00:00Z".to_owned(),
        })
        .await
        .expect("second grant");

    assert_eq!(first_id, "grant-1");
    assert_eq!(second_id, "grant-1");

    let grants: Vec<ClientGrantRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT id, user_id, client_id, organization_id, status
             FROM client_grants
             WHERE client_id = 'client-awesome-dashboard'
             ORDER BY id ASC",
            vec![],
        )
        .await
        .expect("client grants");
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].id, "grant-1");
}

#[tokio::test]
async fn active_reconsent_replaces_client_grant_actions() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "bootstrap-user".to_owned(),
        "Bootstrap User".to_owned(),
    );
    db.ensure_schema().await.expect("ensure schema");

    db.upsert_organization_with_owner("ops-seoul", "서울사업팀", None, "서울사업팀")
        .await
        .expect("organization");
    db.upsert_client(&Client {
        id: "client-awesome-dashboard".to_owned(),
        organization_id: "ops-seoul".to_owned(),
        slug: "awesome-dashboard".to_owned(),
        name: "Awesome Dashboard".to_owned(),
        description: None,
        client_type: ClientType::Confidential,
        app_type: ClientAppType::Web,
        first_party: false,
        consent_policy: ClientConsentPolicy::Explicit,
        active: true,
        created_by_user_id: "bootstrap-user".to_owned(),
    })
    .await
    .expect("client");
    db.upsert_client_action(&ClientAction {
        client_id: "client-awesome-dashboard".to_owned(),
        action_id: "plant.profile.read".to_owned(),
    })
    .await
    .expect("client action profile");
    db.upsert_client_action(&ClientAction {
        client_id: "client-awesome-dashboard".to_owned(),
        action_id: "plant.report.read".to_owned(),
    })
    .await
    .expect("client action report");

    let grant_id = db
        .upsert_client_grant(&ClientGrant {
            id: "grant-1".to_owned(),
            user_id: "bootstrap-user".to_owned(),
            client_id: "client-awesome-dashboard".to_owned(),
            organization_id: "ops-seoul".to_owned(),
            status: ClientGrantStatus::Active,
            consented_at: "2026-04-12T00:00:00Z".to_owned(),
            revoked_at: None,
            revoked_by_user_id: None,
            created_at: "2026-04-12T00:00:00Z".to_owned(),
            updated_at: "2026-04-12T00:00:00Z".to_owned(),
        })
        .await
        .expect("first grant");
    db.upsert_client_grant_action(&ClientGrantAction {
        client_grant_id: grant_id.clone(),
        action_id: "plant.profile.read".to_owned(),
    })
    .await
    .expect("first profile action");
    db.upsert_client_grant_action(&ClientGrantAction {
        client_grant_id: grant_id.clone(),
        action_id: "plant.report.read".to_owned(),
    })
    .await
    .expect("first report action");

    let reused_grant_id = db
        .upsert_client_grant(&ClientGrant {
            id: "grant-2".to_owned(),
            user_id: "bootstrap-user".to_owned(),
            client_id: "client-awesome-dashboard".to_owned(),
            organization_id: "ops-seoul".to_owned(),
            status: ClientGrantStatus::Active,
            consented_at: "2026-04-13T00:00:00Z".to_owned(),
            revoked_at: None,
            revoked_by_user_id: None,
            created_at: "2026-04-13T00:00:00Z".to_owned(),
            updated_at: "2026-04-13T00:00:00Z".to_owned(),
        })
        .await
        .expect("second grant");
    assert_eq!(reused_grant_id, grant_id);
    db.upsert_client_grant_action(&ClientGrantAction {
        client_grant_id: reused_grant_id.clone(),
        action_id: "plant.profile.read".to_owned(),
    })
    .await
    .expect("reconsent profile action");

    let grant_actions: Vec<RoleActionRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT client_grant_id AS role_id, action_id
             FROM client_grant_actions
             WHERE client_grant_id = 'grant-1'
             ORDER BY action_id ASC",
            vec![],
        )
        .await
        .expect("client grant actions");
    assert_eq!(grant_actions.len(), 1);
    assert_eq!(grant_actions[0].action_id, "plant.profile.read");
}

#[tokio::test]
async fn client_upsert_requires_existing_organization_and_creator_user() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "bootstrap-user".to_owned(),
        "Bootstrap User".to_owned(),
    );
    db.ensure_schema().await.expect("ensure schema");

    let err = db
        .upsert_client(&Client {
            id: "client-orphan".to_owned(),
            organization_id: "missing-org".to_owned(),
            slug: "orphan".to_owned(),
            name: "Orphan Client".to_owned(),
            description: None,
            client_type: ClientType::Confidential,
            app_type: ClientAppType::Web,
            first_party: false,
            consent_policy: ClientConsentPolicy::Explicit,
            active: true,
            created_by_user_id: "missing-user".to_owned(),
        })
        .await
        .expect_err("missing org and user must fail");
    assert!(
        err.to_string()
            .contains("client organization missing-org does not exist")
            || err
                .to_string()
                .contains("client creator user missing-user does not exist")
    );
}

#[tokio::test]
async fn client_grant_requires_existing_user_client_and_organization() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "bootstrap-user".to_owned(),
        "Bootstrap User".to_owned(),
    );
    db.ensure_schema().await.expect("ensure schema");

    let err = db
        .upsert_client_grant(&ClientGrant {
            id: "grant-orphan".to_owned(),
            user_id: "missing-user".to_owned(),
            client_id: "missing-client".to_owned(),
            organization_id: "missing-org".to_owned(),
            status: ClientGrantStatus::Active,
            consented_at: "2026-04-12T00:00:00Z".to_owned(),
            revoked_at: None,
            revoked_by_user_id: None,
            created_at: "2026-04-12T00:00:00Z".to_owned(),
            updated_at: "2026-04-12T00:00:00Z".to_owned(),
        })
        .await
        .expect_err("missing grant dependencies must fail");
    assert!(
        err.to_string()
            .contains("client grant user missing-user does not exist")
            || err
                .to_string()
                .contains("client grant client missing-client does not exist")
            || err
                .to_string()
                .contains("client grant organization missing-org does not exist")
    );
}

#[tokio::test]
async fn revoked_client_grants_can_exist_alongside_active_grants() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "bootstrap-user".to_owned(),
        "Bootstrap User".to_owned(),
    );
    db.ensure_schema().await.expect("ensure schema");

    db.upsert_organization_with_owner("ops-seoul", "서울사업팀", None, "서울사업팀")
        .await
        .expect("organization");
    db.upsert_client(&Client {
        id: "client-awesome-dashboard".to_owned(),
        organization_id: "ops-seoul".to_owned(),
        slug: "awesome-dashboard".to_owned(),
        name: "Awesome Dashboard".to_owned(),
        description: None,
        client_type: ClientType::Confidential,
        app_type: ClientAppType::Web,
        first_party: false,
        consent_policy: ClientConsentPolicy::Explicit,
        active: true,
        created_by_user_id: "bootstrap-user".to_owned(),
    })
    .await
    .expect("client");

    let active_id = db
        .upsert_client_grant(&ClientGrant {
            id: "grant-active".to_owned(),
            user_id: "bootstrap-user".to_owned(),
            client_id: "client-awesome-dashboard".to_owned(),
            organization_id: "ops-seoul".to_owned(),
            status: ClientGrantStatus::Active,
            consented_at: "2026-04-12T00:00:00Z".to_owned(),
            revoked_at: None,
            revoked_by_user_id: None,
            created_at: "2026-04-12T00:00:00Z".to_owned(),
            updated_at: "2026-04-12T00:00:00Z".to_owned(),
        })
        .await
        .expect("active grant");
    let revoked_id = db
        .upsert_client_grant(&ClientGrant {
            id: "grant-revoked".to_owned(),
            user_id: "bootstrap-user".to_owned(),
            client_id: "client-awesome-dashboard".to_owned(),
            organization_id: "ops-seoul".to_owned(),
            status: ClientGrantStatus::Revoked,
            consented_at: "2026-04-11T00:00:00Z".to_owned(),
            revoked_at: Some("2026-04-13T00:00:00Z".to_owned()),
            revoked_by_user_id: Some("bootstrap-user".to_owned()),
            created_at: "2026-04-11T00:00:00Z".to_owned(),
            updated_at: "2026-04-13T00:00:00Z".to_owned(),
        })
        .await
        .expect("revoked grant");

    assert_eq!(active_id, "grant-active");
    assert_eq!(revoked_id, "grant-revoked");

    let grants: Vec<ClientGrantRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT id, user_id, client_id, organization_id, status
             FROM client_grants
             WHERE client_id = 'client-awesome-dashboard'
             ORDER BY id ASC",
            vec![],
        )
        .await
        .expect("client grants");
    assert_eq!(grants.len(), 2);
    assert_eq!(grants[0].id, "grant-active");
    assert_eq!(grants[0].status, "active");
    assert_eq!(grants[1].id, "grant-revoked");
    assert_eq!(grants[1].status, "revoked");
}

#[tokio::test]
async fn certificates_are_shared_across_organizations_linked_to_same_party() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "bootstrap-user".to_owned(),
        "Bootstrap User".to_owned(),
    );
    db.ensure_schema().await.expect("ensure schema");

    db.upsert_organization_with_owner(
        "ops-seoul",
        "서울사업팀",
        Some("2768603291"),
        "주식회사 커널로그에너지",
    )
    .await
    .expect("organization a");
    db.upsert_organization_with_owner(
        "ops-busan",
        "부산사업팀",
        Some("2768603291"),
        "주식회사 커널로그에너지",
    )
    .await
    .expect("organization b");

    let saved = db
        .save_certificate(
            "ops-seoul",
            Some("2768603291"),
            "주식회사 커널로그에너지",
            "encrypted-bundle",
            b"wrapped-dek",
            "local-dev-v1",
        )
        .await
        .expect("save certificate");

    let loaded = db
        .load_active_certificate("ops-busan")
        .await
        .expect("load certificate through shared party");

    assert_eq!(loaded.id, saved.id);
    assert_eq!(loaded.party_id, saved.party_id);
    assert_eq!(loaded.party_id, "party-2768603291");
    assert_eq!(loaded.business_no.as_deref(), Some("2768603291"));
    assert_eq!(loaded.encrypted_blob, "encrypted-bundle");

    let auth_credentials: Vec<CountRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT COUNT(*) AS count
             FROM auth_credentials
             WHERE organization_id = 'ops-seoul'
               AND provider = 'korea_energy_rps'
               AND credential_type = 'certificate'
               AND active = 1",
            vec![],
        )
        .await
        .expect("auth credential count");
    assert_eq!(auth_credentials[0].count, 1);
}

#[tokio::test]
async fn save_certificate_reuses_existing_party_without_overwriting_legal_name() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "bootstrap-user".to_owned(),
        "Bootstrap User".to_owned(),
    );
    db.ensure_schema().await.expect("ensure schema");

    db.upsert_organization_with_owner(
        "ops-seoul",
        "서울사업팀",
        Some("2768603291"),
        "주식회사 커널로그에너지",
    )
    .await
    .expect("organization");

    db.save_certificate(
        "ops-seoul",
        Some("2768603291"),
        "커널로그에너지 운영 인증서",
        "encrypted-bundle",
        b"wrapped-dek",
        "local-dev-v1",
    )
    .await
    .expect("save certificate");

    let parties: Vec<PartyRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT id, business_no, legal_name FROM parties WHERE business_no = '2768603291'",
            vec![],
        )
        .await
        .expect("parties");
    assert_eq!(parties.len(), 1);
    assert_eq!(parties[0].legal_name, "주식회사 커널로그에너지");
}

#[tokio::test]
async fn organization_identity_bootstraps_placeholder_organization_when_missing() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "bootstrap-user".to_owned(),
        "Bootstrap User".to_owned(),
    );
    db.ensure_schema().await.expect("ensure schema");

    let organization = db
        .upsert_organization_identity("missing-org", None, "ignored")
        .await
        .expect("organization identity");

    assert_eq!(organization.id, "missing-org");
    assert_eq!(organization.name, "missing-org");
    assert_eq!(organization.party_id, None);
    assert!(organization.inactivated_at.is_none());
}

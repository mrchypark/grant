use std::collections::HashSet;

use domain::{RoleBinding, RoleBindingResourceType, RoleBindingState, RoleBindingSubjectType};
use hiqlite::{Node, NodeConfig, Row, s3::EncKeys};
use infra::{config::ReadConsistency, database::Database, runtime::HiqliteRuntime};
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
async fn keeps_organization_access_explicit_without_hierarchy_tables() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "bootstrap-user".to_owned(),
        "Bootstrap User".to_owned(),
    );
    db.ensure_schema().await.expect("ensure schema");

    db.upsert_organization_with_owner("distributor-org", "Distributor Org", "Distributor Org")
        .await
        .expect("upsert distributor");
    db.upsert_organization_with_owner("dealer-org", "Dealer Org", "Dealer Org")
        .await
        .expect("upsert dealer");

    runtime
        .execute(
            "INSERT INTO plants (id, facility_code, owner_organization_id, created_by_organization_id, name, contract_no, address, installed_kw, rec_start_date, confirm_issued, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
            vec![
                "plant-1".into(),
                "FAC-001".into(),
                "distributor-org".into(),
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

    runtime
        .execute(
            "INSERT INTO roles (id, organization_id, code, name, description, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            vec![
                "role-plant-reader".into(),
                Option::<String>::None.into(),
                "plant.reader".into(),
                "Plant Reader".into(),
                Option::<String>::None.into(),
                "2026-04-03T00:00:00Z".into(),
                "2026-04-03T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert role");

    runtime
        .execute(
            "INSERT INTO role_actions (role_id, action_id) VALUES ($1, $2)",
            vec!["role-plant-reader".into(), "plant.profile.read".into()],
        )
        .await
        .expect("insert role action");

    runtime
        .execute(
            "INSERT INTO agreements (id, agreement_type, grantor_organization_id, grantee_organization_id, status, title, reference_no, starts_at, ends_at, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
            vec![
                "agreement-1".into(),
                "delegation".into(),
                "distributor-org".into(),
                "dealer-org".into(),
                "active".into(),
                "Delegation Agreement".into(),
                Option::<String>::None.into(),
                Option::<String>::None.into(),
                Option::<String>::None.into(),
                "2026-04-03T00:00:00Z".into(),
                "2026-04-03T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert agreement");

    let binding = RoleBinding {
        id: Uuid::new_v4().simple().to_string(),
        subject_type: RoleBindingSubjectType::Organization,
        subject_id: "dealer-org".to_owned(),
        subject_organization_id: "dealer-org".to_owned(),
        resource_type: RoleBindingResourceType::Plant,
        resource_id: "plant-1".to_owned(),
        role_id: "role-plant-reader".to_owned(),
        agreement_id: Some("agreement-1".to_owned()),
        state: RoleBindingState::Active,
        created_by_user_id: "bootstrap-user".to_owned(),
        created_at: "2026-04-03T00:00:00Z".to_owned(),
        updated_at: "2026-04-03T00:00:00Z".to_owned(),
    };
    db.save_role_binding(&binding)
        .await
        .expect("save role binding");

    let org_actions: Vec<OrganizationPlantActionRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT organization_id, plant_id, action_id
             FROM effective_organization_plant_actions
             ORDER BY organization_id ASC, plant_id ASC, action_id ASC",
            vec![],
        )
        .await
        .expect("effective organization plant actions");
    let org_actions: HashSet<OrganizationPlantActionRow> = org_actions.into_iter().collect();
    assert!(
        org_actions.contains(&OrganizationPlantActionRow {
            organization_id: "dealer-org".to_owned(),
            plant_id: "plant-1".to_owned(),
            action_id: "plant.profile.read".to_owned(),
        }),
        "missing direct dealer plant access"
    );
    assert!(
        !org_actions.contains(&OrganizationPlantActionRow {
            organization_id: "distributor-org".to_owned(),
            plant_id: "plant-1".to_owned(),
            action_id: "plant.profile.read".to_owned(),
        }),
        "organization hierarchy should not synthesize distributor access"
    );

    let user_actions: Vec<UserPlantActionRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT user_id, organization_id, plant_id, action_id
             FROM effective_user_plant_actions
             ORDER BY user_id ASC, organization_id ASC, plant_id ASC, action_id ASC",
            vec![],
        )
        .await
        .expect("effective user plant actions");
    assert!(
        user_actions.is_empty(),
        "organization inheritance should not imply user-level actions yet"
    );
}

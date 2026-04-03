use std::collections::HashSet;

use domain::{
    Agreement, AgreementStatus, AgreementType, Role, RoleBinding, RoleBindingResourceType,
    RoleBindingState, RoleBindingSubjectType,
};
use hiqlite::{Node, NodeConfig, Row, s3::EncKeys};
use infra::{
    config::ReadConsistency,
    database::Database,
    runtime::HiqliteRuntime,
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
    db.upsert_agreement(&agreement).await.expect("upsert agreement");

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
    assert!(org_rows_after.is_empty(), "agreement termination should remove inherited organization access");

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
    assert!(user_rows_after.is_empty(), "agreement termination should remove user actions");

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
    assert!(visibility_rows_after.is_empty(), "agreement termination should remove plant visibility");
}

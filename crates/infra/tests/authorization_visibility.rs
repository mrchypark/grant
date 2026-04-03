use std::collections::HashSet;

use domain::{RoleBinding, RoleBindingResourceType, RoleBindingState, RoleBindingSubjectType};
use hiqlite::{Node, NodeConfig, Row, s3::EncKeys};
use infra::{
    config::ReadConsistency,
    database::Database,
    runtime::HiqliteRuntime,
};
use tempfile::TempDir;
use uuid::Uuid;

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
async fn expands_group_and_user_visibility_from_profile_read_only() {
    let (runtime, _dir) = test_runtime().await;
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

    let user_actions: HashSet<UserPlantActionRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT user_id, organization_id, plant_id, action_id
             FROM effective_user_plant_actions
             ORDER BY user_id ASC, organization_id ASC, plant_id ASC, action_id ASC",
            vec![],
        )
        .await
        .expect("user plant actions")
        .into_iter()
        .collect();
    assert!(
        user_actions.contains(&UserPlantActionRow {
            user_id: "group-user".to_owned(),
            organization_id: "worker-org".to_owned(),
            plant_id: "plant-profile".to_owned(),
            action_id: "plant.profile.read".to_owned(),
        }),
        "missing group-derived profile read"
    );
    assert!(
        user_actions.contains(&UserPlantActionRow {
            user_id: "direct-user".to_owned(),
            organization_id: "worker-org".to_owned(),
            plant_id: "plant-document".to_owned(),
            action_id: "plant.document.read".to_owned(),
        }),
        "missing direct user document read"
    );
    assert!(
        !user_actions.contains(&UserPlantActionRow {
            user_id: "group-user".to_owned(),
            organization_id: "worker-org".to_owned(),
            plant_id: "plant-document".to_owned(),
            action_id: "plant.document.read".to_owned(),
        }),
        "group user should not inherit direct-user document binding"
    );

    let visibility_rows: HashSet<UserPlantVisibilityRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT user_id, organization_id, plant_id
             FROM effective_user_plant_visibility
             ORDER BY user_id ASC, organization_id ASC, plant_id ASC",
            vec![],
        )
        .await
        .expect("user plant visibility")
        .into_iter()
        .collect();
    assert!(
        visibility_rows.contains(&UserPlantVisibilityRow {
            user_id: "group-user".to_owned(),
            organization_id: "worker-org".to_owned(),
            plant_id: "plant-profile".to_owned(),
        }),
        "profile read should generate visibility"
    );
    assert!(
        !visibility_rows.contains(&UserPlantVisibilityRow {
            user_id: "direct-user".to_owned(),
            organization_id: "worker-org".to_owned(),
            plant_id: "plant-document".to_owned(),
        }),
        "document read alone should not generate visibility"
    );
}

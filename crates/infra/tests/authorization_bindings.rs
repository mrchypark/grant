use domain::{RoleBinding, RoleBindingResourceType, RoleBindingState, RoleBindingSubjectType};
use hiqlite::{Node, NodeConfig, s3::EncKeys};
use infra::{config::ReadConsistency, database::Database, runtime::HiqliteRuntime};
use tempfile::TempDir;

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
async fn saves_and_lists_role_bindings_for_subjects_and_resources() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "conalog-account".to_owned(),
        "Conalog Account".to_owned(),
    );

    db.ensure_schema().await.expect("ensure schema");

    runtime
        .execute(
            "INSERT INTO organizations (id, business_no, name, legal_entity_id, active, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            vec![
                "epc-org".into(),
                Option::<String>::Some("2768603291".to_owned()).into(),
                "EPC Org".into(),
                Option::<String>::None.into(),
                1_i64.into(),
                "2026-04-03T00:00:00Z".into(),
                "2026-04-03T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert organization");
    runtime
        .execute(
            "INSERT INTO organizations (id, business_no, name, legal_entity_id, active, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            vec![
                "foreign-org".into(),
                Option::<String>::Some("9999999999".to_owned()).into(),
                "Foreign Org".into(),
                Option::<String>::None.into(),
                1_i64.into(),
                "2026-04-03T00:00:00Z".into(),
                "2026-04-03T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert foreign organization");
    runtime
        .execute(
            "INSERT INTO users (id, email, display_name, active, created_at)
             VALUES ($1, $2, $3, $4, $5)",
            vec![
                "operator-user".into(),
                "operator@example.com".into(),
                "Operator User".into(),
                1_i64.into(),
                "2026-04-03T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert operator user");
    runtime
        .execute(
            "INSERT INTO organization_memberships (id, organization_id, user_id, membership_role, status, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            vec![
                "membership-operator-conalog".into(),
                "conalog-account".into(),
                "operator-user".into(),
                "member".into(),
                "active".into(),
                "2026-04-03T00:00:00Z".into(),
                "2026-04-03T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert operator membership");
    runtime
        .execute(
            "INSERT INTO agreements (id, agreement_type, grantor_organization_id, grantee_organization_id, status, title, reference_no, starts_at, ends_at, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
            vec![
                "agreement-1".into(),
                "delegation".into(),
                "conalog-account".into(),
                "epc-org".into(),
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
    runtime
        .execute(
            "INSERT INTO agreements (id, agreement_type, grantor_organization_id, grantee_organization_id, status, title, reference_no, starts_at, ends_at, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
            vec![
                "agreement-mismatch".into(),
                "delegation".into(),
                "conalog-account".into(),
                "other-org".into(),
                "active".into(),
                "Mismatched Delegation".into(),
                Option::<String>::None.into(),
                Option::<String>::None.into(),
                Option::<String>::None.into(),
                "2026-04-03T00:00:00Z".into(),
                "2026-04-03T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert mismatched agreement");
    runtime
        .execute(
            "INSERT INTO agreements (id, agreement_type, grantor_organization_id, grantee_organization_id, status, title, reference_no, starts_at, ends_at, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
            vec![
                "agreement-inactive".into(),
                "delegation".into(),
                "conalog-account".into(),
                "epc-org".into(),
                "inactive".into(),
                "Inactive Delegation".into(),
                Option::<String>::None.into(),
                Option::<String>::None.into(),
                Option::<String>::None.into(),
                "2026-04-03T00:00:00Z".into(),
                "2026-04-03T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert inactive agreement");
    runtime
        .execute(
            "INSERT INTO agreements (id, agreement_type, grantor_organization_id, grantee_organization_id, status, title, reference_no, starts_at, ends_at, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
            vec![
                "agreement-expired".into(),
                "delegation".into(),
                "conalog-account".into(),
                "epc-org".into(),
                "active".into(),
                "Expired Delegation".into(),
                Option::<String>::None.into(),
                Option::<String>::Some("2026-01-01T00:00:00Z".to_owned()).into(),
                Option::<String>::Some("2026-03-01T00:00:00Z".to_owned()).into(),
                "2026-04-03T00:00:00Z".into(),
                "2026-04-03T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert expired agreement");
    runtime
        .execute(
            "INSERT INTO groups (id, organization_id, name, description, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6)",
            vec![
                "field-team".into(),
                "epc-org".into(),
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
            "INSERT INTO plants (id, facility_code, owner_organization_id, created_by_organization_id, name, contract_no, address, installed_kw, rec_start_date, confirm_issued, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
            vec![
                "plant-1".into(),
                "FAC-001".into(),
                "conalog-account".into(),
                Option::<String>::Some("epc-org".to_owned()).into(),
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
                "role-external-operator".into(),
                Option::<String>::None.into(),
                "external.operator".into(),
                "External Operator".into(),
                Option::<String>::None.into(),
                "2026-04-03T00:00:00Z".into(),
                "2026-04-03T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert plant role");
    runtime
        .execute(
            "INSERT INTO roles (id, organization_id, code, name, description, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            vec![
                "role-plant-creator".into(),
                Option::<String>::None.into(),
                "organization.plant_creator".into(),
                "Plant Creator".into(),
                Option::<String>::None.into(),
                "2026-04-03T00:00:00Z".into(),
                "2026-04-03T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert org role");
    runtime
        .execute(
            "INSERT INTO roles (id, organization_id, code, name, description, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            vec![
                "role-foreign-private".into(),
                Option::<String>::Some("foreign-org".to_owned()).into(),
                "foreign.private".into(),
                "Foreign Private".into(),
                Option::<String>::None.into(),
                "2026-04-03T00:00:00Z".into(),
                "2026-04-03T00:00:00Z".into(),
            ],
        )
        .await
        .expect("insert foreign private role");

    let organization_binding = RoleBinding {
        id: "binding-org-plant".to_owned(),
        subject_type: RoleBindingSubjectType::Organization,
        subject_id: "epc-org".to_owned(),
        subject_organization_id: "epc-org".to_owned(),
        resource_type: RoleBindingResourceType::Plant,
        resource_id: "plant-1".to_owned(),
        role_id: "role-external-operator".to_owned(),
        agreement_id: Some("agreement-1".to_owned()),
        state: RoleBindingState::Active,
        created_by_user_id: "conalog-account".to_owned(),
        created_at: "2026-04-03T00:00:00Z".to_owned(),
        updated_at: "2026-04-03T00:00:00Z".to_owned(),
    };
    let group_binding = RoleBinding {
        id: "binding-group-plant".to_owned(),
        subject_type: RoleBindingSubjectType::Group,
        subject_id: "field-team".to_owned(),
        subject_organization_id: "epc-org".to_owned(),
        resource_type: RoleBindingResourceType::Plant,
        resource_id: "plant-1".to_owned(),
        role_id: "role-external-operator".to_owned(),
        agreement_id: None,
        state: RoleBindingState::Active,
        created_by_user_id: "conalog-account".to_owned(),
        created_at: "2026-04-03T00:00:00Z".to_owned(),
        updated_at: "2026-04-03T00:00:00Z".to_owned(),
    };
    let user_binding = RoleBinding {
        id: "binding-user-org".to_owned(),
        subject_type: RoleBindingSubjectType::User,
        subject_id: "conalog-account".to_owned(),
        subject_organization_id: "conalog-account".to_owned(),
        resource_type: RoleBindingResourceType::Organization,
        resource_id: "conalog-account".to_owned(),
        role_id: "role-plant-creator".to_owned(),
        agreement_id: None,
        state: RoleBindingState::Active,
        created_by_user_id: "conalog-account".to_owned(),
        created_at: "2026-04-03T00:00:00Z".to_owned(),
        updated_at: "2026-04-03T00:00:00Z".to_owned(),
    };

    db.save_role_binding(&organization_binding)
        .await
        .expect("save organization binding");
    db.save_role_binding(&group_binding)
        .await
        .expect("save group binding");
    db.save_role_binding(&user_binding)
        .await
        .expect("save user binding");

    let organization_bindings = db
        .list_role_bindings_for_subject(
            RoleBindingSubjectType::Organization,
            "epc-org",
            Some("epc-org"),
        )
        .await
        .expect("organization bindings");
    assert_eq!(organization_bindings.len(), 1);
    assert_eq!(
        organization_bindings[0].resource_type,
        RoleBindingResourceType::Plant
    );
    assert_eq!(organization_bindings[0].resource_id, "plant-1");

    let group_bindings = db
        .list_role_bindings_for_subject(
            RoleBindingSubjectType::Group,
            "field-team",
            Some("epc-org"),
        )
        .await
        .expect("group bindings");
    assert_eq!(group_bindings.len(), 1);
    assert_eq!(group_bindings[0].subject_id, "field-team");

    let user_bindings = db
        .list_role_bindings_for_subject(
            RoleBindingSubjectType::User,
            "conalog-account",
            Some("conalog-account"),
        )
        .await
        .expect("user bindings");
    assert!(user_bindings.len() >= 1);
    assert_eq!(
        user_bindings
            .iter()
            .find(|binding| binding.id == "binding-user-org")
            .expect("primary user binding")
            .resource_type,
        RoleBindingResourceType::Organization
    );

    let plant_bindings = db
        .list_role_bindings_for_resource(RoleBindingResourceType::Plant, "plant-1")
        .await
        .expect("plant bindings");
    assert_eq!(plant_bindings.len(), 2);
    assert!(
        plant_bindings
            .iter()
            .any(|binding| binding.id == "binding-org-plant")
    );
    assert!(
        plant_bindings
            .iter()
            .any(|binding| binding.id == "binding-group-plant")
    );

    let organization_resource_bindings = db
        .list_role_bindings_for_resource(RoleBindingResourceType::Organization, "conalog-account")
        .await
        .expect("organization resource bindings");
    assert!(organization_resource_bindings.iter().any(
        |binding| binding.id == "binding-user-org" && binding.state == RoleBindingState::Active
    ));

    let updated_user_binding = RoleBinding {
        created_by_user_id: "operator-user".to_owned(),
        state: RoleBindingState::Disabled,
        updated_at: "2026-04-04T00:00:00Z".to_owned(),
        ..user_binding.clone()
    };
    db.save_role_binding(&updated_user_binding)
        .await
        .expect("update existing user binding");
    let updated_user_bindings = db
        .list_role_bindings_for_subject(
            RoleBindingSubjectType::User,
            "conalog-account",
            Some("conalog-account"),
        )
        .await
        .expect("updated user bindings");
    let updated_primary = updated_user_bindings
        .iter()
        .find(|binding| binding.id == "binding-user-org")
        .expect("updated primary user binding");
    assert_eq!(updated_primary.created_at, "2026-04-03T00:00:00Z");
    assert_eq!(updated_primary.updated_at, "2026-04-04T00:00:00Z");
    assert_eq!(updated_primary.created_by_user_id, "conalog-account");
    assert_eq!(updated_primary.state, RoleBindingState::Disabled);

    let missing_agreement = RoleBinding {
        id: "binding-org-no-agreement".to_owned(),
        agreement_id: None,
        ..organization_binding.clone()
    };
    let err = db
        .save_role_binding(&missing_agreement)
        .await
        .expect_err("organization plant binding should require agreement");
    assert!(err.to_string().contains("agreement"));

    let invalid_org_resource = RoleBinding {
        id: "binding-user-invalid-org".to_owned(),
        resource_id: "epc-org".to_owned(),
        ..user_binding.clone()
    };
    let err = db
        .save_role_binding(&invalid_org_resource)
        .await
        .expect_err("organization resource binding should stay within subject organization");
    assert!(err.to_string().contains("subject organization"));

    let invalid_org_subject = RoleBinding {
        id: "binding-org-invalid-subject".to_owned(),
        subject_id: "foreign-org".to_owned(),
        ..organization_binding.clone()
    };
    let err = db
        .save_role_binding(&invalid_org_subject)
        .await
        .expect_err("organization binding should keep subject id and subject organization aligned");
    assert!(err.to_string().contains("same subject id"));

    let invalid_group_agreement = RoleBinding {
        id: "binding-group-agreement".to_owned(),
        agreement_id: Some("agreement-2".to_owned()),
        ..group_binding.clone()
    };
    let err = db
        .save_role_binding(&invalid_group_agreement)
        .await
        .expect_err("group binding must not accept agreement");
    assert!(err.to_string().contains("must not carry agreement"));

    let invalid_group_org = RoleBinding {
        id: "binding-group-wrong-org".to_owned(),
        subject_organization_id: "conalog-account".to_owned(),
        ..group_binding.clone()
    };
    let err = db
        .save_role_binding(&invalid_group_org)
        .await
        .expect_err("group binding should validate owning organization");
    assert!(err.to_string().contains("does not belong"));

    let missing_role = RoleBinding {
        id: "binding-missing-role".to_owned(),
        role_id: "missing-role".to_owned(),
        ..user_binding.clone()
    };
    let err = db
        .save_role_binding(&missing_role)
        .await
        .expect_err("binding should reject missing role");
    assert!(err.to_string().contains("role"));

    let foreign_private_role = RoleBinding {
        id: "binding-foreign-role".to_owned(),
        role_id: "role-foreign-private".to_owned(),
        ..organization_binding.clone()
    };
    let err = db
        .save_role_binding(&foreign_private_role)
        .await
        .expect_err("binding should reject foreign organization private role");
    assert!(err.to_string().contains("belongs to organization"));

    let missing_plant = RoleBinding {
        id: "binding-missing-plant".to_owned(),
        resource_id: "missing-plant".to_owned(),
        ..group_binding.clone()
    };
    let err = db
        .save_role_binding(&missing_plant)
        .await
        .expect_err("binding should reject missing plant");
    assert!(err.to_string().contains("plant resource"));

    let missing_agreement_ref = RoleBinding {
        id: "binding-missing-agreement-ref".to_owned(),
        agreement_id: Some("agreement-missing".to_owned()),
        ..organization_binding.clone()
    };
    let err = db
        .save_role_binding(&missing_agreement_ref)
        .await
        .expect_err("binding should reject missing agreement");
    assert!(err.to_string().contains("agreement"));

    let duplicate_binding = RoleBinding {
        id: "binding-org-plant-duplicate".to_owned(),
        ..organization_binding.clone()
    };
    let err = db
        .save_role_binding(&duplicate_binding)
        .await
        .expect_err("duplicate logical binding should fail");
    let duplicate_err = err.to_string();
    assert!(
        duplicate_err.contains("UNIQUE")
            || duplicate_err.contains("constraint")
            || duplicate_err.contains("save role binding"),
        "{duplicate_err}"
    );

    let mismatched_agreement = RoleBinding {
        id: "binding-mismatched-agreement".to_owned(),
        agreement_id: Some("agreement-mismatch".to_owned()),
        ..organization_binding.clone()
    };
    let err = db
        .save_role_binding(&mismatched_agreement)
        .await
        .expect_err("binding should reject unrelated agreement");
    assert!(err.to_string().contains(
        "agreement agreement-mismatch does not match plant plant-1 and organization epc-org"
    ));

    let inactive_agreement = RoleBinding {
        id: "binding-inactive-agreement".to_owned(),
        agreement_id: Some("agreement-inactive".to_owned()),
        ..organization_binding.clone()
    };
    let err = db
        .save_role_binding(&inactive_agreement)
        .await
        .expect_err("binding should reject inactive agreement");
    assert!(err.to_string().contains("not active"));

    let expired_agreement = RoleBinding {
        id: "binding-expired-agreement".to_owned(),
        agreement_id: Some("agreement-expired".to_owned()),
        ..organization_binding.clone()
    };
    let err = db
        .save_role_binding(&expired_agreement)
        .await
        .expect_err("binding should reject expired agreement");
    assert!(err.to_string().contains("not active"));

    let backdated_expired_agreement = RoleBinding {
        id: "binding-expired-agreement-backdated".to_owned(),
        agreement_id: Some("agreement-expired".to_owned()),
        created_at: "2026-02-01T00:00:00Z".to_owned(),
        updated_at: "2026-02-01T00:00:00Z".to_owned(),
        ..organization_binding.clone()
    };
    let err = db
        .save_role_binding(&backdated_expired_agreement)
        .await
        .expect_err("binding should reject expired agreement even when backdated");
    assert!(err.to_string().contains("not active"));

    let missing_creator = RoleBinding {
        id: "binding-missing-creator".to_owned(),
        created_by_user_id: "missing-user".to_owned(),
        ..user_binding
    };
    let err = db
        .save_role_binding(&missing_creator)
        .await
        .expect_err("binding should reject missing creator");
    assert!(err.to_string().contains("creator user"));
}

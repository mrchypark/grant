mod support;

use domain::{
    Application, ApplicationAppType, ApplicationConsentPolicy, ApplicationProvisioningStatus,
    ApplicationType, MembershipRole, MembershipStatus, RoleBindingState, RoleBindingSubjectType,
};
use infra::{
    config::ReadConsistency,
    database::{
        PatchDataAuthorityImport, PatchDataHierarchyEdge, PatchDataImportedApplicationAssignment,
        PatchDataImportedMembership, PatchDataImportedOrganization, PatchDataImportedPlantBinding,
        PatchDataImportedUser, UserActionCheck, patch_data_manifest_digest,
    },
};

fn source_digest(byte: char) -> String {
    std::iter::repeat_n(byte, 64).collect()
}

fn command(generation: &str, previous: Option<&str>, plant: &str) -> PatchDataAuthorityImport {
    let mut command = PatchDataAuthorityImport {
        manifest_version: "grant.patch-data-authority.v1".to_owned(),
        generation: generation.to_owned(),
        ledger_digest: source_digest(if previous.is_some() { 'b' } else { 'a' }),
        manifest_digest: source_digest('0'),
        expected_previous_generation: previous.map(ToOwned::to_owned),
        users: vec![PatchDataImportedUser {
            user_id: "user-a".to_owned(),
            external_user_id: "patch-user-a".to_owned(),
            email: Some("user-a@example.test".to_owned()),
            display_name: "User A".to_owned(),
            active: true,
        }],
        organizations: vec![
            PatchDataImportedOrganization {
                organization_id: "org-a".to_owned(),
                name: "Org A".to_owned(),
                active: true,
            },
            PatchDataImportedOrganization {
                organization_id: "org-b".to_owned(),
                name: "Org B".to_owned(),
                active: true,
            },
        ],
        memberships: vec![PatchDataImportedMembership {
            membership_id: "membership-a".to_owned(),
            organization_id: "org-a".to_owned(),
            user_id: "user-a".to_owned(),
            role: MembershipRole::Member,
            status: MembershipStatus::Active,
        }],
        hierarchy_edges: vec![PatchDataHierarchyEdge {
            parent_organization_id: "org-a".to_owned(),
            child_organization_id: "org-b".to_owned(),
        }],
        plant_bindings: vec![PatchDataImportedPlantBinding {
            binding_id: "binding-a".to_owned(),
            subject_type: RoleBindingSubjectType::User,
            subject_id: "user-a".to_owned(),
            subject_organization_id: "org-a".to_owned(),
            plant_id: plant.to_owned(),
            state: RoleBindingState::Active,
        }],
        application_assignments: vec![],
    };
    command.manifest_digest = patch_data_manifest_digest(&command);
    command
}

async fn plant_allowed(db: &infra::database::Database, plant: &str) -> bool {
    db.has_user_actions(&[UserActionCheck {
        user_id: "user-a",
        organization_id: "org-a",
        provider_id: "patch-data",
        resource_type: "plant",
        resource_id: plant,
        action_id: "patch-data.plant.read",
        application_id: None,
        consistency: Some(ReadConsistency::Strong),
    }])
    .await
    .expect("plant decision")[0]
}

async fn plant_allowed_for_application(db: &infra::database::Database, plant: &str) -> bool {
    db.has_user_actions(&[UserActionCheck {
        user_id: "user-a",
        organization_id: "org-a",
        provider_id: "patch-data",
        resource_type: "plant",
        resource_id: plant,
        action_id: "patch-data.plant.read",
        application_id: Some("exposure-app"),
        consistency: Some(ReadConsistency::Strong),
    }])
    .await
    .expect("application plant decision")[0]
}

async fn insert_exposure_application(db: &infra::database::Database) {
    db.insert_application(&Application {
        id: "exposure-app".to_owned(),
        organization_id: "org-a".to_owned(),
        slug: "exposure-app".to_owned(),
        name: "Exposure".to_owned(),
        description: None,
        application_type: ApplicationType::Confidential,
        app_type: ApplicationAppType::Server,
        first_party: true,
        consent_policy: ApplicationConsentPolicy::AutoApprove,
        active: true,
        provisioning_status: ApplicationProvisioningStatus::Ready,
        rauthy_client_id: None,
        created_by_user_id: "user-a".to_owned(),
    })
    .await
    .expect("application");
}

#[tokio::test]
async fn complete_snapshot_is_atomic_idempotent_and_drives_strong_decisions() {
    let (runtime, db, _dir) = support::ensured_database().await;
    let first = command("generation-a", None, "plant-a");
    let receipt = db
        .import_patch_data_authority(first.clone())
        .await
        .expect("first import");
    assert!(receipt.applied);
    assert!(plant_allowed(&db, "plant-a").await);
    assert!(!plant_allowed(&db, "plant-b").await);

    let retry = db
        .import_patch_data_authority(first.clone())
        .await
        .expect("exact retry");
    assert!(!retry.applied);
    assert_eq!(retry.applied_at, receipt.applied_at);

    runtime
        .execute(
            "UPDATE role_bindings SET state='disabled' WHERE id='binding-a'",
            vec![],
        )
        .await
        .expect("simulate drift");
    let drift = db
        .import_patch_data_authority(first.clone())
        .await
        .expect_err("retry detects drift");
    assert!(drift.to_string().contains("state drift"));
    runtime
        .execute(
            "UPDATE role_bindings SET state='active' WHERE id='binding-a'",
            vec![],
        )
        .await
        .expect("restore state");

    let second = command("generation-b", Some("generation-a"), "plant-b");
    db.import_patch_data_authority(second)
        .await
        .expect("CAS replacement");
    assert!(!plant_allowed(&db, "plant-a").await);
    assert!(plant_allowed(&db, "plant-b").await);
}

#[tokio::test]
async fn manifest_rejects_bad_digest_references_cycles_and_generation_replay() {
    let (_runtime, db, _dir) = support::ensured_database().await;
    let mut bad_digest = command("generation-a", None, "plant-a");
    bad_digest.manifest_digest = source_digest('f');
    assert!(db.import_patch_data_authority(bad_digest).await.is_err());

    let first = command("generation-a", None, "plant-a");
    db.import_patch_data_authority(first)
        .await
        .expect("first import");
    let replay = command("generation-a", None, "plant-b");
    assert!(
        db.import_patch_data_authority(replay)
            .await
            .expect_err("same generation mismatch")
            .to_string()
            .contains("generation conflict")
    );

    let mut cycle = command("generation-b", Some("generation-a"), "plant-b");
    cycle.hierarchy_edges = vec![
        PatchDataHierarchyEdge {
            parent_organization_id: "org-a".to_owned(),
            child_organization_id: "org-b".to_owned(),
        },
        PatchDataHierarchyEdge {
            parent_organization_id: "org-b".to_owned(),
            child_organization_id: "org-a".to_owned(),
        },
    ];
    cycle.manifest_digest = patch_data_manifest_digest(&cycle);
    assert!(db.import_patch_data_authority(cycle).await.is_err());
    assert!(plant_allowed(&db, "plant-a").await);
}

#[tokio::test]
async fn storage_failure_rolls_back_receipt_and_previous_authority() {
    let (runtime, db, _dir) = support::ensured_database().await;
    db.import_patch_data_authority(command("generation-a", None, "plant-a"))
        .await
        .expect("first import");
    runtime.batch(
        "CREATE TRIGGER fail_imported_binding BEFORE INSERT ON role_bindings
         WHEN NEW.resource_id='plant-b' BEGIN SELECT RAISE(ABORT, 'injected binding failure'); END;"
    ).await.expect("failure trigger");
    let error = db
        .import_patch_data_authority(command("generation-b", Some("generation-a"), "plant-b"))
        .await
        .expect_err("replacement must fail");
    assert!(error.to_string().contains("transaction"));
    assert!(plant_allowed(&db, "plant-a").await);
    assert!(!plant_allowed(&db, "plant-b").await);
    runtime
        .batch("DROP TRIGGER fail_imported_binding;")
        .await
        .expect("drop trigger");
    let retry = db
        .import_patch_data_authority(command("generation-a", None, "plant-a"))
        .await
        .expect("original receipt remains exact");
    assert!(!retry.applied);
}

#[tokio::test]
async fn application_assignment_is_atomic_drift_checked_and_removal_revokes() {
    let (runtime, db, _dir) = support::ensured_database().await;
    db.import_patch_data_authority(command("generation-a", None, "plant-a"))
        .await
        .expect("base import");
    insert_exposure_application(&db).await;
    let mut first = command("generation-b", Some("generation-a"), "plant-a");
    first.application_assignments = vec![PatchDataImportedApplicationAssignment {
        application_id: "exposure-app".to_owned(),
        organization_id: "org-a".to_owned(),
    }];
    first.manifest_digest = patch_data_manifest_digest(&first);
    db.import_patch_data_authority(first.clone())
        .await
        .expect("assigned import");
    assert!(plant_allowed_for_application(&db, "plant-a").await);

    runtime.execute("DELETE FROM application_actions WHERE application_id='exposure-app' AND action_id='patch-data.plant.read'", vec![]).await.expect("drift");
    assert!(
        db.import_patch_data_authority(first)
            .await
            .expect_err("assignment drift")
            .to_string()
            .contains("state drift")
    );
    runtime.execute("INSERT INTO application_actions (application_id,action_id) VALUES ('exposure-app','patch-data.plant.read')", vec![]).await.expect("restore");

    db.import_patch_data_authority(command("generation-c", Some("generation-b"), "plant-a"))
        .await
        .expect("remove assignment");
    assert!(!plant_allowed_for_application(&db, "plant-a").await);
}

#[tokio::test]
async fn native_application_action_is_never_adopted_or_revoked() {
    let (runtime, db, _dir) = support::ensured_database().await;
    db.import_patch_data_authority(command("generation-a", None, "plant-a"))
        .await
        .expect("base import");
    insert_exposure_application(&db).await;
    runtime.execute("INSERT INTO application_actions (application_id,action_id) VALUES ('exposure-app','patch-data.plant.read')", vec![]).await.expect("native assignment");

    let mut adoption = command("generation-b", Some("generation-a"), "plant-a");
    adoption.application_assignments = vec![PatchDataImportedApplicationAssignment {
        application_id: "exposure-app".to_owned(),
        organization_id: "org-a".to_owned(),
    }];
    adoption.manifest_digest = patch_data_manifest_digest(&adoption);
    assert!(db.import_patch_data_authority(adoption).await.is_err());
    assert!(plant_allowed_for_application(&db, "plant-a").await);

    db.import_patch_data_authority(command("generation-b", Some("generation-a"), "plant-a"))
        .await
        .expect("unrelated replacement");
    assert!(
        plant_allowed_for_application(&db, "plant-a").await,
        "unowned action must survive replacement"
    );
}

#[tokio::test]
async fn matching_native_user_is_not_implicitly_adopted() {
    let (runtime, db, _dir) = support::ensured_database().await;
    runtime.execute("INSERT INTO users (id,email,display_name,active,created_at) VALUES ('user-a','user-a@example.test','User A',1,'2026-01-01')", vec![]).await.expect("native user");
    let mut manifest = command("generation-a", None, "plant-a");
    manifest.organizations.clear();
    manifest.memberships.clear();
    manifest.hierarchy_edges.clear();
    manifest.plant_bindings.clear();
    manifest.manifest_digest = patch_data_manifest_digest(&manifest);
    assert!(db.import_patch_data_authority(manifest).await.is_err());
    let rows: Vec<support::NameRow> = runtime.query_rows(ReadConsistency::Strong,
        "SELECT display_name AS name FROM users WHERE id='user-a' UNION ALL SELECT generation AS name FROM patch_data_authority_imports",
        vec![]).await.expect("native user state");
    assert_eq!(
        rows.into_iter().map(|row| row.name).collect::<Vec<_>>(),
        vec!["User A"]
    );
}

#[tokio::test]
async fn matching_native_organization_is_not_implicitly_adopted() {
    let (runtime, db, _dir) = support::ensured_database().await;
    runtime
        .execute(
            "INSERT INTO organizations (id,business_no,name,inactivated_at,created_at,updated_at) VALUES ('org-a',NULL,'Org A',NULL,'2026-01-01','2026-01-01')",
            vec![],
        )
        .await
        .expect("native organization");
    let mut manifest = command("generation-a", None, "plant-a");
    manifest.users.clear();
    manifest.organizations.truncate(1);
    manifest.memberships.clear();
    manifest.hierarchy_edges.clear();
    manifest.plant_bindings.clear();
    manifest.manifest_digest = patch_data_manifest_digest(&manifest);

    assert!(db.import_patch_data_authority(manifest).await.is_err());
    let rows: Vec<support::NameRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT name FROM organizations WHERE id='org-a' UNION ALL SELECT generation AS name FROM patch_data_authority_imports",
            vec![],
        )
        .await
        .expect("native organization state");
    assert_eq!(
        rows.into_iter().map(|row| row.name).collect::<Vec<_>>(),
        vec!["Org A"]
    );
}

#[tokio::test]
async fn matching_unowned_membership_blocks_replacement_without_partial_write() {
    let (runtime, db, _dir) = support::ensured_database().await;
    db.import_patch_data_authority(command("generation-a", None, "plant-a"))
        .await
        .expect("base import");
    runtime
        .execute(
            "DELETE FROM patch_data_authority_import_entities WHERE entity_type='membership' AND entity_id='membership-a'",
            vec![],
        )
        .await
        .expect("remove ownership marker");

    assert!(
        db.import_patch_data_authority(command("generation-b", Some("generation-a"), "plant-b"))
            .await
            .is_err()
    );
    let rows: Vec<support::NameRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT id AS name FROM organization_memberships WHERE id='membership-a' UNION ALL SELECT generation AS name FROM patch_data_authority_imports",
            vec![],
        )
        .await
        .expect("membership and receipt state");
    assert_eq!(
        rows.into_iter().map(|row| row.name).collect::<Vec<_>>(),
        vec!["membership-a", "generation-a"]
    );
    assert!(plant_allowed(&db, "plant-a").await);
    assert!(!plant_allowed(&db, "plant-b").await);
}

#[tokio::test]
async fn matching_unowned_binding_blocks_replacement_without_partial_write() {
    let (runtime, db, _dir) = support::ensured_database().await;
    db.import_patch_data_authority(command("generation-a", None, "plant-a"))
        .await
        .expect("base import");
    runtime
        .execute(
            "DELETE FROM patch_data_authority_import_entities WHERE entity_type='plant_binding' AND entity_id='binding-a'",
            vec![],
        )
        .await
        .expect("remove ownership marker");

    assert!(
        db.import_patch_data_authority(command("generation-b", Some("generation-a"), "plant-b"))
            .await
            .is_err()
    );
    let rows: Vec<support::NameRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT id AS name FROM role_bindings WHERE id='binding-a' UNION ALL SELECT generation AS name FROM patch_data_authority_imports",
            vec![],
        )
        .await
        .expect("binding and receipt state");
    assert_eq!(
        rows.into_iter().map(|row| row.name).collect::<Vec<_>>(),
        vec!["binding-a", "generation-a"]
    );
    assert!(plant_allowed(&db, "plant-a").await);
    assert!(!plant_allowed(&db, "plant-b").await);
}

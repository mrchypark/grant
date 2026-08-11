mod support;

use domain::{Action, ActionRiskLevel, ServiceProvider};
use infra::{config::ReadConsistency, database::Database, runtime::HiqliteRuntime};
use support::{BOOTSTRAP_USER_ID, test_runtime};

async fn coarse_database() -> (HiqliteRuntime, Database, tempfile::TempDir) {
    let (runtime, dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        BOOTSTRAP_USER_ID.to_owned(),
        "Bootstrap User".to_owned(),
    );
    db.ensure_schema().await.expect("ensure coarse schema");
    (runtime, db, dir)
}

async fn assert_compatibility_blocked(db: &Database, expected: &str) {
    let error = db
        .verify_coarse_deployment_compatible(ReadConsistency::Local)
        .await
        .expect_err("active field state must block coarse deployment");
    assert!(
        error.to_string().contains(expected),
        "expected {expected:?} in {error:#}"
    );
}

#[tokio::test]
async fn coarse_compatibility_passes_blank_fresh_empty_and_missing_field_tables() {
    let (runtime, dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        BOOTSTRAP_USER_ID.to_owned(),
        "Bootstrap User".to_owned(),
    );

    db.verify_coarse_deployment_compatible(ReadConsistency::Local)
        .await
        .expect("blank database must pass preflight");
    db.ensure_schema().await.expect("ensure coarse schema");
    db.verify_coarse_deployment_compatible(ReadConsistency::Local)
        .await
        .expect("fresh database must pass after migration and bootstrap");

    runtime
        .batch(
            "CREATE TABLE field_view_policy_audit_deliveries (id TEXT PRIMARY KEY);
             CREATE TABLE mask_catalog_manifests (id TEXT PRIMARY KEY);
             CREATE TABLE mask_catalog_active (id TEXT PRIMARY KEY);"
                .to_owned(),
        )
        .await
        .expect("create empty optional field tables");
    db.verify_coarse_deployment_compatible(ReadConsistency::Local)
        .await
        .expect("empty legacy tables must remain dormant");

    runtime
        .batch(
            "DROP TABLE role_action_field_views;
             DROP TABLE field_view_policy_audit_outbox;
             DROP TABLE field_view_policy_audit_deliveries;
             DROP TABLE mask_catalog_manifests;
             DROP TABLE mask_catalog_active;"
                .to_owned(),
        )
        .await
        .expect("remove dormant field tables");
    db.verify_coarse_deployment_compatible(ReadConsistency::Local)
        .await
        .expect("missing field tables must not affect coarse deployment");
    db.verify_schema_ready(ReadConsistency::Local)
        .await
        .expect("coarse readiness must not require field tables");

    assert!(
        db.has_user_organization_action_with_consistency(
            BOOTSTRAP_USER_ID,
            BOOTSTRAP_USER_ID,
            BOOTSTRAP_USER_ID,
            "organization.profile.read",
            Some(ReadConsistency::Local),
        )
        .await
        .expect("coarse allow decision")
    );
    assert!(
        !db.has_user_organization_action_with_consistency(
            BOOTSTRAP_USER_ID,
            BOOTSTRAP_USER_ID,
            BOOTSTRAP_USER_ID,
            "unknown.action",
            Some(ReadConsistency::Local),
        )
        .await
        .expect("unknown action decision")
    );
    assert!(
        !db.has_user_organization_action_with_consistency(
            BOOTSTRAP_USER_ID,
            BOOTSTRAP_USER_ID,
            "mismatched-resource",
            "organization.profile.read",
            Some(ReadConsistency::Local),
        )
        .await
        .expect("resource mismatch decision")
    );

    drop(dir);
}

#[tokio::test]
async fn coarse_compatibility_blocks_each_active_field_state() {
    let (runtime, db, _dir) = coarse_database().await;

    runtime
        .execute(
            "UPDATE actions SET visibility_granting = 1 WHERE id = 'organization.profile.read'",
            vec![],
        )
        .await
        .expect("activate visibility action");
    assert_compatibility_blocked(&db, "actions.visibility_granting").await;
    runtime
        .execute(
            "UPDATE actions SET visibility_granting = 0 WHERE id = 'organization.profile.read'",
            vec![],
        )
        .await
        .expect("deactivate visibility action");

    runtime
        .execute(
            "INSERT INTO role_action_field_views (
                role_id, action_id, resource_type, schema_key, field_pointer,
                view_mode, created_at, updated_at
             ) VALUES (
                'role-system-bootstrap-owner', 'organization.profile.read', 'organization',
                'legacy-schema', '/name', 'real', '2026-07-30T00:00:00Z',
                '2026-07-30T00:00:00Z'
             )",
            vec![],
        )
        .await
        .expect("insert legacy field policy");
    assert_compatibility_blocked(&db, "role_action_field_views").await;
    runtime
        .execute("DELETE FROM role_action_field_views", vec![])
        .await
        .expect("clear field policy fixture");

    runtime
        .execute(
            "INSERT INTO field_view_policy_audit_outbox (
                event_id, idempotency_key, request_hash, organization_id, actor_user_id,
                role_id, action_id, resource_type, schema_key, before_json, after_json, created_at
             ) VALUES (
                'event-1', 'key-1', 'hash-1', 'org-1', 'user-1', 'role-1', 'action-1',
                'resource', 'schema', '{}', '{}', '2026-07-30T00:00:00Z'
             )",
            vec![],
        )
        .await
        .expect("insert legacy audit event");
    assert_compatibility_blocked(&db, "field_view_policy_audit_outbox").await;
    runtime
        .execute("DELETE FROM field_view_policy_audit_outbox", vec![])
        .await
        .expect("clear audit fixture");

    for table_name in [
        "field_view_policy_audit_deliveries",
        "mask_catalog_manifests",
        "mask_catalog_active",
    ] {
        runtime
            .batch(format!(
                "CREATE TABLE {table_name} (id TEXT PRIMARY KEY); INSERT INTO {table_name} (id) VALUES ('active');"
            ))
            .await
            .unwrap_or_else(|error| panic!("create active {table_name}: {error:#}"));
        assert_compatibility_blocked(&db, table_name).await;
        runtime
            .batch(format!("DROP TABLE {table_name};"))
            .await
            .unwrap_or_else(|error| panic!("drop {table_name}: {error:#}"));
    }

    db.verify_coarse_deployment_compatible(ReadConsistency::Local)
        .await
        .expect("cleared fixtures must return to compatible state");
}

#[tokio::test]
async fn visibility_granting_manifest_is_rejected_before_existing_actions_are_changed() {
    let (_runtime, db, _dir) = coarse_database().await;
    let provider_id = "external-provider";
    db.upsert_service_provider(&ServiceProvider {
        id: provider_id.to_owned(),
        name: "External Provider".to_owned(),
        description: None,
        active: true,
    })
    .await
    .expect("insert provider");
    db.upsert_action(&Action {
        id: "external.read".to_owned(),
        provider_id: provider_id.to_owned(),
        code: "external.read".to_owned(),
        name: "Read External Resource".to_owned(),
        description: None,
        risk_level: ActionRiskLevel::Low,
        visibility_granting: false,
        grantable_to_applications: false,
        consent_label: None,
        consent_description: None,
    })
    .await
    .expect("insert coarse action");

    let error = db
        .replace_provider_actions(
            provider_id,
            &[Action {
                id: "external.field.read".to_owned(),
                provider_id: provider_id.to_owned(),
                code: "external.field.read".to_owned(),
                name: "Field Read".to_owned(),
                description: None,
                risk_level: ActionRiskLevel::Low,
                visibility_granting: true,
                grantable_to_applications: false,
                consent_label: None,
                consent_description: None,
            }],
        )
        .await
        .expect_err("visibility-granting manifest must fail");
    assert!(error.to_string().contains("visibility_granting=true"));

    let actions = db
        .list_actions_for_provider_with_consistency(provider_id, Some(ReadConsistency::Local))
        .await
        .expect("list retained provider actions");
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].id, "external.read");
    assert!(!actions[0].visibility_granting);
}

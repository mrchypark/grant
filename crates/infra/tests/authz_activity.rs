mod support;

use chrono::{Duration, Utc};
use domain::{
    Action, ActionRiskLevel, ApplicationProvisioningStatus, RoleBinding, RoleBindingState,
    RoleBindingSubjectType, ServiceProvider,
};
use infra::{
    config::ReadConsistency,
    database::{BindIdentityInput, CachedExternalIdentity, Database, UserActionCheck},
    runtime::HiqliteRuntime,
};

const USER_ID: &str = "activity-user";
const ORGANIZATION_ID: &str = "activity-org";
const FIELD_VIEW_RESOURCE_TYPE: &str = "compos.object";
const FIELD_VIEW_SCHEMA_KEY: &str = "019f-object-type-customer";

async fn activity_database() -> (Database, infra::runtime::HiqliteRuntime, tempfile::TempDir) {
    let (runtime, dir) = support::test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        USER_ID.to_owned(),
        "Activity User".to_owned(),
    );
    db.ensure_schema().await.expect("ensure schema");
    db.upsert_organization_with_owner(
        ORGANIZATION_ID,
        "Activity Organization",
        None,
        "Activity Organization",
    )
    .await
    .expect("seed organization");
    db.save_role_binding(&RoleBinding {
        id: "activity-binding".to_owned(),
        subject_type: RoleBindingSubjectType::User,
        subject_id: USER_ID.to_owned(),
        subject_organization_id: ORGANIZATION_ID.to_owned(),
        provider_id: "grant".to_owned(),
        resource_type: "organization".to_owned(),
        resource_id: ORGANIZATION_ID.to_owned(),
        role_id: "role-system-bootstrap-owner".to_owned(),
        agreement_id: None,
        state: RoleBindingState::Active,
        created_by_user_id: USER_ID.to_owned(),
        created_at: "2026-07-14T00:00:00Z".to_owned(),
        updated_at: "2026-07-14T00:00:00Z".to_owned(),
    })
    .await
    .expect("seed binding");
    (db, runtime, dir)
}

fn test_action(action_id: &str) -> Action {
    Action {
        id: action_id.to_owned(),
        provider_id: "compos".to_owned(),
        code: action_id.to_owned(),
        name: action_id.to_owned(),
        description: None,
        risk_level: ActionRiskLevel::Low,
        visibility_granting: false,
        grantable_to_applications: false,
        consent_label: None,
        consent_description: None,
    }
}

async fn seed_field_view_lifecycle(
    db: &Database,
    runtime: &HiqliteRuntime,
    role_id: &str,
    retained_action_id: &str,
    removed_action_id: &str,
    retained_pointer: &str,
    removed_pointer: &str,
) {
    let timestamp = "2026-07-15T00:00:00Z";
    runtime
        .execute(
            "INSERT INTO roles (id, organization_id, code, name, created_at, updated_at)
             VALUES ($1, $2, $1, $1, $3, $3)",
            vec![
                hiqlite::Param::from(role_id.to_owned()),
                hiqlite::Param::from(ORGANIZATION_ID.to_owned()),
                hiqlite::Param::from(timestamp.to_owned()),
            ],
        )
        .await
        .expect("seed role");
    runtime
        .execute(
            "INSERT INTO service_providers (id, name, active, created_at, updated_at)
             VALUES ('compos', 'compos Provider', 1, $1, $1)",
            vec![hiqlite::Param::from(timestamp.to_owned())],
        )
        .await
        .expect("seed provider");
    for action_id in [retained_action_id, removed_action_id] {
        db.upsert_action(&test_action(action_id))
            .await
            .expect("seed action");
        db.upsert_role_action(role_id, action_id)
            .await
            .expect("seed role action");
    }
    runtime
        .execute(
            "INSERT INTO role_action_field_views (
                role_id, action_id, resource_type, schema_key, field_pointer, view_mode, created_at, updated_at
             ) VALUES ($1, $2, $3, $4, $5, 'real', $6, $6), ($1, $7, $3, $4, $8, 'anonymized', $6, $6)",
            vec![
                hiqlite::Param::from(role_id.to_owned()),
                hiqlite::Param::from(retained_action_id.to_owned()),
                hiqlite::Param::from(FIELD_VIEW_RESOURCE_TYPE.to_owned()),
                hiqlite::Param::from(FIELD_VIEW_SCHEMA_KEY.to_owned()),
                hiqlite::Param::from(retained_pointer.to_owned()),
                hiqlite::Param::from(timestamp.to_owned()),
                hiqlite::Param::from(removed_action_id.to_owned()),
                hiqlite::Param::from(removed_pointer.to_owned()),
            ],
        )
        .await
        .expect("seed field view policies");
}

async fn assert_field_view_lifecycle(
    runtime: &HiqliteRuntime,
    role_id: &str,
    retained_action_id: &str,
    removed_action_id: &str,
    retained_pointer: &str,
    removed_pointer: &str,
) {
    let actual: Vec<support::NameRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT CAST(COUNT(*) AS TEXT) AS name FROM role_action_field_views
             WHERE role_id = $1 AND action_id = $2 AND field_pointer = $3
             UNION ALL
             SELECT CAST(COUNT(*) AS TEXT) FROM role_action_field_views
             WHERE role_id = $1 AND action_id = $4 AND field_pointer = $5
             UNION ALL
             SELECT CAST(COUNT(*) AS TEXT) FROM role_actions WHERE role_id = $1 AND action_id = $2
             UNION ALL
             SELECT CAST(COUNT(*) AS TEXT) FROM role_actions WHERE role_id = $1 AND action_id = $4
             UNION ALL
             SELECT CAST(COUNT(*) AS TEXT) FROM role_action_field_views rafv
             LEFT JOIN role_actions ra ON ra.role_id = rafv.role_id AND ra.action_id = rafv.action_id
             WHERE ra.role_id IS NULL",
            vec![
                hiqlite::Param::from(role_id.to_owned()),
                hiqlite::Param::from(retained_action_id.to_owned()),
                hiqlite::Param::from(retained_pointer.to_owned()),
                hiqlite::Param::from(removed_action_id.to_owned()),
                hiqlite::Param::from(removed_pointer.to_owned()),
            ],
        )
        .await
        .expect("query field view lifecycle");
    assert_eq!(
        actual.into_iter().map(|row| row.name).collect::<Vec<_>>(),
        ["1", "0", "1", "0", "0"].map(str::to_owned)
    );
}

#[tokio::test]
async fn identity_sync_bind_and_bootstrap_preserve_local_inactive_state() {
    let (db, _runtime, _dir) = activity_database().await;
    db.set_user_active(USER_ID, false)
        .await
        .expect("disable user");

    let synced = db
        .resolve_or_create_user_identity(
            "rauthy",
            USER_ID,
            Some("activity@example.com"),
            "Synced Name",
        )
        .await
        .expect("sync identity");
    assert!(!synced.active);

    let bound = db
        .bind_identity_to_user(BindIdentityInput {
            user_id: USER_ID,
            provider: "rauthy",
            external_user_id: "bound-external-user",
            email: Some("bound@example.com"),
            display_name: "Bound Name",
            active: true,
            allow_rebind: false,
        })
        .await
        .expect("bind identity");
    assert!(!bound.active);

    db.ensure_bootstrap_data().await.expect("repeat bootstrap");
    assert!(!db.get_user(USER_ID).await.expect("stored user").active);
}

#[tokio::test]
async fn authz_denies_inactive_user_provider_and_organization_but_reactivation_is_narrow() {
    let (db, _runtime, _dir) = activity_database().await;
    let allowed = || async {
        db.has_user_organization_action_with_consistency(
            USER_ID,
            ORGANIZATION_ID,
            ORGANIZATION_ID,
            "organization.permission.manage",
            Some(ReadConsistency::Strong),
        )
        .await
        .expect("authorization decision")
    };

    assert!(allowed().await);
    db.set_user_active(USER_ID, false)
        .await
        .expect("disable user");
    assert!(!allowed().await);
    db.set_user_active(USER_ID, true)
        .await
        .expect("enable user");

    db.upsert_service_provider(&ServiceProvider {
        id: "grant".to_owned(),
        name: "Grant".to_owned(),
        description: None,
        active: false,
    })
    .await
    .expect("disable provider");
    assert!(!allowed().await);
    assert!(
        !db.has_user_organization_reactivation_action_with_consistency(
            USER_ID,
            ORGANIZATION_ID,
            Some(ReadConsistency::Strong),
        )
        .await
        .expect("provider-disabled reactivation decision")
    );
    db.upsert_service_provider(&ServiceProvider {
        id: "grant".to_owned(),
        name: "Grant".to_owned(),
        description: None,
        active: true,
    })
    .await
    .expect("enable provider");

    db.deactivate_organization(ORGANIZATION_ID)
        .await
        .expect("deactivate organization");
    assert!(!allowed().await);
    assert!(
        db.has_user_organization_reactivation_action_with_consistency(
            USER_ID,
            ORGANIZATION_ID,
            Some(ReadConsistency::Strong),
        )
        .await
        .expect("reactivation decision")
    );
}

#[tokio::test]
async fn bulk_authz_preserves_absolute_result_indexes_across_query_chunks() {
    const QUERY_CHUNK_SIZE: usize = 90;
    const ALLOWED_ACTION: &str = "organization.permission.manage";
    const DENIED_ACTION: &str = "action-that-does-not-exist";

    let (db, _runtime, _dir) = activity_database().await;

    for check_count in [QUERY_CHUNK_SIZE + 1, QUERY_CHUNK_SIZE * 2] {
        let expected = (0..check_count)
            .map(|index| {
                let chunk_number = index / QUERY_CHUNK_SIZE;
                let chunk_index = index % QUERY_CHUNK_SIZE;
                (chunk_number + chunk_index).is_multiple_of(2)
            })
            .collect::<Vec<_>>();
        let checks = expected
            .iter()
            .map(|allowed| UserActionCheck {
                user_id: USER_ID,
                organization_id: ORGANIZATION_ID,
                provider_id: "grant",
                resource_type: "organization",
                resource_id: ORGANIZATION_ID,
                action_id: if *allowed {
                    ALLOWED_ACTION
                } else {
                    DENIED_ACTION
                },
                application_id: None,
                consistency: Some(ReadConsistency::Strong),
            })
            .collect::<Vec<_>>();

        let decisions = db
            .evaluate_user_actions(&checks)
            .await
            .expect("bulk authorization decisions");

        assert_eq!(decisions.len(), check_count);
        for (index, (decision, expected_allowed)) in
            decisions.iter().zip(expected.iter()).enumerate()
        {
            assert_eq!(
                decision.allowed, *expected_allowed,
                "allowed decision mismatch at absolute index {index} for {check_count} checks"
            );
            assert_eq!(
                decision.rbac_allowed, *expected_allowed,
                "RBAC decision mismatch at absolute index {index} for {check_count} checks"
            );
            assert!(!decision.trusted_application);
        }
    }
}

#[tokio::test]
async fn application_authz_bulk_sql_requires_ready_for_explicit_and_trusted_apps() {
    let (db, _runtime, _dir) = activity_database().await;
    let mut explicit = support::test_application("application-explicit", ORGANIZATION_ID);
    explicit.created_by_user_id = USER_ID.to_owned();
    explicit.provisioning_status = ApplicationProvisioningStatus::Unprovisioned;
    db.insert_application(&explicit)
        .await
        .expect("insert explicit application");

    let mut trusted = support::test_application("application-trusted", ORGANIZATION_ID);
    trusted.created_by_user_id = USER_ID.to_owned();
    trusted.provisioning_status = ApplicationProvisioningStatus::Unprovisioned;
    db.insert_application(&trusted)
        .await
        .expect("insert trusted application");

    for application_id in [&explicit.id, &trusted.id] {
        db.replace_application_actions(application_id, &["organization.profile.read".to_owned()])
            .await
            .expect("application actions");
    }
    db.set_application_trust(ORGANIZATION_ID, &trusted.id, true)
        .await
        .expect("trust application");
    db.replace_active_application_consent(
        &support::test_application_grant("grant-explicit", USER_ID, &explicit.id, ORGANIZATION_ID),
        &["organization.profile.read".to_owned()],
    )
    .await
    .expect("explicit application consent");

    let checks = [
        UserActionCheck {
            user_id: USER_ID,
            organization_id: ORGANIZATION_ID,
            provider_id: "grant",
            resource_type: "organization",
            resource_id: ORGANIZATION_ID,
            action_id: "organization.profile.read",
            application_id: Some(&explicit.id),
            consistency: Some(ReadConsistency::Strong),
        },
        UserActionCheck {
            user_id: USER_ID,
            organization_id: ORGANIZATION_ID,
            provider_id: "grant",
            resource_type: "organization",
            resource_id: ORGANIZATION_ID,
            action_id: "organization.profile.read",
            application_id: Some(&trusted.id),
            consistency: Some(ReadConsistency::Strong),
        },
    ];
    let unprovisioned = db
        .evaluate_user_actions(&checks)
        .await
        .expect("unprovisioned decisions");
    assert!(unprovisioned.iter().all(|decision| decision.rbac_allowed));
    assert!(unprovisioned.iter().all(|decision| !decision.allowed));
    assert!(
        unprovisioned
            .iter()
            .all(|decision| !decision.trusted_application)
    );

    for application_id in [&explicit.id, &trusted.id] {
        db.mark_application_provisioning_ready(application_id, ORGANIZATION_ID)
            .await
            .expect("mark application ready");
    }
    let ready = db
        .evaluate_user_actions(&checks)
        .await
        .expect("ready decisions");
    assert!(ready.iter().all(|decision| decision.allowed));
    assert!(!ready[0].trusted_application);
    assert!(ready[1].trusted_application);
}

#[tokio::test]
async fn shared_identity_cache_round_trips_verified_client_id() {
    let (db, _runtime, _dir) = activity_database().await;
    let expires_at = Utc::now() + Duration::minutes(5);
    db.put_cached_external_identity(
        "token-hash",
        &CachedExternalIdentity {
            provider: "rauthy".to_owned(),
            external_user_id: "external-user".to_owned(),
            client_id: "oauth-client".to_owned(),
            email: None,
            display_name: "External User".to_owned(),
            active: true,
            expires_at,
        },
        expires_at,
    )
    .await
    .expect("cache identity");

    let cached = db
        .get_cached_external_identity("token-hash", Utc::now(), Some(ReadConsistency::Strong))
        .await
        .expect("read cache")
        .expect("cached identity");
    assert_eq!(cached.client_id, "oauth-client");
}

#[tokio::test]
async fn replace_role_actions_preserves_retained_field_policies_and_removes_deleted_ones() {
    let (db, runtime, _dir) = activity_database().await;
    let role_id = "activity-field-role";
    let kept_action_id = "compos.object.read.retained";
    let removed_action_id = "compos.object.read.removed";
    let kept_pointer = "/values/email";
    let removed_pointer = "/values/internal_note";

    seed_field_view_lifecycle(
        &db,
        &runtime,
        role_id,
        kept_action_id,
        removed_action_id,
        kept_pointer,
        removed_pointer,
    )
    .await;

    let updated = db
        .replace_role_actions(role_id, &[kept_action_id.to_owned()])
        .await
        .expect("replace role actions");

    assert_eq!(updated.len(), 1);
    assert_eq!(updated[0].id, kept_action_id);
    assert_field_view_lifecycle(
        &runtime,
        role_id,
        kept_action_id,
        removed_action_id,
        kept_pointer,
        removed_pointer,
    )
    .await;
}

#[tokio::test]
async fn replace_provider_actions_removes_stale_field_policies_before_role_actions_and_actions() {
    let (db, runtime, _dir) = activity_database().await;
    let role_id = "activity-provider-field-role";
    let kept_action_id = "compos.object.read.retained";
    let stale_action_id = "compos.object.read.stale";
    let kept_pointer = "/values/name";
    let stale_pointer = "/values/legacy_note";

    seed_field_view_lifecycle(
        &db,
        &runtime,
        role_id,
        kept_action_id,
        stale_action_id,
        kept_pointer,
        stale_pointer,
    )
    .await;

    let updated = db
        .replace_provider_actions("compos", &[test_action(kept_action_id)])
        .await
        .expect("replace provider actions");

    assert_eq!(updated.len(), 1);
    assert_eq!(updated[0].id, kept_action_id);
    assert_field_view_lifecycle(
        &runtime,
        role_id,
        kept_action_id,
        stale_action_id,
        kept_pointer,
        stale_pointer,
    )
    .await;
}

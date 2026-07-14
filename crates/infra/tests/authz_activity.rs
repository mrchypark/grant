mod support;

use chrono::{Duration, Utc};
use domain::{
    ApplicationProvisioningStatus, RoleBinding, RoleBindingState, RoleBindingSubjectType,
    ServiceProvider,
};
use infra::{
    config::ReadConsistency,
    database::{BindIdentityInput, CachedExternalIdentity, Database, UserActionCheck},
};

const USER_ID: &str = "activity-user";
const ORGANIZATION_ID: &str = "activity-org";

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
            true,
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

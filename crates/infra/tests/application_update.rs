mod support;

use chrono::{Duration, Utc};
use domain::{
    Application, ApplicationAppType, ApplicationConsentPolicy, ApplicationProvisioningStatus,
    ApplicationType,
};
use hiqlite::Row;
use infra::{config::ReadConsistency, database::Database, runtime::HiqliteRuntime};

const USER_ID: &str = "application-update-user";
const ORGANIZATION_ID: &str = "application-update-org";
const UPDATED_CREATOR_ID: &str = "rauthy:application-updated-creator";

#[derive(Debug, PartialEq, Eq)]
struct SecurityLeaseRow {
    security_mutation_token: Option<String>,
    security_mutation_expires_at: Option<String>,
}

#[derive(Debug)]
struct UpdateStorageRow {
    active: i64,
    created_by_user_id: String,
    updated_at: String,
}

impl From<&mut Row<'_>> for SecurityLeaseRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            security_mutation_token: row.get("security_mutation_token"),
            security_mutation_expires_at: row.get("security_mutation_expires_at"),
        }
    }
}

impl From<&mut Row<'_>> for UpdateStorageRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            active: row.get("active"),
            created_by_user_id: row.get("created_by_user_id"),
            updated_at: row.get("updated_at"),
        }
    }
}

async fn application_database(
    application_id: &str,
    slug: &str,
) -> (Database, HiqliteRuntime, tempfile::TempDir, Application) {
    let (runtime, dir) = support::test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        USER_ID.to_owned(),
        "Application Update User".to_owned(),
    );
    db.ensure_schema().await.expect("ensure schema");
    let updated_creator = db
        .resolve_or_create_user_identity(
            "rauthy",
            "application-updated-creator",
            None,
            "Updated Application Creator",
        )
        .await
        .expect("seed updated creator");
    assert_eq!(updated_creator.id, UPDATED_CREATOR_ID);
    db.upsert_organization_with_owner(
        ORGANIZATION_ID,
        "Application Update Organization",
        None,
        "Application Update Organization",
    )
    .await
    .expect("seed organization");

    let mut application = support::test_application(application_id, ORGANIZATION_ID);
    application.slug = slug.to_owned();
    application.created_by_user_id = USER_ID.to_owned();
    application.rauthy_client_id = Some(format!("client-{application_id}"));
    db.insert_application(&application)
        .await
        .expect("insert application");
    (db, runtime, dir, application)
}

async fn acquire_lease(db: &Database, application_id: &str, token: &str) -> String {
    let acquired_at = Utc::now();
    let expires_at = (acquired_at + Duration::minutes(10)).to_rfc3339();
    assert!(
        db.try_acquire_application_security_mutation(
            application_id,
            ORGANIZATION_ID,
            token,
            &acquired_at.to_rfc3339(),
            &expires_at,
        )
        .await
        .expect("acquire security lease")
    );
    expires_at
}

async fn security_lease(runtime: &HiqliteRuntime, application_id: &str) -> SecurityLeaseRow {
    let rows: Vec<SecurityLeaseRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT security_mutation_token, security_mutation_expires_at
             FROM applications
             WHERE id = $1",
            vec![hiqlite::Param::from(application_id.to_owned())],
        )
        .await
        .expect("read application security lease");
    rows.into_iter().next().expect("application lease row")
}

async fn update_storage(runtime: &HiqliteRuntime, application_id: &str) -> UpdateStorageRow {
    let rows: Vec<UpdateStorageRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT active, created_by_user_id, updated_at
             FROM applications
             WHERE id = $1",
            vec![hiqlite::Param::from(application_id.to_owned())],
        )
        .await
        .expect("read application update storage");
    rows.into_iter().next().expect("application storage row")
}

#[tokio::test]
async fn update_application_maps_mutable_fields_for_unchanged_and_changed_slug() {
    let (db, runtime, _dir, original) =
        application_database("application-mapping", "mapping").await;
    let lease_token = "mapping-lease";
    let lease_expiry = acquire_lease(&db, &original.id, lease_token).await;
    assert_eq!(
        security_lease(&runtime, &original.id).await,
        SecurityLeaseRow {
            security_mutation_token: Some(lease_token.to_owned()),
            security_mutation_expires_at: Some(lease_expiry.clone()),
        }
    );
    let mut unchanged_slug = original.clone();
    unchanged_slug.name = "Updated Without Slug Change".to_owned();
    unchanged_slug.description = Some("updated description".to_owned());
    unchanged_slug.application_type = ApplicationType::Public;
    unchanged_slug.app_type = ApplicationAppType::Mobile;
    unchanged_slug.active = false;
    unchanged_slug.created_by_user_id = UPDATED_CREATOR_ID.to_owned();
    unchanged_slug.first_party = true;
    unchanged_slug.consent_policy = ApplicationConsentPolicy::AutoApprove;
    unchanged_slug.provisioning_status = ApplicationProvisioningStatus::Unprovisioned;
    unchanged_slug.rauthy_client_id = Some("attempted-client-overwrite".to_owned());

    db.update_application_with_security_lease(&unchanged_slug, false, lease_token)
        .await
        .expect("update application without slug change");

    let mut expected = unchanged_slug;
    expected.first_party = original.first_party;
    expected.consent_policy = original.consent_policy;
    expected.provisioning_status = original.provisioning_status;
    expected.rauthy_client_id = original.rauthy_client_id;
    assert_eq!(
        db.get_application_with_consistency(&original.id, Some(ReadConsistency::Strong))
            .await
            .expect("read unchanged-slug update"),
        expected
    );
    assert_eq!(
        security_lease(&runtime, &original.id).await,
        SecurityLeaseRow {
            security_mutation_token: Some(lease_token.to_owned()),
            security_mutation_expires_at: Some(lease_expiry.clone()),
        }
    );
    let unchanged_storage = update_storage(&runtime, &original.id).await;
    assert_eq!(unchanged_storage.active, 0);
    assert_eq!(unchanged_storage.created_by_user_id, UPDATED_CREATOR_ID);
    chrono::DateTime::parse_from_rfc3339(&unchanged_storage.updated_at)
        .expect("unchanged-slug updated_at must remain an RFC 3339 timestamp");

    let mut changed_slug = expected.clone();
    changed_slug.slug = "mapping-renamed".to_owned();
    changed_slug.name = "Updated With Slug Change".to_owned();
    changed_slug.description = None;
    changed_slug.application_type = ApplicationType::Confidential;
    changed_slug.app_type = ApplicationAppType::Internal;
    changed_slug.active = true;

    db.update_application_with_security_lease(&changed_slug, true, lease_token)
        .await
        .expect("update application with slug change");
    assert_eq!(
        db.get_application_with_consistency(&original.id, Some(ReadConsistency::Strong))
            .await
            .expect("read changed-slug update"),
        changed_slug
    );
    assert_eq!(
        security_lease(&runtime, &original.id).await,
        SecurityLeaseRow {
            security_mutation_token: Some(lease_token.to_owned()),
            security_mutation_expires_at: Some(lease_expiry),
        }
    );
    let changed_storage = update_storage(&runtime, &original.id).await;
    assert_eq!(changed_storage.active, 1);
    assert_eq!(changed_storage.created_by_user_id, UPDATED_CREATOR_ID);
    chrono::DateTime::parse_from_rfc3339(&changed_storage.updated_at)
        .expect("changed-slug updated_at must remain an RFC 3339 timestamp");
}

#[tokio::test]
async fn update_application_rejects_wrong_and_expired_leases_without_corruption() {
    let (db, runtime, _dir, original) = application_database("application-lease", "lease").await;
    let valid_token = "valid-lease";
    let valid_expiry = acquire_lease(&db, &original.id, valid_token).await;
    let mut attempted = original.clone();
    attempted.name = "Must Not Be Written".to_owned();

    db.update_application_with_security_lease(&attempted, false, "wrong-lease")
        .await
        .expect("wrong lease token produces no mutation");
    assert_eq!(
        db.get_application_with_consistency(&original.id, Some(ReadConsistency::Strong))
            .await
            .expect("read after wrong token"),
        original
    );
    assert_eq!(
        security_lease(&runtime, &original.id).await,
        SecurityLeaseRow {
            security_mutation_token: Some(valid_token.to_owned()),
            security_mutation_expires_at: Some(valid_expiry),
        }
    );

    db.release_application_security_mutation(&original.id, valid_token)
        .await
        .expect("release valid lease");
    let acquired_at = Utc::now();
    let expired_at = (acquired_at - Duration::seconds(1)).to_rfc3339();
    assert!(
        db.try_acquire_application_security_mutation(
            &original.id,
            ORGANIZATION_ID,
            "expired-lease",
            &acquired_at.to_rfc3339(),
            &expired_at,
        )
        .await
        .expect("store expired lease")
    );
    db.update_application_with_security_lease(&attempted, false, "expired-lease")
        .await
        .expect("expired lease produces no mutation");
    assert_eq!(
        db.get_application_with_consistency(&original.id, Some(ReadConsistency::Strong))
            .await
            .expect("read after expired lease"),
        original
    );
    assert_eq!(
        security_lease(&runtime, &original.id).await,
        SecurityLeaseRow {
            security_mutation_token: Some("expired-lease".to_owned()),
            security_mutation_expires_at: Some(expired_at),
        }
    );
}

#[tokio::test]
async fn update_application_preserves_trusted_application_invariant() {
    let (db, runtime, _dir, original) =
        application_database("application-trusted-update", "trusted-update").await;
    let trusted = db
        .set_application_trust(ORGANIZATION_ID, &original.id, true)
        .await
        .expect("trust application");
    let mut attempted = trusted.clone();
    attempted.name = "Must Not Change Trusted App".to_owned();

    let error = db
        .update_application_with_security_lease(&attempted, false, "unused-lease")
        .await
        .expect_err("trusted application update must fail");
    assert!(error.to_string().contains("trusted applications"));
    assert_eq!(
        db.get_application_with_consistency(&original.id, Some(ReadConsistency::Strong))
            .await
            .expect("read trusted application"),
        trusted
    );
    assert_eq!(
        security_lease(&runtime, &original.id).await,
        SecurityLeaseRow {
            security_mutation_token: None,
            security_mutation_expires_at: None,
        }
    );
}

#[tokio::test]
async fn update_application_rejects_duplicate_slug_without_corruption() {
    let (db, runtime, _dir, original) =
        application_database("application-duplicate-update", "original-slug").await;
    let mut occupied = support::test_application("application-occupied", ORGANIZATION_ID);
    occupied.slug = "occupied-slug".to_owned();
    occupied.created_by_user_id = USER_ID.to_owned();
    db.insert_application(&occupied)
        .await
        .expect("insert occupied slug application");
    let lease_token = "duplicate-lease";
    let lease_expiry = acquire_lease(&db, &original.id, lease_token).await;
    let mut attempted = original.clone();
    attempted.slug = occupied.slug.clone();
    attempted.name = "Must Not Be Corrupted".to_owned();

    let error = db
        .update_application_with_security_lease(&attempted, true, lease_token)
        .await
        .expect_err("duplicate slug must fail");
    assert!(error.to_string().contains("slug already exists"));
    assert_eq!(
        db.get_application_with_consistency(&original.id, Some(ReadConsistency::Strong))
            .await
            .expect("read after duplicate slug"),
        original
    );
    assert_eq!(
        security_lease(&runtime, &original.id).await,
        SecurityLeaseRow {
            security_mutation_token: Some(lease_token.to_owned()),
            security_mutation_expires_at: Some(lease_expiry),
        }
    );
}

mod support;

use chrono::{Duration, Utc};
use domain::{
    Application, ApplicationAppType, ApplicationConsentPolicy, ApplicationProvisioningStatus,
    ApplicationRedirectUri, ApplicationType,
};
use hiqlite::Row;
use infra::{
    config::ReadConsistency,
    database::{
        ApplicationConsentWriteError, ApplicationSecurityMutationError,
        ApplicationTrustMutationError, Database,
    },
    runtime::HiqliteRuntime,
};

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
    let mut occupied = support::test_application("application-lease-occupied", ORGANIZATION_ID);
    occupied.slug = "lease-occupied".to_owned();
    occupied.created_by_user_id = USER_ID.to_owned();
    db.insert_application(&occupied)
        .await
        .expect("insert occupied application");
    let mut attempted = original.clone();
    attempted.name = "Must Not Be Written".to_owned();
    attempted.slug = occupied.slug;

    let wrong_lease_error = db
        .update_application_with_security_lease(&attempted, true, "wrong-lease")
        .await
        .expect_err("wrong lease token must fail");
    assert_eq!(
        wrong_lease_error.downcast_ref::<ApplicationSecurityMutationError>(),
        Some(&ApplicationSecurityMutationError::LeaseExpiredOrInvalid {
            application_id: original.id.clone(),
        })
    );
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
    let expired_lease_error = db
        .update_application_with_security_lease(&attempted, true, "expired-lease")
        .await
        .expect_err("expired lease must fail");
    assert_eq!(
        expired_lease_error.downcast_ref::<ApplicationSecurityMutationError>(),
        Some(&ApplicationSecurityMutationError::LeaseExpiredOrInvalid {
            application_id: original.id.clone(),
        })
    );
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
async fn security_lease_mutations_report_invalid_trusted_and_missing_targets() {
    let (db, runtime, _dir, original) =
        application_database("application-security-mutations", "security-mutations").await;
    let lease_token = "security-mutation-lease";
    let lease_expiry = acquire_lease(&db, &original.id, lease_token).await;
    let redirect_uri = ApplicationRedirectUri {
        application_id: original.id.clone(),
        redirect_uri: "https://security-mutations.example/callback".to_owned(),
        created_at: support::FIXED_TEST_TIME.to_owned(),
    };

    let insert_error = db
        .insert_application_redirect_uri_with_security_lease(&redirect_uri, "wrong-lease")
        .await
        .expect_err("wrong lease must reject redirect URI insertion");
    assert_eq!(
        insert_error.downcast_ref::<ApplicationSecurityMutationError>(),
        Some(&ApplicationSecurityMutationError::LeaseExpiredOrInvalid {
            application_id: original.id.clone(),
        })
    );
    assert!(
        db.list_application_redirect_uris_with_consistency(
            &original.id,
            Some(ReadConsistency::Strong),
        )
        .await
        .expect("list redirect URIs after rejected insert")
        .is_empty()
    );

    db.insert_application_redirect_uri_with_security_lease(&redirect_uri, lease_token)
        .await
        .expect("insert redirect URI with valid lease");
    db.insert_application_redirect_uri_with_security_lease(&redirect_uri, lease_token)
        .await
        .expect("duplicate redirect URI with valid lease is idempotent");
    assert_eq!(
        db.list_application_redirect_uris_with_consistency(
            &original.id,
            Some(ReadConsistency::Strong),
        )
        .await
        .expect("list redirect URIs after idempotent duplicate insert"),
        vec![redirect_uri.clone()]
    );
    let duplicate_wrong_lease_error = db
        .insert_application_redirect_uri_with_security_lease(&redirect_uri, "wrong-lease")
        .await
        .expect_err("duplicate redirect URI must not hide a wrong lease");
    assert_eq!(
        duplicate_wrong_lease_error.downcast_ref::<ApplicationSecurityMutationError>(),
        Some(&ApplicationSecurityMutationError::LeaseExpiredOrInvalid {
            application_id: original.id.clone(),
        })
    );
    let delete_error = db
        .delete_application_redirect_uri_with_security_lease(
            &original.id,
            &redirect_uri.redirect_uri,
            "wrong-lease",
        )
        .await
        .expect_err("wrong lease must reject redirect URI deletion");
    assert_eq!(
        delete_error.downcast_ref::<ApplicationSecurityMutationError>(),
        Some(&ApplicationSecurityMutationError::LeaseExpiredOrInvalid {
            application_id: original.id.clone(),
        })
    );
    assert_eq!(
        db.list_application_redirect_uris_with_consistency(
            &original.id,
            Some(ReadConsistency::Strong),
        )
        .await
        .expect("list redirect URIs after rejected delete"),
        vec![redirect_uri.clone()]
    );

    let replace_error = db
        .replace_application_actions_with_security_lease(&original.id, &[], "wrong-lease")
        .await
        .expect_err("wrong lease must reject application action replacement");
    assert_eq!(
        replace_error.downcast_ref::<ApplicationSecurityMutationError>(),
        Some(&ApplicationSecurityMutationError::LeaseExpiredOrInvalid {
            application_id: original.id.clone(),
        })
    );

    let release_error = db
        .release_application_security_mutation(&original.id, "wrong-lease")
        .await
        .expect_err("wrong token must reject security lease release");
    assert_eq!(
        release_error.downcast_ref::<ApplicationSecurityMutationError>(),
        Some(&ApplicationSecurityMutationError::LeaseExpiredOrInvalid {
            application_id: original.id.clone(),
        })
    );
    assert_eq!(
        security_lease(&runtime, &original.id).await,
        SecurityLeaseRow {
            security_mutation_token: Some(lease_token.to_owned()),
            security_mutation_expires_at: Some(lease_expiry),
        }
    );

    db.release_application_security_mutation(&original.id, lease_token)
        .await
        .expect("release valid lease");
    db.set_application_trust(ORGANIZATION_ID, &original.id, true)
        .await
        .expect("trust application");
    let trusted_error = db
        .insert_application_redirect_uri_with_security_lease(&redirect_uri, "unused-lease")
        .await
        .expect_err("trusted application must reject redirect URI insertion");
    assert_eq!(
        trusted_error.downcast_ref::<ApplicationSecurityMutationError>(),
        Some(&ApplicationSecurityMutationError::TrustedApplication {
            application_id: original.id.clone(),
        })
    );

    let missing_redirect_uri = ApplicationRedirectUri {
        application_id: "application-missing-security-mutation".to_owned(),
        redirect_uri: "https://missing.example/callback".to_owned(),
        created_at: support::FIXED_TEST_TIME.to_owned(),
    };
    let missing_error = db
        .insert_application_redirect_uri_with_security_lease(&missing_redirect_uri, "unused-lease")
        .await
        .expect_err("missing application must reject redirect URI insertion");
    assert_eq!(
        missing_error.downcast_ref::<ApplicationSecurityMutationError>(),
        Some(&ApplicationSecurityMutationError::ApplicationNotFound {
            application_id: missing_redirect_uri.application_id,
        })
    );
}

#[tokio::test]
async fn database_emits_typed_consent_and_trust_conflicts() {
    let (db, runtime, _dir, application) =
        application_database("application-typed-conflicts", "typed-conflicts").await;

    runtime
        .execute(
            "UPDATE applications SET active = 0 WHERE id = $1",
            vec![hiqlite::Param::from(application.id.clone())],
        )
        .await
        .expect("deactivate application");
    let grant = support::test_application_grant(
        "typed-consent-grant",
        USER_ID,
        &application.id,
        ORGANIZATION_ID,
    );
    let consent_error = db
        .replace_active_application_consent(&grant, &[])
        .await
        .expect_err("inactive application must fail consent preconditions");
    assert_eq!(
        consent_error.downcast_ref::<ApplicationConsentWriteError>(),
        Some(&ApplicationConsentWriteError::PreconditionsChanged)
    );

    runtime
        .execute(
            "UPDATE applications SET active = 1 WHERE id = $1",
            vec![hiqlite::Param::from(application.id.clone())],
        )
        .await
        .expect("reactivate application");
    acquire_lease(&db, &application.id, "typed-conflict-lease").await;
    let trust_error = db
        .set_application_trust(ORGANIZATION_ID, &application.id, true)
        .await
        .expect_err("active security lease must block trust mutation");
    assert_eq!(
        trust_error.downcast_ref::<ApplicationTrustMutationError>(),
        Some(&ApplicationTrustMutationError::SecurityMutationInProgress {
            application_id: application.id,
        })
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
    assert_eq!(
        error.downcast_ref::<ApplicationSecurityMutationError>(),
        Some(&ApplicationSecurityMutationError::TrustedApplication {
            application_id: original.id.clone(),
        })
    );
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

    db.release_application_security_mutation(&original.id, lease_token)
        .await
        .expect("release valid lease");
    db.set_application_trust(ORGANIZATION_ID, &original.id, true)
        .await
        .expect("trust application");
    let trusted_error = db
        .update_application_with_security_lease(&attempted, true, "unused-lease")
        .await
        .expect_err("duplicate slug must not hide a trusted target");
    assert_eq!(
        trusted_error.downcast_ref::<ApplicationSecurityMutationError>(),
        Some(&ApplicationSecurityMutationError::TrustedApplication {
            application_id: original.id.clone(),
        })
    );

    let mut missing = attempted;
    missing.id = "application-missing-duplicate-update".to_owned();
    let missing_error = db
        .update_application_with_security_lease(&missing, true, "unused-lease")
        .await
        .expect_err("duplicate slug must not hide a missing target");
    assert_eq!(
        missing_error.downcast_ref::<ApplicationSecurityMutationError>(),
        Some(&ApplicationSecurityMutationError::ApplicationNotFound {
            application_id: missing.id,
        })
    );
}

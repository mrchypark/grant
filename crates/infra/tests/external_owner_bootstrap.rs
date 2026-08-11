mod support;

use infra::{
    config::{
        AppConfig, AuthConfig, AuthProviderMode, HiqliteConfig, LocalNodeRole, PatchDataAuthConfig,
        RauthyAuthConfig, ReadConsistency, RuntimeMode, SecurityConfig, ServerConfig,
    },
    database::{
        Database, ExternalIdentityMaterialization, ExternalOwnerBootstrapCommand,
        ExternalOwnerBootstrapError,
    },
    runtime::HiqliteRuntime,
    service::AppService,
};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Barrier;

async fn patch_data_identity_server() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind Patch Data test server");
    let address = listener.local_addr().expect("Patch Data test address");
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept Patch Data request");
        let mut request = [0_u8; 4096];
        let _ = stream
            .read(&mut request)
            .await
            .expect("read Patch Data request");
        let body = r#"{"token":"opaque","record":{"id":"external-owner","email":"owner@example.test","verified":true,"displayName":"External Owner","organizationId":"external-organization"}}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write Patch Data response");
    });
    format!("http://{address}")
}

async fn patch_data_service() -> (Arc<AppService>, Database, HiqliteRuntime, tempfile::TempDir) {
    let base_url = patch_data_identity_server().await;
    let (runtime, dir) = support::test_runtime().await;
    let cfg = AppConfig {
        server: ServerConfig {
            bind_addr: "127.0.0.1:0".to_owned(),
        },
        security: SecurityConfig {
            master_key: "test-master-key".to_owned(),
            app_auth_key: "test-app-key".to_owned(),
            bootstrap_user_id: "bootstrap-user".to_owned(),
            bootstrap_user_name: "Bootstrap User".to_owned(),
        },
        auth: AuthConfig {
            provider_mode: AuthProviderMode::PatchData,
            rauthy: RauthyAuthConfig {
                base_url: "http://127.0.0.1:1".to_owned(),
                introspect_path: "/unused".to_owned(),
                userinfo_path: "/unused".to_owned(),
                client_id: "unused".to_owned(),
                client_secret: "unused".to_owned(),
                client_read_api_key: "unused".to_owned(),
                cache_ttl_seconds: 30,
            },
            patch_data: Some(PatchDataAuthConfig {
                base_url,
                exposure_application_id: "missing-exposure-application".to_owned(),
            }),
        },
        hiqlite: HiqliteConfig {
            runtime_mode: RuntimeMode::Embedded,
            local_node_role: LocalNodeRole::Voter,
            default_read_consistency: ReadConsistency::Local,
            remote_nodes: vec![],
            remote_tls: false,
            remote_tls_no_verify: false,
            remote_api_secret: None,
            remote_with_proxy: false,
        },
    };
    let db = Database::new(
        runtime.clone(),
        cfg.security.bootstrap_user_id.clone(),
        cfg.security.bootstrap_user_name.clone(),
    );
    let service = Arc::new(AppService::new(cfg, db.clone()));
    service.initialize().await.expect("initialize service");
    (service, db, runtime, dir)
}

async fn seeded_user_and_organization(db: &Database) -> (String, String) {
    let user = db
        .resolve_or_create_user_identity("patch-data", "external-owner", None, "External Owner")
        .await
        .expect("seed external identity");
    let organization_id = "external-organization".to_owned();
    db.upsert_organization_identity(&organization_id, None, &organization_id)
        .await
        .expect("seed external organization");
    (user.id, organization_id)
}

async fn membership_role(runtime: &HiqliteRuntime, organization_id: &str, user_id: &str) -> String {
    let rows: Vec<support::NameRow> = runtime
        .query_rows(
            infra::config::ReadConsistency::Strong,
            "SELECT membership_role AS name
             FROM organization_memberships
             WHERE organization_id = $1 AND user_id = $2",
            vec![organization_id.into(), user_id.into()],
        )
        .await
        .expect("membership role");
    rows.into_iter().next().expect("membership row").name
}

async fn role_binding_count(runtime: &HiqliteRuntime, organization_id: &str, user_id: &str) -> i64 {
    let rows: Vec<support::NameRow> = runtime
        .query_rows(
            infra::config::ReadConsistency::Strong,
            "SELECT id AS name FROM role_bindings
             WHERE subject_type = 'user'
               AND subject_id = $1
               AND subject_organization_id = $2
               AND provider_id = 'grant'
               AND resource_type = 'organization'
               AND resource_id = $2
               AND role_id = 'role-system-bootstrap-owner'
               AND state = 'active'",
            vec![user_id.into(), organization_id.into()],
        )
        .await
        .expect("role binding");
    i64::try_from(rows.len()).expect("binding count")
}

/// A complete, order-stable record of every table the atomic operation may
/// mutate.  Keeping timestamps in the fingerprint catches accidental upserts
/// on rejection and on an otherwise-idempotent retry.
async fn atomic_fingerprint(runtime: &HiqliteRuntime, organization_id: &str) -> Vec<String> {
    let rows: Vec<support::NameRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT 'user|' || id || '|' || COALESCE(email, '') || '|' || display_name || '|' || active || '|' || created_at AS name FROM users
             UNION ALL SELECT 'identity|' || id || '|' || user_id || '|' || provider || '|' || external_user_id || '|' || COALESCE(email_snapshot, '') || '|' || COALESCE(display_name_snapshot, '') || '|' || COALESCE(last_seen_at, '') || '|' || created_at || '|' || updated_at FROM user_identities
             UNION ALL SELECT 'org|' || id || '|' || COALESCE(business_no, '') || '|' || name || '|' || COALESCE(inactivated_at, '') || '|' || created_at || '|' || updated_at FROM organizations WHERE id = $1
             UNION ALL SELECT 'membership|' || id || '|' || organization_id || '|' || user_id || '|' || membership_role || '|' || status || '|' || created_at || '|' || updated_at FROM organization_memberships WHERE organization_id = $1
             UNION ALL SELECT 'binding|' || id || '|' || subject_type || '|' || subject_id || '|' || subject_organization_id || '|' || provider_id || '|' || resource_type || '|' || resource_id || '|' || role_id || '|' || state || '|' || created_at || '|' || updated_at FROM role_bindings WHERE subject_organization_id = $1
             ORDER BY name",
            vec![organization_id.into()],
        )
        .await
        .expect("atomic table fingerprint");
    rows.into_iter().map(|row| row.name).collect()
}

async fn prepare_ready_atomic_command(db: &Database, organization_id: &str) {
    db.upsert_organization_identity(organization_id, None, organization_id)
        .await
        .expect("organization");
    let mut application = support::test_application("atomic-exposure", organization_id);
    application.created_by_user_id = db
        .ensure_bootstrap_principals()
        .await
        .expect("bootstrap principal")
        .id;
    db.upsert_application(&application)
        .await
        .expect("ready application");
}

#[tokio::test]
async fn bootstrap_external_owner_creates_owner_and_binding_idempotently() {
    let (runtime, db, _dir) = support::ensured_database().await;
    let (user_id, organization_id) = seeded_user_and_organization(&db).await;

    db.bootstrap_external_owner(&organization_id, &user_id)
        .await
        .expect("bootstrap owner");
    db.bootstrap_external_owner(&organization_id, &user_id)
        .await
        .expect("repeat bootstrap owner");

    assert_eq!(
        membership_role(&runtime, &organization_id, &user_id).await,
        "owner"
    );
    assert_eq!(
        role_binding_count(&runtime, &organization_id, &user_id).await,
        1
    );
}

#[tokio::test]
async fn atomic_external_owner_bootstrap_creates_only_canonical_identity_and_is_idempotent() {
    let (runtime, db, _dir) = support::ensured_database().await;
    let organization_id = "atomic-external-org";
    db.upsert_organization_identity(organization_id, None, organization_id)
        .await
        .expect("external organization");
    let mut application = support::test_application("atomic-exposure", organization_id);
    application.created_by_user_id = db
        .ensure_bootstrap_principals()
        .await
        .expect("bootstrap principal")
        .id;
    db.upsert_application(&application)
        .await
        .expect("ready exposure application");
    let command = || ExternalOwnerBootstrapCommand {
        identity: ExternalIdentityMaterialization {
            provider: "patch-data",
            external_user_id: "external-subject",
            email: Some("atomic@example.test"),
            display_name: "Atomic Owner",
        },
        organization_id,
        application_id: "atomic-exposure",
    };
    let first = db
        .bootstrap_external_owner_atomic(command())
        .await
        .expect("atomic bootstrap");
    let second = db
        .bootstrap_external_owner_atomic(command())
        .await
        .expect("idempotent atomic bootstrap");
    assert_eq!(first, second);
    assert_ne!(first.user_id, "external-subject");
    assert_eq!(
        membership_role(&runtime, organization_id, &first.user_id).await,
        "owner"
    );
    assert_eq!(
        role_binding_count(&runtime, organization_id, &first.user_id).await,
        1
    );
}

async fn assert_atomic_rejection_preserves_every_table_and_timestamp(case: &str) {
    let (runtime, db, _dir) = support::ensured_database().await;
    let organization_id = "atomic-rejection-org";
    prepare_ready_atomic_command(&db, organization_id).await;
    match case {
        "missing-app" => {
            runtime
                .execute(
                    "DELETE FROM applications WHERE id='atomic-exposure'",
                    vec![],
                )
                .await
                .expect("remove app");
        }
        "inactive-app" => {
            runtime
                .execute(
                    "UPDATE applications SET active=0 WHERE id='atomic-exposure'",
                    vec![],
                )
                .await
                .expect("deactivate app");
        }
        "unready-app" => {
            runtime.execute("UPDATE applications SET provisioning_status='unprovisioned' WHERE id='atomic-exposure'", vec![]).await.expect("unready app");
        }
        "inactive-org" => {
            db.deactivate_organization(organization_id)
                .await
                .expect("deactivate organization");
        }
        "identity-email-collision" => {
            db.resolve_or_create_user_identity(
                "other-provider",
                "same-email",
                Some("atomic@example.test"),
                "Other User",
            )
            .await
            .expect("email collision");
        }
        "existing-member" => {
            let user = db
                .resolve_or_create_user_identity(
                    "patch-data",
                    "atomic-subject",
                    Some("atomic@example.test"),
                    "Atomic Owner",
                )
                .await
                .expect("existing identity");
            runtime.execute(
                    "INSERT INTO organization_memberships (id,organization_id,user_id,membership_role,status,created_at,updated_at) VALUES ('existing-member',$1,$2,'member','active',$3,$3)",
                    vec![organization_id.into(), user.id.into(), support::FIXED_TEST_TIME.into()],
                ).await.expect("member");
        }
        "existing-admin" | "disabled-bootstrap-binding" => {
            let user = db
                .resolve_or_create_user_identity(
                    "patch-data",
                    "atomic-subject",
                    Some("atomic@example.test"),
                    "Atomic Owner",
                )
                .await
                .expect("existing identity");
            if case == "existing-admin" {
                runtime.execute(
                        "INSERT INTO organization_memberships (id,organization_id,user_id,membership_role,status,created_at,updated_at) VALUES ('existing-admin',$1,$2,'admin','active',$3,$3)",
                        vec![organization_id.into(), user.id.into(), support::FIXED_TEST_TIME.into()],
                    ).await.expect("admin");
            } else {
                runtime.execute(
                        "INSERT INTO role_bindings (id,subject_type,subject_id,subject_organization_id,provider_id,resource_type,resource_id,role_id,agreement_id,state,created_by_user_id,created_at,updated_at) VALUES ('disabled-bootstrap','user',$1,$2,'grant','organization',$2,'role-system-bootstrap-owner',NULL,'disabled',$1,$3,$3)",
                        vec![user.id.into(), organization_id.into(), support::FIXED_TEST_TIME.into()],
                    ).await.expect("disabled binding");
            }
        }
        "other-owner" => {
            let other = db
                .resolve_or_create_user_identity("patch-data", "other-owner", None, "Other Owner")
                .await
                .expect("other identity");
            runtime.execute(
                    "INSERT INTO organization_memberships (id,organization_id,user_id,membership_role,status,created_at,updated_at) VALUES ('other-owner',$1,$2,'owner','active',$3,$3)",
                    vec![organization_id.into(), other.id.into(), support::FIXED_TEST_TIME.into()],
                ).await.expect("other owner");
        }
        "disabled-provider" => {
            runtime
                .execute(
                    "UPDATE service_providers SET active=0 WHERE id='grant'",
                    vec![],
                )
                .await
                .expect("disable provider");
        }
        "missing-provider" => {
            runtime
                .execute("DELETE FROM service_providers WHERE id='grant'", vec![])
                .await
                .expect("remove provider");
        }
        "missing-role" => {
            runtime
                .execute(
                    "DELETE FROM role_actions WHERE role_id='role-system-bootstrap-owner'",
                    vec![],
                )
                .await
                .expect("remove role actions");
            runtime
                .execute(
                    "DELETE FROM roles WHERE id='role-system-bootstrap-owner'",
                    vec![],
                )
                .await
                .expect("remove role");
        }
        _ => unreachable!(),
    }
    let before = atomic_fingerprint(&runtime, organization_id).await;
    let result = db
        .bootstrap_external_owner_atomic(ExternalOwnerBootstrapCommand {
            identity: ExternalIdentityMaterialization {
                provider: "patch-data",
                external_user_id: "atomic-subject",
                email: Some("atomic@example.test"),
                display_name: "Atomic Owner",
            },
            organization_id,
            application_id: "atomic-exposure",
        })
        .await;
    assert!(result.is_err(), "{case} must reject");
    assert_eq!(
        atomic_fingerprint(&runtime, organization_id).await,
        before,
        "{case} must have zero writes, including timestamps"
    );
}

macro_rules! atomic_rejection_test {
    ($name:ident, $case:literal) => {
        #[tokio::test]
        async fn $name() {
            assert_atomic_rejection_preserves_every_table_and_timestamp($case).await;
        }
    };
}

atomic_rejection_test!(atomic_bootstrap_rejects_missing_application, "missing-app");
atomic_rejection_test!(
    atomic_bootstrap_rejects_inactive_application,
    "inactive-app"
);
atomic_rejection_test!(atomic_bootstrap_rejects_unready_application, "unready-app");
atomic_rejection_test!(
    atomic_bootstrap_rejects_inactive_organization,
    "inactive-org"
);
atomic_rejection_test!(
    atomic_bootstrap_rejects_identity_email_collision,
    "identity-email-collision"
);
atomic_rejection_test!(atomic_bootstrap_rejects_existing_member, "existing-member");
atomic_rejection_test!(atomic_bootstrap_rejects_existing_admin, "existing-admin");
atomic_rejection_test!(atomic_bootstrap_rejects_other_owner, "other-owner");
atomic_rejection_test!(
    atomic_bootstrap_rejects_disabled_provider,
    "disabled-provider"
);
atomic_rejection_test!(
    atomic_bootstrap_rejects_missing_provider,
    "missing-provider"
);
atomic_rejection_test!(atomic_bootstrap_rejects_missing_role, "missing-role");
atomic_rejection_test!(
    atomic_bootstrap_rejects_disabled_bootstrap_binding,
    "disabled-bootstrap-binding"
);

#[tokio::test]
async fn materialization_reserves_email_across_concurrent_distinct_external_identities() {
    let (runtime, db, _dir) = support::ensured_database().await;
    let barrier = Arc::new(Barrier::new(2));
    let first_db = db.clone();
    let first_barrier = barrier.clone();
    let first = tokio::spawn(async move {
        first_barrier.wait().await;
        first_db
            .materialize_external_identity(ExternalIdentityMaterialization {
                provider: "patch-data",
                external_user_id: "concurrent-subject-a",
                email: Some("reserved@example.test"),
                display_name: "Concurrent A",
            })
            .await
    });
    let second_db = db.clone();
    let second = tokio::spawn(async move {
        barrier.wait().await;
        second_db
            .materialize_external_identity(ExternalIdentityMaterialization {
                provider: "patch-data",
                external_user_id: "concurrent-subject-b",
                email: Some("reserved@example.test"),
                display_name: "Concurrent B",
            })
            .await
    });
    let results = [
        first.await.expect("first task"),
        second.await.expect("second task"),
    ];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);
    let users: Vec<support::NameRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT id AS name FROM users WHERE email='reserved@example.test'",
            vec![],
        )
        .await
        .expect("reserved user");
    let identities: Vec<support::NameRow> = runtime.query_rows(
        ReadConsistency::Strong,
        "SELECT external_user_id AS name FROM user_identities WHERE provider='patch-data' AND external_user_id IN ('concurrent-subject-a','concurrent-subject-b')",
        vec![],
    ).await.expect("reserved identity");
    assert_eq!(users.len(), 1);
    assert_eq!(identities.len(), 1);
}

#[tokio::test]
async fn atomic_bootstrap_rolls_back_membership_when_binding_insert_is_ignored() {
    let (runtime, db, _dir) = support::ensured_database().await;
    let organization_id = "binding-guard-org";
    prepare_ready_atomic_command(&db, organization_id).await;
    runtime
        .batch(
            "CREATE TRIGGER force_bootstrap_binding_noop BEFORE INSERT ON role_bindings
         WHEN NEW.role_id = 'role-system-bootstrap-owner'
         BEGIN SELECT RAISE(IGNORE); END;",
        )
        .await
        .expect("install test-only binding noop trigger");
    let before = atomic_fingerprint(&runtime, organization_id).await;
    assert!(
        db.bootstrap_external_owner_atomic(ExternalOwnerBootstrapCommand {
            identity: ExternalIdentityMaterialization {
                provider: "patch-data",
                external_user_id: "binding-guard-subject",
                email: Some("binding-guard@example.test"),
                display_name: "Binding Guard",
            },
            organization_id,
            application_id: "atomic-exposure",
        })
        .await
        .is_err()
    );
    assert_eq!(atomic_fingerprint(&runtime, organization_id).await, before);
}

#[tokio::test]
async fn atomic_bootstrap_succeeds_when_a_postcommit_observer_would_see_changed_app_state() {
    let (runtime, db, _dir) = support::ensured_database().await;
    let organization_id = "postcommit-app-change-org";
    prepare_ready_atomic_command(&db, organization_id).await;
    // This mutation is part of the committing transaction. It models a
    // concurrent observer seeing the application change immediately after
    // admission; bootstrap must still report its committed success.
    runtime
        .batch(
            "CREATE TRIGGER mutate_app_after_bootstrap_binding AFTER INSERT ON role_bindings
             WHEN NEW.role_id = 'role-system-bootstrap-owner'
             BEGIN UPDATE applications SET active=0 WHERE id='atomic-exposure'; END;",
        )
        .await
        .expect("install postcommit-observer trigger");

    let result = db
        .bootstrap_external_owner_atomic(ExternalOwnerBootstrapCommand {
            identity: ExternalIdentityMaterialization {
                provider: "patch-data",
                external_user_id: "postcommit-subject",
                email: Some("postcommit@example.test"),
                display_name: "Postcommit Owner",
            },
            organization_id,
            application_id: "atomic-exposure",
        })
        .await
        .expect("committed bootstrap must not become a false failure");
    assert_eq!(
        membership_role(&runtime, organization_id, &result.user_id).await,
        "owner"
    );
    let app_active: Vec<support::NameRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT CAST(active AS TEXT) AS name FROM applications WHERE id='atomic-exposure'",
            vec![],
        )
        .await
        .expect("application state after commit");
    assert_eq!(app_active[0].name, "0");
}

#[tokio::test]
async fn concurrent_distinct_first_owner_commands_admit_exactly_one_owner() {
    let (runtime, db, _dir) = support::ensured_database().await;
    let organization_id = "concurrent-owner-org";
    prepare_ready_atomic_command(&db, organization_id).await;
    let barrier = Arc::new(Barrier::new(2));
    let first_db = db.clone();
    let first_barrier = barrier.clone();
    let first = tokio::spawn(async move {
        first_barrier.wait().await;
        first_db
            .bootstrap_external_owner_atomic(ExternalOwnerBootstrapCommand {
                identity: ExternalIdentityMaterialization {
                    provider: "patch-data",
                    external_user_id: "first-owner-a",
                    email: Some("first-owner-a@example.test"),
                    display_name: "First Owner A",
                },
                organization_id,
                application_id: "atomic-exposure",
            })
            .await
    });
    let second_db = db.clone();
    let second = tokio::spawn(async move {
        barrier.wait().await;
        second_db
            .bootstrap_external_owner_atomic(ExternalOwnerBootstrapCommand {
                identity: ExternalIdentityMaterialization {
                    provider: "patch-data",
                    external_user_id: "first-owner-b",
                    email: Some("first-owner-b@example.test"),
                    display_name: "First Owner B",
                },
                organization_id,
                application_id: "atomic-exposure",
            })
            .await
    });
    let results = [
        first.await.expect("first task"),
        second.await.expect("second task"),
    ];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);
    let owners: Vec<support::NameRow> = runtime.query_rows(
        ReadConsistency::Strong,
        "SELECT user_id AS name FROM organization_memberships WHERE organization_id=$1 AND membership_role='owner' AND status='active'",
        vec![organization_id.into()],
    ).await.expect("single owner");
    let bindings: Vec<support::NameRow> = runtime.query_rows(
        ReadConsistency::Strong,
        "SELECT subject_id AS name FROM role_bindings WHERE subject_organization_id=$1 AND role_id='role-system-bootstrap-owner' AND state='active'",
        vec![organization_id.into()],
    ).await.expect("single owner binding");
    assert_eq!(owners.len(), 1);
    assert_eq!(bindings.len(), 1);
    assert_eq!(owners[0].name, bindings[0].name);
}

#[tokio::test]
async fn atomic_external_owner_bootstrap_identical_retry_preserves_complete_fingerprint() {
    let (runtime, db, _dir) = support::ensured_database().await;
    let organization_id = "atomic-retry-org";
    prepare_ready_atomic_command(&db, organization_id).await;
    let command = || ExternalOwnerBootstrapCommand {
        identity: ExternalIdentityMaterialization {
            provider: "patch-data",
            external_user_id: "atomic-subject",
            email: Some("atomic@example.test"),
            display_name: "Atomic Owner",
        },
        organization_id,
        application_id: "atomic-exposure",
    };
    let first = db
        .bootstrap_external_owner_atomic(command())
        .await
        .expect("first");
    let before_retry = atomic_fingerprint(&runtime, organization_id).await;
    let second = db
        .bootstrap_external_owner_atomic(command())
        .await
        .expect("retry");
    assert_eq!(second, first);
    assert_eq!(
        atomic_fingerprint(&runtime, organization_id).await,
        before_retry,
        "idempotent retry must not touch timestamps"
    );
}

#[tokio::test]
async fn materialization_never_merges_provider_email_or_bare_subject_collisions() {
    let (runtime, db, _dir) = support::ensured_database().await;
    for (external_user_id, email, seed_id) in [
        ("email-collision", "shared@example.test", "local-email-user"),
        (
            "bare-id-collision",
            "other@example.test",
            "bare-id-collision",
        ),
    ] {
        runtime.execute(
            "INSERT INTO users (id,email,display_name,active,created_at) VALUES ($1,$2,'Existing User',1,$3)",
            vec![seed_id.into(), email.into(), support::FIXED_TEST_TIME.into()],
        ).await.expect("collision user");
        let before = atomic_fingerprint(&runtime, "unrelated-org").await;
        let result = db
            .materialize_external_identity(ExternalIdentityMaterialization {
                provider: "patch-data",
                external_user_id,
                email: Some(email),
                display_name: "Patch Data User",
            })
            .await;
        let error = result.expect_err("collision must not merge with local user");
        assert_eq!(
            error.downcast_ref::<ExternalOwnerBootstrapError>(),
            Some(&ExternalOwnerBootstrapError::PreconditionsChanged)
        );
        assert_eq!(
            atomic_fingerprint(&runtime, "unrelated-org").await,
            before,
            "{external_user_id} collision must write nothing"
        );
    }
}

#[tokio::test]
async fn bootstrap_external_owner_rejects_existing_member_admin_and_other_owner() {
    for (role, expected) in [
        ("member", ExternalOwnerBootstrapError::ExistingMemberRole),
        ("admin", ExternalOwnerBootstrapError::ExistingAdminRole),
    ] {
        let (runtime, db, _dir) = support::ensured_database().await;
        let (user_id, organization_id) = seeded_user_and_organization(&db).await;
        runtime
            .execute(
                "INSERT INTO organization_memberships (
                    id, organization_id, user_id, membership_role, status, created_at, updated_at
                 ) VALUES ($1, $2, $3, $4, 'active', $5, $5)",
                vec![
                    format!("{role}-membership").into(),
                    organization_id.clone().into(),
                    user_id.clone().into(),
                    role.into(),
                    support::FIXED_TEST_TIME.into(),
                ],
            )
            .await
            .expect("seed conflicting membership");

        let error = db
            .bootstrap_external_owner(&organization_id, &user_id)
            .await
            .expect_err("conflicting membership must be rejected");
        assert_eq!(
            error.downcast_ref::<ExternalOwnerBootstrapError>(),
            Some(&expected)
        );
        assert_eq!(
            membership_role(&runtime, &organization_id, &user_id).await,
            role
        );
        assert_eq!(
            role_binding_count(&runtime, &organization_id, &user_id).await,
            0
        );
    }

    let (runtime, db, _dir) = support::ensured_database().await;
    let (user_id, organization_id) = seeded_user_and_organization(&db).await;
    let other = db
        .resolve_or_create_user_identity("patch-data", "existing-owner", None, "Existing Owner")
        .await
        .expect("seed existing owner");
    runtime
        .execute(
            "INSERT INTO organization_memberships (
                id, organization_id, user_id, membership_role, status, created_at, updated_at
             ) VALUES ($1, $2, $3, 'owner', 'active', $4, $4)",
            vec![
                "other-owner-membership".into(),
                organization_id.clone().into(),
                other.id.clone().into(),
                support::FIXED_TEST_TIME.into(),
            ],
        )
        .await
        .expect("seed existing owner");

    let error = db
        .bootstrap_external_owner(&organization_id, &user_id)
        .await
        .expect_err("different owner must be rejected");
    assert_eq!(
        error.downcast_ref::<ExternalOwnerBootstrapError>(),
        Some(&ExternalOwnerBootstrapError::ExistingOwner { user_id: other.id })
    );
    assert_eq!(
        role_binding_count(&runtime, &organization_id, &user_id).await,
        0
    );
}

#[tokio::test]
async fn bootstrap_external_owner_with_unready_exposure_application_makes_no_local_writes() {
    let (service, _db, runtime, _dir) = patch_data_service().await;
    let before: Vec<support::NameRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT id AS name FROM users WHERE id = 'patch-data:external-owner'
             UNION ALL SELECT id AS name FROM organizations WHERE id = 'external-organization'
             UNION ALL SELECT id AS name FROM organization_memberships WHERE organization_id = 'external-organization'
             UNION ALL SELECT id AS name FROM role_bindings WHERE subject_organization_id = 'external-organization'",
            vec![],
        )
        .await
        .expect("pre-bootstrap local rows");

    let error = service
        .bootstrap_external_owner(Some("Bearer external-token"))
        .await
        .expect_err("unready exposure application must reject bootstrap");
    assert_eq!(error.to_string(), "external owner bootstrap conflict");
    assert!(!error.to_string().contains("application"));

    let after: Vec<support::NameRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT id AS name FROM users WHERE id = 'patch-data:external-owner'
             UNION ALL SELECT id AS name FROM organizations WHERE id = 'external-organization'
             UNION ALL SELECT id AS name FROM organization_memberships WHERE organization_id = 'external-organization'
             UNION ALL SELECT id AS name FROM role_bindings WHERE subject_organization_id = 'external-organization'",
            vec![],
        )
        .await
        .expect("post-bootstrap local rows");
    assert_eq!(
        after.len(),
        before.len(),
        "invalid readiness must write nothing"
    );
}

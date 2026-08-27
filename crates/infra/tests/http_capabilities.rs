mod support;

use domain::{
    Application, ApplicationAppType, ApplicationConsentPolicy, ApplicationProvisioningStatus,
    ApplicationType, ServiceProvider,
};
use infra::database::{
    ActivateHttpCapabilityGeneration, HttpCapability, HttpCapabilityAccessMode,
    HttpCapabilityResource, HttpQueryPolicy, HttpRedirectPolicy, HttpRequestBodyMode,
    HttpRequestPolicy, HttpResourceExtractor, HttpResponsePolicy, ProviderPepClientLifecycle,
    StageHttpCapabilityGeneration, UpsertProviderPepClient, http_capability_catalog_digest,
    http_capability_contract_digest,
};

const PROVIDER: &str = "patch-data";
const APP: &str = "exposure-app";
const PEP: &str = "pep-exposure";

async fn database() -> (
    infra::runtime::HiqliteRuntime,
    infra::database::Database,
    tempfile::TempDir,
) {
    let (runtime, dir) = support::test_runtime().await;
    let db = infra::database::Database::new(
        runtime.clone(),
        "bootstrap-user".to_owned(),
        "Bootstrap User".to_owned(),
    );
    db.ensure_schema().await.expect("ensure schema");
    db.upsert_service_provider(&ServiceProvider {
        id: PROVIDER.to_owned(),
        name: "Patch Data".to_owned(),
        description: None,
        active: true,
    })
    .await
    .expect("insert provider");
    db.insert_application(&Application {
        id: APP.to_owned(),
        organization_id: "bootstrap-user".to_owned(),
        slug: APP.to_owned(),
        name: "Exposure".to_owned(),
        description: None,
        application_type: ApplicationType::Confidential,
        app_type: ApplicationAppType::Server,
        first_party: true,
        consent_policy: ApplicationConsentPolicy::AutoApprove,
        active: true,
        provisioning_status: ApplicationProvisioningStatus::Ready,
        rauthy_client_id: None,
        created_by_user_id: "bootstrap-user".to_owned(),
    })
    .await
    .expect("insert PEP application");
    (runtime, db, dir)
}

fn capability(id: &str, path: &str) -> HttpCapability {
    let mut value = HttpCapability {
        capability_id: id.to_owned(),
        version: 1,
        enabled: true,
        access_mode: HttpCapabilityAccessMode::Authorized,
        method: "GET".to_owned(),
        path_template: path.to_owned(),
        action_id: Some("patch-data.plant.read".to_owned()),
        resource: Some(HttpCapabilityResource {
            resource_type: "plant".to_owned(),
            id: HttpResourceExtractor::PathSegment {
                name: "plantId".to_owned(),
                format: "pocketbaseId".to_owned(),
            },
        }),
        query: HttpQueryPolicy::None {},
        request: HttpRequestPolicy {
            body: HttpRequestBodyMode::None,
            max_bytes: 0,
            forward_headers: vec![],
        },
        response: HttpResponsePolicy {
            max_bytes: 4_194_304,
            forward_headers: vec![],
            redirects: HttpRedirectPolicy::Deny,
        },
        contract_digest: format!("sha256:{}", "0".repeat(64)),
    };
    value.contract_digest = http_capability_contract_digest(&value).expect("contract digest");
    value
}

fn generation(
    generation: u64,
    expected_latest_generation: Option<u64>,
    capability: HttpCapability,
) -> StageHttpCapabilityGeneration {
    let mut value = StageHttpCapabilityGeneration {
        schema_version: 1,
        generation,
        expected_latest_generation,
        capabilities: vec![capability],
        catalog_digest: format!("sha256:{}", "0".repeat(64)),
    };
    value.catalog_digest =
        http_capability_catalog_digest(PROVIDER, &value).expect("catalog digest");
    value
}

async fn bind_pep(db: &infra::database::Database) {
    db.upsert_provider_pep_client(
        PROVIDER,
        PEP,
        UpsertProviderPepClient {
            application_id: APP.to_owned(),
            lifecycle: ProviderPepClientLifecycle::Active,
            valid_until: None,
        },
        "test",
    )
    .await
    .expect("bind PEP");
}

#[tokio::test]
async fn generations_are_immutable_idempotent_and_active_pointer_is_cas_versioned() {
    let (_runtime, db, _dir) = database().await;
    bind_pep(&db).await;

    let first = generation(
        1,
        None,
        capability("plant.get", "/api/collections/plants/records/{plantId}"),
    );
    let staged = db
        .stage_http_capability_generation(PROVIDER, first.clone())
        .await
        .expect("stage generation 1");
    assert!(staged.staged);
    assert!(
        !db.stage_http_capability_generation(PROVIDER, first.clone())
            .await
            .expect("exact retry")
            .staged
    );

    let conflicting = generation(
        1,
        None,
        capability(
            "plant.get.changed",
            "/api/collections/plants/records/{plantId}",
        ),
    );
    assert!(
        db.stage_http_capability_generation(PROVIDER, conflicting)
            .await
            .expect_err("same generation cannot change")
            .to_string()
            .contains("conflict")
    );

    let activate = || {
        db.activate_http_capability_generation(
            PROVIDER,
            ActivateHttpCapabilityGeneration {
                expected_generation: None,
                expected_catalog_digest: None,
                target_generation: 1,
                target_catalog_digest: first.catalog_digest.clone(),
            },
            "test",
        )
    };
    let (left, right) = tokio::join!(activate(), activate());
    assert!(
        left.is_ok() ^ right.is_ok(),
        "exactly one CAS activation wins"
    );
    let first_activation = left.or(right).expect("one activation receipt");
    assert_eq!(first_activation.activation_sequence, 1);
    assert_eq!(first_activation.from_generation, None);
    assert_eq!(first_activation.activation_receipt_id.len(), 32);

    let second = generation(
        2,
        Some(1),
        capability("plant.get.v2", "/api/v2/plants/{plantId}"),
    );
    db.stage_http_capability_generation(PROVIDER, second.clone())
        .await
        .expect("stage generation 2");
    assert!(
        db.activate_http_capability_generation(
            PROVIDER,
            ActivateHttpCapabilityGeneration {
                expected_generation: Some(1),
                expected_catalog_digest: Some(format!("sha256:{}", "f".repeat(64))),
                target_generation: 2,
                target_catalog_digest: second.catalog_digest.clone(),
            },
            "test",
        )
        .await
        .is_err(),
        "wrong active digest must fail closed"
    );
    let second_activation = db
        .activate_http_capability_generation(
            PROVIDER,
            ActivateHttpCapabilityGeneration {
                expected_generation: Some(1),
                expected_catalog_digest: Some(first.catalog_digest.clone()),
                target_generation: 2,
                target_catalog_digest: second.catalog_digest.clone(),
            },
            "test",
        )
        .await
        .expect("activate generation 2");
    assert_eq!(second_activation.activation_sequence, 2);
    assert_eq!(second_activation.from_generation, Some(1));

    let catalog = db
        .active_http_capability_catalog(PROVIDER, PEP)
        .await
        .expect("active catalog");
    assert_eq!(catalog.generation, 2);
    assert_eq!(catalog.capabilities[0].capability_id, "plant.get.v2");

    db.ensure_schema_only()
        .await
        .expect("reapplying the migration is idempotent");
    let catalog_after_migration_retry = db
        .active_http_capability_catalog(PROVIDER, PEP)
        .await
        .expect("catalog survives migration retry");
    assert_eq!(
        catalog_after_migration_retry.provider_id,
        catalog.provider_id
    );
    assert_eq!(
        catalog_after_migration_retry.pep_client_id,
        catalog.pep_client_id
    );
    assert_eq!(catalog_after_migration_retry.generation, catalog.generation);
    assert_eq!(
        catalog_after_migration_retry.catalog_digest,
        catalog.catalog_digest
    );
    assert_eq!(
        catalog_after_migration_retry.capabilities,
        catalog.capabilities
    );

    let rollback = db
        .activate_http_capability_generation(
            PROVIDER,
            ActivateHttpCapabilityGeneration {
                expected_generation: Some(2),
                expected_catalog_digest: Some(second.catalog_digest),
                target_generation: 1,
                target_catalog_digest: first.catalog_digest,
            },
            "test",
        )
        .await
        .expect("rollback to immutable generation 1");
    assert_eq!(rollback.activation_sequence, 3);
    assert_eq!(rollback.from_generation, Some(2));
}

#[tokio::test]
async fn pep_application_binding_is_global_and_disabled_binding_cannot_read_catalog() {
    let (_runtime, db, _dir) = database().await;
    bind_pep(&db).await;
    db.upsert_service_provider(&ServiceProvider {
        id: "other-provider".to_owned(),
        name: "Other Provider".to_owned(),
        description: None,
        active: true,
    })
    .await
    .expect("other provider");
    assert!(
        db.upsert_provider_pep_client(
            "other-provider",
            "other-pep",
            UpsertProviderPepClient {
                application_id: APP.to_owned(),
                lifecycle: ProviderPepClientLifecycle::Active,
                valid_until: None,
            },
            "test",
        )
        .await
        .is_err(),
        "one application cannot become a cross-provider PEP"
    );

    let first = generation(
        1,
        None,
        capability("plant.get", "/api/collections/plants/records/{plantId}"),
    );
    db.stage_http_capability_generation(PROVIDER, first.clone())
        .await
        .expect("stage");
    db.activate_http_capability_generation(
        PROVIDER,
        ActivateHttpCapabilityGeneration {
            expected_generation: None,
            expected_catalog_digest: None,
            target_generation: 1,
            target_catalog_digest: first.catalog_digest,
        },
        "test",
    )
    .await
    .expect("activate");
    db.upsert_provider_pep_client(
        PROVIDER,
        PEP,
        UpsertProviderPepClient {
            application_id: APP.to_owned(),
            lifecycle: ProviderPepClientLifecycle::Disabled,
            valid_until: None,
        },
        "test",
    )
    .await
    .expect("disable PEP");
    assert!(
        db.active_http_capability_catalog(PROVIDER, PEP)
            .await
            .is_err()
    );
}

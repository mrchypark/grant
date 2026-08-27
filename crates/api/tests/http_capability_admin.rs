mod support;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
};
use domain::{
    Action, ActionRiskLevel, Application, ApplicationAppType, ApplicationConsentPolicy,
    ApplicationProvisioningStatus, ApplicationType, Role, RoleBinding, RoleBindingState,
    RoleBindingSubjectType, ServiceProvider,
};
use infra::database::{
    ActivateHttpCapabilityGeneration, HttpCapability, HttpCapabilityAccessMode,
    HttpCapabilityResource, HttpQueryPolicy, HttpRedirectPolicy, HttpRequestBodyMode,
    HttpRequestPolicy, HttpResourceExtractor, HttpResponsePolicy, ProviderPepClientLifecycle,
    StageHttpCapabilityGeneration, UpsertProviderPepClient, http_capability_catalog_digest,
    http_capability_contract_digest,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

const PROVIDER: &str = "patch-data";
const APPLICATION: &str = "exposure-app";
const PEP: &str = "pep-exposure";
const PLANT: &str = "abcdefghijklmn1";

fn sha256(value: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(value.as_bytes()))
}

fn credential_digest(value: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"grant.http-capability.provider-credential.v1");
    digest.update((value.len() as u64).to_be_bytes());
    digest.update(value.as_bytes());
    format!("sha256:{:x}", digest.finalize())
}

fn authorized_capability() -> HttpCapability {
    let mut value = HttpCapability {
        capability_id: "plant.get".to_owned(),
        version: 1,
        enabled: true,
        access_mode: HttpCapabilityAccessMode::Authorized,
        method: "GET".to_owned(),
        path_template: "/api/collections/plants/records/{plantId}".to_owned(),
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

fn public_authn_capability() -> HttpCapability {
    let mut value = HttpCapability {
        capability_id: "auth.refresh".to_owned(),
        version: 1,
        enabled: true,
        access_mode: HttpCapabilityAccessMode::PublicAuthn,
        method: "POST".to_owned(),
        path_template: "/api/collections/users/auth-refresh".to_owned(),
        action_id: None,
        resource: None,
        query: HttpQueryPolicy::None {},
        request: HttpRequestPolicy {
            body: HttpRequestBodyMode::Json,
            max_bytes: 65_536,
            forward_headers: vec!["content-type".to_owned()],
        },
        response: HttpResponsePolicy {
            max_bytes: 1_048_576,
            forward_headers: vec![],
            redirects: HttpRedirectPolicy::Deny,
        },
        contract_digest: format!("sha256:{}", "0".repeat(64)),
    };
    value.contract_digest = http_capability_contract_digest(&value).expect("contract digest");
    value
}

fn generation() -> StageHttpCapabilityGeneration {
    let mut value = StageHttpCapabilityGeneration {
        schema_version: 1,
        generation: 1,
        expected_latest_generation: None,
        capabilities: vec![authorized_capability(), public_authn_capability()],
        catalog_digest: format!("sha256:{}", "0".repeat(64)),
    };
    value.catalog_digest =
        http_capability_catalog_digest(PROVIDER, &value).expect("catalog digest");
    value
}

async fn decision_fixture() -> (support::TestApi, StageHttpCapabilityGeneration, String) {
    let test = support::TestApi::with_http_capability_pep(APPLICATION).await;
    let user = support::seed_identity(
        &test.db,
        "bootstrap-user",
        Some("bootstrap-user@rauthy.test"),
        "Bootstrap User",
    )
    .await;
    test.db
        .upsert_service_provider(&ServiceProvider {
            id: PROVIDER.to_owned(),
            name: "Patch Data".to_owned(),
            description: None,
            active: true,
        })
        .await
        .expect("provider");
    test.db
        .upsert_action(&Action {
            id: "patch-data.plant.read".to_owned(),
            provider_id: PROVIDER.to_owned(),
            code: "patch-data.plant.read".to_owned(),
            name: "Read plant".to_owned(),
            description: None,
            risk_level: ActionRiskLevel::Low,
            visibility_granting: false,
            grantable_to_applications: true,
            consent_label: None,
            consent_description: None,
        })
        .await
        .expect("action");
    test.db
        .insert_application(&Application {
            id: APPLICATION.to_owned(),
            organization_id: "bootstrap-user".to_owned(),
            slug: APPLICATION.to_owned(),
            name: "Exposure".to_owned(),
            description: None,
            application_type: ApplicationType::Confidential,
            app_type: ApplicationAppType::Server,
            first_party: false,
            consent_policy: ApplicationConsentPolicy::Explicit,
            active: true,
            provisioning_status: ApplicationProvisioningStatus::Ready,
            rauthy_client_id: Some(APPLICATION.to_owned()),
            created_by_user_id: user.id.clone(),
        })
        .await
        .expect("application");
    test.db
        .replace_application_actions(APPLICATION, &["patch-data.plant.read".to_owned()])
        .await
        .expect("application action");
    test.db
        .set_application_trust("bootstrap-user", APPLICATION, true)
        .await
        .expect("trust application");
    test.db
        .upsert_role(&Role {
            id: "role-plant-reader".to_owned(),
            organization_id: Some("bootstrap-user".to_owned()),
            code: "plant.reader".to_owned(),
            name: "Plant Reader".to_owned(),
            description: None,
        })
        .await
        .expect("role");
    test.db
        .upsert_role_action("role-plant-reader", "patch-data.plant.read")
        .await
        .expect("role action");
    test.db
        .save_role_binding(&RoleBinding {
            id: "binding-plant-reader".to_owned(),
            subject_type: RoleBindingSubjectType::User,
            subject_id: user.id,
            subject_organization_id: "bootstrap-user".to_owned(),
            provider_id: PROVIDER.to_owned(),
            resource_type: "plant".to_owned(),
            resource_id: PLANT.to_owned(),
            role_id: "role-plant-reader".to_owned(),
            agreement_id: None,
            state: RoleBindingState::Active,
            created_by_user_id: "bootstrap-user".to_owned(),
            created_at: support::FIXED_TEST_TIME.to_owned(),
            updated_at: support::FIXED_TEST_TIME.to_owned(),
        })
        .await
        .expect("binding");
    test.db
        .upsert_provider_pep_client(
            PROVIDER,
            PEP,
            UpsertProviderPepClient {
                application_id: APPLICATION.to_owned(),
                lifecycle: ProviderPepClientLifecycle::Active,
                valid_until: None,
            },
            "test",
        )
        .await
        .expect("PEP binding");
    let generation = generation();
    test.db
        .stage_http_capability_generation(PROVIDER, generation.clone())
        .await
        .expect("stage catalog");
    test.db
        .activate_http_capability_generation(
            PROVIDER,
            ActivateHttpCapabilityGeneration {
                expected_generation: None,
                expected_catalog_digest: None,
                target_generation: 1,
                target_catalog_digest: generation.catalog_digest.clone(),
            },
            "test",
        )
        .await
        .expect("activate catalog");
    let token = format!("app-token/{APPLICATION}/bootstrap-user");
    (test, generation, token)
}

fn decision_body(generation: &StageHttpCapabilityGeneration, token: &str) -> Value {
    let capability = &generation.capabilities[0];
    json!({
        "providerId": PROVIDER,
        "pepClientId": PEP,
        "catalogGeneration": 1,
        "catalogDigest": generation.catalog_digest,
        "capabilityId": capability.capability_id,
        "capabilityVersion": capability.version,
        "contractDigest": capability.contract_digest,
        "method": "GET",
        "canonicalPath": format!("/api/collections/plants/records/{PLANT}"),
        "canonicalQuery": "",
        "canonicalQueryDigest": sha256(""),
        "forwardedHeaderDigest": sha256(""),
        "bodyDigest": sha256(""),
        "bodyLength": 0,
        "providerCredentialDigest": credential_digest(&support::auth::bearer_header(token)),
        "providerCredential": {"subject":"bootstrap-user","audience":APPLICATION},
        "resourceEvidence": {"kind":"pathSegment","name":"plantId","canonicalValue":PLANT},
        "resourceType": "plant",
        "resourceId": PLANT,
        "requestBinding": "abcdefghijklmnopqrstuv"
    })
}

fn decision_request(token: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri("/api/v2/http-capabilities/decide")
        .header("content-type", "application/json")
        .header("authorization", support::auth::bearer_header(token))
        .header("x-organization-id", "bootstrap-user")
        .header("x-grant-app-auth-key", support::APP_AUTH_KEY)
        .body(Body::from(body.to_string()))
        .expect("decision request")
}

#[tokio::test]
async fn capability_admin_routes_are_master_only() {
    let test = support::TestApi::new().await;
    for (method, path, body) in [
        (
            Method::POST,
            "/api/v1/admin/providers/patch-data/http-capability-generations",
            r#"{"schemaVersion":1,"generation":1,"expectedLatestGeneration":null,"capabilities":[],"catalogDigest":"sha256:0000000000000000000000000000000000000000000000000000000000000000"}"#,
        ),
        (
            Method::POST,
            "/api/v1/admin/providers/patch-data/http-capability-active",
            r#"{"expectedGeneration":null,"expectedCatalogDigest":null,"targetGeneration":1,"targetCatalogDigest":"sha256:0000000000000000000000000000000000000000000000000000000000000000"}"#,
        ),
        (
            Method::PUT,
            "/api/v1/admin/providers/patch-data/pep-clients/pep-exposure",
            r#"{"applicationId":"exposure","lifecycle":"active","validUntil":null}"#,
        ),
    ] {
        let response = test
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}

#[tokio::test]
async fn capability_catalog_route_requires_the_app_credential() {
    let test = support::TestApi::new().await;
    let catalog = test
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/v2/http-capabilities/catalog/patch-data")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(catalog.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn string_template_resource_extractors_are_rejected_before_staging() {
    let test = support::TestApi::new().await;
    let mut body = serde_json::to_value(generation()).expect("serialize generation");
    body["capabilities"][0]["resource"]["id"] = json!({
        "kind": "stringTemplate",
        "source": "query",
        "template": "plant:{plantId}",
        "slots": [{"name": "plantId", "format": "pocketbaseId"}]
    });

    let response = test
        .app
        .clone()
        .oneshot(support::master_json_request(
            Method::POST,
            "/api/v1/admin/providers/patch-data/http-capability-generations",
            body,
        ))
        .await
        .expect("stage response");

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn capability_semantic_and_precondition_failures_are_deterministic_client_errors() {
    let test = support::TestApi::new().await;
    test.db
        .upsert_service_provider(&ServiceProvider {
            id: PROVIDER.to_owned(),
            name: "Patch Data".to_owned(),
            description: None,
            active: true,
        })
        .await
        .expect("provider");

    let mut invalid_generation = serde_json::to_value(generation()).expect("generation JSON");
    invalid_generation["schemaVersion"] = json!(2);
    invalid_generation["capabilities"] = json!([]);
    let invalid = test
        .app
        .clone()
        .oneshot(support::master_json_request(
            Method::POST,
            "/api/v1/admin/providers/patch-data/http-capability-generations",
            invalid_generation,
        ))
        .await
        .expect("invalid generation response");
    assert_eq!(invalid.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let invalid_expiry = test
        .app
        .clone()
        .oneshot(support::master_json_request(
            Method::PUT,
            "/api/v1/admin/providers/patch-data/pep-clients/pep-exposure",
            json!({
                "applicationId": APPLICATION,
                "lifecycle": "active",
                "validUntil": "not-a-timestamp"
            }),
        ))
        .await
        .expect("invalid expiry response");
    assert_eq!(invalid_expiry.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let precondition = test
        .app
        .clone()
        .oneshot(support::master_json_request(
            Method::POST,
            "/api/v1/admin/providers/patch-data/http-capability-active",
            json!({
                "expectedGeneration": null,
                "expectedCatalogDigest": null,
                "targetGeneration": 1,
                "targetCatalogDigest": format!("sha256:{}", "0".repeat(64))
            }),
        ))
        .await
        .expect("activation precondition response");
    assert_eq!(precondition.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn active_catalog_and_authorized_decision_bind_every_request_dimension() {
    let (test, generation, token) = decision_fixture().await;
    let catalog = test
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/api/v2/http-capabilities/catalog/{PROVIDER}"))
                .header("x-grant-app-auth-key", support::APP_AUTH_KEY)
                .body(Body::empty())
                .expect("catalog request"),
        )
        .await
        .expect("catalog response");
    assert_eq!(catalog.status(), StatusCode::OK);
    let catalog = support::response_json(catalog).await;
    assert_eq!(catalog["pepClientId"], PEP);
    assert_eq!(catalog["generation"], 1);
    assert_eq!(catalog["capabilities"].as_array().map(Vec::len), Some(2));

    let body = decision_body(&generation, &token);
    let allowed = test
        .app
        .clone()
        .oneshot(decision_request(&token, body.clone()))
        .await
        .expect("decision response");
    let allowed_status = allowed.status();
    let allowed = support::response_json(allowed).await;
    assert_eq!(allowed_status, StatusCode::OK, "decision body: {allowed}");
    assert_eq!(allowed["allowed"], true);
    assert_eq!(allowed["providerId"], PROVIDER);
    assert_eq!(allowed["resourceId"], PLANT);
    assert_eq!(allowed["method"], "GET");

    for (field, value) in [
        ("method", json!("POST")),
        (
            "canonicalPath",
            json!("/api/collections/plants/records/zzzzzzzzzzzzzzz"),
        ),
        ("canonicalQuery", json!("x=1")),
        ("resourceId", json!("zzzzzzzzzzzzzzz")),
        ("providerCredentialDigest", json!(sha256("wrong"))),
        ("pepClientId", json!("other-pep")),
        ("providerId", json!("other-provider")),
    ] {
        let mut tampered = body.clone();
        tampered[field] = value;
        let response = test
            .app
            .clone()
            .oneshot(decision_request(&token, tampered))
            .await
            .expect("tampered decision response");
        assert!(
            matches!(
                response.status(),
                StatusCode::BAD_REQUEST | StatusCode::FORBIDDEN
            ),
            "{field} tamper unexpectedly returned {}",
            response.status()
        );
    }

    let mut forbidden_query = body.clone();
    forbidden_query["canonicalQuery"] = json!("filter=plant");
    forbidden_query["canonicalQueryDigest"] = json!(sha256("filter=plant"));
    let response = test
        .app
        .clone()
        .oneshot(decision_request(&token, forbidden_query))
        .await
        .expect("recomputed query digest response");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn public_authn_capability_cannot_be_used_as_an_authorization_decision() {
    let (test, generation, token) = decision_fixture().await;
    let mut body = decision_body(&generation, &token);
    let capability = &generation.capabilities[1];
    body["capabilityId"] = json!(capability.capability_id);
    body["contractDigest"] = json!(capability.contract_digest);
    body["method"] = json!("POST");
    body["canonicalPath"] = json!(capability.path_template);
    let response = test
        .app
        .clone()
        .oneshot(decision_request(&token, body))
        .await
        .expect("public authn decision response");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn inactive_provider_blocks_catalog_including_public_authn_and_all_decisions() {
    let (test, generation, token) = decision_fixture().await;
    test.db
        .upsert_service_provider(&ServiceProvider {
            id: PROVIDER.to_owned(),
            name: "Patch Data".to_owned(),
            description: None,
            active: false,
        })
        .await
        .expect("deactivate provider");

    let catalog = test
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/api/v2/http-capabilities/catalog/{PROVIDER}"))
                .header("x-grant-app-auth-key", support::APP_AUTH_KEY)
                .body(Body::empty())
                .expect("catalog request"),
        )
        .await
        .expect("catalog response");
    assert_eq!(catalog.status(), StatusCode::FORBIDDEN);

    let decision = test
        .app
        .clone()
        .oneshot(decision_request(&token, decision_body(&generation, &token)))
        .await
        .expect("decision response");
    assert_eq!(decision.status(), StatusCode::FORBIDDEN);
}

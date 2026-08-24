mod support;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
};
use domain::{
    Agreement, AgreementStatus, AgreementType, Application, ApplicationAppType,
    ApplicationConsentPolicy, ApplicationProvisioningStatus, ApplicationType, RoleBinding,
    RoleBindingState, RoleBindingSubjectType,
};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use tower::ServiceExt;

const EXPOSURE_ID: &str = "customer-profile";
const RESOURCE_ID: &str = "customer-123";
const APPLICATION_ID: &str = "exposure-app";

fn digest(character: char) -> String {
    format!(
        "sha256:{}",
        std::iter::repeat_n(character, 64).collect::<String>()
    )
}

fn exposure_request(body: Value) -> Request<Body> {
    exposure_request_with_token(body, &format!("app-token/{APPLICATION_ID}/bootstrap-user"))
}

fn exposure_request_with_token(body: Value, token: &str) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri("/api/v1/exposure-authz/read")
        .header("content-type", "application/json")
        .header("authorization", support::auth::bearer_header(token))
        .header("x-organization-id", "bootstrap-user")
        .header("x-grant-app-auth-key", support::APP_AUTH_KEY)
        .body(Body::from(body.to_string()))
        .expect("exposure read request")
}

fn plant_request(body: Value) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri("/api/v1/exposure-authz/plant-read")
        .header("content-type", "application/json")
        .header(
            "authorization",
            support::auth::bearer_header(&format!("app-token/{APPLICATION_ID}/bootstrap-user")),
        )
        .header("x-organization-id", "bootstrap-user")
        .header("x-grant-app-auth-key", support::APP_AUTH_KEY)
        .body(Body::from(body.to_string()))
        .expect("plant read request")
}

fn exact_body() -> Value {
    json!({
        "exposureId": EXPOSURE_ID,
        "exposureVersion": 7,
        "artifactDigest": digest('a'),
        "resourceId": RESOURCE_ID,
        "requestBinding": "request-7-customer-123"
    })
}

async fn unprovisioned_exposure_fixture() -> (support::TestApi, String) {
    let test = support::TestApi::new().await;
    let bootstrap = test
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/admin/bootstrap/rauthy-owner")
                .header(
                    "authorization",
                    support::auth::bearer_header("bootstrap-user"),
                )
                .header("x-grant-master-key", support::MASTER_KEY)
                .body(Body::empty())
                .expect("bootstrap owner request"),
        )
        .await
        .expect("bootstrap owner response");
    assert_eq!(bootstrap.status(), StatusCode::CREATED);
    // The application/Rauthy client readiness is the existing external IaC boundary;
    // Exposure policy/catalog/grant state below is provisioned only through HTTP APIs.
    test.db
        .insert_application(&Application {
            id: APPLICATION_ID.to_owned(),
            organization_id: "bootstrap-user".to_owned(),
            slug: APPLICATION_ID.to_owned(),
            name: "Exposure".to_owned(),
            description: None,
            application_type: ApplicationType::Confidential,
            app_type: ApplicationAppType::Server,
            first_party: false,
            consent_policy: ApplicationConsentPolicy::Explicit,
            active: true,
            provisioning_status: ApplicationProvisioningStatus::Ready,
            rauthy_client_id: Some(APPLICATION_ID.to_owned()),
            created_by_user_id: support::BOOTSTRAP_USER_ID.to_owned(),
        })
        .await
        .expect("exposure application");
    (test, support::BOOTSTRAP_USER_ID.to_owned())
}

async fn provision_exposure_access(test: &support::TestApi, subject_id: &str) {
    let app = test.app.clone();
    let provider = app
        .clone()
        .oneshot(support::master_json_request(
            Method::POST,
            "/api/v1/admin/providers",
            json!({
                "id": "exposure",
                "name": "Exposure",
                "description": "Immutable named data exposure provider",
                "active": true
            }),
        ))
        .await
        .expect("provider response");
    assert_eq!(provider.status(), StatusCode::CREATED);

    let actions = app
        .clone()
        .oneshot(support::master_json_request(
            Method::PUT,
            "/api/v1/admin/providers/exposure/actions",
            json!({
                "actions": [{
                    "id": "exposure.read",
                    "code": "exposure.read",
                    "name": "Read Exposure",
                    "description": null,
                    "riskLevel": "low",
                    "visibilityGranting": false,
                    "grantableToApplications": true,
                    "consentLabel": "Read Exposure",
                    "consentDescription": null
                }]
            }),
        ))
        .await
        .expect("provider actions response");
    assert_eq!(actions.status(), StatusCode::OK);

    let role = app
        .clone()
        .oneshot(support::json_request(
            Method::POST,
            "/api/v1/orgs/bootstrap-user/roles",
            "bootstrap-user",
            "bootstrap-user",
            json!({
                "id": "role-exposure-reader",
                "code": "exposure.reader",
                "name": "Exposure Reader",
                "description": null
            }),
        ))
        .await
        .expect("role response");
    assert_eq!(role.status(), StatusCode::CREATED);

    let role_actions = app
        .clone()
        .oneshot(support::json_request(
            Method::PUT,
            "/api/v1/orgs/bootstrap-user/roles/role-exposure-reader/actions",
            "bootstrap-user",
            "bootstrap-user",
            json!({ "actionIds": ["exposure.read"] }),
        ))
        .await
        .expect("role actions response");
    assert_eq!(role_actions.status(), StatusCode::OK);

    let binding = app
        .clone()
        .oneshot(support::json_request(
            Method::POST,
            "/api/v1/orgs/bootstrap-user/bindings",
            "bootstrap-user",
            "bootstrap-user",
            json!({
                "id": "binding-exposure-reader",
                "subjectType": "user",
                "subjectId": subject_id,
                "subjectOrganizationId": "bootstrap-user",
                "providerId": "exposure",
                "resourceType": EXPOSURE_ID,
                "resourceId": RESOURCE_ID,
                "roleId": "role-exposure-reader",
                "agreementId": null,
                "state": "active"
            }),
        ))
        .await
        .expect("binding response");
    assert_eq!(binding.status(), StatusCode::CREATED);

    let application_actions = app
        .clone()
        .oneshot(support::json_request(
            Method::PUT,
            "/api/v1/orgs/bootstrap-user/apps/exposure-app/actions",
            "bootstrap-user",
            "bootstrap-user",
            json!({ "actionIds": ["exposure.read"] }),
        ))
        .await
        .expect("application actions response");
    assert_eq!(application_actions.status(), StatusCode::OK);

    let trust = app
        .oneshot(support::master_json_request(
            Method::PUT,
            "/api/v1/admin/orgs/bootstrap-user/apps/exposure-app/trust",
            json!({ "trusted": true }),
        ))
        .await
        .expect("application trust response");
    assert_eq!(trust.status(), StatusCode::OK);
}

async fn exposure_fixture() -> (support::TestApi, String) {
    let (test, subject_id) = unprovisioned_exposure_fixture().await;
    provision_exposure_access(&test, &subject_id).await;
    (test, subject_id)
}

#[tokio::test]
async fn plant_read_is_a_closed_server_owned_operation_mapping() {
    let (test, _) = exposure_fixture().await;
    test.runtime.batch(
        "INSERT INTO service_providers (id,name,description,active,created_at,updated_at) VALUES ('patch-data','Patch Data',NULL,1,'2026-01-01','2026-01-01');
         INSERT INTO actions (id,provider_id,code,name,description,risk_level,visibility_granting,grantable_to_applications,created_at,updated_at) VALUES ('patch-data.plant.read','patch-data','patch-data.plant.read','Plant read',NULL,'low',0,1,'2026-01-01','2026-01-01');
         INSERT INTO roles (id,organization_id,code,name,description,created_at,updated_at) VALUES ('role-plant-reader',NULL,'role-plant-reader','Plant reader',NULL,'2026-01-01','2026-01-01');
         INSERT INTO role_actions (role_id,action_id) VALUES ('role-plant-reader','patch-data.plant.read');
         INSERT INTO role_bindings (id,subject_type,subject_id,subject_organization_id,provider_id,resource_type,resource_id,role_id,agreement_id,state,created_by_user_id,created_at,updated_at) VALUES ('binding-plant-reader','user','bootstrap-user','bootstrap-user','patch-data','plant','customer-123','role-plant-reader',NULL,'active','bootstrap-user','2026-01-01','2026-01-01');
         INSERT INTO application_actions (application_id,action_id) VALUES ('exposure-app','patch-data.plant.read');"
    ).await.expect("plant authority fixture");
    let mut body = exact_body();
    body["operationId"] = json!("plant.get");
    let response = test
        .app
        .clone()
        .oneshot(plant_request(body.clone()))
        .await
        .expect("plant response");
    assert_eq!(response.status(), StatusCode::OK);
    let response = support::response_json(response).await;
    assert_eq!(response["allowed"], true);
    assert_eq!(response["operationId"], "plant.get");
    assert_eq!(response["providerId"], "patch-data");
    assert_eq!(response["actionId"], "patch-data.plant.read");
    assert_eq!(response["resourceType"], "plant");
    assert!(response_matches_request(&response, &body));

    body["operationId"] = json!("plant.delete");
    let rejected = test
        .app
        .clone()
        .oneshot(plant_request(body.clone()))
        .await
        .expect("operation reject");
    assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);
    body["operationId"] = json!("plant.get");
    body["providerId"] = json!("patch-data");
    let selector = test
        .app
        .clone()
        .oneshot(plant_request(body))
        .await
        .expect("selector reject");
    assert_eq!(selector.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

fn response_matches_request(response: &Value, request: &Value) -> bool {
    [
        "exposureId",
        "exposureVersion",
        "artifactDigest",
        "resourceId",
        "requestBinding",
    ]
    .into_iter()
    .all(|key| response.get(key) == request.get(key))
}

#[tokio::test]
async fn exact_tuple_allows_and_response_is_replay_bound_without_field_output() {
    let (test, subject_id) = unprovisioned_exposure_fixture().await;
    let body = exact_body();
    let denied_before_provisioning = test
        .app
        .clone()
        .oneshot(exposure_request(body.clone()))
        .await
        .expect("pre-provisioning response");
    assert_eq!(denied_before_provisioning.status(), StatusCode::OK);
    let denied_before_provisioning = support::response_json(denied_before_provisioning).await;
    assert_eq!(denied_before_provisioning["allowed"], false);
    assert!(response_matches_request(&denied_before_provisioning, &body));

    provision_exposure_access(&test, &subject_id).await;
    let response = test
        .app
        .clone()
        .oneshot(exposure_request(body.clone()))
        .await
        .expect("exposure allow response");
    assert_eq!(response.status(), StatusCode::OK);
    let response = support::response_json(response).await;
    assert_eq!(response["allowed"], true, "response: {response}");
    assert!(response_matches_request(&response, &body));
    assert_eq!(response["principalDigest"].as_str().map(str::len), Some(64));
    assert!(
        response["principalDigest"]
            .as_str()
            .is_some_and(|value| value.bytes().all(|byte| byte.is_ascii_hexdigit()))
    );
    assert!(
        response["issuedAt"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );
    assert!(
        response["expiresAt"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );
    let keys = response
        .as_object()
        .expect("decision object")
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        keys,
        BTreeSet::from([
            "allowed",
            "artifactDigest",
            "expiresAt",
            "exposureId",
            "exposureVersion",
            "issuedAt",
            "principalDigest",
            "reason",
            "requestBinding",
            "resourceId",
        ])
    );
}

#[tokio::test]
async fn policy_uses_exposure_and_resource_only_while_decision_binds_the_full_tuple() {
    let (test, _) = exposure_fixture().await;

    for (key, changed) in [
        ("exposureId", json!("different-exposure")),
        ("resourceId", json!("different-resource")),
    ] {
        let mut body = exact_body();
        body[key] = changed;
        let response = test
            .app
            .clone()
            .oneshot(exposure_request(body.clone()))
            .await
            .expect("scope mismatch response");
        assert_eq!(response.status(), StatusCode::OK);
        let response = support::response_json(response).await;
        assert_eq!(response["allowed"], false, "response: {response}");
        assert!(response_matches_request(&response, &body));
    }

    let original = exact_body();
    for (key, changed) in [
        ("exposureVersion", json!(8)),
        ("artifactDigest", json!(digest('b'))),
        ("requestBinding", json!("different-request-binding")),
    ] {
        let mut changed_body = original.clone();
        changed_body[key] = changed;
        let response = test
            .app
            .clone()
            .oneshot(exposure_request(changed_body.clone()))
            .await
            .expect("changed replay binding response");
        assert_eq!(response.status(), StatusCode::OK);
        let response = support::response_json(response).await;
        assert_eq!(response["allowed"], true, "response: {response}");
        assert!(response_matches_request(&response, &changed_body));
        assert!(
            !response_matches_request(&response, &original),
            "changed decision must not validate against the original request"
        );
    }
}

#[tokio::test]
async fn malformed_versions_digests_ids_and_legacy_field_inputs_fail_closed() {
    let (test, _) = exposure_fixture().await;
    let mut bad_requests = Vec::new();

    let mut zero_version = exact_body();
    zero_version["exposureVersion"] = json!(0);
    bad_requests.push((zero_version, StatusCode::BAD_REQUEST));
    for version in [json!(-1), json!(1.5), json!("7")] {
        let mut body = exact_body();
        body["exposureVersion"] = version;
        bad_requests.push((body, StatusCode::UNPROCESSABLE_ENTITY));
    }
    for invalid_digest in [
        "a".repeat(64),
        format!("SHA256:{}", "a".repeat(64)),
        format!("sha512:{}", "a".repeat(64)),
        digest('A'),
        format!("sha256:{}", "a".repeat(63)),
        format!("sha256:{}", "a".repeat(65)),
        format!("sha256:{}g", "a".repeat(63)),
        format!(" {}", digest('a')),
        format!("{} ", digest('a')),
    ] {
        let mut body = exact_body();
        body["artifactDigest"] = json!(invalid_digest);
        bad_requests.push((body, StatusCode::BAD_REQUEST));
    }
    for (key, value) in [
        ("exposureId", " surrounded "),
        ("resourceId", ""),
        ("requestBinding", " request "),
    ] {
        let mut body = exact_body();
        body[key] = json!(value);
        bad_requests.push((body, StatusCode::BAD_REQUEST));
    }
    for legacy_key in ["view", "fields", "masks", "pointers", "schemaDigest"] {
        let mut body = exact_body();
        body[legacy_key] = json!([]);
        bad_requests.push((body, StatusCode::UNPROCESSABLE_ENTITY));
    }

    for (body, expected_status) in bad_requests {
        let response = test
            .app
            .clone()
            .oneshot(exposure_request(body))
            .await
            .expect("invalid exposure request response");
        assert_eq!(response.status(), expected_status);
    }
}

#[tokio::test]
async fn missing_invalid_and_expired_authentication_return_no_decision() {
    let (test, _) = exposure_fixture().await;

    let missing = Request::builder()
        .method(Method::POST)
        .uri("/api/v1/exposure-authz/read")
        .header("content-type", "application/json")
        .header("x-organization-id", "bootstrap-user")
        .header("x-grant-app-auth-key", support::APP_AUTH_KEY)
        .body(Body::from(exact_body().to_string()))
        .expect("missing auth request");
    let response = test
        .app
        .clone()
        .oneshot(missing)
        .await
        .expect("missing auth response");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = support::response_json(response).await;
    assert_eq!(body.as_object().map(|object| object.len()), Some(1));
    assert!(body.get("error").is_some());

    let invalid_app_key = Request::builder()
        .method(Method::POST)
        .uri("/api/v1/exposure-authz/read")
        .header("content-type", "application/json")
        .header(
            "authorization",
            support::auth::bearer_header(&format!("app-token/{APPLICATION_ID}/bootstrap-user")),
        )
        .header("x-organization-id", "bootstrap-user")
        .header("x-grant-app-auth-key", "wrong-app-auth-key")
        .body(Body::from(exact_body().to_string()))
        .expect("invalid app key request");
    let response = test
        .app
        .clone()
        .oneshot(invalid_app_key)
        .await
        .expect("invalid app key response");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = support::response_json(response).await;
    assert_eq!(body.as_object().map(|object| object.len()), Some(1));
    assert!(body.get("error").is_some());

    let unknown_application = test
        .app
        .clone()
        .oneshot(exposure_request_with_token(
            exact_body(),
            "app-token/unknown-application/bootstrap-user",
        ))
        .await
        .expect("unknown application response");
    assert_eq!(unknown_application.status(), StatusCode::FORBIDDEN);

    let expired = test
        .app
        .oneshot(exposure_request_with_token(exact_body(), "expired-token"))
        .await
        .expect("expired token response");
    assert_eq!(expired.status(), StatusCode::UNAUTHORIZED);
    let body = support::response_json(expired).await;
    assert_eq!(body.as_object().map(|object| object.len()), Some(1));
    assert!(body.get("error").is_some());
}

#[tokio::test]
async fn unknown_and_expired_policy_deny_with_exact_non_business_response() {
    let (test, token_user_id) = exposure_fixture().await;

    let mut unknown = exact_body();
    unknown["resourceId"] = json!("unknown-resource");
    let response = test
        .app
        .clone()
        .oneshot(exposure_request(unknown.clone()))
        .await
        .expect("unknown resource response");
    assert_eq!(response.status(), StatusCode::OK);
    let response = support::response_json(response).await;
    assert_eq!(response["allowed"], false);
    assert!(response_matches_request(&response, &unknown));

    let agreement = Agreement {
        id: "agreement-exposure-read".to_owned(),
        agreement_type: AgreementType::Delegation,
        grantor_organization_id: "bootstrap-user".to_owned(),
        grantee_organization_id: "bootstrap-user".to_owned(),
        status: AgreementStatus::Active,
        title: "Exposure read".to_owned(),
        reference_no: None,
        starts_at: None,
        ends_at: Some("2999-01-01T00:00:00Z".to_owned()),
    };
    test.db
        .upsert_agreement(&agreement)
        .await
        .expect("active exposure agreement");
    test.db
        .save_role_binding(&RoleBinding {
            id: "binding-exposure-reader".to_owned(),
            subject_type: RoleBindingSubjectType::User,
            subject_id: token_user_id,
            subject_organization_id: "bootstrap-user".to_owned(),
            provider_id: "exposure".to_owned(),
            resource_type: EXPOSURE_ID.to_owned(),
            resource_id: RESOURCE_ID.to_owned(),
            role_id: "role-exposure-reader".to_owned(),
            agreement_id: None,
            state: RoleBindingState::Disabled,
            created_by_user_id: support::BOOTSTRAP_USER_ID.to_owned(),
            created_at: support::FIXED_TEST_TIME.to_owned(),
            updated_at: support::FIXED_TEST_TIME.to_owned(),
        })
        .await
        .expect("disable direct exposure binding");
    test.db
        .save_role_binding(&RoleBinding {
            id: "binding-exposure-reader-agreement".to_owned(),
            subject_type: RoleBindingSubjectType::Organization,
            subject_id: "bootstrap-user".to_owned(),
            subject_organization_id: "bootstrap-user".to_owned(),
            provider_id: "exposure".to_owned(),
            resource_type: EXPOSURE_ID.to_owned(),
            resource_id: RESOURCE_ID.to_owned(),
            role_id: "role-exposure-reader".to_owned(),
            agreement_id: Some(agreement.id.clone()),
            state: RoleBindingState::Active,
            created_by_user_id: support::BOOTSTRAP_USER_ID.to_owned(),
            created_at: support::FIXED_TEST_TIME.to_owned(),
            updated_at: support::FIXED_TEST_TIME.to_owned(),
        })
        .await
        .expect("agreement-backed exposure binding");
    test.db
        .upsert_agreement(&Agreement {
            status: AgreementStatus::Expired,
            ..agreement
        })
        .await
        .expect("expired exposure agreement");

    let body = exact_body();
    let response = test
        .app
        .oneshot(exposure_request(body.clone()))
        .await
        .expect("expired policy response");
    assert_eq!(response.status(), StatusCode::OK);
    let response = support::response_json(response).await;
    assert_eq!(response["allowed"], false);
    assert_eq!(response["reason"], "no_matching_binding");
    assert!(response_matches_request(&response, &body));
}

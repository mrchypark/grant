mod support;

use axum::http::{Method, StatusCode};
use infra::config::ReadConsistency;
use serde_json::json;
use tower::ServiceExt;

const FIELD_POLICY_PATH: &str =
    "/api/v1/orgs/test-org/roles/test-role/actions/test-action/field-views";

#[tokio::test]
async fn field_policy_routes_are_absent_on_voter_and_learner_routers() {
    for test in [
        support::TestApi::new().await,
        support::TestApi::learner().await,
    ] {
        for method in [Method::GET, Method::PUT] {
            let response = test
                .app
                .clone()
                .oneshot(support::public_json_request(
                    method,
                    FIELD_POLICY_PATH,
                    json!({}),
                ))
                .await
                .expect("unregistered field policy route response");
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }
    }
}

#[tokio::test]
async fn visibility_granting_actions_are_rejected_without_mutating_the_catalog() {
    let test = support::TestApi::new().await;
    let provider = test
        .app
        .clone()
        .oneshot(support::master_json_request(
            Method::POST,
            "/api/v1/admin/providers",
            json!({
                "id": "compos",
                "name": "Compos",
                "description": null,
                "active": true
            }),
        ))
        .await
        .expect("provider response");
    assert_eq!(provider.status(), StatusCode::CREATED);

    let response = test
        .app
        .clone()
        .oneshot(support::master_json_request(
            Method::PUT,
            "/api/v1/admin/providers/compos/actions",
            json!({
                "actions": [{
                    "id": "compos.object.read",
                    "code": "compos.object.read",
                    "name": "Read Compos Object",
                    "description": null,
                    "riskLevel": "low",
                    "visibilityGranting": true,
                    "grantableToApplications": true,
                    "consentLabel": "Read Compos Object",
                    "consentDescription": null
                }]
            }),
        ))
        .await
        .expect("visibility-granting rejection response");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        support::response_json(response).await["error"],
        "visibilityGranting=true is not supported by the coarse authorization API"
    );

    let stored = test
        .db
        .list_actions_for_provider_with_consistency("compos", Some(ReadConsistency::Strong))
        .await
        .expect("stored provider actions");
    assert!(
        stored.is_empty(),
        "rejected manifest must not mutate actions"
    );
}

mod support;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
};
use serde_json::{Value, json};
use tower::ServiceExt;

const FIELD_VIEW_UNSUPPORTED: &str =
    "resource.view is not supported by the coarse authorization API";

fn trusted_request(path: &str, body: Value) -> Request<Body> {
    support::master_json_request(Method::POST, path, body)
}

fn application_request(path: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(path)
        .header("content-type", "application/json")
        .header(
            "authorization",
            support::auth::bearer_header("app-token/unknown-client/unknown-user"),
        )
        .header("x-organization-id", "unknown-org")
        .header("x-grant-app-auth-key", support::APP_AUTH_KEY)
        .body(Body::from(body.to_string()))
        .expect("application authorization request")
}

fn field_aware_resource() -> Value {
    json!({
        "provider": "compos",
        "type": "object_instance",
        "id": "object-1",
        "view": {
            "schemaKey": "must-remain-opaque",
            "fields": ["/must/not/be/parsed"]
        }
    })
}

async fn assert_field_view_rejected(response: axum::response::Response) {
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let payload = support::response_json(response).await;
    assert_eq!(payload["error"], FIELD_VIEW_UNSUPPORTED);
}

#[tokio::test]
async fn all_trusted_and_application_authorization_endpoints_reject_resource_view() {
    let test = support::TestApi::new().await;
    let trusted_subject = json!({"type": "user", "id": "unknown-user", "org_id": "unknown-org"});

    let requests = [
        trusted_request(
            "/api/v1/authz/check",
            json!({
                "subject": trusted_subject.clone(),
                "action": "compos.object.read",
                "resource": field_aware_resource()
            }),
        ),
        trusted_request(
            "/api/v1/authz/batch-check",
            json!({
                "checks": [{
                    "subject": trusted_subject.clone(),
                    "action": "compos.object.read",
                    "resource": field_aware_resource()
                }]
            }),
        ),
        trusted_request(
            "/api/v1/authz/filter",
            json!({
                "subject": trusted_subject,
                "action": "compos.object.read",
                "resources": [field_aware_resource()]
            }),
        ),
        application_request(
            "/api/v1/application-authz/check",
            json!({
                "action": "compos.object.read",
                "resource": field_aware_resource()
            }),
        ),
        application_request(
            "/api/v1/application-authz/batch-check",
            json!({
                "checks": [{
                    "action": "compos.object.read",
                    "resource": field_aware_resource()
                }]
            }),
        ),
        application_request(
            "/api/v1/application-authz/filter",
            json!({
                "action": "compos.object.read",
                "resources": [field_aware_resource()]
            }),
        ),
    ];

    for request in requests {
        let response = test
            .app
            .clone()
            .oneshot(request)
            .await
            .expect("field-aware rejection response");
        assert_field_view_rejected(response).await;
    }
}

#[tokio::test]
async fn resource_view_is_rejected_by_presence_without_parsing_its_shape() {
    let test = support::TestApi::new().await;
    for opaque_view in [Value::Null, json!("not-a-view-object"), json!([1, 2, 3])] {
        let response = test
            .app
            .clone()
            .oneshot(trusted_request(
                "/api/v1/authz/check",
                json!({
                    "subject": {"type": "user", "id": "unknown-user", "org_id": "unknown-org"},
                    "action": "compos.object.read",
                    "resource": {
                        "provider": "compos",
                        "type": "object_instance",
                        "id": "object-1",
                        "view": opaque_view
                    }
                }),
            ))
            .await
            .expect("opaque field-aware rejection response");
        assert_field_view_rejected(response).await;
    }
}

mod support;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
};
use serde_json::json;
use tower::ServiceExt;

#[tokio::test]
async fn master_user_active_is_the_local_kill_switch_and_can_reenable() {
    let test = support::TestApi::new().await;
    let path = "/api/v1/admin/users/bootstrap-user/active";

    let missing = test
        .app
        .clone()
        .oneshot(support::public_json_request(
            Method::PUT,
            path,
            json!({ "active": false }),
        ))
        .await
        .expect("missing master key response");
    assert_eq!(missing.status(), StatusCode::BAD_REQUEST);

    let wrong = test
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri(path)
                .header("content-type", "application/json")
                .header("x-grant-master-key", "wrong")
                .body(Body::from(json!({ "active": false }).to_string()))
                .expect("request"),
        )
        .await
        .expect("wrong master key response");
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);

    let disabled = test
        .app
        .clone()
        .oneshot(support::master_json_request(
            Method::PUT,
            path,
            json!({ "active": false }),
        ))
        .await
        .expect("disable response");
    assert_eq!(disabled.status(), StatusCode::OK);

    let me_disabled = test
        .app
        .clone()
        .oneshot(support::auth_empty_request(
            Method::GET,
            "/api/v1/me?consistency=strong",
            "bootstrap-user",
        ))
        .await
        .expect("disabled me response");
    assert_eq!(me_disabled.status(), StatusCode::FORBIDDEN);

    let enabled = test
        .app
        .clone()
        .oneshot(support::master_json_request(
            Method::PUT,
            path,
            json!({ "active": true }),
        ))
        .await
        .expect("enable response");
    assert_eq!(enabled.status(), StatusCode::OK);

    let me_enabled = test
        .app
        .oneshot(support::auth_empty_request(
            Method::GET,
            "/api/v1/me?consistency=strong",
            "bootstrap-user",
        ))
        .await
        .expect("enabled me response");
    assert_eq!(me_enabled.status(), StatusCode::OK);
}

#[tokio::test]
async fn learner_rejects_user_active_mutation() {
    let test = support::TestApi::learner().await;
    let response = test
        .app
        .oneshot(support::master_json_request(
            Method::PUT,
            "/api/v1/admin/users/bootstrap-user/active",
            json!({ "active": false }),
        ))
        .await
        .expect("learner response");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

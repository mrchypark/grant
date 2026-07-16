mod support;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
};
use domain::{Action, ActionRiskLevel, Role, ServiceProvider};
use hiqlite::Row;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use support::TestApi;
use tower::ServiceExt;

const ORGANIZATION_ID: &str = "field-view-org";
const ROLE_ID: &str = "field-view-role";
const SCHEMA_KEY: &str = "019f-object-type-customer";
const GENERIC_ACTION_ID: &str = "record-read-action";
const NON_FIELD_VIEW_ACTION_ID: &str = "record-update-action";

#[derive(Debug)]
struct CountRow {
    count: i64,
}

impl From<&mut Row<'_>> for CountRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            count: row.get("count"),
        }
    }
}

async fn fixture(learner: bool, grant_manage_permission: bool) -> TestApi {
    let api = if learner {
        TestApi::learner().await
    } else {
        TestApi::new().await
    };
    api.db
        .upsert_organization_with_owner(ORGANIZATION_ID, "Field View Org", None, "Field View Org")
        .await
        .expect("organization");
    api.db
        .upsert_role(&Role {
            id: "field-view-manager".to_owned(),
            organization_id: None,
            code: "organization.permission.manage".to_owned(),
            name: "Field View Manager".to_owned(),
            description: None,
        })
        .await
        .expect("manager role");
    api.db
        .upsert_role_action("field-view-manager", "organization.permission.manage")
        .await
        .expect("manager action");
    if grant_manage_permission {
        api.db
            .save_role_binding(&support::user_org_role_binding(
                "field-view-manager",
                ORGANIZATION_ID,
                support::BOOTSTRAP_USER_ID,
            ))
            .await
            .expect("manager binding");
    }
    api.db
        .upsert_service_provider(&ServiceProvider {
            id: "compos".to_owned(),
            name: "Compos".to_owned(),
            description: None,
            active: true,
        })
        .await
        .expect("provider");
    api.db
        .upsert_service_provider(&ServiceProvider {
            id: "records".to_owned(),
            name: "Records".to_owned(),
            description: None,
            active: true,
        })
        .await
        .expect("generic provider");
    for action_id in [
        "compos.object.read",
        "compos.object_type.schema.read",
        "compos.object_type.command.read",
    ] {
        api.db
            .upsert_action(&Action {
                id: action_id.to_owned(),
                provider_id: "compos".to_owned(),
                code: action_id.to_owned(),
                name: action_id.to_owned(),
                description: None,
                risk_level: ActionRiskLevel::Low,
                visibility_granting: true,
                grantable_to_applications: true,
                consent_label: None,
                consent_description: None,
            })
            .await
            .expect("action");
    }
    for (action_id, code, visibility_granting) in [
        (GENERIC_ACTION_ID, "records.entry.read", true),
        (NON_FIELD_VIEW_ACTION_ID, "records.entry.update", false),
    ] {
        api.db
            .upsert_action(&Action {
                id: action_id.to_owned(),
                provider_id: "records".to_owned(),
                code: code.to_owned(),
                name: code.to_owned(),
                description: None,
                risk_level: ActionRiskLevel::Low,
                visibility_granting,
                grantable_to_applications: true,
                consent_label: None,
                consent_description: None,
            })
            .await
            .expect("generic action");
    }
    api.db
        .upsert_role(&Role {
            id: ROLE_ID.to_owned(),
            organization_id: Some(ORGANIZATION_ID.to_owned()),
            code: "field.view.reader".to_owned(),
            name: "Field View Reader".to_owned(),
            description: None,
        })
        .await
        .expect("target role");
    for action_id in [
        "compos.object.read",
        "compos.object_type.schema.read",
        "compos.object_type.command.read",
        GENERIC_ACTION_ID,
        NON_FIELD_VIEW_ACTION_ID,
    ] {
        api.db
            .upsert_role_action(ROLE_ID, action_id)
            .await
            .expect("target role action");
    }
    api
}

fn endpoint(action_id: &str) -> String {
    format!("/api/v1/orgs/{ORGANIZATION_ID}/roles/{ROLE_ID}/actions/{action_id}/field-views")
}

fn request(method: Method, path: &str, body: Option<Value>, key: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header(
            "authorization",
            support::auth::bearer_header(support::BOOTSTRAP_USER_ID),
        )
        .header("x-organization-id", ORGANIZATION_ID);
    if let Some(key) = key {
        builder = builder.header("idempotency-key", key);
    }
    if body.is_some() {
        builder = builder.header("content-type", "application/json");
    }
    builder
        .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
        .expect("request")
}

async fn response_json(response: axum::response::Response) -> Value {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("response body")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("response JSON")
}

async fn count(api: &TestApi, sql: &str) -> i64 {
    api.runtime
        .query_rows::<CountRow, _>(
            infra::config::ReadConsistency::Strong,
            sql.to_owned(),
            vec![],
        )
        .await
        .expect("count query")[0]
        .count
}

#[tokio::test]
async fn put_get_and_empty_replace_round_trip_positive_rows() {
    let api = fixture(false, true).await;
    let path = endpoint("compos.object.read");
    let put = api
        .app
        .clone()
        .oneshot(request(
            Method::PUT,
            &path,
            Some(json!({
                "resourceType": "compos.object",
                "schemaKey": SCHEMA_KEY,
                "fieldViews": {
                    "/id": "real",
                    "/values/name": "real",
                    "/values/internal": "none",
                    "/values/email": "anonymized",
                    "/values/a*b": "real"
                }
            })),
            Some("round-trip-1"),
        ))
        .await
        .expect("PUT response");
    assert_eq!(put.status(), StatusCode::OK);
    let put_body = response_json(put).await;
    assert_eq!(
        put_body["fieldViews"],
        json!({
            "/id": "real",
            "/values/a*b": "real",
            "/values/email": "anonymized",
            "/values/name": "real"
        })
    );
    assert!(put_body["updatedAt"].as_str().is_some());

    let get_path = format!("{path}?resourceType=compos.object&schemaKey={SCHEMA_KEY}");
    let get = api
        .app
        .clone()
        .oneshot(request(Method::GET, &get_path, None, None))
        .await
        .expect("GET response");
    assert_eq!(get.status(), StatusCode::OK);
    assert_eq!(response_json(get).await, put_body);
    assert_eq!(
        count(
            &api,
            "SELECT COUNT(*) AS count FROM role_action_field_views"
        )
        .await,
        4
    );

    let empty = api
        .app
        .clone()
        .oneshot(request(
            Method::PUT,
            &path,
            Some(json!({
                "resourceType": "compos.object",
                "schemaKey": SCHEMA_KEY,
                "fieldViews": {}
            })),
            Some("round-trip-2"),
        ))
        .await
        .expect("empty PUT response");
    assert_eq!(empty.status(), StatusCode::OK);
    assert_eq!(response_json(empty).await["fieldViews"], json!({}));
    assert_eq!(
        count(
            &api,
            "SELECT COUNT(*) AS count FROM role_action_field_views"
        )
        .await,
        0
    );
    assert_eq!(
        count(
            &api,
            "SELECT COUNT(*) AS count FROM field_view_policy_audit_outbox"
        )
        .await,
        2
    );
}

#[tokio::test]
async fn idempotency_replay_race_returns_one_stored_response_and_conflicts_on_hash_change() {
    let api = fixture(false, true).await;
    let path = endpoint("compos.object.read");
    let body = json!({
        "resourceType": "compos.object",
        "schemaKey": SCHEMA_KEY,
        "fieldViews": {
            "/values/name": "real",
            "/values/hidden": "none"
        }
    });
    let first = api.app.clone().oneshot(request(
        Method::PUT,
        &path,
        Some(body.clone()),
        Some("same-key"),
    ));
    let second = api
        .app
        .clone()
        .oneshot(request(Method::PUT, &path, Some(body), Some("same-key")));
    let (first, second) = tokio::join!(first, second);
    let first = first.expect("first replay race response");
    let second = second.expect("second replay race response");
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(second.status(), StatusCode::OK);
    assert_eq!(response_json(first).await, response_json(second).await);
    assert_eq!(
        count(
            &api,
            "SELECT COUNT(*) AS count FROM field_view_policy_audit_outbox"
        )
        .await,
        1
    );

    let conflict = api
        .app
        .clone()
        .oneshot(request(
            Method::PUT,
            &path,
            Some(json!({
                "resourceType": "compos.object",
                "schemaKey": SCHEMA_KEY,
                "fieldViews": { "/values/name": "anonymized" }
            })),
            Some("same-key"),
        ))
        .await
        .expect("conflict response");
    assert_eq!(conflict.status(), StatusCode::CONFLICT);
    assert_eq!(
        count(
            &api,
            "SELECT COUNT(*) AS count FROM field_view_policy_audit_outbox"
        )
        .await,
        1
    );
}

#[tokio::test]
async fn idempotency_replay_applies_before_role_action_lifecycle_validation_after_cleanup() {
    let api = fixture(false, true).await;
    let path = endpoint("compos.object.read");
    let body = json!({
        "resourceType": "compos.object",
        "schemaKey": SCHEMA_KEY,
        "fieldViews": {
            "/values/name": "real",
            "/values/internal": "anonymized"
        }
    });
    let response = api
        .app
        .clone()
        .oneshot(request(
            Method::PUT,
            &path,
            Some(body.clone()),
            Some("cleanup-key"),
        ))
        .await
        .expect("initial field view write");
    assert_eq!(response.status(), StatusCode::OK);
    let stored_body = response_json(response).await;

    api.db
        .replace_role_actions(ROLE_ID, &[])
        .await
        .expect("remove role actions for cleanup");

    let replay_same_hash = api
        .app
        .clone()
        .oneshot(request(
            Method::PUT,
            &path,
            Some(body.clone()),
            Some("cleanup-key"),
        ))
        .await
        .expect("replay same hash");
    assert_eq!(replay_same_hash.status(), StatusCode::OK);
    assert_eq!(response_json(replay_same_hash).await, stored_body);

    let conflict = api
        .app
        .clone()
        .oneshot(request(
            Method::PUT,
            &path,
            Some(json!({
                "resourceType": "compos.object",
                "schemaKey": SCHEMA_KEY,
                "fieldViews": { "/values/name": "anonymized" },
            })),
            Some("cleanup-key"),
        ))
        .await
        .expect("replay hash mismatch");
    assert_eq!(conflict.status(), StatusCode::CONFLICT);

    let notfound_request = api
        .app
        .clone()
        .oneshot(request(
            Method::PUT,
            &path,
            Some(json!({
                "resourceType": "compos.object",
                "schemaKey": SCHEMA_KEY,
                "fieldViews": {
                    "/values/name": "anonymized",
                    "/values/internal": "anonymized"
                }
            })),
            Some("cleanup-new-key"),
        ))
        .await
        .expect("new key after cleanup");
    assert_eq!(notfound_request.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        count(
            &api,
            "SELECT COUNT(*) AS count FROM field_view_policy_audit_outbox"
        )
        .await,
        1
    );
}

#[tokio::test]
async fn permissions_role_action_and_visibility_capability_are_enforced() {
    let denied = fixture(false, false).await;
    let denied_response = denied
        .app
        .clone()
        .oneshot(request(
            Method::PUT,
            &endpoint("compos.object.read"),
            Some(json!({
                "resourceType": "compos.object",
                "schemaKey": SCHEMA_KEY,
                "fieldViews": {}
            })),
            Some("denied"),
        ))
        .await
        .expect("permission response");
    assert_eq!(denied_response.status(), StatusCode::FORBIDDEN);

    let api = fixture(false, true).await;
    for (index, (action_id, resource_type)) in [
        ("compos.object.read", "compos.object"),
        ("compos.object_type.schema.read", "compos.object_type"),
        ("compos.object_type.command.read", "compos.object_type"),
    ]
    .into_iter()
    .enumerate()
    {
        let response = api
            .app
            .clone()
            .oneshot(request(
                Method::PUT,
                &endpoint(action_id),
                Some(json!({
                    "resourceType": resource_type,
                    "schemaKey": SCHEMA_KEY,
                    "fieldViews": {}
                })),
                Some(&format!("tuple-{index}")),
            ))
            .await
            .expect("canonical tuple response");
        assert_eq!(response.status(), StatusCode::OK);
    }

    let provider_neutral_scope = api
        .app
        .clone()
        .oneshot(request(
            Method::PUT,
            &endpoint(GENERIC_ACTION_ID),
            Some(json!({
                "resourceType": "records.entry.v2",
                "schemaKey": "opaque:entry/schema@2026",
                "fieldViews": {"/payload/*": "real"}
            })),
            Some("provider-neutral"),
        ))
        .await
        .expect("provider-neutral response");
    assert_eq!(provider_neutral_scope.status(), StatusCode::OK);
    assert_eq!(
        response_json(provider_neutral_scope).await["fieldViews"]["/payload/*"],
        "real"
    );

    let unsupported_action = api
        .app
        .clone()
        .oneshot(request(
            Method::PUT,
            &endpoint(NON_FIELD_VIEW_ACTION_ID),
            Some(json!({
                "resourceType": "records.entry.v2",
                "schemaKey": "opaque:entry/schema@2026",
                "fieldViews": {}
            })),
            Some("unsupported-action"),
        ))
        .await
        .expect("unsupported action response");
    assert_eq!(unsupported_action.status(), StatusCode::BAD_REQUEST);

    api.db
        .replace_role_actions(ROLE_ID, &[])
        .await
        .expect("remove role actions");
    let missing_role_action = api
        .app
        .clone()
        .oneshot(request(
            Method::GET,
            &format!(
                "{}?resourceType=compos.object&schemaKey={SCHEMA_KEY}",
                endpoint("compos.object.read")
            ),
            None,
            None,
        ))
        .await
        .expect("missing role action response");
    assert_eq!(missing_role_action.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn validation_rejects_missing_key_invalid_pointers_and_none_hash_changes() {
    let api = fixture(false, true).await;
    let path = endpoint("compos.object.read");
    let valid_body = json!({
        "resourceType": "compos.object",
        "schemaKey": SCHEMA_KEY,
        "fieldViews": { "/values/name": "real" }
    });
    let missing_key = api
        .app
        .clone()
        .oneshot(request(Method::PUT, &path, Some(valid_body.clone()), None))
        .await
        .expect("missing key response");
    assert_eq!(missing_key.status(), StatusCode::BAD_REQUEST);
    let duplicate_pointer = api
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri(&path)
                .header(
                    "authorization",
                    support::auth::bearer_header(support::BOOTSTRAP_USER_ID),
                )
                .header("x-organization-id", ORGANIZATION_ID)
                .header("idempotency-key", "duplicate-pointer")
                .header("content-type", "application/json")
                .body(Body::from(format!(
                    r#"{{"resourceType":"compos.object","schemaKey":"{SCHEMA_KEY}","fieldViews":{{"/values/name":"real","/values/name":"anonymized"}}}}"#
                )))
                .expect("duplicate pointer request"),
        )
        .await
        .expect("duplicate pointer response");
    assert_eq!(duplicate_pointer.status(), StatusCode::BAD_REQUEST);
    for (index, pointer) in ["", "values/name", "/values/~2bad"].into_iter().enumerate() {
        let response = api
            .app
            .clone()
            .oneshot(request(
                Method::PUT,
                &path,
                Some(json!({
                    "resourceType": "compos.object",
                    "schemaKey": SCHEMA_KEY,
                    "fieldViews": { pointer: "real" }
                })),
                Some(&format!("invalid-{index}")),
            ))
            .await
            .expect("invalid pointer response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{pointer}");
    }
    for (index, body) in [
        json!({
            "resourceType": "compos.object ",
            "schemaKey": SCHEMA_KEY,
            "fieldViews": { "/values/name": "real" }
        }),
        json!({
            "resourceType": "compos.object",
            "schemaKey": format!(" {SCHEMA_KEY}"),
            "fieldViews": { "/values/name": "real" }
        }),
    ]
    .into_iter()
    .enumerate()
    {
        let response = api
            .app
            .clone()
            .oneshot(request(
                Method::PUT,
                &path,
                Some(body),
                Some(&format!("whitespace-{index}")),
            ))
            .await
            .expect("whitespace scope response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    let first = api
        .app
        .clone()
        .oneshot(request(
            Method::PUT,
            &path,
            Some(valid_body),
            Some("none-hash"),
        ))
        .await
        .expect("first hash response");
    assert_eq!(first.status(), StatusCode::OK);
    let none_change = api
        .app
        .clone()
        .oneshot(request(
            Method::PUT,
            &path,
            Some(json!({
                "resourceType": "compos.object",
                "schemaKey": SCHEMA_KEY,
                "fieldViews": {
                    "/values/name": "real",
                    "/values/extra": "none"
                }
            })),
            Some("none-hash"),
        ))
        .await
        .expect("none hash response");
    assert_eq!(none_change.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn corrupt_policy_rows_and_persisted_replays_fail_closed() {
    let api = fixture(false, true).await;
    let path = endpoint("compos.object.read");
    let body = json!({
        "resourceType": "compos.object",
        "schemaKey": SCHEMA_KEY,
        "fieldViews": { "/values/name": "real" }
    });
    let initial = api
        .app
        .clone()
        .oneshot(request(
            Method::PUT,
            &path,
            Some(body.clone()),
            Some("corrupt-replay"),
        ))
        .await
        .expect("initial policy write");
    assert_eq!(initial.status(), StatusCode::OK);

    api.runtime
        .execute(
            "UPDATE role_action_field_views
             SET field_pointer = '/values/~2bad'
             WHERE role_id = $1 AND action_id = 'compos.object.read'",
            vec![ROLE_ID.to_owned().into()],
        )
        .await
        .expect("corrupt stored policy pointer");
    let get_path = format!("{path}?resourceType=compos.object&schemaKey={SCHEMA_KEY}");
    let corrupt_get = api
        .app
        .clone()
        .oneshot(request(Method::GET, &get_path, None, None))
        .await
        .expect("corrupt policy response");
    assert_eq!(corrupt_get.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let corrupt_after = json!({
        "roleId": ROLE_ID,
        "actionId": "compos.object.read",
        "resourceType": "compos.object",
        "schemaKey": SCHEMA_KEY,
        "fieldViews": { "/values/~2bad": "real" },
        "updatedAt": support::FIXED_TEST_TIME
    })
    .to_string();
    api.runtime
        .execute(
            "UPDATE field_view_policy_audit_outbox
             SET after_json = $1
             WHERE idempotency_key = $2",
            vec![corrupt_after.into(), "corrupt-replay".to_owned().into()],
        )
        .await
        .expect("corrupt persisted replay");
    let corrupt_replay = api
        .app
        .oneshot(request(
            Method::PUT,
            &path,
            Some(body),
            Some("corrupt-replay"),
        ))
        .await
        .expect("corrupt replay response");
    assert_eq!(corrupt_replay.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn learner_allows_get_rejects_put_and_audit_failure_rolls_back_policy() {
    let learner = fixture(true, true).await;
    learner
        .runtime
        .execute(
            "INSERT INTO role_action_field_views (
                role_id, action_id, resource_type, schema_key, field_pointer,
                view_mode, created_at, updated_at
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $7)",
            vec![
                ROLE_ID.into(),
                "compos.object.read".into(),
                "compos.object".into(),
                SCHEMA_KEY.into(),
                "/values/name".into(),
                "real".into(),
                support::FIXED_TEST_TIME.into(),
            ],
        )
        .await
        .expect("seed learner policy");
    let path = endpoint("compos.object.read");
    let get = learner
        .app
        .clone()
        .oneshot(request(
            Method::GET,
            &format!("{path}?resourceType=compos.object&schemaKey={SCHEMA_KEY}&consistency=strong"),
            None,
            None,
        ))
        .await
        .expect("learner GET response");
    assert_eq!(get.status(), StatusCode::OK);
    assert_eq!(
        response_json(get).await["fieldViews"],
        json!({ "/values/name": "real" })
    );
    let put = learner
        .app
        .clone()
        .oneshot(request(
            Method::PUT,
            &path,
            Some(json!({
                "resourceType": "compos.object",
                "schemaKey": SCHEMA_KEY,
                "fieldViews": {}
            })),
            Some("learner-put"),
        ))
        .await
        .expect("learner PUT response");
    assert_eq!(put.status(), StatusCode::SERVICE_UNAVAILABLE);

    let api = fixture(false, true).await;
    api.runtime
        .execute(
            "INSERT INTO role_action_field_views (
                role_id, action_id, resource_type, schema_key, field_pointer,
                view_mode, created_at, updated_at
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $7)",
            vec![
                ROLE_ID.into(),
                "compos.object.read".into(),
                "compos.object".into(),
                SCHEMA_KEY.into(),
                "/values/old".into(),
                "real".into(),
                support::FIXED_TEST_TIME.into(),
            ],
        )
        .await
        .expect("seed old policy");
    api.runtime
        .batch(
            "CREATE TRIGGER fail_field_view_audit
             BEFORE INSERT ON field_view_policy_audit_outbox
             BEGIN
                 SELECT RAISE(ABORT, 'forced audit failure');
             END;",
        )
        .await
        .expect("audit failure trigger");
    let failed = api
        .app
        .clone()
        .oneshot(request(
            Method::PUT,
            &path,
            Some(json!({
                "resourceType": "compos.object",
                "schemaKey": SCHEMA_KEY,
                "fieldViews": { "/values/new": "anonymized" }
            })),
            Some("rollback"),
        ))
        .await
        .expect("failed audit response");
    assert_eq!(failed.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        count(
            &api,
            "SELECT COUNT(*) AS count FROM role_action_field_views WHERE field_pointer = '/values/old'"
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &api,
            "SELECT COUNT(*) AS count FROM role_action_field_views WHERE field_pointer = '/values/new'"
        )
        .await,
        0
    );
    assert_eq!(
        count(
            &api,
            "SELECT COUNT(*) AS count FROM field_view_policy_audit_outbox"
        )
        .await,
        0
    );
}

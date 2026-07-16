mod support;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
};
use domain::{
    Action, ActionRiskLevel, Application, ApplicationAppType, ApplicationConsentPolicy,
    ApplicationGrant, ApplicationGrantStatus, ApplicationProvisioningStatus, ApplicationType, Role,
    RoleBinding, RoleBindingState, RoleBindingSubjectType, ServiceProvider,
};
use serde_json::{Value, json};
use tower::ServiceExt;

const ORGANIZATION_ID: &str = "bootstrap-user";
const OBJECT_ID: &str = "customer-1";
const SCHEMA_KEY: &str = "customer-type-1";
const APPLICATION_ID: &str = "field-view-application";
const GENERIC_PROVIDER_ID: &str = "records";
const GENERIC_ACTION_ID: &str = "record-read-action";
const GENERIC_RESOURCE_TYPE: &str = "records.entry.v2";
const GENERIC_RESOURCE_ID: &str = "entry-1";
const GENERIC_SCHEMA_KEY: &str = "opaque:entry/schema@2026";

struct FieldViewFixture {
    api: support::TestApi,
    user_id: String,
}

async fn field_view_fixture() -> FieldViewFixture {
    let api = support::TestApi::new().await;
    let user = support::seed_identity(
        &api.db,
        "field-view-user",
        Some("field-view-user@rauthy.test"),
        "Field View User",
    )
    .await;
    if !api
        .db
        .has_active_organization_membership(&user.id, ORGANIZATION_ID)
        .await
        .expect("membership check")
    {
        support::seed_org_user_membership(
            &api.runtime,
            "membership-field-view-user",
            ORGANIZATION_ID,
            &user.id,
        )
        .await;
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
            id: GENERIC_PROVIDER_ID.to_owned(),
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
                consent_label: Some(action_id.to_owned()),
                consent_description: None,
            })
            .await
            .expect("action");
    }
    for (id, code, visibility_granting) in [
        (GENERIC_ACTION_ID, "records.entry.read", true),
        ("record-update-action", "records.entry.update", false),
    ] {
        api.db
            .upsert_action(&Action {
                id: id.to_owned(),
                provider_id: GENERIC_PROVIDER_ID.to_owned(),
                code: code.to_owned(),
                name: code.to_owned(),
                description: None,
                risk_level: ActionRiskLevel::Low,
                visibility_granting,
                grantable_to_applications: true,
                consent_label: Some(code.to_owned()),
                consent_description: None,
            })
            .await
            .expect("generic action");
    }
    for role_id in [
        "role-object-view",
        "role-object-type-view",
        "role-generic-view",
    ] {
        api.db
            .upsert_role(&Role {
                id: role_id.to_owned(),
                organization_id: Some(ORGANIZATION_ID.to_owned()),
                code: role_id.to_owned(),
                name: role_id.to_owned(),
                description: None,
            })
            .await
            .expect("role");
    }
    api.db
        .upsert_role_action("role-generic-view", GENERIC_ACTION_ID)
        .await
        .expect("generic role action");
    api.db
        .upsert_role_action("role-object-view", "compos.object.read")
        .await
        .expect("object role action");
    for action_id in [
        "compos.object_type.schema.read",
        "compos.object_type.command.read",
    ] {
        api.db
            .upsert_role_action("role-object-type-view", action_id)
            .await
            .expect("object type role action");
    }
    api.db
        .save_role_binding(&RoleBinding {
            id: "binding-generic-view".to_owned(),
            subject_type: RoleBindingSubjectType::User,
            subject_id: user.id.clone(),
            subject_organization_id: ORGANIZATION_ID.to_owned(),
            provider_id: GENERIC_PROVIDER_ID.to_owned(),
            resource_type: GENERIC_RESOURCE_TYPE.to_owned(),
            resource_id: GENERIC_RESOURCE_ID.to_owned(),
            role_id: "role-generic-view".to_owned(),
            agreement_id: None,
            state: RoleBindingState::Active,
            created_by_user_id: support::BOOTSTRAP_USER_ID.to_owned(),
            created_at: support::FIXED_TEST_TIME.to_owned(),
            updated_at: support::FIXED_TEST_TIME.to_owned(),
        })
        .await
        .expect("generic binding");
    for (id, resource_type, resource_id, role_id) in [
        (
            "binding-object-view",
            "compos.object",
            OBJECT_ID,
            "role-object-view",
        ),
        (
            "binding-object-type-view",
            "compos.object_type",
            SCHEMA_KEY,
            "role-object-type-view",
        ),
    ] {
        api.db
            .save_role_binding(&RoleBinding {
                id: id.to_owned(),
                subject_type: RoleBindingSubjectType::User,
                subject_id: user.id.clone(),
                subject_organization_id: ORGANIZATION_ID.to_owned(),
                provider_id: "compos".to_owned(),
                resource_type: resource_type.to_owned(),
                resource_id: resource_id.to_owned(),
                role_id: role_id.to_owned(),
                agreement_id: None,
                state: RoleBindingState::Active,
                created_by_user_id: support::BOOTSTRAP_USER_ID.to_owned(),
                created_at: support::FIXED_TEST_TIME.to_owned(),
                updated_at: support::FIXED_TEST_TIME.to_owned(),
            })
            .await
            .expect("binding");
    }
    api.runtime
        .execute(
            "INSERT INTO role_action_field_views (
                role_id, action_id, resource_type, schema_key, field_pointer, view_mode, created_at, updated_at
             ) VALUES
                ('role-object-view', 'compos.object.read', 'compos.object', $1, '/title', 'anonymized', $2, $2),
                ('role-object-view', 'compos.object.read', 'compos.object', $1, '/values/secret', 'real', $2, $2),
                ('role-object-type-view', 'compos.object_type.schema.read', 'compos.object_type', $1, '/title', 'anonymized', $2, $2),
                ('role-object-type-view', 'compos.object_type.command.read', 'compos.object_type', $1, '/title', 'real', $2, $2)",
            vec![SCHEMA_KEY.into(), support::FIXED_TEST_TIME.into()],
        )
        .await
        .expect("field policies");
    api.runtime
        .execute(
            "INSERT INTO role_action_field_views (
                role_id, action_id, resource_type, schema_key, field_pointer, view_mode, created_at, updated_at
             ) VALUES
                ('role-generic-view', $1, $2, $3, '/payload/*', 'real', $4, $4),
                ('role-generic-view', $1, $2, $3, '/id', 'real', $4, $4)",
            vec![
                GENERIC_ACTION_ID.into(),
                GENERIC_RESOURCE_TYPE.into(),
                GENERIC_SCHEMA_KEY.into(),
                support::FIXED_TEST_TIME.into(),
            ],
        )
        .await
        .expect("generic field policy");
    FieldViewFixture {
        api,
        user_id: user.id,
    }
}

fn trusted_check(user_id: &str, action: &str, resource: Value) -> Value {
    json!({
        "subject": {"type": "user", "id": user_id, "org_id": ORGANIZATION_ID},
        "action": action,
        "resource": resource
    })
}

fn object_resource(id: &str, fields: &[&str]) -> Value {
    json!({
        "provider": "compos",
        "type": "compos.object",
        "id": id,
        "view": {"schemaKey": SCHEMA_KEY, "fields": fields}
    })
}

fn application_request(path: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(path)
        .header("content-type", "application/json")
        .header(
            "authorization",
            support::auth::bearer_header(&format!("app-token/{APPLICATION_ID}/field-view-user")),
        )
        .header("x-organization-id", ORGANIZATION_ID)
        .header("x-grant-app-auth-key", support::APP_AUTH_KEY)
        .body(Body::from(body.to_string()))
        .expect("application request")
}

#[tokio::test]
async fn trusted_wire_preserves_binary_shape_and_returns_complete_ordered_field_batches() {
    let fixture = field_view_fixture().await;
    let app = fixture.api.app.clone();
    let binary = app
        .clone()
        .oneshot(support::master_json_request(
            Method::POST,
            "/api/v1/authz/check",
            trusted_check(
                &fixture.user_id,
                "compos.object.read",
                json!({"provider": "compos", "type": "compos.object", "id": OBJECT_ID}),
            ),
        ))
        .await
        .expect("binary check");
    assert_eq!(binary.status(), StatusCode::OK);
    assert_eq!(
        support::response_json(binary).await,
        json!({"allowed": true, "reason": "role_binding"})
    );

    let oversized_legacy_resource_type = "r".repeat(129);
    let oversized_binary = app
        .clone()
        .oneshot(support::master_json_request(
            Method::POST,
            "/api/v1/authz/check",
            trusted_check(
                &fixture.user_id,
                "compos.object.read",
                json!({
                    "provider": "compos",
                    "type": oversized_legacy_resource_type,
                    "id": OBJECT_ID
                }),
            ),
        ))
        .await
        .expect("oversized legacy binary check");
    assert_eq!(oversized_binary.status(), StatusCode::OK);
    assert!(support::response_json(oversized_binary).await["fieldViews"].is_null());

    let checks = vec![
        trusted_check(
            &fixture.user_id,
            "compos.object.read",
            object_resource(
                OBJECT_ID,
                &[
                    "/title",
                    "/values/secret",
                    "/values/a~1b",
                    "/values/a*b",
                    "/missing",
                ],
            ),
        ),
        trusted_check(
            &fixture.user_id,
            "compos.object.read",
            object_resource(
                "denied-object",
                &[
                    "/title",
                    "/values/secret",
                    "/values/a~1b",
                    "/values/a*b",
                    "/missing",
                ],
            ),
        ),
    ];
    let response = app
        .oneshot(support::master_json_request(
            Method::POST,
            "/api/v1/authz/batch-check",
            json!({"checks": checks}),
        ))
        .await
        .expect("field batch");
    assert_eq!(response.status(), StatusCode::OK);
    let payload = support::response_json(response).await;
    assert_eq!(payload["results"].as_array().map(Vec::len), Some(2));
    assert_eq!(payload["results"][0]["allowed"], true);
    assert_eq!(
        payload["results"][0]["fieldViews"],
        json!({
            "/missing": "none",
            "/title": "anonymized",
            "/values/a~1b": "none",
            "/values/a*b": "none",
            "/values/secret": "real"
        })
    );
    assert_eq!(payload["results"][1]["allowed"], false);
    assert_eq!(
        payload["results"][1]["fieldViews"],
        json!({
            "/missing": "none",
            "/title": "none",
            "/values/a~1b": "none",
            "/values/a*b": "none",
            "/values/secret": "none"
        })
    );
}

#[tokio::test]
async fn all_three_canonical_tuples_share_schema_key_and_pointer_space() {
    let fixture = field_view_fixture().await;
    let resources = [
        (
            "compos.object.read",
            json!({
                "provider": "compos", "type": "compos.object", "id": OBJECT_ID,
                "view": {"schemaKey": SCHEMA_KEY, "fields": ["/title"]}
            }),
            "anonymized",
        ),
        (
            "compos.object_type.schema.read",
            json!({
                "provider": "compos", "type": "compos.object_type", "id": SCHEMA_KEY,
                "view": {"schemaKey": SCHEMA_KEY, "fields": ["/title"]}
            }),
            "anonymized",
        ),
        (
            "compos.object_type.command.read",
            json!({
                "provider": "compos", "type": "compos.object_type", "id": SCHEMA_KEY,
                "view": {"schemaKey": SCHEMA_KEY, "fields": ["/title"]}
            }),
            "real",
        ),
    ];
    let checks = resources
        .iter()
        .map(|(action, resource, _)| trusted_check(&fixture.user_id, action, resource.clone()))
        .collect::<Vec<_>>();
    let response = fixture
        .api
        .app
        .oneshot(support::master_json_request(
            Method::POST,
            "/api/v1/authz/batch-check",
            json!({"checks": checks}),
        ))
        .await
        .expect("canonical tuple batch");
    assert_eq!(response.status(), StatusCode::OK);
    let payload = support::response_json(response).await;
    for (index, (_, _, expected_mode)) in resources.iter().enumerate() {
        assert_eq!(payload["results"][index]["allowed"], true);
        assert_eq!(
            payload["results"][index]["fieldViews"]["/title"],
            *expected_mode
        );
    }
}

#[tokio::test]
async fn field_views_support_any_visibility_granting_provider_action_and_opaque_scope() {
    let fixture = field_view_fixture().await;
    let generic = fixture
        .api
        .app
        .clone()
        .oneshot(support::master_json_request(
            Method::POST,
            "/api/v1/authz/check",
            trusted_check(
                &fixture.user_id,
                GENERIC_ACTION_ID,
                json!({
                    "provider": GENERIC_PROVIDER_ID,
                    "type": GENERIC_RESOURCE_TYPE,
                    "id": GENERIC_RESOURCE_ID,
                    "view": {
                        "schemaKey": GENERIC_SCHEMA_KEY,
                        "fields": ["/payload/*", "/id", "/missing"]
                    }
                }),
            ),
        ))
        .await
        .expect("generic field view check");
    assert_eq!(generic.status(), StatusCode::OK);
    assert_eq!(
        support::response_json(generic).await,
        json!({
            "allowed": true,
            "reason": "role_binding",
            "fieldViews": {
                "/payload/*": "real",
                "/id": "real",
                "/missing": "none"
            }
        })
    );

    let resource_id_independent_from_schema_key = fixture
        .api
        .app
        .clone()
        .oneshot(support::master_json_request(
            Method::POST,
            "/api/v1/authz/check",
            trusted_check(
                &fixture.user_id,
                "compos.object_type.schema.read",
                json!({
                    "provider": "compos",
                    "type": "compos.object_type",
                    "id": "consumer-owned-resource-id",
                    "view": {"schemaKey": SCHEMA_KEY, "fields": ["/title"]}
                }),
            ),
        ))
        .await
        .expect("opaque resource mapping check");
    assert_eq!(
        resource_id_independent_from_schema_key.status(),
        StatusCode::OK
    );
    assert_eq!(
        support::response_json(resource_id_independent_from_schema_key).await["allowed"],
        false
    );

    let unsupported = fixture
        .api
        .app
        .clone()
        .oneshot(support::master_json_request(
            Method::POST,
            "/api/v1/authz/check",
            trusted_check(
                &fixture.user_id,
                "record-update-action",
                json!({
                    "provider": GENERIC_PROVIDER_ID,
                    "type": GENERIC_RESOURCE_TYPE,
                    "id": GENERIC_RESOURCE_ID,
                    "view": {"schemaKey": GENERIC_SCHEMA_KEY, "fields": ["/payload/*"]}
                }),
            ),
        ))
        .await
        .expect("unsupported field view action");
    assert_eq!(unsupported.status(), StatusCode::BAD_REQUEST);

    let wrong_provider = fixture
        .api
        .app
        .clone()
        .oneshot(support::master_json_request(
            Method::POST,
            "/api/v1/authz/check",
            trusted_check(
                &fixture.user_id,
                GENERIC_ACTION_ID,
                json!({
                    "provider": "compos",
                    "type": GENERIC_RESOURCE_TYPE,
                    "id": GENERIC_RESOURCE_ID,
                    "view": {"schemaKey": GENERIC_SCHEMA_KEY, "fields": ["/id"]}
                }),
            ),
        ))
        .await
        .expect("provider ownership check");
    assert_eq!(wrong_provider.status(), StatusCode::BAD_REQUEST);

    fixture
        .api
        .db
        .upsert_service_provider(&ServiceProvider {
            id: GENERIC_PROVIDER_ID.to_owned(),
            name: "Records".to_owned(),
            description: None,
            active: false,
        })
        .await
        .expect("disable generic provider");
    let inactive_provider = fixture
        .api
        .app
        .clone()
        .oneshot(support::master_json_request(
            Method::POST,
            "/api/v1/authz/check",
            trusted_check(
                &fixture.user_id,
                GENERIC_ACTION_ID,
                json!({
                    "provider": GENERIC_PROVIDER_ID,
                    "type": GENERIC_RESOURCE_TYPE,
                    "id": GENERIC_RESOURCE_ID,
                    "view": {"schemaKey": GENERIC_SCHEMA_KEY, "fields": ["/id"]}
                }),
            ),
        ))
        .await
        .expect("inactive provider check");
    assert_eq!(inactive_provider.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn field_view_validation_rejects_invalid_shapes_and_filter_views() {
    let fixture = field_view_fixture().await;
    let too_many = (0..257)
        .map(|index| format!("/field-{index}"))
        .collect::<Vec<_>>();
    let oversized_resource_type = "r".repeat(129);
    let oversized_schema_key = "s".repeat(129);
    let oversized_pointer = format!("/{}", "p".repeat(512));
    let invalid = vec![
        trusted_check(
            &fixture.user_id,
            "compos.object.read",
            object_resource(OBJECT_ID, &[]),
        ),
        trusted_check(
            &fixture.user_id,
            "compos.object.read",
            object_resource(OBJECT_ID, &["/values/~2bad"]),
        ),
        trusted_check(
            &fixture.user_id,
            "compos.object.read",
            object_resource(OBJECT_ID, &["/title", "/title"]),
        ),
        trusted_check(
            &fixture.user_id,
            "compos.object.read",
            json!({
                "provider": "compos", "type": "compos.object", "id": OBJECT_ID,
                "view": {"schemaKey": SCHEMA_KEY, "fields": too_many}
            }),
        ),
        trusted_check(
            &fixture.user_id,
            "compos.object.read",
            json!({
                "provider": "compos", "type": oversized_resource_type, "id": OBJECT_ID,
                "view": {"schemaKey": SCHEMA_KEY, "fields": ["/title"]}
            }),
        ),
        trusted_check(
            &fixture.user_id,
            "compos.object.read",
            json!({
                "provider": "compos", "type": "compos.object", "id": OBJECT_ID,
                "view": {"schemaKey": oversized_schema_key, "fields": ["/title"]}
            }),
        ),
        trusted_check(
            &fixture.user_id,
            "compos.object.read",
            json!({
                "provider": "compos", "type": "compos.object", "id": OBJECT_ID,
                "view": {"schemaKey": SCHEMA_KEY, "fields": [oversized_pointer]}
            }),
        ),
        trusted_check(
            &fixture.user_id,
            "compos.object.read",
            json!({
                "provider": " compos", "type": "compos.object", "id": OBJECT_ID,
                "view": {"schemaKey": SCHEMA_KEY, "fields": ["/title"]}
            }),
        ),
        trusted_check(
            &fixture.user_id,
            "compos.object.read",
            json!({
                "provider": "compos", "type": "compos.object ", "id": OBJECT_ID,
                "view": {"schemaKey": SCHEMA_KEY, "fields": ["/title"]}
            }),
        ),
        trusted_check(
            &fixture.user_id,
            "compos.object.read",
            json!({
                "provider": "compos", "type": "compos.object", "id": format!("{OBJECT_ID} "),
                "view": {"schemaKey": SCHEMA_KEY, "fields": ["/title"]}
            }),
        ),
        trusted_check(
            &fixture.user_id,
            "compos.object.read ",
            object_resource(OBJECT_ID, &["/title"]),
        ),
        trusted_check(
            &fixture.user_id,
            "compos.object.read",
            json!({
                "provider": "compos", "type": "compos.object", "id": OBJECT_ID,
                "view": {"schemaKey": format!(" {SCHEMA_KEY}"), "fields": ["/title"]}
            }),
        ),
    ];
    for body in invalid {
        let response = fixture
            .api
            .app
            .clone()
            .oneshot(support::master_json_request(
                Method::POST,
                "/api/v1/authz/check",
                body,
            ))
            .await
            .expect("invalid check");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    let maximum_fields = (0..256)
        .map(|index| format!("/field-{index}"))
        .collect::<Vec<_>>();
    let maximum = fixture
        .api
        .app
        .clone()
        .oneshot(support::master_json_request(
            Method::POST,
            "/api/v1/authz/check",
            trusted_check(
                &fixture.user_id,
                "compos.object.read",
                json!({
                    "provider": "compos", "type": "compos.object", "id": OBJECT_ID,
                    "view": {"schemaKey": SCHEMA_KEY, "fields": maximum_fields}
                }),
            ),
        ))
        .await
        .expect("maximum field check");
    assert_eq!(maximum.status(), StatusCode::OK);
    assert_eq!(
        support::response_json(maximum).await["fieldViews"]
            .as_object()
            .map(serde_json::Map::len),
        Some(256)
    );

    let boundary_schema_key = "s".repeat(128);
    let boundary_pointer = format!("/{}", "p".repeat(511));
    let boundary = fixture
        .api
        .app
        .clone()
        .oneshot(support::master_json_request(
            Method::POST,
            "/api/v1/authz/check",
            trusted_check(
                &fixture.user_id,
                "compos.object.read",
                json!({
                    "provider": "compos", "type": "compos.object", "id": OBJECT_ID,
                    "view": {"schemaKey": boundary_schema_key, "fields": [boundary_pointer]}
                }),
            ),
        ))
        .await
        .expect("boundary field view check");
    assert_eq!(boundary.status(), StatusCode::OK);

    let filter = fixture
        .api
        .app
        .oneshot(support::master_json_request(
            Method::POST,
            "/api/v1/authz/filter",
            json!({
                "subject": {"type": "user", "id": fixture.user_id, "org_id": ORGANIZATION_ID},
                "action": "compos.object.read",
                "resources": [object_resource(OBJECT_ID, &["/title"])]
            }),
        ))
        .await
        .expect("filter view");
    assert_eq!(filter.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn candidate_role_cap_accepts_256_and_overflow_maps_to_service_unavailable() {
    let fixture = field_view_fixture().await;

    let seed_roles = |start: usize, end: usize| {
        let mut sql = String::new();
        for index in start..end {
            let role_id = format!("role-cap-{index}");
            let binding_id = format!("binding-cap-{index}");
            sql.push_str(&format!(
                "INSERT INTO roles (id, organization_id, code, name, created_at, updated_at) \
                 VALUES ('{role_id}', '{ORGANIZATION_ID}', '{role_id}', '{role_id}', \
                         '{time}', '{time}');\n\
                 INSERT INTO role_actions (role_id, action_id) \
                 VALUES ('{role_id}', 'compos.object.read');\n\
                 INSERT INTO role_bindings (\
                     id, subject_type, subject_id, subject_organization_id, provider_id, \
                     resource_type, resource_id, role_id, state, created_by_user_id, \
                     created_at, updated_at\
                 ) VALUES (\
                     '{binding_id}', 'user', '{user_id}', '{ORGANIZATION_ID}', 'compos', \
                     'compos.object', '{OBJECT_ID}', '{role_id}', 'active', \
                     '{created_by}', '{time}', '{time}'\
                 );\n",
                user_id = fixture.user_id,
                created_by = support::BOOTSTRAP_USER_ID,
                time = support::FIXED_TEST_TIME,
            ));
        }
        sql
    };

    // The fixture already has one matching object role, so 255 additional roles reach the cap.
    fixture
        .api
        .runtime
        .batch(seed_roles(0, 255))
        .await
        .expect("seed role cap boundary");
    let request = || {
        support::master_json_request(
            Method::POST,
            "/api/v1/authz/check",
            trusted_check(
                &fixture.user_id,
                "compos.object.read",
                object_resource(OBJECT_ID, &["/title"]),
            ),
        )
    };
    let boundary = fixture
        .api
        .app
        .clone()
        .oneshot(request())
        .await
        .expect("candidate role cap boundary");
    assert_eq!(boundary.status(), StatusCode::OK);

    fixture
        .api
        .runtime
        .batch(seed_roles(255, 256))
        .await
        .expect("seed candidate role overflow");
    let overflow = fixture
        .api
        .app
        .oneshot(request())
        .await
        .expect("candidate role overflow");
    assert_eq!(overflow.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn candidate_role_cap_does_not_apply_to_binary_checks_in_mixed_batches() {
    let fixture = field_view_fixture().await;

    let seed_roles = |start: usize, end: usize| {
        let mut sql = String::new();
        for index in start..end {
            let role_id = format!("role-binary-cap-{index}");
            let binding_id = format!("binding-binary-cap-{index}");
            sql.push_str(&format!(
                "INSERT INTO roles (id, organization_id, code, name, created_at, updated_at) \
                 VALUES ('{role_id}', '{ORGANIZATION_ID}', '{role_id}', '{role_id}', \
                         '{time}', '{time}');\n\
                 INSERT INTO role_actions (role_id, action_id) \
                 VALUES ('{role_id}', 'compos.object.read');\n\
                 INSERT INTO role_bindings (\
                     id, subject_type, subject_id, subject_organization_id, provider_id, \
                     resource_type, resource_id, role_id, state, created_by_user_id, \
                     created_at, updated_at\
                 ) VALUES (\
                     '{binding_id}', 'user', '{user_id}', '{ORGANIZATION_ID}', 'compos', \
                     'compos.object', '{OBJECT_ID}', '{role_id}', 'active', \
                     '{created_by}', '{time}', '{time}'\
                 );\n",
                user_id = fixture.user_id,
                created_by = support::BOOTSTRAP_USER_ID,
                time = support::FIXED_TEST_TIME,
            ));
        }
        sql
    };

    fixture
        .api
        .runtime
        .batch(seed_roles(0, 256))
        .await
        .expect("seed candidate role binary check overflow");

    let response = fixture
        .api
        .app
        .clone()
        .oneshot(support::master_json_request(
            Method::POST,
            "/api/v1/authz/batch-check",
            json!({
                "checks": [
                    trusted_check(
                        &fixture.user_id,
                        "compos.object.read",
                        json!({"provider": "compos", "type": "compos.object", "id": OBJECT_ID}),
                    ),
                    trusted_check(
                        &fixture.user_id,
                        "compos.object.read",
                        object_resource("denied-object", &["/title"])
                    )
                ]
            }),
        ))
        .await
        .expect("mixed binary/field batch");
    assert_eq!(response.status(), StatusCode::OK);
    let payload = support::response_json(response).await;
    assert_eq!(payload["results"].as_array().map(Vec::len), Some(2));
    assert_eq!(payload["results"][0]["allowed"], true);
    assert!(payload["results"][0].get("fieldViews").is_none());
    assert_eq!(payload["results"][1]["allowed"], false);
    assert_eq!(payload["results"][1]["fieldViews"]["/title"], "none");
}

#[tokio::test]
async fn application_consent_and_trust_gate_access_without_expanding_field_modes() {
    let fixture = field_view_fixture().await;
    fixture
        .api
        .db
        .insert_application(&Application {
            id: APPLICATION_ID.to_owned(),
            organization_id: ORGANIZATION_ID.to_owned(),
            slug: APPLICATION_ID.to_owned(),
            name: "Field View Application".to_owned(),
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
        .expect("application");
    fixture
        .api
        .db
        .replace_application_actions(APPLICATION_ID, &["compos.object.read".to_owned()])
        .await
        .expect("application action");
    let binary = fixture
        .api
        .app
        .clone()
        .oneshot(application_request(
            "/api/v1/application-authz/check",
            json!({
                "action": "compos.object.read",
                "resource": {"provider": "compos", "type": "compos.object", "id": OBJECT_ID}
            }),
        ))
        .await
        .expect("binary application check");
    assert_eq!(binary.status(), StatusCode::OK);
    assert_eq!(
        support::response_json(binary).await,
        json!({"allowed": false, "reason": "application_consent_required"})
    );
    let check_body = json!({
        "action": "compos.object.read",
        "resource": object_resource(OBJECT_ID, &["/title", "/missing"])
    });

    let denied = fixture
        .api
        .app
        .clone()
        .oneshot(application_request(
            "/api/v1/application-authz/check",
            check_body.clone(),
        ))
        .await
        .expect("application denied");
    assert_eq!(denied.status(), StatusCode::OK);
    let denied = support::response_json(denied).await;
    assert_eq!(denied["allowed"], false);
    assert_eq!(
        denied["fieldViews"],
        json!({"/missing": "none", "/title": "none"})
    );

    fixture
        .api
        .db
        .replace_active_application_consent(
            &ApplicationGrant {
                id: "field-view-consent".to_owned(),
                user_id: fixture.user_id.clone(),
                application_id: APPLICATION_ID.to_owned(),
                organization_id: ORGANIZATION_ID.to_owned(),
                status: ApplicationGrantStatus::Active,
                consented_at: support::FIXED_TEST_TIME.to_owned(),
                revoked_at: None,
                revoked_by_user_id: None,
                created_at: support::FIXED_TEST_TIME.to_owned(),
                updated_at: support::FIXED_TEST_TIME.to_owned(),
            },
            &["compos.object.read".to_owned()],
        )
        .await
        .expect("application consent");
    let consented = fixture
        .api
        .app
        .clone()
        .oneshot(application_request(
            "/api/v1/application-authz/check",
            check_body.clone(),
        ))
        .await
        .expect("application consented");
    let consented = support::response_json(consented).await;
    assert_eq!(consented["allowed"], true);
    assert_eq!(consented["reason"], "explicit_consent");
    assert_eq!(
        consented["fieldViews"],
        json!({"/missing": "none", "/title": "anonymized"})
    );

    fixture
        .api
        .db
        .set_application_trust(ORGANIZATION_ID, APPLICATION_ID, true)
        .await
        .expect("trust application");
    let trusted = fixture
        .api
        .app
        .clone()
        .oneshot(application_request(
            "/api/v1/application-authz/batch-check",
            json!({
                "checks": [
                    check_body,
                    {
                        "action": "compos.object.read",
                        "resource": object_resource("denied-object", &["/title", "/missing"])
                    }
                ]
            }),
        ))
        .await
        .expect("trusted application batch");
    assert_eq!(trusted.status(), StatusCode::OK);
    let trusted = support::response_json(trusted).await;
    assert_eq!(trusted["results"].as_array().map(Vec::len), Some(2));
    assert_eq!(trusted["results"][0]["reason"], "trusted_application");
    assert_eq!(trusted["results"][0]["fieldViews"]["/title"], "anonymized");
    assert_eq!(trusted["results"][1]["allowed"], false);
    assert_eq!(trusted["results"][1]["fieldViews"]["/title"], "none");

    let filter = fixture
        .api
        .app
        .oneshot(application_request(
            "/api/v1/application-authz/filter",
            json!({
                "action": "compos.object.read",
                "resources": [object_resource(OBJECT_ID, &["/title"])]
            }),
        ))
        .await
        .expect("application filter view");
    assert_eq!(filter.status(), StatusCode::BAD_REQUEST);
}

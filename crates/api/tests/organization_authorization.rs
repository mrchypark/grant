mod support;

use axum::{
    Router,
    http::{Method, StatusCode},
};
use domain::Role;
use infra::{config::ReadConsistency, database::Database};
use serde_json::{Value, json};
use tower::ServiceExt;

async fn seed_creator_context(db: &Database) {
    db.upsert_organization_with_owner("creator-org", "Creator Org", None, "Creator Org")
        .await
        .expect("creator org");
    db.upsert_organization_with_owner("owner-org", "Owner Org", None, "Owner Org")
        .await
        .expect("owner org");
}

async fn bind_user_action(db: &Database, role_id: &str) {
    bind_user_action_for_org(db, role_id, "creator-org").await;
}

async fn bind_user_action_for_org(db: &Database, role_id: &str, organization_id: &str) {
    db.save_role_binding(&support::user_org_role_binding(
        role_id,
        organization_id,
        support::BOOTSTRAP_USER_ID,
    ))
    .await
    .expect("save user org binding");
}

async fn seed_org_permission_manage_role(db: &Database) {
    db.upsert_role(&Role {
        id: "role-org-permission-manage".to_owned(),
        organization_id: None,
        code: "organization.permission.manage".to_owned(),
        name: "Org Permission Manage".to_owned(),
        description: None,
    })
    .await
    .expect("upsert org permission manage role");
    db.upsert_role_action(
        "role-org-permission-manage",
        "organization.permission.manage",
    )
    .await
    .expect("upsert org permission manage action");
}

async fn compos_reader_fixture() -> support::TestApi {
    let api = support::TestApi::new().await;
    let db = api.db.clone();
    seed_creator_context(&db).await;
    seed_org_permission_manage_role(&db).await;
    bind_user_action(&db, "role-org-permission-manage").await;
    let app = api.app.clone();

    let provider = app
        .clone()
        .oneshot(support::master_json_request(
            Method::POST,
            "/api/v1/admin/providers",
            json!({
                "id": "compos",
                "name": "Compos",
                "description": "Object data provider",
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
        .expect("actions response");
    assert_eq!(actions.status(), StatusCode::OK);

    let role = app
        .clone()
        .oneshot(support::json_request(
            Method::POST,
            "/api/v1/orgs/creator-org/roles",
            "bootstrap-user",
            "creator-org",
            json!({
                "id": "role-compos-reader",
                "code": "compos.reader",
                "name": "Compos Reader",
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
            "/api/v1/orgs/creator-org/roles/role-compos-reader/actions",
            "bootstrap-user",
            "creator-org",
            json!({
                "actionIds": ["compos.object.read"]
            }),
        ))
        .await
        .expect("role actions response");
    assert_eq!(role_actions.status(), StatusCode::OK);

    let binding = app
        .clone()
        .oneshot(support::json_request(
            Method::POST,
            "/api/v1/orgs/creator-org/bindings",
            "bootstrap-user",
            "creator-org",
            json!({
                "id": "binding-compos-reader",
                "subjectType": "user",
                "subjectId": "bootstrap-user",
                "subjectOrganizationId": "creator-org",
                "providerId": "compos",
                "resourceType": "object_instance",
                "resourceId": "obj_1",
                "roleId": "role-compos-reader",
                "agreementId": null,
                "state": "active"
            }),
        ))
        .await
        .expect("binding response");
    assert_eq!(binding.status(), StatusCode::CREATED);

    api
}

async fn authz_check(
    app: &Router,
    action: &str,
    provider: &str,
    resource_type: &str,
    resource_id: &str,
) -> Value {
    let response = app
        .clone()
        .oneshot(support::master_json_request(
            Method::POST,
            "/api/v1/authz/check",
            json!({
                "subject": {"type": "user", "id": "bootstrap-user", "org_id": "creator-org"},
                "action": action,
                "resource": {"provider": provider, "type": resource_type, "id": resource_id}
            }),
        ))
        .await
        .expect("check response");
    assert_eq!(response.status(), StatusCode::OK);
    support::response_json(response).await
}

#[tokio::test]
async fn agreement_routes_require_permission_manage_and_round_trip() {
    let api = support::TestApi::new().await;
    let db = api.db.clone();
    seed_creator_context(&db).await;
    seed_org_permission_manage_role(&db).await;
    let app = api.app.clone();
    let create_body = json!({
        "id": "agreement-creator-owner",
        "agreementType": "delegation",
        "grantorOrganizationId": "creator-org",
        "granteeOrganizationId": "owner-org",
        "status": "draft",
        "title": "Creator Owner Agreement",
        "referenceNo": "AGR-1",
        "startsAt": "2026-07-01T00:00:00Z",
        "endsAt": null
    });

    let forbidden = app
        .clone()
        .oneshot(support::json_request(
            Method::POST,
            "/api/v1/orgs/creator-org/agreements",
            "bootstrap-user",
            "creator-org",
            create_body.clone(),
        ))
        .await
        .expect("forbidden response");
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);

    bind_user_action(&db, "role-org-permission-manage").await;
    let create = app
        .clone()
        .oneshot(support::json_request(
            Method::POST,
            "/api/v1/orgs/creator-org/agreements",
            "bootstrap-user",
            "creator-org",
            create_body,
        ))
        .await
        .expect("create response");
    assert_eq!(create.status(), StatusCode::CREATED);
    let created = support::response_json(create).await;
    assert_eq!(created["id"], "agreement-creator-owner");
    assert_eq!(created["status"], "draft");

    let list = app
        .clone()
        .oneshot(support::empty_request(
            Method::GET,
            "/api/v1/orgs/creator-org/agreements",
            "bootstrap-user",
            "creator-org",
        ))
        .await
        .expect("list response");
    assert_eq!(list.status(), StatusCode::OK);
    let listed = support::response_json(list).await;
    assert_eq!(listed.as_array().map(Vec::len), Some(1));

    let get = app
        .clone()
        .oneshot(support::empty_request(
            Method::GET,
            "/api/v1/orgs/creator-org/agreements/agreement-creator-owner",
            "bootstrap-user",
            "creator-org",
        ))
        .await
        .expect("get response");
    assert_eq!(get.status(), StatusCode::OK);

    let update = app
        .clone()
        .oneshot(support::json_request(
            Method::PUT,
            "/api/v1/orgs/creator-org/agreements/agreement-creator-owner",
            "bootstrap-user",
            "creator-org",
            json!({
                "id": "agreement-creator-owner",
                "agreementType": "delegation",
                "grantorOrganizationId": "creator-org",
                "granteeOrganizationId": "owner-org",
                "status": "active",
                "title": "Creator Owner Agreement",
                "referenceNo": "AGR-1",
                "startsAt": "2026-07-01T00:00:00Z",
                "endsAt": null
            }),
        ))
        .await
        .expect("update response");
    assert_eq!(update.status(), StatusCode::OK);
    let updated = support::response_json(update).await;
    assert_eq!(updated["status"], "active");

    bind_user_action_for_org(&db, "role-org-permission-manage", "owner-org").await;
    let grantee_extend = app
        .clone()
        .oneshot(support::json_request(
            Method::PUT,
            "/api/v1/orgs/owner-org/agreements/agreement-creator-owner",
            "bootstrap-user",
            "owner-org",
            json!({
                "id": "agreement-creator-owner",
                "agreementType": "delegation",
                "grantorOrganizationId": "creator-org",
                "granteeOrganizationId": "owner-org",
                "status": "active",
                "title": "Creator Owner Agreement",
                "referenceNo": "AGR-1",
                "startsAt": "2026-07-01T00:00:00Z",
                "endsAt": "2027-07-01T00:00:00Z"
            }),
        ))
        .await
        .expect("grantee extend response");
    assert_eq!(grantee_extend.status(), StatusCode::FORBIDDEN);

    let rehome = app
        .oneshot(support::json_request(
            Method::PUT,
            "/api/v1/orgs/creator-org/agreements/agreement-creator-owner",
            "bootstrap-user",
            "creator-org",
            json!({
                "id": "agreement-creator-owner",
                "agreementType": "delegation",
                "grantorOrganizationId": "owner-org",
                "granteeOrganizationId": "creator-org",
                "status": "active",
                "title": "Rehome Attempt",
                "referenceNo": "AGR-1",
                "startsAt": "2026-07-01T00:00:00Z",
                "endsAt": null
            }),
        ))
        .await
        .expect("rehome response");
    assert_eq!(rehome.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn grantee_cannot_self_issue_cross_org_agreement() {
    let api = support::TestApi::new().await;
    let db = api.db.clone();
    seed_creator_context(&db).await;
    seed_org_permission_manage_role(&db).await;
    bind_user_action_for_org(&db, "role-org-permission-manage", "owner-org").await;
    let app = api.app.clone();

    let response = app
        .oneshot(support::json_request(
            Method::POST,
            "/api/v1/orgs/owner-org/agreements",
            "bootstrap-user",
            "owner-org",
            json!({
                "id": "agreement-self-issued",
                "agreementType": "delegation",
                "grantorOrganizationId": "creator-org",
                "granteeOrganizationId": "owner-org",
                "status": "active",
                "title": "Self Issued Agreement",
                "referenceNo": null,
                "startsAt": "2026-07-01T00:00:00Z",
                "endsAt": null
            }),
        ))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn role_create_cannot_rehome_existing_role_id() {
    let api = support::TestApi::new().await;
    let db = api.db.clone();
    seed_creator_context(&db).await;
    seed_org_permission_manage_role(&db).await;
    bind_user_action(&db, "role-org-permission-manage").await;
    bind_user_action_for_org(&db, "role-org-permission-manage", "owner-org").await;
    let app = api.app.clone();

    let create = app
        .clone()
        .oneshot(support::json_request(
            Method::POST,
            "/api/v1/orgs/creator-org/roles",
            "bootstrap-user",
            "creator-org",
            json!({
                "id": "role-shared-id",
                "code": "creator.shared",
                "name": "Creator Shared",
                "description": null
            }),
        ))
        .await
        .expect("create response");
    assert_eq!(create.status(), StatusCode::CREATED);

    let rehome = app
        .oneshot(support::json_request(
            Method::POST,
            "/api/v1/orgs/owner-org/roles",
            "bootstrap-user",
            "owner-org",
            json!({
                "id": "role-shared-id",
                "code": "owner.shared",
                "name": "Owner Shared",
                "description": null
            }),
        ))
        .await
        .expect("rehome response");
    assert_eq!(rehome.status(), StatusCode::CONFLICT);

    let stored = db
        .get_role_with_consistency("role-shared-id", Some(ReadConsistency::Strong))
        .await
        .expect("stored role");
    assert_eq!(stored.organization_id.as_deref(), Some("creator-org"));
    assert_eq!(stored.code, "creator.shared");
}

#[tokio::test]
async fn authz_check_and_filter_use_role_binding() {
    let api = compos_reader_fixture().await;
    let app = api.app.clone();

    let checked = authz_check(
        &app,
        "compos.object.read",
        "compos",
        "object_instance",
        "obj_1",
    )
    .await;
    assert_eq!(checked["allowed"], true);
    assert_eq!(checked["reason"], "role_binding");

    let filter = app
        .clone()
        .oneshot(support::master_json_request(
            Method::POST,
            "/api/v1/authz/filter",
            json!({
                "subject": {"type": "user", "id": "bootstrap-user", "org_id": "creator-org"},
                "action": "compos.object.read",
                "resources": [
                    {"provider": "compos", "type": "object_instance", "id": "obj_1"},
                    {"provider": "compos", "type": "object_instance", "id": "obj_2"}
                ]
            }),
        ))
        .await
        .expect("filter response");
    assert_eq!(filter.status(), StatusCode::OK);
    let filtered = support::response_json(filter).await;
    assert_eq!(filtered["resource_ids"], json!(["obj_1"]));

    let batch = app
        .oneshot(support::master_json_request(
            Method::POST,
            "/api/v1/authz/batch-check",
            json!({
                "checks": [
                    {
                        "subject": {"type": "user", "id": "bootstrap-user", "org_id": "creator-org"},
                        "action": "compos.object.read",
                        "resource": {"provider": "compos", "type": "object_instance", "id": "obj_1"}
                    },
                    {
                        "subject": {"type": "user", "id": "bootstrap-user", "org_id": "creator-org"},
                        "action": "compos.object.read",
                        "resource": {"provider": "compos", "type": "object_instance", "id": "obj_2"}
                    }
                ]
            }),
        ))
        .await
        .expect("batch response");
    assert_eq!(batch.status(), StatusCode::OK);
    let batched = support::response_json(batch).await;
    assert_eq!(batched["results"][0]["allowed"], true);
    assert_eq!(batched["results"][1]["allowed"], false);
}

#[tokio::test]
async fn duplicate_provider_manifest_is_rejected_without_deleting_existing_actions() {
    let api = compos_reader_fixture().await;
    let app = api.app.clone();

    let duplicate_manifest = app
        .clone()
        .oneshot(support::master_json_request(
            Method::PUT,
            "/api/v1/admin/providers/compos/actions",
            json!({
                "actions": [
                    {
                        "id": "compos.object.write",
                        "code": "compos.object.write",
                        "name": "Write Compos Object",
                        "description": null,
                        "riskLevel": "medium",
                        "visibilityGranting": false,
                        "grantableToApplications": true,
                        "consentLabel": "Write Compos Object",
                        "consentDescription": null
                    },
                    {
                        "id": "compos.object.write",
                        "code": "compos.object.write",
                        "name": "Write Compos Object Duplicate",
                        "description": null,
                        "riskLevel": "medium",
                        "visibilityGranting": false,
                        "grantableToApplications": true,
                        "consentLabel": "Write Compos Object Duplicate",
                        "consentDescription": null
                    }
                ]
            }),
        ))
        .await
        .expect("duplicate manifest response");
    assert_eq!(duplicate_manifest.status(), StatusCode::BAD_REQUEST);
    let duplicate_payload = support::response_json(duplicate_manifest).await;
    assert_eq!(
        duplicate_payload["error"],
        "duplicate action code in provider manifest: compos.object.write"
    );

    let preserved = authz_check(
        &app,
        "compos.object.read",
        "compos",
        "object_instance",
        "obj_1",
    )
    .await;
    assert_eq!(preserved["allowed"], true);
}

#[tokio::test]
async fn grant_provider_action_does_not_authorize_external_provider_resource() {
    let api = compos_reader_fixture().await;
    let app = api.app.clone();

    let grant_action_role = app
        .clone()
        .oneshot(support::json_request(
            Method::POST,
            "/api/v1/orgs/creator-org/roles",
            "bootstrap-user",
            "creator-org",
            json!({
                "id": "role-grant-action-on-compos",
                "code": "grant.action.on.compos",
                "name": "Grant Action On Compos",
                "description": null
            }),
        ))
        .await
        .expect("grant action role response");
    assert_eq!(grant_action_role.status(), StatusCode::CREATED);
    let grant_role_actions = app
        .clone()
        .oneshot(support::json_request(
            Method::PUT,
            "/api/v1/orgs/creator-org/roles/role-grant-action-on-compos/actions",
            "bootstrap-user",
            "creator-org",
            json!({
                "actionIds": ["organization.profile.read"]
            }),
        ))
        .await
        .expect("grant role actions response");
    assert_eq!(grant_role_actions.status(), StatusCode::OK);
    let provider_mismatch_binding = app
        .clone()
        .oneshot(support::json_request(
            Method::POST,
            "/api/v1/orgs/creator-org/bindings",
            "bootstrap-user",
            "creator-org",
            json!({
                "id": "binding-provider-mismatch",
                "subjectType": "user",
                "subjectId": "bootstrap-user",
                "subjectOrganizationId": "creator-org",
                "providerId": "compos",
                "resourceType": "object_instance",
                "resourceId": "obj_2",
                "roleId": "role-grant-action-on-compos",
                "agreementId": null,
                "state": "active"
            }),
        ))
        .await
        .expect("provider mismatch binding response");
    assert_eq!(provider_mismatch_binding.status(), StatusCode::CREATED);
    let provider_mismatch_check = app
        .clone()
        .oneshot(support::master_json_request(
            Method::POST,
            "/api/v1/authz/check",
            json!({
                "subject": {"type": "user", "id": "bootstrap-user", "org_id": "creator-org"},
                "action": "organization.profile.read",
                "resource": {"provider": "compos", "type": "object_instance", "id": "obj_2"}
            }),
        ))
        .await
        .expect("provider mismatch check response");
    assert_eq!(provider_mismatch_check.status(), StatusCode::OK);
    let provider_mismatch = support::response_json(provider_mismatch_check).await;
    assert_eq!(provider_mismatch["allowed"], false);
}

#[tokio::test]
async fn replacing_provider_manifest_with_empty_actions_revokes_authz() {
    let api = compos_reader_fixture().await;
    let app = api.app.clone();

    let replace_with_empty_manifest = app
        .clone()
        .oneshot(support::master_json_request(
            Method::PUT,
            "/api/v1/admin/providers/compos/actions",
            json!({
                "actions": []
            }),
        ))
        .await
        .expect("empty manifest response");
    assert_eq!(replace_with_empty_manifest.status(), StatusCode::OK);
    let removed = authz_check(
        &app,
        "compos.object.read",
        "compos",
        "object_instance",
        "obj_1",
    )
    .await;
    assert_eq!(removed["allowed"], false);
}

#[tokio::test]
async fn authz_batch_check_rejects_too_many_checks() {
    let api = support::TestApi::new().await;
    let app = api.app.clone();
    let checks = (0..1001)
        .map(|index| {
            json!({
                "subject": {"type": "user", "id": "bootstrap-user", "org_id": "creator-org"},
                "action": "compos.object.read",
                "resource": {"provider": "compos", "type": "object_instance", "id": format!("obj_{index}")}
            })
        })
        .collect::<Vec<_>>();

    let response = app
        .oneshot(support::master_json_request(
            Method::POST,
            "/api/v1/authz/batch-check",
            json!({ "checks": checks }),
        ))
        .await
        .expect("batch check response");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let payload = support::response_json(response).await;
    assert_eq!(payload["error"], "checks must contain at most 1000 items");
}

#[tokio::test]
async fn authz_filter_rejects_too_many_resources() {
    let api = support::TestApi::new().await;
    let app = api.app.clone();
    let resources = (0..1001)
        .map(|index| {
            json!({
                "provider": "compos",
                "type": "object_instance",
                "id": format!("obj_{index}")
            })
        })
        .collect::<Vec<_>>();

    let response = app
        .oneshot(support::master_json_request(
            Method::POST,
            "/api/v1/authz/filter",
            json!({
                "subject": {"type": "user", "id": "bootstrap-user", "org_id": "creator-org"},
                "action": "compos.object.read",
                "resources": resources
            }),
        ))
        .await
        .expect("filter response");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let payload = support::response_json(response).await;
    assert_eq!(
        payload["error"],
        "resources must contain at most 1000 items"
    );
}

#[tokio::test]
async fn deactivate_organization_requires_permission_manage() {
    let api = support::TestApi::new().await;
    let db = api.db.clone();
    seed_creator_context(&db).await;
    seed_org_permission_manage_role(&db).await;
    let app = api.app.clone();

    let response = app
        .oneshot(support::empty_post_request(
            "/api/v1/orgs/creator-org/deactivate",
            "bootstrap-user",
            "creator-org",
        ))
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn deactivate_and_reactivate_organization_updates_current_user_membership_view() {
    let api = support::TestApi::new().await;
    let db = api.db.clone();
    seed_creator_context(&db).await;
    seed_org_permission_manage_role(&db).await;
    bind_user_action(&db, "role-org-permission-manage").await;
    let app = api.app.clone();

    let deactivate = app
        .clone()
        .oneshot(support::empty_post_request(
            "/api/v1/orgs/creator-org/deactivate",
            "bootstrap-user",
            "creator-org",
        ))
        .await
        .expect("deactivate response");
    assert_eq!(deactivate.status(), StatusCode::OK);

    let current_after_deactivate = app
        .clone()
        .oneshot(support::auth_empty_request(
            Method::GET,
            "/api/v1/me",
            "bootstrap-user",
        ))
        .await
        .expect("current user after deactivate");
    assert_eq!(current_after_deactivate.status(), StatusCode::OK);
    let current_after_deactivate_json = support::response_json(current_after_deactivate).await;
    let organizations = current_after_deactivate_json["organizations"]
        .as_array()
        .expect("organizations array");
    assert!(
        organizations
            .iter()
            .all(|item| item["id"].as_str() != Some("creator-org")),
        "deactivated organization should disappear from active organization list"
    );

    let reactivate = app
        .clone()
        .oneshot(support::empty_post_request(
            "/api/v1/orgs/creator-org/reactivate",
            "bootstrap-user",
            "creator-org",
        ))
        .await
        .expect("reactivate response");
    assert_eq!(reactivate.status(), StatusCode::OK);

    let current_after_reactivate = app
        .clone()
        .oneshot(support::empty_request(
            Method::GET,
            "/api/v1/me",
            "bootstrap-user",
            "creator-org",
        ))
        .await
        .expect("current user after reactivate");
    assert_eq!(current_after_reactivate.status(), StatusCode::OK);
    let current_after_reactivate_json = support::response_json(current_after_reactivate).await;
    let organizations = current_after_reactivate_json["organizations"]
        .as_array()
        .expect("organizations array");
    assert!(
        organizations
            .iter()
            .any(|item| item["id"].as_str() == Some("creator-org")),
        "reactivated organization should reappear in active organization list"
    );
}

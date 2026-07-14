mod support;

use axum::http::{Method, StatusCode};
use domain::{RoleBinding, RoleBindingState, RoleBindingSubjectType};
use serde_json::json;
use tower::ServiceExt;

#[tokio::test]
async fn group_crud_membership_scope_and_bound_delete_are_enforced() {
    let test = support::TestApi::new().await;
    let app = test.app.clone();
    let target = support::seed_identity(
        &test.db,
        "group-member",
        Some("group-member@rauthy.test"),
        "Group Member",
        true,
    )
    .await;
    support::seed_org_user_membership(
        &test.runtime,
        "membership-group-member",
        "bootstrap-user",
        &target.id,
    )
    .await;
    let outsider = support::seed_identity(
        &test.db,
        "group-outsider",
        Some("group-outsider@rauthy.test"),
        "Group Outsider",
        true,
    )
    .await;

    let created = app
        .clone()
        .oneshot(support::json_request(
            Method::POST,
            "/api/v1/orgs/bootstrap-user/groups",
            "bootstrap-user",
            "bootstrap-user",
            json!({ "id": "group-a", "name": "Operators", "description": null }),
        ))
        .await
        .expect("create group");
    let created_status = created.status();
    let created_body = support::response_text(created).await;
    assert_eq!(
        created_status,
        StatusCode::CREATED,
        "response: {created_body}"
    );

    let member = app
        .clone()
        .oneshot(support::empty_request(
            Method::PUT,
            &format!(
                "/api/v1/orgs/bootstrap-user/groups/group-a/members/{}",
                target.id
            ),
            "bootstrap-user",
            "bootstrap-user",
        ))
        .await
        .expect("add member");
    assert_eq!(member.status(), StatusCode::OK);

    let outsider_add = app
        .clone()
        .oneshot(support::empty_request(
            Method::PUT,
            &format!(
                "/api/v1/orgs/bootstrap-user/groups/group-a/members/{}",
                outsider.id
            ),
            "bootstrap-user",
            "bootstrap-user",
        ))
        .await
        .expect("add outsider");
    assert_eq!(outsider_add.status(), StatusCode::FORBIDDEN);

    let members = app
        .clone()
        .oneshot(support::empty_request(
            Method::GET,
            "/api/v1/orgs/bootstrap-user/groups/group-a/members?consistency=strong",
            "bootstrap-user",
            "bootstrap-user",
        ))
        .await
        .expect("list members");
    assert_eq!(members.status(), StatusCode::OK);

    test.db
        .save_role_binding(&RoleBinding {
            id: "binding-group-a".to_owned(),
            subject_type: RoleBindingSubjectType::Group,
            subject_id: "group-a".to_owned(),
            subject_organization_id: "bootstrap-user".to_owned(),
            provider_id: "grant".to_owned(),
            resource_type: "organization".to_owned(),
            resource_id: "bootstrap-user".to_owned(),
            role_id: "role-system-bootstrap-owner".to_owned(),
            agreement_id: None,
            state: RoleBindingState::Active,
            created_by_user_id: "bootstrap-user".to_owned(),
            created_at: support::FIXED_TEST_TIME.to_owned(),
            updated_at: support::FIXED_TEST_TIME.to_owned(),
        })
        .await
        .expect("group binding");

    let bound_delete = app
        .clone()
        .oneshot(support::empty_request(
            Method::DELETE,
            "/api/v1/orgs/bootstrap-user/groups/group-a",
            "bootstrap-user",
            "bootstrap-user",
        ))
        .await
        .expect("bound delete");
    assert_eq!(bound_delete.status(), StatusCode::CONFLICT);

    test.db
        .delete_role_binding("binding-group-a")
        .await
        .expect("delete binding");
    let removed = app
        .clone()
        .oneshot(support::empty_request(
            Method::DELETE,
            &format!(
                "/api/v1/orgs/bootstrap-user/groups/group-a/members/{}",
                target.id
            ),
            "bootstrap-user",
            "bootstrap-user",
        ))
        .await
        .expect("remove member");
    assert_eq!(removed.status(), StatusCode::NO_CONTENT);
    let deleted = app
        .clone()
        .oneshot(support::empty_request(
            Method::DELETE,
            "/api/v1/orgs/bootstrap-user/groups/group-a",
            "bootstrap-user",
            "bootstrap-user",
        ))
        .await
        .expect("delete group");
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);

    let recreated = app
        .oneshot(support::json_request(
            Method::POST,
            "/api/v1/orgs/bootstrap-user/groups",
            "bootstrap-user",
            "bootstrap-user",
            json!({ "id": "group-a", "name": "Operators", "description": "new" }),
        ))
        .await
        .expect("recreate group");
    assert_eq!(recreated.status(), StatusCode::CREATED);
}

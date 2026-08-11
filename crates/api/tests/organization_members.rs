mod support;

use axum::http::{Method, StatusCode};
use hiqlite::Row;
use infra::database::OrganizationMembershipWriteError;
use serde_json::Value;
use tower::ServiceExt;

#[derive(Debug)]
struct MembershipStateRow {
    status: String,
}

impl From<&mut Row<'_>> for MembershipStateRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            status: row.get("status"),
        }
    }
}

async fn enroll(
    test: &support::TestApi,
    caller_token: &str,
    header_organization_id: &str,
    path_organization_id: &str,
    target_user_id: &str,
) -> (StatusCode, Value) {
    let response = test
        .app
        .clone()
        .oneshot(support::empty_request(
            Method::PUT,
            &format!("/api/v1/orgs/{path_organization_id}/members/{target_user_id}"),
            caller_token,
            header_organization_id,
        ))
        .await
        .expect("organization membership response");
    let status = response.status();
    let body = support::response_json(response).await;
    (status, body)
}

#[tokio::test]
async fn owner_enrolls_active_members_idempotently_without_privilege_escalation() {
    let test = support::TestApi::new().await;
    let allowed = support::seed_identity(
        &test.db,
        "organization-member-allowed",
        Some("organization-member-allowed@rauthy.test"),
        "Allowed Member",
    )
    .await;
    let denied = support::seed_identity(
        &test.db,
        "organization-member-denied",
        Some("organization-member-denied@rauthy.test"),
        "Denied Member",
    )
    .await;
    let existing_admin = support::seed_identity(
        &test.db,
        "organization-existing-admin",
        Some("organization-existing-admin@rauthy.test"),
        "Existing Admin",
    )
    .await;
    let existing_owner = support::seed_identity(
        &test.db,
        "organization-existing-owner",
        Some("organization-existing-owner@rauthy.test"),
        "Existing Owner",
    )
    .await;
    test.runtime
        .execute(
            "INSERT INTO organization_memberships (id, organization_id, user_id, membership_role, status, created_at, updated_at)
             VALUES ($1, $2, $3, 'admin', 'active', $4, $5)",
            vec![
                "membership-existing-admin".into(),
                "bootstrap-user".into(),
                existing_admin.id.clone().into(),
                support::FIXED_TEST_TIME.into(),
                support::FIXED_TEST_TIME.into(),
            ],
        )
        .await
        .expect("seed existing admin membership");
    test.runtime
        .execute(
            "INSERT INTO organization_memberships (id, organization_id, user_id, membership_role, status, created_at, updated_at)
             VALUES ($1, $2, $3, 'owner', 'active', $4, $5)",
            vec![
                "membership-existing-owner".into(),
                "bootstrap-user".into(),
                existing_owner.id.clone().into(),
                support::FIXED_TEST_TIME.into(),
                support::FIXED_TEST_TIME.into(),
            ],
        )
        .await
        .expect("seed existing owner membership");

    let (first_status, first) = enroll(
        &test,
        "bootstrap-user",
        "bootstrap-user",
        "bootstrap-user",
        &allowed.id,
    )
    .await;
    assert_eq!(first_status, StatusCode::OK, "response: {first}");
    assert_eq!(first["organizationId"], "bootstrap-user");
    assert_eq!(first["userId"], allowed.id);
    assert_eq!(first["membershipRole"], "member");
    assert_eq!(first["status"], "active");

    test.runtime
        .execute(
            "UPDATE organization_memberships
             SET status = 'disabled'
             WHERE organization_id = $1 AND user_id = $2",
            vec!["bootstrap-user".into(), allowed.id.clone().into()],
        )
        .await
        .expect("disable member membership");
    let (reactivated_status, reactivated) = enroll(
        &test,
        "bootstrap-user",
        "bootstrap-user",
        "bootstrap-user",
        &allowed.id,
    )
    .await;
    assert_eq!(
        reactivated_status,
        StatusCode::OK,
        "response: {reactivated}"
    );
    assert_eq!(reactivated["id"], first["id"]);
    assert_eq!(reactivated["membershipRole"], "member");
    assert_eq!(reactivated["status"], "active");

    let (second_status, second) = enroll(
        &test,
        "bootstrap-user",
        "bootstrap-user",
        "bootstrap-user",
        &allowed.id,
    )
    .await;
    assert_eq!(second_status, StatusCode::OK, "response: {second}");
    assert_eq!(second["id"], first["id"]);

    let (denied_status, denied_member) = enroll(
        &test,
        "bootstrap-user",
        "bootstrap-user",
        "bootstrap-user",
        &denied.id,
    )
    .await;
    assert_eq!(denied_status, StatusCode::OK, "response: {denied_member}");

    let (unprivileged_status, _) = enroll(
        &test,
        "organization-member-denied",
        "bootstrap-user",
        "bootstrap-user",
        &allowed.id,
    )
    .await;
    assert_eq!(unprivileged_status, StatusCode::FORBIDDEN);

    let (admin_status, _) = enroll(
        &test,
        "bootstrap-user",
        "bootstrap-user",
        "bootstrap-user",
        &existing_admin.id,
    )
    .await;
    assert_eq!(admin_status, StatusCode::CONFLICT);

    let (owner_status, _) = enroll(
        &test,
        "bootstrap-user",
        "bootstrap-user",
        "bootstrap-user",
        &existing_owner.id,
    )
    .await;
    assert_eq!(owner_status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn enrollment_fails_closed_for_unknown_or_inactive_targets_and_scope_mismatch() {
    let test = support::TestApi::new().await;
    let inactive = support::seed_identity(
        &test.db,
        "organization-member-inactive",
        Some("organization-member-inactive@rauthy.test"),
        "Inactive Member",
    )
    .await;
    test.db
        .enroll_active_member("bootstrap-user", &inactive.id)
        .await
        .expect("seed target member before disabling it");
    test.runtime
        .execute(
            "UPDATE organization_memberships
             SET status = 'disabled'
             WHERE organization_id = $1 AND user_id = $2",
            vec!["bootstrap-user".into(), inactive.id.clone().into()],
        )
        .await
        .expect("disable target membership");
    test.db
        .set_user_active(&inactive.id, false)
        .await
        .expect("disable target user");
    test.db
        .upsert_organization_with_owner(
            "different-organization",
            "Different Organization",
            None,
            "Different Organization",
        )
        .await
        .expect("seed caller membership in second organization");

    let (unknown_status, _) = enroll(
        &test,
        "bootstrap-user",
        "bootstrap-user",
        "bootstrap-user",
        "unknown-user",
    )
    .await;
    assert_eq!(unknown_status, StatusCode::NOT_FOUND);

    let (inactive_status, _) = enroll(
        &test,
        "bootstrap-user",
        "bootstrap-user",
        "bootstrap-user",
        &inactive.id,
    )
    .await;
    assert_eq!(inactive_status, StatusCode::FORBIDDEN);
    let inactive_membership: Vec<MembershipStateRow> = test
        .runtime
        .query_rows(
            infra::config::ReadConsistency::Strong,
            "SELECT status FROM organization_memberships
             WHERE organization_id = $1 AND user_id = $2",
            vec!["bootstrap-user".into(), inactive.id.clone().into()],
        )
        .await
        .expect("read disabled membership after rejected enrollment");
    assert_eq!(inactive_membership.len(), 1);
    assert_eq!(inactive_membership[0].status, "disabled");

    let (mismatch_status, _) = enroll(
        &test,
        "bootstrap-user",
        "different-organization",
        "bootstrap-user",
        &inactive.id,
    )
    .await;
    assert_eq!(mismatch_status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn database_enrollment_predicate_rejects_inactive_target_or_organization_without_writing() {
    let test = support::TestApi::new().await;
    let target = support::seed_identity(
        &test.db,
        "database-predicate-target",
        Some("database-predicate-target@rauthy.test"),
        "Database Predicate Target",
    )
    .await;
    test.db
        .enroll_active_member("bootstrap-user", &target.id)
        .await
        .expect("initial active enrollment");
    test.db
        .set_user_active(&target.id, false)
        .await
        .expect("disable target after initial enrollment");
    let inactive_target = test
        .db
        .enroll_active_member("bootstrap-user", &target.id)
        .await
        .expect_err("inactive target must not reactivate membership");
    assert_eq!(
        inactive_target.downcast_ref::<OrganizationMembershipWriteError>(),
        Some(&OrganizationMembershipWriteError::TargetUserInactive)
    );

    test.db
        .upsert_organization_with_owner(
            "inactive-organization",
            "Inactive Organization",
            None,
            "Inactive Organization",
        )
        .await
        .expect("seed inactive organization");
    test.runtime
        .execute(
            "UPDATE organizations SET inactivated_at = $1 WHERE id = $2",
            vec![
                support::FIXED_TEST_TIME.into(),
                "inactive-organization".into(),
            ],
        )
        .await
        .expect("inactivate organization");
    let active_target = support::seed_identity(
        &test.db,
        "database-predicate-active-target",
        Some("database-predicate-active-target@rauthy.test"),
        "Database Predicate Active Target",
    )
    .await;
    let inactive_organization = test
        .db
        .enroll_active_member("inactive-organization", &active_target.id)
        .await
        .expect_err("inactive organization must not create membership");
    assert_eq!(
        inactive_organization.downcast_ref::<OrganizationMembershipWriteError>(),
        Some(&OrganizationMembershipWriteError::OrganizationInactive)
    );
    let inactive_organization_memberships: Vec<MembershipStateRow> = test
        .runtime
        .query_rows(
            infra::config::ReadConsistency::Strong,
            "SELECT status FROM organization_memberships
             WHERE organization_id = $1 AND user_id = $2",
            vec!["inactive-organization".into(), active_target.id.into()],
        )
        .await
        .expect("read inactive organization membership after rejected enrollment");
    assert!(inactive_organization_memberships.is_empty());
}

#[tokio::test]
async fn learner_rejects_organization_membership_enrollment() {
    let test = support::TestApi::learner().await;
    let response = test
        .app
        .oneshot(support::empty_request(
            Method::PUT,
            "/api/v1/orgs/bootstrap-user/members/target-user",
            "bootstrap-user",
            "bootstrap-user",
        ))
        .await
        .expect("learner response");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn enrolled_member_can_receive_an_exact_exposure_binding_while_unbound_member_is_denied() {
    let test = support::TestApi::new().await;
    let allowed = support::seed_identity(
        &test.db,
        "organization-binding-allowed",
        Some("organization-binding-allowed@rauthy.test"),
        "Allowed Binding Member",
    )
    .await;
    let denied = support::seed_identity(
        &test.db,
        "organization-binding-denied",
        Some("organization-binding-denied@rauthy.test"),
        "Denied Binding Member",
    )
    .await;
    for target in [&allowed, &denied] {
        let (status, body) = enroll(
            &test,
            "bootstrap-user",
            "bootstrap-user",
            "bootstrap-user",
            &target.id,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "response: {body}");
    }

    let provider = test
        .app
        .clone()
        .oneshot(support::master_json_request(
            Method::POST,
            "/api/v1/admin/providers",
            serde_json::json!({
                "id": "exposure", "name": "Exposure", "description": null, "active": true
            }),
        ))
        .await
        .expect("provider response");
    assert_eq!(provider.status(), StatusCode::CREATED);
    let action = test
        .app
        .clone()
        .oneshot(support::master_json_request(
            Method::PUT,
            "/api/v1/admin/providers/exposure/actions",
            serde_json::json!({"actions": [{
                "id": "exposure.read", "code": "exposure.read", "name": "Read Exposure",
                "description": null, "riskLevel": "low", "visibilityGranting": false,
                "grantableToApplications": true, "consentLabel": null, "consentDescription": null
            }]}),
        ))
        .await
        .expect("action response");
    assert_eq!(action.status(), StatusCode::OK);
    let role = test
        .app
        .clone()
        .oneshot(support::json_request(
            Method::POST,
            "/api/v1/orgs/bootstrap-user/roles",
            "bootstrap-user",
            "bootstrap-user",
            serde_json::json!({
                "id": "role-membership-exposure-reader", "code": "membership.exposure.reader",
                "name": "Membership Exposure Reader", "description": null
            }),
        ))
        .await
        .expect("role response");
    assert_eq!(role.status(), StatusCode::CREATED);
    let role_actions = test
        .app
        .clone()
        .oneshot(support::json_request(
            Method::PUT,
            "/api/v1/orgs/bootstrap-user/roles/role-membership-exposure-reader/actions",
            "bootstrap-user",
            "bootstrap-user",
            serde_json::json!({"actionIds": ["exposure.read"]}),
        ))
        .await
        .expect("role actions response");
    assert_eq!(role_actions.status(), StatusCode::OK);
    let binding = test
        .app
        .clone()
        .oneshot(support::json_request(
            Method::POST,
            "/api/v1/orgs/bootstrap-user/bindings",
            "bootstrap-user",
            "bootstrap-user",
            serde_json::json!({
                "id": "binding-membership-exposure-reader", "subjectType": "user",
                "subjectId": allowed.id, "subjectOrganizationId": "bootstrap-user",
                "providerId": "exposure", "resourceType": "customer-profile",
                "resourceId": "customer-123", "roleId": "role-membership-exposure-reader",
                "agreementId": null, "state": "active"
            }),
        ))
        .await
        .expect("binding response");
    assert_eq!(binding.status(), StatusCode::CREATED);

    for (subject, expected_allowed) in [(&allowed, true), (&denied, false)] {
        let response = test
            .app
            .clone()
            .oneshot(support::master_json_request(
                Method::POST,
                "/api/v1/authz/check?consistency=strong",
                serde_json::json!({
                    "subject": {"type": "user", "id": subject.id, "org_id": "bootstrap-user"},
                    "action": "exposure.read",
                    "resource": {"provider": "exposure", "type": "customer-profile", "id": "customer-123"}
                }),
            ))
            .await
            .expect("authorization response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = support::response_json(response).await;
        assert_eq!(body["allowed"], expected_allowed, "response: {body}");
    }
}

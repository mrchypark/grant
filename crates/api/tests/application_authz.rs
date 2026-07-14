mod support;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
};
use domain::{
    Application, ApplicationAppType, ApplicationConsentPolicy, ApplicationProvisioningStatus,
    ApplicationType, Role, RoleBinding, RoleBindingState, RoleBindingSubjectType,
};
use serde_json::json;
use tower::ServiceExt;

fn application_authz_request(token: &str) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri("/api/v1/application-authz/check")
        .header("content-type", "application/json")
        .header("authorization", support::auth::bearer_header(token))
        .header("x-organization-id", "bootstrap-user")
        .header("x-grant-app-auth-key", support::APP_AUTH_KEY)
        .body(Body::from(
            json!({
                "action": "organization.profile.read",
                "resource": {
                    "provider": "grant",
                    "type": "organization",
                    "id": "bootstrap-user"
                }
            })
            .to_string(),
        ))
        .expect("request")
}

#[tokio::test]
async fn application_authz_derives_client_and_subject_and_requires_consent_or_trust() {
    let test = support::TestApi::new().await;
    let token_user = support::seed_identity(
        &test.db,
        "bootstrap-user",
        Some("bootstrap-user@rauthy.test"),
        "Bootstrap User",
        true,
    )
    .await;
    if !test
        .db
        .has_active_organization_membership(&token_user.id, "bootstrap-user")
        .await
        .expect("membership check")
    {
        support::seed_org_user_membership(
            &test.runtime,
            "membership-app-authz-token-user",
            "bootstrap-user",
            &token_user.id,
        )
        .await;
    }
    test.db
        .upsert_role(&Role {
            id: "role-app-authz-profile-read".to_owned(),
            organization_id: Some("bootstrap-user".to_owned()),
            code: "app-authz-profile-read".to_owned(),
            name: "App Authz Profile Read".to_owned(),
            description: None,
        })
        .await
        .expect("role");
    test.db
        .upsert_role_action("role-app-authz-profile-read", "organization.profile.read")
        .await
        .expect("role action");
    test.db
        .save_role_binding(&RoleBinding {
            id: "binding-app-authz-profile-read".to_owned(),
            subject_type: RoleBindingSubjectType::User,
            subject_id: token_user.id.clone(),
            subject_organization_id: "bootstrap-user".to_owned(),
            provider_id: "grant".to_owned(),
            resource_type: "organization".to_owned(),
            resource_id: "bootstrap-user".to_owned(),
            role_id: "role-app-authz-profile-read".to_owned(),
            agreement_id: None,
            state: RoleBindingState::Active,
            created_by_user_id: "bootstrap-user".to_owned(),
            created_at: support::FIXED_TEST_TIME.to_owned(),
            updated_at: support::FIXED_TEST_TIME.to_owned(),
        })
        .await
        .expect("binding");
    test.db
        .insert_application(&Application {
            id: "application-authz".to_owned(),
            organization_id: "bootstrap-user".to_owned(),
            slug: "application-authz".to_owned(),
            name: "Application Authz".to_owned(),
            description: None,
            application_type: ApplicationType::Confidential,
            app_type: ApplicationAppType::Server,
            first_party: false,
            consent_policy: ApplicationConsentPolicy::Explicit,
            active: true,
            provisioning_status: ApplicationProvisioningStatus::Unprovisioned,
            rauthy_client_id: Some("application-authz".to_owned()),
            created_by_user_id: "bootstrap-user".to_owned(),
        })
        .await
        .expect("application");
    test.db
        .replace_application_actions(
            "application-authz",
            &["organization.profile.read".to_owned()],
        )
        .await
        .expect("application actions");

    let consent = test
        .app
        .clone()
        .oneshot(support::json_request(
            Method::PUT,
            "/api/v1/orgs/bootstrap-user/apps/application-authz/consent",
            "bootstrap-user",
            "bootstrap-user",
            json!({ "actionIds": ["organization.profile.read"] }),
        ))
        .await
        .expect("consent");
    assert_eq!(consent.status(), StatusCode::OK);
    let consent = support::response_json(consent).await;
    assert_eq!(consent["grant"]["userId"], token_user.id);
    assert_eq!(consent["actions"].as_array().map(Vec::len), Some(1));
    assert_eq!(
        consent["actions"][0]["id"], "organization.profile.read",
        "consent: {consent}"
    );
    assert!(
        !test
            .db
            .has_application_consent_for_action(
                &token_user.id,
                "bootstrap-user",
                "application-authz",
                "organization.profile.read",
            )
            .await
            .expect("unprovisioned direct consent decision")
    );

    let unprovisioned = test
        .app
        .clone()
        .oneshot(application_authz_request(
            "app-token/application-authz/bootstrap-user",
        ))
        .await
        .expect("unprovisioned check");
    assert_eq!(unprovisioned.status(), StatusCode::FORBIDDEN);

    test.db
        .mark_application_provisioning_ready("application-authz", "bootstrap-user")
        .await
        .expect("mark application provisioning ready");

    assert!(
        test.db
            .has_application_consent_for_action(
                &token_user.id,
                "bootstrap-user",
                "application-authz",
                "organization.profile.read",
            )
            .await
            .expect("direct consent decision")
    );

    let allowed = test
        .app
        .clone()
        .oneshot(application_authz_request(
            "app-token/application-authz/bootstrap-user",
        ))
        .await
        .expect("allowed check");
    assert_eq!(allowed.status(), StatusCode::OK);
    let allowed = support::response_json(allowed).await;
    assert_eq!(allowed["allowed"], true, "response: {allowed}");

    let large_batch = test
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/application-authz/batch-check")
                .header("content-type", "application/json")
                .header(
                    "authorization",
                    support::auth::bearer_header("app-token/application-authz/bootstrap-user"),
                )
                .header("x-organization-id", "bootstrap-user")
                .header("x-grant-app-auth-key", support::APP_AUTH_KEY)
                .body(Body::from(
                    json!({
                        "checks": (0..1000)
                            .map(|_| json!({
                                "action": "  organization.profile.read  ",
                                "resource": {
                                    "provider": "grant",
                                    "type": "organization",
                                    "id": "bootstrap-user"
                                }
                            }))
                            .collect::<Vec<_>>()
                    })
                    .to_string(),
                ))
                .expect("large batch request"),
        )
        .await
        .expect("large batch response");
    assert_eq!(large_batch.status(), StatusCode::OK);
    let large_batch = support::response_json(large_batch).await;
    let results = large_batch["results"]
        .as_array()
        .expect("large batch results array");
    assert_eq!(results.len(), 1000);
    assert!(
        results.iter().all(|decision| decision["allowed"] == true),
        "all normalized batch actions must be allowed"
    );

    let mismatch = test
        .app
        .clone()
        .oneshot(application_authz_request(
            "app-token/different-client/bootstrap-user",
        ))
        .await
        .expect("client mismatch");
    assert_eq!(mismatch.status(), StatusCode::FORBIDDEN);

    test.db
        .replace_application_actions("application-authz", &[])
        .await
        .expect("shrink application actions");
    test.db
        .replace_application_actions(
            "application-authz",
            &["organization.profile.read".to_owned()],
        )
        .await
        .expect("re-add application action");
    let stale_not_restored = test
        .app
        .clone()
        .oneshot(application_authz_request(
            "app-token/application-authz/bootstrap-user",
        ))
        .await
        .expect("stale grant check");
    let stale_not_restored = support::response_json(stale_not_restored).await;
    assert_eq!(stale_not_restored["allowed"], false);

    let renewed = test
        .app
        .clone()
        .oneshot(support::json_request(
            Method::PUT,
            "/api/v1/orgs/bootstrap-user/apps/application-authz/consent",
            "bootstrap-user",
            "bootstrap-user",
            json!({ "actionIds": ["organization.profile.read"] }),
        ))
        .await
        .expect("renew consent");
    assert_eq!(renewed.status(), StatusCode::OK);

    let revoked = test
        .app
        .clone()
        .oneshot(support::empty_request(
            Method::DELETE,
            "/api/v1/orgs/bootstrap-user/apps/application-authz/consent",
            "bootstrap-user",
            "bootstrap-user",
        ))
        .await
        .expect("revoke");
    assert_eq!(revoked.status(), StatusCode::NO_CONTENT);
    let denied = test
        .app
        .clone()
        .oneshot(application_authz_request(
            "app-token/application-authz/bootstrap-user",
        ))
        .await
        .expect("denied check");
    let denied = support::response_json(denied).await;
    assert_eq!(denied["allowed"], false);

    let trusted = test
        .app
        .clone()
        .oneshot(support::master_json_request(
            Method::PUT,
            "/api/v1/admin/orgs/bootstrap-user/apps/application-authz/trust",
            json!({ "trusted": true }),
        ))
        .await
        .expect("trust");
    assert_eq!(trusted.status(), StatusCode::OK);
    let trusted_allowed = test
        .app
        .oneshot(application_authz_request(
            "app-token/application-authz/bootstrap-user",
        ))
        .await
        .expect("trusted check");
    let trusted_allowed = support::response_json(trusted_allowed).await;
    assert_eq!(trusted_allowed["allowed"], true);
}

#[tokio::test]
async fn application_and_admin_credentials_are_not_interchangeable() {
    let test = support::TestApi::new().await;

    let app_auth_with_master = Request::builder()
        .method(Method::POST)
        .uri("/api/v1/application-authz/check")
        .header("content-type", "application/json")
        .header(
            "authorization",
            support::auth::bearer_header("app-token/unknown/bootstrap-user"),
        )
        .header("x-organization-id", "bootstrap-user")
        .header("x-grant-app-auth-key", support::MASTER_KEY)
        .body(Body::from(
            json!({
                "action": "organization.profile.read",
                "resource": {"provider": "grant", "type": "organization", "id": "bootstrap-user"}
            })
            .to_string(),
        ))
        .expect("app auth request");
    let response = test
        .app
        .clone()
        .oneshot(app_auth_with_master)
        .await
        .expect("app auth response");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let admin_with_app_key = Request::builder()
        .method(Method::PUT)
        .uri("/api/v1/admin/users/bootstrap-user/active")
        .header("content-type", "application/json")
        .header("x-grant-master-key", support::APP_AUTH_KEY)
        .body(Body::from(json!({"active": true}).to_string()))
        .expect("admin request");
    let response = test
        .app
        .oneshot(admin_with_app_key)
        .await
        .expect("admin response");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

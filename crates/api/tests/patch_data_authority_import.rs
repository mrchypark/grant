mod support;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
};
use domain::{MembershipRole, MembershipStatus, RoleBindingState, RoleBindingSubjectType};
use infra::database::{
    PatchDataAuthorityImport, PatchDataImportedMembership, PatchDataImportedOrganization,
    PatchDataImportedPlantBinding, PatchDataImportedUser, patch_data_manifest_digest,
};
use tower::ServiceExt;

fn manifest() -> PatchDataAuthorityImport {
    let mut value = PatchDataAuthorityImport {
        manifest_version: "grant.patch-data-authority.v1".to_owned(),
        generation: "api-generation-a".to_owned(),
        ledger_digest: "a".repeat(64),
        manifest_digest: "0".repeat(64),
        expected_previous_generation: None,
        users: vec![PatchDataImportedUser {
            user_id: "api-user".to_owned(),
            external_user_id: "api-external".to_owned(),
            email: None,
            display_name: "API User".to_owned(),
            active: true,
        }],
        organizations: vec![PatchDataImportedOrganization {
            organization_id: "api-org".to_owned(),
            name: "API Org".to_owned(),
            active: true,
        }],
        memberships: vec![PatchDataImportedMembership {
            membership_id: "api-membership".to_owned(),
            organization_id: "api-org".to_owned(),
            user_id: "api-user".to_owned(),
            role: MembershipRole::Member,
            status: MembershipStatus::Active,
        }],
        hierarchy_edges: vec![],
        plant_bindings: vec![PatchDataImportedPlantBinding {
            binding_id: "api-binding".to_owned(),
            subject_type: RoleBindingSubjectType::User,
            subject_id: "api-user".to_owned(),
            subject_organization_id: "api-org".to_owned(),
            plant_id: "api-plant".to_owned(),
            state: RoleBindingState::Active,
        }],
        application_assignments: vec![],
    };
    value.manifest_digest = patch_data_manifest_digest(&value);
    value
}

#[tokio::test]
async fn import_is_master_only_strict_and_idempotent() {
    let test = support::TestApi::new().await;
    let path = "/api/v1/admin/imports/patch-data-authority";
    let body = serde_json::to_value(manifest()).expect("manifest json");
    let missing = test
        .app
        .clone()
        .oneshot(support::public_json_request(
            Method::POST,
            path,
            body.clone(),
        ))
        .await
        .expect("missing key");
    assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);

    let applied = test
        .app
        .clone()
        .oneshot(support::master_json_request(
            Method::POST,
            path,
            body.clone(),
        ))
        .await
        .expect("apply");
    assert_eq!(applied.status(), StatusCode::OK);
    let applied = support::response_json(applied).await;
    assert_eq!(applied["applied"], true);
    assert_eq!(applied["manifestVersion"], "grant.patch-data-authority.v1");

    let retry = test
        .app
        .clone()
        .oneshot(support::master_json_request(Method::POST, path, body))
        .await
        .expect("retry");
    assert_eq!(retry.status(), StatusCode::OK);
    assert_eq!(support::response_json(retry).await["applied"], false);

    let mut mismatch = manifest();
    mismatch.ledger_digest = "b".repeat(64);
    mismatch.manifest_digest = patch_data_manifest_digest(&mismatch);
    let conflict = test
        .app
        .clone()
        .oneshot(support::master_json_request(
            Method::POST,
            path,
            serde_json::to_value(mismatch).unwrap(),
        ))
        .await
        .expect("conflict");
    assert_eq!(conflict.status(), StatusCode::CONFLICT);

    let mut unknown = serde_json::to_value(manifest()).expect("manifest json");
    unknown["unexpected"] = serde_json::json!(true);
    let rejected = test
        .app
        .clone()
        .oneshot(support::master_json_request(Method::POST, path, unknown))
        .await
        .expect("strict reject");
    assert_eq!(rejected.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let learner = support::TestApi::learner().await;
    let learner_response = learner
        .app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(path)
                .header("content-type", "application/json")
                .header("x-grant-master-key", support::MASTER_KEY)
                .body(Body::from(serde_json::to_string(&manifest()).unwrap()))
                .unwrap(),
        )
        .await
        .expect("learner");
    assert_eq!(learner_response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

mod support;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
};
use infra::database::{AuthoritySnapshotV2, snapshot_digest};
use tower::ServiceExt;

fn snapshot() -> AuthoritySnapshotV2 {
    let mut snapshot: AuthoritySnapshotV2 = serde_json::from_value(serde_json::json!({
        "schemaVersion": 2,
        "sourceId": "patch-data",
        "generation": 1,
        "sourceRevision": "9a27580e7fd7e4bb0df63c1e2e8876e3a24c717d",
        "sourceLedgerDigest": format!("sha256:{}", "a".repeat(64)),
        "sourceWatermark": format!("sha256:{}", "b".repeat(64)),
        "producedAt": "2026-08-26T00:00:00Z",
        "principals": [],
        "organizations": [],
        "memberships": [],
        "applications": [],
        "effectiveBindings": [],
        "snapshotDigest": format!("sha256:{}", "0".repeat(64))
    }))
    .expect("snapshot fixture");
    snapshot.snapshot_digest = snapshot_digest(&snapshot).expect("snapshot digest");
    snapshot
}

#[tokio::test]
async fn v2_snapshot_is_master_only_strict_atomic_and_idempotent() {
    let test = support::TestApi::new().await;
    let path = "/api/v2/admin/imports/authority-snapshots";
    let body = serde_json::to_value(snapshot()).expect("snapshot json");

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
    assert_eq!(applied["generation"], 1);
    assert_eq!(applied["priorGeneration"], serde_json::Value::Null);

    let retry = test
        .app
        .clone()
        .oneshot(support::master_json_request(
            Method::POST,
            path,
            body.clone(),
        ))
        .await
        .expect("retry");
    assert_eq!(retry.status(), StatusCode::OK);
    assert_eq!(support::response_json(retry).await["applied"], false);

    let mut next = snapshot();
    next.generation = 2;
    next.produced_at = "2026-08-26T00:00:01Z".to_owned();
    next.snapshot_digest = snapshot_digest(&next).expect("next digest");
    let next = test
        .app
        .clone()
        .oneshot(support::master_json_request(
            Method::POST,
            path,
            serde_json::to_value(next).expect("next json"),
        ))
        .await
        .expect("next generation");
    assert_eq!(next.status(), StatusCode::OK);
    assert_eq!(support::response_json(next).await["priorGeneration"], 1);

    let mut unknown = body.clone();
    unknown["unexpected"] = serde_json::json!(true);
    let rejected = test
        .app
        .clone()
        .oneshot(support::master_json_request(Method::POST, path, unknown))
        .await
        .expect("strict reject");
    assert_eq!(rejected.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let mut changed: AuthoritySnapshotV2 = serde_json::from_value(body).expect("changed snapshot");
    changed.produced_at = "2026-08-26T00:00:01Z".to_owned();
    changed.snapshot_digest = snapshot_digest(&changed).expect("changed digest");
    let conflict = test
        .app
        .oneshot(support::master_json_request(
            Method::POST,
            path,
            serde_json::to_value(changed).expect("changed json"),
        ))
        .await
        .expect("generation conflict");
    assert_eq!(conflict.status(), StatusCode::CONFLICT);
}

#[test]
fn canonical_digest_matches_patch_data_exporter_fixture() {
    assert_eq!(
        snapshot().snapshot_digest,
        "sha256:868dff9d46dcb754ac82aa760837227bc4d0489505bd0063587122f26bc3ee30"
    );
}

#[tokio::test]
async fn generations_are_contiguous_atomic_and_isolated_per_source() {
    let test = support::TestApi::new().await;
    let path = "/api/v2/admin/imports/authority-snapshots";
    let mut gap = snapshot();
    gap.generation = 2;
    gap.snapshot_digest = snapshot_digest(&gap).unwrap();
    let rejected = test
        .app
        .clone()
        .oneshot(support::master_json_request(
            Method::POST,
            path,
            serde_json::to_value(gap).unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::CONFLICT);

    let mut other = snapshot();
    other.source_id = "another-source".to_owned();
    other.snapshot_digest = snapshot_digest(&other).unwrap();
    let accepted = test
        .app
        .clone()
        .oneshot(support::master_json_request(
            Method::POST,
            path,
            serde_json::to_value(other).unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::OK);
    assert_eq!(
        support::response_json(accepted).await["sourceId"],
        "another-source"
    );

    let first = snapshot();
    let request =
        || support::master_json_request(Method::POST, path, serde_json::to_value(&first).unwrap());
    let (left, right) = tokio::join!(
        test.app.clone().oneshot(request()),
        test.app.clone().oneshot(request())
    );
    let left = support::response_json(left.unwrap()).await;
    let right = support::response_json(right.unwrap()).await;
    assert_eq!(
        [
            left["applied"].as_bool().unwrap(),
            right["applied"].as_bool().unwrap()
        ]
        .into_iter()
        .filter(|v| *v)
        .count(),
        1
    );
}

#[tokio::test]
async fn learner_rejects_v2_import_without_mutation() {
    let learner = support::TestApi::learner().await;
    let response = learner
        .app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v2/admin/imports/authority-snapshots")
                .header("content-type", "application/json")
                .header("x-grant-master-key", support::MASTER_KEY)
                .body(Body::from(serde_json::to_string(&snapshot()).unwrap()))
                .unwrap(),
        )
        .await
        .expect("learner response");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn shared_v2_envelope_is_provider_neutral_but_patch_data_adapter_is_strict() {
    let test = support::TestApi::new().await;
    let path = "/api/v2/admin/imports/authority-snapshots";

    let mut neutral = snapshot();
    neutral.source_id = "another-source".to_owned();
    neutral.principals = serde_json::from_value(serde_json::json!([{
        "id": "machine-a",
        "kind": "robot",
        "active": true,
        "externalSubject": "another-source:robot:machine-a"
    }]))
    .unwrap();
    neutral.snapshot_digest = snapshot_digest(&neutral).unwrap();
    let accepted = test
        .app
        .clone()
        .oneshot(support::master_json_request(
            Method::POST,
            path,
            serde_json::to_value(neutral).unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::OK);

    let mut patch_data = snapshot();
    patch_data.principals = serde_json::from_value(serde_json::json!([{
        "id": "machine-a",
        "kind": "robot",
        "active": true,
        "externalSubject": "patch-data:robot:machine-a"
    }]))
    .unwrap();
    patch_data.snapshot_digest = snapshot_digest(&patch_data).unwrap();
    let rejected = test
        .app
        .oneshot(support::master_json_request(
            Method::POST,
            path,
            serde_json::to_value(patch_data).unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);
}

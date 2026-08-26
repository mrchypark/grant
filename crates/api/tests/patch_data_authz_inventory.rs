use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashSet;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Inventory {
    schema_version: u64,
    baseline_commit: String,
    source_ledger_digest: String,
    surface_count: usize,
    surface_id_digest: String,
    rule_expression_digest: String,
    surfaces: Vec<Surface>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Surface {
    id: String,
    operation_id: String,
    rule_expression: String,
}

#[test]
fn patch_data_adapter_owns_the_exact_frozen_99_surface_inventory() {
    let inventory: Inventory =
        serde_json::from_slice(include_bytes!("fixtures/patch_data_authz_surfaces_v1.json"))
            .expect("strict frozen inventory");
    assert_eq!(inventory.schema_version, 1);
    assert_eq!(
        inventory.baseline_commit,
        "9a27580e7fd7e4bb0df63c1e2e8876e3a24c717d"
    );
    assert_eq!(
        inventory.source_ledger_digest,
        "acdf070503f60e556dff3317b3341dfe1c61f6ffd63f542313172b1db2399cd3"
    );
    assert_eq!(inventory.surface_count, 99);
    assert_eq!(inventory.surfaces.len(), 99);
    assert_eq!(
        inventory.rule_expression_digest,
        "fb0d26165b6dab23895fb6fc3f919cf162c0f957d540f477b48a492f73b27cab"
    );
    let mut ids = inventory
        .surfaces
        .iter()
        .map(|surface| surface.id.as_str())
        .collect::<Vec<_>>();
    ids.sort_unstable();
    let id_bytes = format!("{}\n", ids.join("\n"));
    assert_eq!(
        hex::encode(Sha256::digest(id_bytes)),
        inventory.surface_id_digest
    );
    let operation_ids = inventory
        .surfaces
        .iter()
        .map(|surface| surface.operation_id.as_str())
        .collect::<HashSet<_>>();
    assert_eq!(operation_ids.len(), 99);
    assert!(
        inventory
            .surfaces
            .iter()
            .all(|surface| !surface.rule_expression.is_empty())
    );
    let mut canonical_surfaces = inventory
        .surfaces
        .iter()
        .map(|surface| {
            serde_json::json!({
                "id": surface.id,
                "operationId": surface.operation_id,
                "ruleExpression": surface.rule_expression,
            })
        })
        .collect::<Vec<_>>();
    canonical_surfaces.sort_by(|left, right| {
        left["id"]
            .as_str()
            .cmp(&right["id"].as_str())
            .then_with(|| {
                left["operationId"]
                    .as_str()
                    .cmp(&right["operationId"].as_str())
            })
            .then_with(|| {
                left["ruleExpression"]
                    .as_str()
                    .cmp(&right["ruleExpression"].as_str())
            })
    });
    let canonical_digest = hex::encode(Sha256::digest(
        serde_json::to_vec(&canonical_surfaces).expect("canonical surface content"),
    ));
    assert_eq!(
        canonical_digest,
        "21b33ce281f918251909166035962a988cdad842afff0af2e11b66597c97c90e"
    );
}

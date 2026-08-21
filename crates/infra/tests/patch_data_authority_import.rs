mod support;

use infra::{
    config::ReadConsistency,
    database::{PatchDataAuthorityImport, PatchDataHierarchyEdge},
};

fn digest(byte: char) -> String {
    std::iter::repeat_n(byte, 64).collect()
}

fn command(
    generation: &str,
    ledger_digest: String,
    expected_previous_generation: Option<&str>,
    edges: &[(&str, &str)],
) -> PatchDataAuthorityImport {
    PatchDataAuthorityImport {
        generation: generation.to_owned(),
        ledger_digest,
        expected_previous_generation: expected_previous_generation.map(ToOwned::to_owned),
        hierarchy_edges: edges
            .iter()
            .map(|(parent, child)| PatchDataHierarchyEdge {
                parent_organization_id: (*parent).to_owned(),
                child_organization_id: (*child).to_owned(),
            })
            .collect(),
    }
}

#[tokio::test]
async fn patch_data_authority_foundation_is_generation_bound_and_idempotent() {
    let (runtime, db, _dir) = support::ensured_database().await;
    let first_command = || {
        command(
            "main-a:snapshot-a",
            digest('a'),
            None,
            &[("org-a", "org-b")],
        )
    };
    let first = db
        .import_patch_data_authority_foundation(first_command())
        .await
        .expect("first import");
    assert!(first.applied);
    let retry = db
        .import_patch_data_authority_foundation(first_command())
        .await
        .expect("exact retry");
    assert!(!retry.applied);

    runtime
        .execute(
            "DELETE FROM role_actions WHERE role_id='role-patch-data-plant-reader' AND action_id='patch-data.plant.read'",
            vec![],
        )
        .await
        .expect("simulate imported catalog drift");
    let drift = db
        .import_patch_data_authority_foundation(first_command())
        .await
        .expect_err("exact retry must fail closed when imported state drifted");
    assert!(drift.to_string().contains("state drift"));
    runtime
        .execute(
            "INSERT INTO role_actions (role_id,action_id) VALUES ('role-patch-data-plant-reader','patch-data.plant.read')",
            vec![],
        )
        .await
        .expect("restore imported catalog for replacement test");
    runtime
        .execute(
            "UPDATE actions SET grantable_to_applications=0 WHERE id='patch-data.plant.read'",
            vec![],
        )
        .await
        .expect("simulate imported action drift");
    let drift = db
        .import_patch_data_authority_foundation(first_command())
        .await
        .expect_err("exact retry must detect imported action drift");
    assert!(drift.to_string().contains("state drift"));
    runtime
        .execute(
            "UPDATE actions SET grantable_to_applications=1 WHERE id='patch-data.plant.read'",
            vec![],
        )
        .await
        .expect("restore imported action for replacement test");

    let conflicting = db
        .import_patch_data_authority_foundation(command(
            "main-a:snapshot-a",
            digest('b'),
            None,
            &[("org-a", "org-b")],
        ))
        .await
        .expect_err("same generation with another digest must fail");
    assert!(conflicting.to_string().contains("generation conflict"));

    let altered_foundation = db
        .import_patch_data_authority_foundation(command(
            "main-a:snapshot-a",
            digest('a'),
            None,
            &[("org-a", "org-c")],
        ))
        .await
        .expect_err("same ledger receipt cannot hide another hierarchy");
    assert!(
        altered_foundation
            .to_string()
            .contains("generation conflict")
    );

    let replaced = db
        .import_patch_data_authority_foundation(command(
            "main-b:snapshot-b",
            digest('b'),
            Some("main-a:snapshot-a"),
            &[("org-a", "org-c")],
        ))
        .await
        .expect("CAS replacement");
    assert!(replaced.applied);

    let rows: Vec<support::NameRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT generation AS name FROM patch_data_authority_imports
             UNION ALL SELECT source_generation AS name FROM organization_hierarchy_edges
             UNION ALL SELECT id AS name FROM service_providers WHERE id='patch-data'
             UNION ALL SELECT id AS name FROM actions WHERE id='patch-data.plant.read'
             UNION ALL SELECT id AS name FROM roles WHERE id='role-patch-data-plant-reader'
             ORDER BY name",
            vec![],
        )
        .await
        .expect("imported foundation rows");
    let names = rows.into_iter().map(|row| row.name).collect::<Vec<_>>();
    assert_eq!(
        names,
        vec![
            "main-b:snapshot-b",
            "main-b:snapshot-b",
            "patch-data",
            "patch-data.plant.read",
            "role-patch-data-plant-reader",
        ]
    );
}

#[tokio::test]
async fn patch_data_authority_foundation_rejects_nondeterministic_or_cyclic_hierarchy() {
    let (_runtime, db, _dir) = support::ensured_database().await;
    for edges in [
        vec![("org-b", "org-c"), ("org-a", "org-b")],
        vec![("org-a", "org-b"), ("org-b", "org-a")],
        vec![("org-a", "org-a")],
    ] {
        assert!(
            db.import_patch_data_authority_foundation(command(
                "main-a:snapshot-a",
                digest('a'),
                None,
                &edges,
            ))
            .await
            .is_err()
        );
    }
}

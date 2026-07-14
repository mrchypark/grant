mod support;

use domain::Group;
use infra::config::ReadConsistency;

#[tokio::test]
async fn delete_group_rejects_a_mismatched_organization_without_removing_memberships() {
    let (runtime, db, _dir) = support::ensured_database().await;
    db.upsert_organization_with_owner("group-owner", "Group Owner", None, "Group Owner")
        .await
        .expect("owner organization");
    db.upsert_organization_with_owner("other-organization", "Other", None, "Other")
        .await
        .expect("other organization");
    db.insert_group(&Group {
        id: "owned-group".to_owned(),
        organization_id: "group-owner".to_owned(),
        name: "Owned Group".to_owned(),
        description: None,
    })
    .await
    .expect("insert group");
    runtime
        .execute(
            "INSERT INTO group_memberships (group_id, organization_membership_id)
             SELECT $1, id FROM organization_memberships
             WHERE organization_id = $2
             LIMIT 1",
            vec![
                hiqlite::Param::from("owned-group".to_owned()),
                hiqlite::Param::from("group-owner".to_owned()),
            ],
        )
        .await
        .expect("insert group membership");

    let deleted = db
        .delete_group_if_unbound("owned-group", "other-organization")
        .await
        .expect("mismatched organization delete");
    assert!(!deleted, "mismatched organization must not report deletion");
    assert!(
        db.find_group_with_consistency(
            "owned-group",
            "group-owner",
            Some(ReadConsistency::Strong),
        )
        .await
        .expect("find owner group")
        .is_some(),
        "mismatched organization deleted the group",
    );
    assert_eq!(
        db.list_group_members_with_consistency(
            "owned-group",
            "group-owner",
            Some(ReadConsistency::Strong),
        )
        .await
        .expect("list retained memberships")
        .len(),
        1,
        "mismatched organization deleted group memberships",
    );

    assert!(
        db.delete_group_if_unbound("owned-group", "group-owner")
            .await
            .expect("owner organization delete"),
        "owner organization should delete an unbound group",
    );
    assert!(
        db.find_group_with_consistency(
            "owned-group",
            "group-owner",
            Some(ReadConsistency::Strong),
        )
        .await
        .expect("find deleted group")
        .is_none(),
    );
}

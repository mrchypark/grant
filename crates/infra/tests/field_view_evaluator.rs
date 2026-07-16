mod support;

use domain::{
    Action, ActionRiskLevel, FieldViewMode, Group, Role, RoleBinding, RoleBindingState,
    RoleBindingSubjectType, ServiceProvider,
};
use infra::{
    config::ReadConsistency,
    database::{Database, UserActionCheck, UserActionFieldViewCheck},
    runtime::HiqliteRuntime,
};

const USER_ID: &str = "field-view-user";
const ORGANIZATION_ID: &str = "field-view-org";
const RESOURCE_ID: &str = "object-1";
const SCHEMA_KEY: &str = "object-type-1";
const ACTION_ID: &str = "compos.object.read";

async fn field_view_database() -> (Database, HiqliteRuntime, tempfile::TempDir) {
    let (runtime, dir) = support::test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        USER_ID.to_owned(),
        "Field View User".to_owned(),
    );
    db.ensure_schema().await.expect("ensure schema");
    db.upsert_organization_with_owner(
        ORGANIZATION_ID,
        "Field View Organization",
        None,
        "Field View Organization",
    )
    .await
    .expect("seed organization");
    db.upsert_service_provider(&ServiceProvider {
        id: "compos".to_owned(),
        name: "Compos".to_owned(),
        description: None,
        active: true,
    })
    .await
    .expect("seed provider");
    db.upsert_action(&Action {
        id: ACTION_ID.to_owned(),
        provider_id: "compos".to_owned(),
        code: ACTION_ID.to_owned(),
        name: "Read object".to_owned(),
        description: None,
        risk_level: ActionRiskLevel::Low,
        visibility_granting: true,
        grantable_to_applications: true,
        consent_label: Some("Read object".to_owned()),
        consent_description: None,
    })
    .await
    .expect("seed action");

    for role_id in ["role-direct", "role-group", "role-organization"] {
        db.upsert_role(&Role {
            id: role_id.to_owned(),
            organization_id: Some(ORGANIZATION_ID.to_owned()),
            code: role_id.to_owned(),
            name: role_id.to_owned(),
            description: None,
        })
        .await
        .expect("seed role");
        db.upsert_role_action(role_id, ACTION_ID)
            .await
            .expect("seed role action");
    }

    db.insert_group(&Group {
        id: "field-view-group".to_owned(),
        organization_id: ORGANIZATION_ID.to_owned(),
        name: "Field View Group".to_owned(),
        description: None,
    })
    .await
    .expect("seed group");
    db.add_group_member_if_active("field-view-group", ORGANIZATION_ID, USER_ID)
        .await
        .expect("seed group member")
        .expect("active group member");

    for (id, subject_type, subject_id, role_id) in [
        (
            "binding-direct",
            RoleBindingSubjectType::User,
            USER_ID,
            "role-direct",
        ),
        (
            "binding-group",
            RoleBindingSubjectType::Group,
            "field-view-group",
            "role-group",
        ),
        (
            "binding-organization",
            RoleBindingSubjectType::Organization,
            ORGANIZATION_ID,
            "role-organization",
        ),
    ] {
        db.save_role_binding(&RoleBinding {
            id: id.to_owned(),
            subject_type,
            subject_id: subject_id.to_owned(),
            subject_organization_id: ORGANIZATION_ID.to_owned(),
            provider_id: "compos".to_owned(),
            resource_type: "compos.object".to_owned(),
            resource_id: RESOURCE_ID.to_owned(),
            role_id: role_id.to_owned(),
            agreement_id: None,
            state: RoleBindingState::Active,
            created_by_user_id: USER_ID.to_owned(),
            created_at: support::FIXED_TEST_TIME.to_owned(),
            updated_at: support::FIXED_TEST_TIME.to_owned(),
        })
        .await
        .expect("seed role binding");
    }

    runtime
        .execute(
            "INSERT INTO role_action_field_views (
                role_id, action_id, resource_type, schema_key, field_pointer, view_mode, created_at, updated_at
             ) VALUES
                ('role-direct', $1, 'compos.object', $2, '/direct', 'anonymized', $3, $3),
                ('role-group', $1, 'compos.object', $2, '/group', 'real', $3, $3),
                ('role-organization', $1, 'compos.object', $2, '/organization', 'anonymized', $3, $3),
                ('role-direct', $1, 'compos.object', $2, '/merged', 'anonymized', $3, $3),
                ('role-group', $1, 'compos.object', $2, '/merged', 'real', $3, $3),
                ('role-organization', $1, 'compos.object', $2, '/merged', 'anonymized', $3, $3),
                ('role-direct', $1, 'compos.object', $2, '/corrupt', 'real', $3, $3)",
            vec![ACTION_ID.into(), SCHEMA_KEY.into(), support::FIXED_TEST_TIME.into()],
        )
        .await
        .expect("seed field policies");
    (db, runtime, dir)
}

async fn seed_dense_role_field_views(
    runtime: &HiqliteRuntime,
    resource_id: &str,
    role_count: usize,
    field_count: usize,
) {
    let mut role_rows = Vec::with_capacity(role_count);
    let mut role_action_rows = Vec::with_capacity(role_count);
    let mut role_binding_rows = Vec::with_capacity(role_count);
    for role_index in 0..role_count {
        let role_id = format!("stress-role-{role_index:03}");
        role_rows.push(format!(
            "('{role_id}', '{ORGANIZATION_ID}', '{role_id}', '{role_id}', \
             '{time}', '{time}')",
            time = support::FIXED_TEST_TIME
        ));
        role_action_rows.push(format!("('{role_id}', '{ACTION_ID}')"));
        role_binding_rows.push(format!(
            "('stress-binding-{role_index:03}', 'user', '{user_id}', '{org}', \
             'compos', 'compos.object', '{resource_id}', '{role_id}', 'active', \
             '{user_id}', '{time}', '{time}')",
            user_id = USER_ID,
            org = ORGANIZATION_ID,
            time = support::FIXED_TEST_TIME
        ));
    }
    let seeded_roles_sql = format!(
        "INSERT INTO roles (id, organization_id, code, name, created_at, updated_at) \
         VALUES {}\n;\n\
         INSERT INTO role_actions (role_id, action_id) VALUES {}\n;\n\
         INSERT INTO role_bindings (\n            id, subject_type, subject_id, subject_organization_id,\n            provider_id, resource_type, resource_id, role_id, state,\n            created_by_user_id, created_at, updated_at\n         ) VALUES {}\n;",
        role_rows.join(", "),
        role_action_rows.join(", "),
        role_binding_rows.join(", "),
    );
    runtime
        .batch(seeded_roles_sql)
        .await
        .expect("seed dense roles");

    let mut policy_rows = Vec::with_capacity(4096);
    for role_index in 0..role_count {
        let role_id = format!("stress-role-{role_index:03}");
        let mode = if role_index == 0 {
            "real"
        } else {
            "anonymized"
        };
        for field_index in 0..field_count {
            let pointer = format!("/field-{field_index:03}");
            policy_rows.push(format!(
                "('{role_id}', '{ACTION_ID}', 'compos.object', '{SCHEMA_KEY}', \
                 '{pointer}', '{mode}', '{time}', '{time}')",
                time = support::FIXED_TEST_TIME
            ));
            if policy_rows.len() == 2000 {
                let batch_sql = format!(
                    "INSERT INTO role_action_field_views (
                     role_id, action_id, resource_type, schema_key,
                     field_pointer, view_mode, created_at, updated_at
                     ) VALUES {}\n;",
                    policy_rows.join(", ")
                );
                runtime
                    .batch(batch_sql)
                    .await
                    .expect("seed dense field policies");
                policy_rows.clear();
            }
        }
    }
    if !policy_rows.is_empty() {
        let final_batch_sql = format!(
            "INSERT INTO role_action_field_views (
             role_id, action_id, resource_type, schema_key,
             field_pointer, view_mode, created_at, updated_at
             ) VALUES {}\n;",
            policy_rows.join(", ")
        );
        runtime
            .batch(final_batch_sql)
            .await
            .expect("seed dense field policies");
    }
}

async fn seed_many_roles_without_field_policies(
    runtime: &HiqliteRuntime,
    resource_id: &str,
    role_count: usize,
) {
    let mut role_rows = Vec::with_capacity(role_count);
    let mut role_action_rows = Vec::with_capacity(role_count);
    let mut role_binding_rows = Vec::with_capacity(role_count);
    for role_index in 0..role_count {
        let role_id = format!("cap-role-{role_index:03}");
        role_rows.push(format!(
            "('{role_id}', '{ORGANIZATION_ID}', '{role_id}', '{role_id}', \
             '{time}', '{time}')",
            time = support::FIXED_TEST_TIME
        ));
        role_action_rows.push(format!("('{role_id}', '{ACTION_ID}')"));
        role_binding_rows.push(format!(
            "('cap-binding-{role_index:03}', 'user', '{user_id}', '{org}', \
             'compos', 'compos.object', '{resource_id}', '{role_id}', 'active', \
             '{user_id}', '{time}', '{time}')",
            user_id = USER_ID,
            org = ORGANIZATION_ID,
            time = support::FIXED_TEST_TIME
        ));
    }
    let seeded_cap_roles_sql = format!(
        "INSERT INTO roles (id, organization_id, code, name, created_at, updated_at) \
         VALUES {}\n;\n\
         INSERT INTO role_actions (role_id, action_id) VALUES {}\n;\n\
         INSERT INTO role_bindings (
            id, subject_type, subject_id, subject_organization_id,
            provider_id, resource_type, resource_id, role_id, state,
            created_by_user_id, created_at, updated_at
         ) VALUES {};\n",
        role_rows.join(", "),
        role_action_rows.join(", "),
        role_binding_rows.join(", "),
    );
    runtime
        .batch(seeded_cap_roles_sql)
        .await
        .expect("seed many roles");
}

fn action_check(resource_id: &str) -> UserActionCheck<'_> {
    UserActionCheck {
        user_id: USER_ID,
        organization_id: ORGANIZATION_ID,
        provider_id: "compos",
        resource_type: "compos.object",
        resource_id,
        action_id: ACTION_ID,
        application_id: None,
        consistency: Some(ReadConsistency::Local),
    }
}

#[tokio::test]
async fn field_evaluator_merges_direct_group_and_organization_roles_and_preserves_batch_order() {
    let (db, _runtime, _dir) = field_view_database().await;
    let fields = ["/direct", "/group", "/organization", "/merged", "/missing"]
        .map(str::to_owned)
        .to_vec();
    let checks = [action_check(RESOURCE_ID), action_check("denied-object")];
    let views = [
        Some(UserActionFieldViewCheck {
            schema_key: SCHEMA_KEY,
            fields: &fields,
        }),
        Some(UserActionFieldViewCheck {
            schema_key: SCHEMA_KEY,
            fields: &fields,
        }),
    ];

    let results = db
        .evaluate_user_actions_with_field_views(&checks, &views)
        .await
        .expect("field-aware batch");

    assert_eq!(results.len(), 2);
    assert!(results[0].decision.allowed);
    let allowed = results[0]
        .field_views
        .as_ref()
        .expect("allowed field views");
    assert_eq!(allowed.len(), fields.len());
    assert_eq!(allowed["/direct"], FieldViewMode::Anonymized);
    assert_eq!(allowed["/group"], FieldViewMode::Real);
    assert_eq!(allowed["/organization"], FieldViewMode::Anonymized);
    assert_eq!(allowed["/merged"], FieldViewMode::Real);
    assert_eq!(allowed["/missing"], FieldViewMode::None);

    assert!(!results[1].decision.allowed);
    let denied = results[1].field_views.as_ref().expect("denied field views");
    assert_eq!(denied.len(), fields.len());
    assert!(denied.values().all(|mode| *mode == FieldViewMode::None));
}

#[tokio::test]
async fn field_evaluator_rejects_input_cardinality_mismatch() {
    let (db, _runtime, _dir) = field_view_database().await;
    let error = db
        .evaluate_user_actions_with_field_views(&[action_check(RESOURCE_ID)], &[])
        .await
        .expect_err("mismatched field input must fail");
    assert!(error.to_string().contains("does not match check length"));
}

#[tokio::test]
async fn corrupt_field_mode_fails_the_entire_batch() {
    let (db, runtime, _dir) = field_view_database().await;
    runtime
        .batch(
            "PRAGMA ignore_check_constraints = ON;
             UPDATE role_action_field_views
             SET view_mode = 'corrupt'
             WHERE role_id = 'role-direct' AND field_pointer = '/corrupt';",
        )
        .await
        .expect("corrupt stored mode");
    let first_fields = vec!["/direct".to_owned()];
    let second_fields = vec!["/corrupt".to_owned()];
    let checks = [action_check(RESOURCE_ID), action_check(RESOURCE_ID)];
    let views = [
        Some(UserActionFieldViewCheck {
            schema_key: SCHEMA_KEY,
            fields: &first_fields,
        }),
        Some(UserActionFieldViewCheck {
            schema_key: SCHEMA_KEY,
            fields: &second_fields,
        }),
    ];

    let error = db
        .evaluate_user_actions_with_field_views(&checks, &views)
        .await
        .expect_err("corrupt batch must fail closed");
    assert!(
        error
            .to_string()
            .contains("invalid field view mode corrupt")
    );
}

#[tokio::test]
async fn field_evaluator_aggregates_many_roles_and_pointers_with_precedence_and_no_row_explosion() {
    let (db, runtime, _dir) = field_view_database().await;
    let resource_id = "stress-object";
    seed_dense_role_field_views(&runtime, resource_id, 256, 256).await;
    let fields = (0..256)
        .map(|index| format!("/field-{index:03}"))
        .collect::<Vec<_>>();
    const ALLOWED_CHECKS: usize = 32;
    // Before SQL aggregation, these allowed checks returned 2,097,152 role-field rows.
    let mut checks = (0..ALLOWED_CHECKS)
        .map(|_| action_check(resource_id))
        .collect::<Vec<_>>();
    checks.push(action_check("denied-object"));
    let views = (0..checks.len())
        .map(|_| {
            Some(UserActionFieldViewCheck {
                schema_key: SCHEMA_KEY,
                fields: &fields,
            })
        })
        .collect::<Vec<_>>();

    let results = db
        .evaluate_user_actions_with_field_views(&checks, &views)
        .await
        .expect("role-field stress batch");

    assert_eq!(results.len(), ALLOWED_CHECKS + 1);
    assert_eq!(
        results
            .iter()
            .map(|result| result.field_views.as_ref().map_or(0, |views| views.len()))
            .sum::<usize>(),
        (ALLOWED_CHECKS + 1) * fields.len()
    );
    for result in &results[..ALLOWED_CHECKS] {
        assert!(result.decision.allowed);
        let allowed = result
            .field_views
            .as_ref()
            .expect("allowed dense field views");
        assert_eq!(allowed.len(), fields.len());
        assert!(allowed.values().all(|mode| *mode == FieldViewMode::Real));
    }

    let denied_result = &results[ALLOWED_CHECKS];
    assert!(!denied_result.decision.allowed);
    let denied = denied_result
        .field_views
        .as_ref()
        .expect("denied dense field views");
    assert_eq!(denied.len(), fields.len());
    assert!(denied.values().all(|mode| *mode == FieldViewMode::None));
}

#[tokio::test]
async fn candidate_role_cap_only_applies_to_checks_requesting_fields() {
    let (db, runtime, _dir) = field_view_database().await;
    let resource_id = "cap-object";
    seed_many_roles_without_field_policies(&runtime, resource_id, 257).await;
    let checks = [action_check(resource_id), action_check("denied-object")];
    let views = [None, None];

    let results = db
        .evaluate_user_actions_with_field_views(&checks, &views)
        .await
        .expect("binary cap regression");

    assert!(results[0].decision.allowed);
    assert!(!results[1].decision.allowed);
    assert!(results[0].field_views.is_none());
    assert!(results[1].field_views.is_none());
}

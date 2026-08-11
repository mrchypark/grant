use domain::{
    Agreement, AgreementStatus, AgreementType, Application, ApplicationAction, ApplicationAppType,
    ApplicationConsentPolicy, ApplicationGrant, ApplicationGrantAction, ApplicationGrantStatus,
    ApplicationType, Role, RoleBinding, RoleBindingState, RoleBindingSubjectType,
};
use hiqlite::{Node, NodeConfig, Row, s3::EncKeys};
use infra::{config::ReadConsistency, database::Database, runtime::HiqliteRuntime};
use tempfile::TempDir;

#[derive(Debug)]
struct OrganizationRow {
    id: String,
    party_id: Option<String>,
    name: String,
}

impl From<&mut Row<'_>> for OrganizationRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            id: row.get("id"),
            party_id: row.get("party_id"),
            name: row.get("name"),
        }
    }
}

#[derive(Debug)]
struct PartyRow {
    id: String,
    legal_name: String,
}

impl From<&mut Row<'_>> for PartyRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            id: row.get("id"),
            legal_name: row.get("legal_name"),
        }
    }
}

#[derive(Debug)]
struct MembershipRow {
    organization_id: String,
    user_id: String,
    membership_role: String,
    status: String,
}

impl From<&mut Row<'_>> for MembershipRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            organization_id: row.get("organization_id"),
            user_id: row.get("user_id"),
            membership_role: row.get("membership_role"),
            status: row.get("status"),
        }
    }
}

#[derive(Debug)]
struct ActionRow {
    code: String,
    visibility_granting: i64,
    grantable_to_applications: i64,
}

impl From<&mut Row<'_>> for ActionRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            code: row.get("code"),
            visibility_granting: row.get("visibility_granting"),
            grantable_to_applications: row.get("grantable_to_applications"),
        }
    }
}

#[derive(Debug)]
struct RoleActionRow {
    action_id: String,
}

impl From<&mut Row<'_>> for RoleActionRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            action_id: row.get("action_id"),
        }
    }
}

#[derive(Debug)]
struct ApplicationGrantRow {
    id: String,
    user_id: String,
    application_id: String,
    organization_id: String,
    status: String,
}

impl From<&mut Row<'_>> for ApplicationGrantRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            id: row.get("id"),
            user_id: row.get("user_id"),
            application_id: row.get("application_id"),
            organization_id: row.get("organization_id"),
            status: row.get("status"),
        }
    }
}

fn free_local_addr() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    drop(listener);
    addr.to_string()
}

async fn test_runtime() -> (HiqliteRuntime, TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let api_addr = free_local_addr();
    let raft_addr = free_local_addr();
    let cfg = NodeConfig {
        node_id: 1,
        nodes: vec![Node {
            id: 1,
            addr_raft: raft_addr,
            addr_api: api_addr,
        }],
        listen_addr_api: "127.0.0.1".into(),
        listen_addr_raft: "127.0.0.1".into(),
        data_dir: dir.path().to_string_lossy().into_owned().into(),
        filename_db: "test.sqlite".into(),
        secret_api: "0123456789abcdef0123456789abcdef".to_owned(),
        secret_raft: "fedcba9876543210fedcba9876543210".to_owned(),
        enc_keys: EncKeys::generate().expect("enc keys"),
        health_check_delay_secs: 0,
        raft_config: NodeConfig::default_raft_config(128),
        ..Default::default()
    };

    let application = hiqlite::start_node(cfg).await.expect("start hiqlite");
    (
        HiqliteRuntime::embedded(application, ReadConsistency::Local),
        dir,
    )
}

#[tokio::test]
async fn ensure_schema_seeds_bootstrap_organization_and_grant_actions() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "conalog-grant".to_owned(),
        "Conalog Grant".to_owned(),
    );

    db.ensure_schema().await.expect("ensure schema");

    let organizations: Vec<OrganizationRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT o.id, opl.party_id, o.name
             FROM organizations o
             LEFT JOIN organization_party_links opl
               ON opl.organization_id = o.id
              AND opl.relation_type = 'legal'
             ORDER BY o.id ASC",
            vec![],
        )
        .await
        .expect("organizations");
    assert_eq!(organizations.len(), 1);
    assert_eq!(organizations[0].id, "conalog-grant");
    assert_eq!(organizations[0].party_id, None);
    assert_eq!(organizations[0].name, "Conalog Grant");

    let memberships: Vec<MembershipRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT organization_id, user_id, membership_role, status FROM organization_memberships ORDER BY organization_id ASC",
            vec![],
        )
        .await
        .expect("memberships");
    assert_eq!(memberships.len(), 1);
    assert_eq!(memberships[0].organization_id, "conalog-grant");
    assert_eq!(memberships[0].user_id, "conalog-grant");
    assert_eq!(memberships[0].membership_role, "owner");
    assert_eq!(memberships[0].status, "active");

    let actions: Vec<ActionRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT code, visibility_granting, grantable_to_applications FROM actions ORDER BY code ASC",
            vec![],
        )
        .await
        .expect("actions");
    assert!(
        actions
            .iter()
            .any(|action| action.code == "organization.application.manage"),
        "organization.application.manage action must exist"
    );
    let profile_read = actions
        .iter()
        .find(|action| action.code == "organization.profile.read")
        .expect("organization.profile.read action");
    assert_eq!(profile_read.visibility_granting, 0);
    assert_eq!(profile_read.grantable_to_applications, 1);
    assert!(
        actions
            .iter()
            .all(|action| !action.code.starts_with("plant.") && !action.code.starts_with("asset."))
    );
    assert!(
        actions
            .iter()
            .all(|action| !action.code.starts_with("organization.plant."))
    );

    let role_actions: Vec<RoleActionRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT role_id, action_id FROM role_actions WHERE role_id = 'role-system-bootstrap-owner' ORDER BY action_id ASC",
            vec![],
        )
        .await
        .expect("role actions");
    let seeded_actions: Vec<String> = role_actions.into_iter().map(|row| row.action_id).collect();
    assert!(seeded_actions.contains(&"organization.application.manage".to_owned()));
    assert!(seeded_actions.contains(&"organization.profile.read".to_owned()));
}

#[tokio::test]
async fn organization_identity_links_party_without_overwriting_org_name() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "bootstrap-user".to_owned(),
        "Bootstrap User".to_owned(),
    );
    db.ensure_schema().await.expect("ensure schema");

    db.upsert_organization_with_owner("ops-seoul", "서울사업팀", None, "서울사업팀")
        .await
        .expect("organization");
    db.upsert_organization_identity("ops-seoul", Some("2768603291"), "주식회사 커널로그에너지")
        .await
        .expect("organization identity");

    let organizations: Vec<OrganizationRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT o.id, opl.party_id, o.name
             FROM organizations o
             LEFT JOIN organization_party_links opl
               ON opl.organization_id = o.id
              AND opl.relation_type = 'legal'
             WHERE o.id = 'ops-seoul'",
            vec![],
        )
        .await
        .expect("organizations");
    let parties: Vec<PartyRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT id, legal_name FROM parties WHERE business_no = '2768603291'",
            vec![],
        )
        .await
        .expect("parties");

    assert_eq!(organizations.len(), 1);
    assert_eq!(organizations[0].name, "서울사업팀");
    assert!(organizations[0].party_id.is_some());
    assert_eq!(parties.len(), 1);
    assert_eq!(parties[0].id, organizations[0].party_id.clone().unwrap());
    assert_eq!(parties[0].legal_name, "주식회사 커널로그에너지");
}

#[tokio::test]
async fn application_grants_round_trip_through_database() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "bootstrap-user".to_owned(),
        "Bootstrap User".to_owned(),
    );
    db.ensure_schema().await.expect("ensure schema");
    db.upsert_organization_with_owner("ops-seoul", "서울사업팀", None, "서울사업팀")
        .await
        .expect("organization");

    db.upsert_application(&Application {
        id: "application-awesome-dashboard".to_owned(),
        organization_id: "ops-seoul".to_owned(),
        slug: "awesome-dashboard".to_owned(),
        name: "Awesome Dashboard".to_owned(),
        description: Some("외부 대시보드".to_owned()),
        application_type: ApplicationType::Confidential,
        app_type: ApplicationAppType::Web,
        first_party: false,
        consent_policy: ApplicationConsentPolicy::Explicit,
        active: true,
        rauthy_client_id: None,
        provisioning_status: domain::ApplicationProvisioningStatus::Ready,
        created_by_user_id: "bootstrap-user".to_owned(),
    })
    .await
    .expect("application");
    db.upsert_application_action(&ApplicationAction {
        application_id: "application-awesome-dashboard".to_owned(),
        action_id: "organization.profile.read".to_owned(),
    })
    .await
    .expect("application action");
    let persisted_grant_id = db
        .upsert_application_grant(&ApplicationGrant {
            id: "grant-awesome-dashboard".to_owned(),
            user_id: "bootstrap-user".to_owned(),
            application_id: "application-awesome-dashboard".to_owned(),
            organization_id: "ops-seoul".to_owned(),
            status: ApplicationGrantStatus::Active,
            consented_at: "2026-04-12T00:00:00Z".to_owned(),
            revoked_at: None,
            revoked_by_user_id: None,
            created_at: "2026-04-12T00:00:00Z".to_owned(),
            updated_at: "2026-04-12T00:00:00Z".to_owned(),
        })
        .await
        .expect("application grant");
    db.upsert_application_grant_action(&ApplicationGrantAction {
        application_grant_id: persisted_grant_id.clone(),
        action_id: "organization.profile.read".to_owned(),
    })
    .await
    .expect("application grant action");

    let grants: Vec<ApplicationGrantRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT id, user_id, application_id, organization_id, status
             FROM application_grants
             WHERE id = 'grant-awesome-dashboard'",
            vec![],
        )
        .await
        .expect("application grants");
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].id, "grant-awesome-dashboard");
    assert_eq!(grants[0].user_id, "bootstrap-user");
    assert_eq!(grants[0].application_id, "application-awesome-dashboard");
    assert_eq!(grants[0].organization_id, "ops-seoul");
    assert_eq!(grants[0].status, "active");

    let grant_actions: Vec<RoleActionRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT application_grant_id AS role_id, action_id
             FROM application_grant_actions
             WHERE application_grant_id = 'grant-awesome-dashboard'",
            vec![],
        )
        .await
        .expect("application grant actions");
    assert_eq!(grant_actions.len(), 1);
    assert_eq!(grant_actions[0].action_id, "organization.profile.read");
}

#[tokio::test]
async fn application_actions_reject_non_grantable_actions() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime,
        "bootstrap-user".to_owned(),
        "Bootstrap User".to_owned(),
    );
    db.ensure_schema().await.expect("ensure schema");
    db.upsert_organization_with_owner("ops-seoul", "서울사업팀", None, "서울사업팀")
        .await
        .expect("organization");
    db.upsert_application(&Application {
        id: "application-awesome-dashboard".to_owned(),
        organization_id: "ops-seoul".to_owned(),
        slug: "awesome-dashboard".to_owned(),
        name: "Awesome Dashboard".to_owned(),
        description: None,
        application_type: ApplicationType::Confidential,
        app_type: ApplicationAppType::Web,
        first_party: false,
        consent_policy: ApplicationConsentPolicy::Explicit,
        active: true,
        rauthy_client_id: None,
        provisioning_status: domain::ApplicationProvisioningStatus::Ready,
        created_by_user_id: "bootstrap-user".to_owned(),
    })
    .await
    .expect("application");

    let err = db
        .upsert_application_action(&ApplicationAction {
            application_id: "application-awesome-dashboard".to_owned(),
            action_id: "organization.permission.manage".to_owned(),
        })
        .await
        .expect_err("non-grantable action must fail");
    assert!(err.to_string().contains("not grantable to applications"));
}

#[tokio::test]
async fn application_grant_actions_must_be_subset_of_application_actions() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime,
        "bootstrap-user".to_owned(),
        "Bootstrap User".to_owned(),
    );
    db.ensure_schema().await.expect("ensure schema");
    db.upsert_organization_with_owner("ops-seoul", "서울사업팀", None, "서울사업팀")
        .await
        .expect("organization");
    db.upsert_application(&Application {
        id: "application-awesome-dashboard".to_owned(),
        organization_id: "ops-seoul".to_owned(),
        slug: "awesome-dashboard".to_owned(),
        name: "Awesome Dashboard".to_owned(),
        description: None,
        application_type: ApplicationType::Confidential,
        app_type: ApplicationAppType::Web,
        first_party: false,
        consent_policy: ApplicationConsentPolicy::Explicit,
        active: true,
        rauthy_client_id: None,
        provisioning_status: domain::ApplicationProvisioningStatus::Ready,
        created_by_user_id: "bootstrap-user".to_owned(),
    })
    .await
    .expect("application");
    let grant_id = db
        .upsert_application_grant(&ApplicationGrant {
            id: "grant-awesome-dashboard".to_owned(),
            user_id: "bootstrap-user".to_owned(),
            application_id: "application-awesome-dashboard".to_owned(),
            organization_id: "ops-seoul".to_owned(),
            status: ApplicationGrantStatus::Active,
            consented_at: "2026-04-12T00:00:00Z".to_owned(),
            revoked_at: None,
            revoked_by_user_id: None,
            created_at: "2026-04-12T00:00:00Z".to_owned(),
            updated_at: "2026-04-12T00:00:00Z".to_owned(),
        })
        .await
        .expect("application grant");

    let err = db
        .upsert_application_grant_action(&ApplicationGrantAction {
            application_grant_id: grant_id,
            action_id: "organization.profile.read".to_owned(),
        })
        .await
        .expect_err("grant action outside application action subset must fail");
    assert!(
        err.to_string()
            .contains("application grant action must be included in application_actions")
    );
}

#[tokio::test]
async fn agreement_backed_organization_binding_grants_cross_org_action() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime,
        "bootstrap-user".to_owned(),
        "Bootstrap User".to_owned(),
    );
    db.ensure_schema().await.expect("ensure schema");
    db.upsert_organization_with_owner("grantor-org", "Grantor Org", None, "Grantor Org")
        .await
        .expect("grantor org");
    db.upsert_organization_with_owner("grantee-org", "Grantee Org", None, "Grantee Org")
        .await
        .expect("grantee org");
    db.upsert_role(&Role {
        id: "role-organization-profile-reader".to_owned(),
        organization_id: None,
        code: "organization.profile.reader".to_owned(),
        name: "Organization Profile Reader".to_owned(),
        description: None,
    })
    .await
    .expect("role");
    db.upsert_role_action(
        "role-organization-profile-reader",
        "organization.profile.read",
    )
    .await
    .expect("role action");

    let without_agreement = db
        .save_role_binding(&RoleBinding {
            id: "binding-cross-org-without-agreement".to_owned(),
            subject_type: RoleBindingSubjectType::Organization,
            subject_id: "grantee-org".to_owned(),
            subject_organization_id: "grantee-org".to_owned(),
            provider_id: "grant".to_owned(),
            resource_type: "organization".to_owned(),
            resource_id: "grantor-org".to_owned(),
            role_id: "role-organization-profile-reader".to_owned(),
            agreement_id: None,
            state: RoleBindingState::Active,
            created_by_user_id: "bootstrap-user".to_owned(),
            created_at: "2026-04-12T00:00:00Z".to_owned(),
            updated_at: "2026-04-12T00:00:00Z".to_owned(),
        })
        .await
        .expect_err("cross-org binding without agreement must fail");
    assert!(
        without_agreement
            .to_string()
            .contains("requires an agreement")
    );

    db.upsert_agreement(&Agreement {
        id: "agreement-grantor-grantee".to_owned(),
        agreement_type: AgreementType::Delegation,
        grantor_organization_id: "grantor-org".to_owned(),
        grantee_organization_id: "grantee-org".to_owned(),
        status: AgreementStatus::Active,
        title: "Grantor to Grantee".to_owned(),
        reference_no: None,
        starts_at: Some("2026-01-01T00:00:00Z".to_owned()),
        ends_at: None,
    })
    .await
    .expect("agreement");
    db.save_role_binding(&RoleBinding {
        id: "binding-cross-org-with-agreement".to_owned(),
        subject_type: RoleBindingSubjectType::Organization,
        subject_id: "grantee-org".to_owned(),
        subject_organization_id: "grantee-org".to_owned(),
        provider_id: "grant".to_owned(),
        resource_type: "organization".to_owned(),
        resource_id: "grantor-org".to_owned(),
        role_id: "role-organization-profile-reader".to_owned(),
        agreement_id: Some("agreement-grantor-grantee".to_owned()),
        state: RoleBindingState::Active,
        created_by_user_id: "bootstrap-user".to_owned(),
        created_at: "2026-04-12T00:00:00Z".to_owned(),
        updated_at: "2026-04-12T00:00:00Z".to_owned(),
    })
    .await
    .expect("agreement-backed binding");

    let allowed = db
        .has_user_organization_action_with_consistency(
            "bootstrap-user",
            "grantee-org",
            "grantor-org",
            "organization.profile.read",
            Some(ReadConsistency::Strong),
        )
        .await
        .expect("action lookup");
    assert!(allowed);

    let mismatched_resource = db
        .has_user_organization_action_with_consistency(
            "bootstrap-user",
            "grantee-org",
            "unknown-org",
            "organization.profile.read",
            Some(ReadConsistency::Strong),
        )
        .await
        .expect("mismatched resource lookup");
    assert!(!mismatched_resource);

    db.upsert_agreement(&Agreement {
        id: "agreement-grantor-grantee".to_owned(),
        agreement_type: AgreementType::Delegation,
        grantor_organization_id: "grantor-org".to_owned(),
        grantee_organization_id: "grantee-org".to_owned(),
        status: AgreementStatus::Active,
        title: "Expired Grantor to Grantee".to_owned(),
        reference_no: None,
        starts_at: Some("2020-01-01T00:00:00Z".to_owned()),
        ends_at: Some("2020-12-31T23:59:59Z".to_owned()),
    })
    .await
    .expect("expire agreement");
    let expired = db
        .has_user_organization_action_with_consistency(
            "bootstrap-user",
            "grantee-org",
            "grantor-org",
            "organization.profile.read",
            Some(ReadConsistency::Strong),
        )
        .await
        .expect("expired agreement lookup");
    assert!(!expired);
}

#[tokio::test]
async fn organization_identity_bootstraps_placeholder_organization_when_missing() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime,
        "bootstrap-user".to_owned(),
        "Bootstrap User".to_owned(),
    );
    db.ensure_schema().await.expect("ensure schema");

    let organization = db
        .upsert_organization_identity("missing-org", None, "ignored")
        .await
        .expect("placeholder organization");

    assert_eq!(organization.id, "missing-org");
    assert_eq!(organization.name, "missing-org");
    assert_eq!(organization.party_id, None);
}

#[tokio::test]
async fn agreements_round_trip_through_database() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime,
        "bootstrap-user".to_owned(),
        "Bootstrap User".to_owned(),
    );
    db.ensure_schema().await.expect("ensure schema");

    db.upsert_agreement(&Agreement {
        id: "agreement-1".to_owned(),
        agreement_type: AgreementType::Om,
        grantor_organization_id: "grantor-org".to_owned(),
        grantee_organization_id: "grantee-org".to_owned(),
        status: AgreementStatus::Active,
        title: "운영 위탁 계약".to_owned(),
        reference_no: Some("OM-2026-001".to_owned()),
        starts_at: Some("2026-01-01T00:00:00Z".to_owned()),
        ends_at: None,
    })
    .await
    .expect("agreement");
}

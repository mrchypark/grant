use domain::{
    Agreement, AgreementDocument, AgreementDocumentType, AgreementStatus, AgreementType, Client,
    ClientAction, ClientAppType, ClientConsentPolicy, ClientGrant, ClientGrantAction,
    ClientGrantStatus, ClientType,
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
    #[allow(dead_code)]
    business_no: String,
    legal_name: String,
}

impl From<&mut Row<'_>> for PartyRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            id: row.get("id"),
            business_no: row.get("business_no"),
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
}

impl From<&mut Row<'_>> for ActionRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            code: row.get("code"),
            visibility_granting: row.get("visibility_granting"),
        }
    }
}

#[derive(Debug)]
struct RoleRow {
    id: String,
    code: String,
}

impl From<&mut Row<'_>> for RoleRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            id: row.get("id"),
            code: row.get("code"),
        }
    }
}

#[derive(Debug)]
struct RoleActionRow {
    role_id: String,
    action_id: String,
}

impl From<&mut Row<'_>> for RoleActionRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            role_id: row.get("role_id"),
            action_id: row.get("action_id"),
        }
    }
}

#[derive(Debug)]
struct RoleBindingRow {
    subject_type: String,
    subject_id: String,
    subject_organization_id: String,
    resource_type: String,
    resource_id: String,
    role_id: String,
    state: String,
}

#[derive(Debug)]
struct ClientRow {
    id: String,
    organization_id: String,
    slug: String,
    name: String,
    client_type: String,
    app_type: String,
    first_party: i64,
    consent_policy: String,
    active: i64,
    created_by_user_id: String,
}

impl From<&mut Row<'_>> for ClientRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            id: row.get("id"),
            organization_id: row.get("organization_id"),
            slug: row.get("slug"),
            name: row.get("name"),
            client_type: row.get("client_type"),
            app_type: row.get("app_type"),
            first_party: row.get("first_party"),
            consent_policy: row.get("consent_policy"),
            active: row.get("active"),
            created_by_user_id: row.get("created_by_user_id"),
        }
    }
}

#[derive(Debug)]
struct ClientGrantRow {
    id: String,
    user_id: String,
    client_id: String,
    organization_id: String,
    status: String,
}

impl From<&mut Row<'_>> for ClientGrantRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            id: row.get("id"),
            user_id: row.get("user_id"),
            client_id: row.get("client_id"),
            organization_id: row.get("organization_id"),
            status: row.get("status"),
        }
    }
}

impl From<&mut Row<'_>> for RoleBindingRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            subject_type: row.get("subject_type"),
            subject_id: row.get("subject_id"),
            subject_organization_id: row.get("subject_organization_id"),
            resource_type: row.get("resource_type"),
            resource_id: row.get("resource_id"),
            role_id: row.get("role_id"),
            state: row.get("state"),
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
    let mut cfg = NodeConfig::default();
    cfg.node_id = 1;
    cfg.nodes = vec![Node {
        id: 1,
        addr_raft: raft_addr.clone(),
        addr_api: api_addr.clone(),
    }];
    cfg.listen_addr_api = "127.0.0.1".into();
    cfg.listen_addr_raft = "127.0.0.1".into();
    cfg.data_dir = dir.path().to_string_lossy().into_owned().into();
    cfg.filename_db = "test.sqlite".into();
    cfg.secret_api = "0123456789abcdef0123456789abcdef".to_owned();
    cfg.secret_raft = "fedcba9876543210fedcba9876543210".to_owned();
    cfg.enc_keys = EncKeys::generate().expect("enc keys");
    cfg.health_check_delay_secs = 0;
    cfg.raft_config = NodeConfig::default_raft_config(128);

    let client = hiqlite::start_node(cfg).await.expect("start hiqlite");
    (
        HiqliteRuntime::embedded(client, ReadConsistency::Local),
        dir,
    )
}

#[tokio::test]
async fn ensure_schema_seeds_bootstrap_organization_and_actions() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "conalog-account".to_owned(),
        "Conalog Account".to_owned(),
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
    assert_eq!(organizations[0].id, "conalog-account");
    assert_eq!(organizations[0].party_id, None);
    assert_eq!(organizations[0].name, "Conalog Account");

    let memberships: Vec<MembershipRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT organization_id, user_id, membership_role, status FROM organization_memberships ORDER BY organization_id ASC",
            vec![],
        )
        .await
        .expect("memberships");
    assert_eq!(memberships.len(), 1);
    assert_eq!(memberships[0].organization_id, "conalog-account");
    assert_eq!(memberships[0].user_id, "conalog-account");
    assert_eq!(memberships[0].membership_role, "owner");
    assert_eq!(memberships[0].status, "active");

    let actions: Vec<ActionRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT code, visibility_granting FROM actions ORDER BY code ASC",
            vec![],
        )
        .await
        .expect("actions");
    assert!(!actions.is_empty(), "action catalog must be seeded");
    let profile_read = actions
        .iter()
        .find(|action| action.code == "plant.profile.read")
        .expect("plant.profile.read action");
    assert_eq!(profile_read.visibility_granting, 1);
    assert!(
        actions
            .iter()
            .any(|action| action.code == "organization.plant.create"),
        "organization.plant.create action must exist"
    );

    let roles: Vec<RoleRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT id, code FROM roles ORDER BY code ASC",
            vec![],
        )
        .await
        .expect("roles");
    assert!(
        roles
            .iter()
            .any(|role| role.id == "role-system-bootstrap-owner"
                && role.code == "system.bootstrap.owner"),
        "bootstrap system role must be seeded"
    );

    let role_actions: Vec<RoleActionRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT role_id, action_id FROM role_actions WHERE role_id = 'role-system-bootstrap-owner' ORDER BY action_id ASC",
            vec![],
        )
        .await
        .expect("role actions");
    let seeded_actions: Vec<String> = role_actions
        .into_iter()
        .map(|row| {
            assert_eq!(row.role_id, "role-system-bootstrap-owner");
            row.action_id
        })
        .collect();
    assert!(seeded_actions.contains(&"organization.plant.create".to_owned()));

    let bindings: Vec<RoleBindingRow> = runtime
        .query_rows(
            ReadConsistency::Local,
            "SELECT subject_type, subject_id, subject_organization_id, resource_type, resource_id, role_id, state
             FROM role_bindings
             WHERE id = 'binding-system-bootstrap-owner'",
            vec![],
        )
        .await
        .expect("role bindings");
    assert_eq!(bindings.len(), 1);
    let binding = &bindings[0];
    assert_eq!(binding.subject_type, "user");
    assert_eq!(binding.subject_id, "conalog-account");
    assert_eq!(binding.subject_organization_id, "conalog-account");
    assert_eq!(binding.resource_type, "organization");
    assert_eq!(binding.resource_id, "conalog-account");
    assert_eq!(binding.role_id, "role-system-bootstrap-owner");
    assert_eq!(binding.state, "active");
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
            "SELECT id, business_no, legal_name FROM parties WHERE business_no = '2768603291'",
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
async fn bootstrap_principals_preserve_existing_party_link() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "conalog-account".to_owned(),
        "Conalog Account".to_owned(),
    );
    db.ensure_schema().await.expect("ensure schema");
    db.upsert_organization_identity(
        "conalog-account",
        Some("2768603291"),
        "주식회사 커널로그에너지",
    )
    .await
    .expect("bootstrap identity");

    db.ensure_bootstrap_principals()
        .await
        .expect("reseed bootstrap principals");

    let organizations: Vec<OrganizationRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT o.id, opl.party_id, o.name
             FROM organizations o
             LEFT JOIN organization_party_links opl
               ON opl.organization_id = o.id
              AND opl.relation_type = 'legal'
             WHERE o.id = 'conalog-account'",
            vec![],
        )
        .await
        .expect("organizations");
    assert_eq!(organizations.len(), 1);
    assert_eq!(
        organizations[0].party_id.as_deref(),
        Some("party-2768603291")
    );
}

#[tokio::test]
async fn organization_identity_without_business_number_returns_existing_party_link() {
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
    let organization = db
        .upsert_organization_identity("ops-seoul", None, "ignored")
        .await
        .expect("organization identity without business number");

    assert_eq!(organization.party_id.as_deref(), Some("party-2768603291"));
}

#[tokio::test]
async fn organization_identity_reuses_existing_party_id_for_business_number() {
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

    runtime
        .execute(
            "INSERT INTO parties (id, business_no, legal_name, active, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6)",
            vec![
                "party-custom".into(),
                "2768603291".into(),
                "주식회사 커널로그에너지".into(),
                1_i64.into(),
                "2026-04-11T00:00:00Z".into(),
                "2026-04-11T00:00:00Z".into(),
            ],
        )
        .await
        .expect("seed custom party");

    let organization = db
        .upsert_organization_identity(
            "ops-seoul",
            Some("2768603291"),
            "주식회사 커널로그에너지(변경)",
        )
        .await
        .expect("organization identity");

    assert_eq!(organization.party_id.as_deref(), Some("party-custom"));
    let parties: Vec<PartyRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT id, business_no, legal_name FROM parties WHERE business_no = '2768603291'",
            vec![],
        )
        .await
        .expect("parties");
    assert_eq!(parties.len(), 1);
    assert_eq!(parties[0].legal_name, "주식회사 커널로그에너지(변경)");
}

#[tokio::test]
async fn organization_creation_with_business_number_creates_legal_party_link() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "bootstrap-user".to_owned(),
        "Bootstrap User".to_owned(),
    );
    db.ensure_schema().await.expect("ensure schema");

    let organization = db
        .upsert_organization_with_owner(
            "ops-seoul",
            "서울사업팀",
            Some("2768603291"),
            "주식회사 커널로그에너지",
        )
        .await
        .expect("organization");

    assert_eq!(organization.party_id.as_deref(), Some("party-2768603291"));

    let parties: Vec<PartyRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT id, business_no, legal_name FROM parties WHERE business_no = '2768603291'",
            vec![],
        )
        .await
        .expect("parties");
    assert_eq!(parties.len(), 1);
    assert_eq!(parties[0].legal_name, "주식회사 커널로그에너지");
}

#[tokio::test]
async fn agreement_documents_round_trip_through_database() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
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

    db.upsert_agreement_document(&AgreementDocument {
        id: "agreement-doc-1".to_owned(),
        agreement_id: "agreement-1".to_owned(),
        document_type: AgreementDocumentType::MasterContract,
        file_name: "om-2026-001.pdf".to_owned(),
        object_key: "agreements/agreement-1/master.pdf".to_owned(),
        content_type: "application/pdf".to_owned(),
        document_no: Some("DOC-001".to_owned()),
        uploaded_at: "2026-04-11T00:00:00Z".to_owned(),
    })
    .await
    .expect("agreement document");

    let listed = db
        .list_agreement_documents("agreement-1")
        .await
        .expect("list agreement documents");
    assert_eq!(listed.len(), 1);
    assert_eq!(
        listed[0].document_type,
        AgreementDocumentType::MasterContract
    );
    assert_eq!(listed[0].document_no.as_deref(), Some("DOC-001"));
    assert_eq!(listed[0].uploaded_at, "2026-04-11T00:00:00Z");

    let fetched = db
        .get_agreement_document("agreement-doc-1")
        .await
        .expect("get agreement document");
    assert_eq!(fetched.agreement_id, "agreement-1");
    assert_eq!(fetched.object_key, "agreements/agreement-1/master.pdf");
    assert_eq!(fetched.uploaded_at, "2026-04-11T00:00:00Z");
}

#[tokio::test]
async fn client_grants_round_trip_through_database() {
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

    db.upsert_client(&Client {
        id: "client-awesome-dashboard".to_owned(),
        organization_id: "ops-seoul".to_owned(),
        slug: "awesome-dashboard".to_owned(),
        name: "Awesome Dashboard".to_owned(),
        description: Some("외부 발전소 대시보드".to_owned()),
        client_type: ClientType::Confidential,
        app_type: ClientAppType::Web,
        first_party: false,
        consent_policy: ClientConsentPolicy::Explicit,
        active: true,
        created_by_user_id: "bootstrap-user".to_owned(),
    })
    .await
    .expect("client");
    db.upsert_client_action(&ClientAction {
        client_id: "client-awesome-dashboard".to_owned(),
        action_id: "plant.profile.read".to_owned(),
    })
    .await
    .expect("client action");
    let persisted_grant_id = db
        .upsert_client_grant(&ClientGrant {
            id: "grant-awesome-dashboard".to_owned(),
            user_id: "bootstrap-user".to_owned(),
            client_id: "client-awesome-dashboard".to_owned(),
            organization_id: "ops-seoul".to_owned(),
            status: ClientGrantStatus::Active,
            consented_at: "2026-04-12T00:00:00Z".to_owned(),
            revoked_at: None,
            revoked_by_user_id: None,
            created_at: "2026-04-12T00:00:00Z".to_owned(),
            updated_at: "2026-04-12T00:00:00Z".to_owned(),
        })
        .await
        .expect("client grant");
    assert_eq!(persisted_grant_id, "grant-awesome-dashboard");
    let pre_client_actions: Vec<RoleActionRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT client_id AS role_id, action_id
             FROM client_actions
             WHERE client_id = 'client-awesome-dashboard'",
            vec![],
        )
        .await
        .expect("pre client actions");
    assert_eq!(pre_client_actions.len(), 1);
    let pre_grants: Vec<ClientGrantRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT id, user_id, client_id, organization_id, status
             FROM client_grants
             WHERE id = 'grant-awesome-dashboard'",
            vec![],
        )
        .await
        .expect("pre client grants");
    assert_eq!(pre_grants.len(), 1);
    db.upsert_client_grant_action(&ClientGrantAction {
        client_grant_id: persisted_grant_id.clone(),
        action_id: "plant.profile.read".to_owned(),
    })
    .await
    .expect("client grant action");

    let clients: Vec<ClientRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT id, organization_id, slug, name, client_type, app_type, first_party, consent_policy, active, created_by_user_id
             FROM clients
             WHERE id = 'client-awesome-dashboard'",
            vec![],
        )
        .await
        .expect("clients");
    assert_eq!(clients.len(), 1);
    assert_eq!(clients[0].id, "client-awesome-dashboard");
    assert_eq!(clients[0].organization_id, "ops-seoul");
    assert_eq!(clients[0].slug, "awesome-dashboard");
    assert_eq!(clients[0].name, "Awesome Dashboard");
    assert_eq!(clients[0].client_type, "confidential");
    assert_eq!(clients[0].app_type, "web");
    assert_eq!(clients[0].first_party, 0);
    assert_eq!(clients[0].consent_policy, "explicit");
    assert_eq!(clients[0].active, 1);
    assert_eq!(clients[0].created_by_user_id, "bootstrap-user");

    let grants: Vec<ClientGrantRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT id, user_id, client_id, organization_id, status
             FROM client_grants
             WHERE id = 'grant-awesome-dashboard'",
            vec![],
        )
        .await
        .expect("client grants");
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].id, "grant-awesome-dashboard");
    assert_eq!(grants[0].user_id, "bootstrap-user");
    assert_eq!(grants[0].client_id, "client-awesome-dashboard");
    assert_eq!(grants[0].organization_id, "ops-seoul");
    assert_eq!(grants[0].status, "active");

    let client_actions: Vec<RoleActionRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT client_id AS role_id, action_id
             FROM client_actions
             WHERE client_id = 'client-awesome-dashboard'",
            vec![],
        )
        .await
        .expect("client actions");
    assert_eq!(client_actions.len(), 1);
    assert_eq!(client_actions[0].role_id, "client-awesome-dashboard");
    assert_eq!(client_actions[0].action_id, "plant.profile.read");

    let grant_actions: Vec<RoleActionRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT client_grant_id AS role_id, action_id
             FROM client_grant_actions
             WHERE client_grant_id = 'grant-awesome-dashboard'",
            vec![],
        )
        .await
        .expect("client grant actions");
    assert_eq!(grant_actions.len(), 1);
    assert_eq!(grant_actions[0].role_id, "grant-awesome-dashboard");
    assert_eq!(grant_actions[0].action_id, "plant.profile.read");
}

#[tokio::test]
async fn client_actions_reject_non_grantable_actions() {
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
    db.upsert_client(&Client {
        id: "client-awesome-dashboard".to_owned(),
        organization_id: "ops-seoul".to_owned(),
        slug: "awesome-dashboard".to_owned(),
        name: "Awesome Dashboard".to_owned(),
        description: None,
        client_type: ClientType::Confidential,
        app_type: ClientAppType::Web,
        first_party: false,
        consent_policy: ClientConsentPolicy::Explicit,
        active: true,
        created_by_user_id: "bootstrap-user".to_owned(),
    })
    .await
    .expect("client");

    let err = db
        .upsert_client_action(&ClientAction {
            client_id: "client-awesome-dashboard".to_owned(),
            action_id: "plant.certificate.use".to_owned(),
        })
        .await
        .expect_err("non-grantable action must fail");
    assert!(err.to_string().contains("not grantable to clients"));
}

#[tokio::test]
async fn client_grant_actions_must_be_subset_of_client_actions() {
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
    db.upsert_client(&Client {
        id: "client-awesome-dashboard".to_owned(),
        organization_id: "ops-seoul".to_owned(),
        slug: "awesome-dashboard".to_owned(),
        name: "Awesome Dashboard".to_owned(),
        description: None,
        client_type: ClientType::Confidential,
        app_type: ClientAppType::Web,
        first_party: false,
        consent_policy: ClientConsentPolicy::Explicit,
        active: true,
        created_by_user_id: "bootstrap-user".to_owned(),
    })
    .await
    .expect("client");
    let grant_id = db
        .upsert_client_grant(&ClientGrant {
            id: "grant-awesome-dashboard".to_owned(),
            user_id: "bootstrap-user".to_owned(),
            client_id: "client-awesome-dashboard".to_owned(),
            organization_id: "ops-seoul".to_owned(),
            status: ClientGrantStatus::Active,
            consented_at: "2026-04-12T00:00:00Z".to_owned(),
            revoked_at: None,
            revoked_by_user_id: None,
            created_at: "2026-04-12T00:00:00Z".to_owned(),
            updated_at: "2026-04-12T00:00:00Z".to_owned(),
        })
        .await
        .expect("client grant");

    let err = db
        .upsert_client_grant_action(&ClientGrantAction {
            client_grant_id: grant_id,
            action_id: "plant.report.read".to_owned(),
        })
        .await
        .expect_err("grant action outside client action subset must fail");
    assert!(
        err.to_string()
            .contains("client grant action must be included in client_actions")
    );
}

#[tokio::test]
async fn active_client_grants_are_upserted_by_logical_key() {
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
    db.upsert_client(&Client {
        id: "client-awesome-dashboard".to_owned(),
        organization_id: "ops-seoul".to_owned(),
        slug: "awesome-dashboard".to_owned(),
        name: "Awesome Dashboard".to_owned(),
        description: None,
        client_type: ClientType::Confidential,
        app_type: ClientAppType::Web,
        first_party: false,
        consent_policy: ClientConsentPolicy::Explicit,
        active: true,
        created_by_user_id: "bootstrap-user".to_owned(),
    })
    .await
    .expect("client");

    let first_id = db
        .upsert_client_grant(&ClientGrant {
            id: "grant-1".to_owned(),
            user_id: "bootstrap-user".to_owned(),
            client_id: "client-awesome-dashboard".to_owned(),
            organization_id: "ops-seoul".to_owned(),
            status: ClientGrantStatus::Active,
            consented_at: "2026-04-12T00:00:00Z".to_owned(),
            revoked_at: None,
            revoked_by_user_id: None,
            created_at: "2026-04-12T00:00:00Z".to_owned(),
            updated_at: "2026-04-12T00:00:00Z".to_owned(),
        })
        .await
        .expect("first grant");
    let second_id = db
        .upsert_client_grant(&ClientGrant {
            id: "grant-2".to_owned(),
            user_id: "bootstrap-user".to_owned(),
            client_id: "client-awesome-dashboard".to_owned(),
            organization_id: "ops-seoul".to_owned(),
            status: ClientGrantStatus::Active,
            consented_at: "2026-04-13T00:00:00Z".to_owned(),
            revoked_at: None,
            revoked_by_user_id: None,
            created_at: "2026-04-13T00:00:00Z".to_owned(),
            updated_at: "2026-04-13T00:00:00Z".to_owned(),
        })
        .await
        .expect("second grant");

    assert_eq!(first_id, "grant-1");
    assert_eq!(second_id, "grant-1");

    let grants: Vec<ClientGrantRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT id, user_id, client_id, organization_id, status
             FROM client_grants
             WHERE client_id = 'client-awesome-dashboard'
             ORDER BY id ASC",
            vec![],
        )
        .await
        .expect("client grants");
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].id, "grant-1");
}

#[tokio::test]
async fn active_reconsent_replaces_client_grant_actions() {
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
    db.upsert_client(&Client {
        id: "client-awesome-dashboard".to_owned(),
        organization_id: "ops-seoul".to_owned(),
        slug: "awesome-dashboard".to_owned(),
        name: "Awesome Dashboard".to_owned(),
        description: None,
        client_type: ClientType::Confidential,
        app_type: ClientAppType::Web,
        first_party: false,
        consent_policy: ClientConsentPolicy::Explicit,
        active: true,
        created_by_user_id: "bootstrap-user".to_owned(),
    })
    .await
    .expect("client");
    db.upsert_client_action(&ClientAction {
        client_id: "client-awesome-dashboard".to_owned(),
        action_id: "plant.profile.read".to_owned(),
    })
    .await
    .expect("client action profile");
    db.upsert_client_action(&ClientAction {
        client_id: "client-awesome-dashboard".to_owned(),
        action_id: "plant.report.read".to_owned(),
    })
    .await
    .expect("client action report");

    let grant_id = db
        .upsert_client_grant(&ClientGrant {
            id: "grant-1".to_owned(),
            user_id: "bootstrap-user".to_owned(),
            client_id: "client-awesome-dashboard".to_owned(),
            organization_id: "ops-seoul".to_owned(),
            status: ClientGrantStatus::Active,
            consented_at: "2026-04-12T00:00:00Z".to_owned(),
            revoked_at: None,
            revoked_by_user_id: None,
            created_at: "2026-04-12T00:00:00Z".to_owned(),
            updated_at: "2026-04-12T00:00:00Z".to_owned(),
        })
        .await
        .expect("first grant");
    db.upsert_client_grant_action(&ClientGrantAction {
        client_grant_id: grant_id.clone(),
        action_id: "plant.profile.read".to_owned(),
    })
    .await
    .expect("first profile action");
    db.upsert_client_grant_action(&ClientGrantAction {
        client_grant_id: grant_id.clone(),
        action_id: "plant.report.read".to_owned(),
    })
    .await
    .expect("first report action");

    let reused_grant_id = db
        .upsert_client_grant(&ClientGrant {
            id: "grant-2".to_owned(),
            user_id: "bootstrap-user".to_owned(),
            client_id: "client-awesome-dashboard".to_owned(),
            organization_id: "ops-seoul".to_owned(),
            status: ClientGrantStatus::Active,
            consented_at: "2026-04-13T00:00:00Z".to_owned(),
            revoked_at: None,
            revoked_by_user_id: None,
            created_at: "2026-04-13T00:00:00Z".to_owned(),
            updated_at: "2026-04-13T00:00:00Z".to_owned(),
        })
        .await
        .expect("second grant");
    assert_eq!(reused_grant_id, grant_id);
    db.upsert_client_grant_action(&ClientGrantAction {
        client_grant_id: reused_grant_id.clone(),
        action_id: "plant.profile.read".to_owned(),
    })
    .await
    .expect("reconsent profile action");

    let grant_actions: Vec<RoleActionRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT client_grant_id AS role_id, action_id
             FROM client_grant_actions
             WHERE client_grant_id = 'grant-1'
             ORDER BY action_id ASC",
            vec![],
        )
        .await
        .expect("client grant actions");
    assert_eq!(grant_actions.len(), 1);
    assert_eq!(grant_actions[0].action_id, "plant.profile.read");
}

#[tokio::test]
async fn client_upsert_requires_existing_organization_and_creator_user() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "bootstrap-user".to_owned(),
        "Bootstrap User".to_owned(),
    );
    db.ensure_schema().await.expect("ensure schema");

    let err = db
        .upsert_client(&Client {
            id: "client-orphan".to_owned(),
            organization_id: "missing-org".to_owned(),
            slug: "orphan".to_owned(),
            name: "Orphan Client".to_owned(),
            description: None,
            client_type: ClientType::Confidential,
            app_type: ClientAppType::Web,
            first_party: false,
            consent_policy: ClientConsentPolicy::Explicit,
            active: true,
            created_by_user_id: "missing-user".to_owned(),
        })
        .await
        .expect_err("missing org and user must fail");
    assert!(
        err.to_string()
            .contains("client organization missing-org does not exist")
            || err
                .to_string()
                .contains("client creator user missing-user does not exist")
    );
}

#[tokio::test]
async fn client_grant_requires_existing_user_client_and_organization() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "bootstrap-user".to_owned(),
        "Bootstrap User".to_owned(),
    );
    db.ensure_schema().await.expect("ensure schema");

    let err = db
        .upsert_client_grant(&ClientGrant {
            id: "grant-orphan".to_owned(),
            user_id: "missing-user".to_owned(),
            client_id: "missing-client".to_owned(),
            organization_id: "missing-org".to_owned(),
            status: ClientGrantStatus::Active,
            consented_at: "2026-04-12T00:00:00Z".to_owned(),
            revoked_at: None,
            revoked_by_user_id: None,
            created_at: "2026-04-12T00:00:00Z".to_owned(),
            updated_at: "2026-04-12T00:00:00Z".to_owned(),
        })
        .await
        .expect_err("missing grant dependencies must fail");
    assert!(
        err.to_string()
            .contains("client grant user missing-user does not exist")
            || err
                .to_string()
                .contains("client grant client missing-client does not exist")
            || err
                .to_string()
                .contains("client grant organization missing-org does not exist")
    );
}

#[tokio::test]
async fn revoked_client_grants_can_exist_alongside_active_grants() {
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
    db.upsert_client(&Client {
        id: "client-awesome-dashboard".to_owned(),
        organization_id: "ops-seoul".to_owned(),
        slug: "awesome-dashboard".to_owned(),
        name: "Awesome Dashboard".to_owned(),
        description: None,
        client_type: ClientType::Confidential,
        app_type: ClientAppType::Web,
        first_party: false,
        consent_policy: ClientConsentPolicy::Explicit,
        active: true,
        created_by_user_id: "bootstrap-user".to_owned(),
    })
    .await
    .expect("client");

    let active_id = db
        .upsert_client_grant(&ClientGrant {
            id: "grant-active".to_owned(),
            user_id: "bootstrap-user".to_owned(),
            client_id: "client-awesome-dashboard".to_owned(),
            organization_id: "ops-seoul".to_owned(),
            status: ClientGrantStatus::Active,
            consented_at: "2026-04-12T00:00:00Z".to_owned(),
            revoked_at: None,
            revoked_by_user_id: None,
            created_at: "2026-04-12T00:00:00Z".to_owned(),
            updated_at: "2026-04-12T00:00:00Z".to_owned(),
        })
        .await
        .expect("active grant");
    let revoked_id = db
        .upsert_client_grant(&ClientGrant {
            id: "grant-revoked".to_owned(),
            user_id: "bootstrap-user".to_owned(),
            client_id: "client-awesome-dashboard".to_owned(),
            organization_id: "ops-seoul".to_owned(),
            status: ClientGrantStatus::Revoked,
            consented_at: "2026-04-11T00:00:00Z".to_owned(),
            revoked_at: Some("2026-04-13T00:00:00Z".to_owned()),
            revoked_by_user_id: Some("bootstrap-user".to_owned()),
            created_at: "2026-04-11T00:00:00Z".to_owned(),
            updated_at: "2026-04-13T00:00:00Z".to_owned(),
        })
        .await
        .expect("revoked grant");

    assert_eq!(active_id, "grant-active");
    assert_eq!(revoked_id, "grant-revoked");

    let grants: Vec<ClientGrantRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT id, user_id, client_id, organization_id, status
             FROM client_grants
             WHERE client_id = 'client-awesome-dashboard'
             ORDER BY id ASC",
            vec![],
        )
        .await
        .expect("client grants");
    assert_eq!(grants.len(), 2);
    assert_eq!(grants[0].id, "grant-active");
    assert_eq!(grants[0].status, "active");
    assert_eq!(grants[1].id, "grant-revoked");
    assert_eq!(grants[1].status, "revoked");
}

#[tokio::test]
async fn certificates_are_shared_across_organizations_linked_to_same_party() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "bootstrap-user".to_owned(),
        "Bootstrap User".to_owned(),
    );
    db.ensure_schema().await.expect("ensure schema");

    db.upsert_organization_with_owner(
        "ops-seoul",
        "서울사업팀",
        Some("2768603291"),
        "주식회사 커널로그에너지",
    )
    .await
    .expect("organization a");
    db.upsert_organization_with_owner(
        "ops-busan",
        "부산사업팀",
        Some("2768603291"),
        "주식회사 커널로그에너지",
    )
    .await
    .expect("organization b");

    let saved = db
        .save_certificate(
            "ops-seoul",
            Some("2768603291"),
            "주식회사 커널로그에너지",
            "encrypted-bundle",
        )
        .await
        .expect("save certificate");

    let loaded = db
        .load_active_certificate("ops-busan")
        .await
        .expect("load certificate through shared party");

    assert_eq!(loaded.id, saved.id);
    assert_eq!(loaded.party_id, saved.party_id);
    assert_eq!(loaded.party_id, "party-2768603291");
    assert_eq!(loaded.business_no.as_deref(), Some("2768603291"));
    assert_eq!(loaded.encrypted_blob, "encrypted-bundle");
}

#[tokio::test]
async fn save_certificate_reuses_existing_party_without_overwriting_legal_name() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "bootstrap-user".to_owned(),
        "Bootstrap User".to_owned(),
    );
    db.ensure_schema().await.expect("ensure schema");

    db.upsert_organization_with_owner(
        "ops-seoul",
        "서울사업팀",
        Some("2768603291"),
        "주식회사 커널로그에너지",
    )
    .await
    .expect("organization");

    db.save_certificate(
        "ops-seoul",
        Some("2768603291"),
        "커널로그에너지 운영 인증서",
        "encrypted-bundle",
    )
    .await
    .expect("save certificate");

    let parties: Vec<PartyRow> = runtime
        .query_rows(
            ReadConsistency::Strong,
            "SELECT id, business_no, legal_name FROM parties WHERE business_no = '2768603291'",
            vec![],
        )
        .await
        .expect("parties");
    assert_eq!(parties.len(), 1);
    assert_eq!(parties[0].legal_name, "주식회사 커널로그에너지");
}

#[tokio::test]
async fn organization_identity_bootstraps_placeholder_organization_when_missing() {
    let (runtime, _dir) = test_runtime().await;
    let db = Database::new(
        runtime.clone(),
        "bootstrap-user".to_owned(),
        "Bootstrap User".to_owned(),
    );
    db.ensure_schema().await.expect("ensure schema");

    let organization = db
        .upsert_organization_identity("missing-org", None, "ignored")
        .await
        .expect("organization identity");

    assert_eq!(organization.id, "missing-org");
    assert_eq!(organization.name, "missing-org");
    assert_eq!(organization.party_id, None);
    assert!(organization.active);
}

use domain::{
    Action, ActionRiskLevel, Agreement, AgreementStatus, AgreementType, Application,
    ApplicationAction, ApplicationAppType, ApplicationConsentPolicy, ApplicationGrant,
    ApplicationGrantAction, ApplicationGrantStatus, ApplicationType, Group, MembershipRole,
    MembershipStatus, Organization, OrganizationMembership, OrganizationPartyLink,
    OrganizationPartyRelationType, Party, Role, RoleBinding, RoleBindingState,
    RoleBindingSubjectType, User,
};

#[test]
fn serializes_account_models() {
    let user = User {
        id: "conalog-account".into(),
        email: Some("ops@example.com".into()),
        display_name: "Conalog Account".into(),
        active: true,
    };
    let organization = Organization {
        id: "2768603291".into(),
        party_id: Some("party_1".into()),
        business_no: Some("2768603291".into()),
        name: "커널로그에너지".into(),
        inactivated_at: None,
    };
    let party = Party {
        id: "party_1".into(),
        business_no: "2768603291".into(),
        legal_name: "주식회사 커널로그에너지".into(),
        active: true,
    };
    let party_link = OrganizationPartyLink {
        organization_id: organization.id.clone(),
        party_id: party.id.clone(),
        relation_type: OrganizationPartyRelationType::Legal,
    };
    let membership = OrganizationMembership {
        id: "org_mem_1".into(),
        organization_id: organization.id.clone(),
        user_id: user.id.clone(),
        membership_role: MembershipRole::Owner,
        status: MembershipStatus::Active,
    };
    let group = Group {
        id: "group_1".into(),
        organization_id: organization.id.clone(),
        name: "현장팀".into(),
        description: Some("현장 운영 그룹".into()),
    };
    let agreement = Agreement {
        id: "agr_1".into(),
        agreement_type: AgreementType::Om,
        grantor_organization_id: organization.id.clone(),
        grantee_organization_id: "org_2".into(),
        status: AgreementStatus::Active,
        title: "운영 위임 계약".into(),
        reference_no: Some("AG-2026-001".into()),
        starts_at: Some("2026-01-01T00:00:00Z".into()),
        ends_at: Some("2027-01-01T00:00:00Z".into()),
    };
    let action = Action {
        id: "action_1".into(),
        provider_id: "account".into(),
        code: "organization.application.manage".into(),
        name: "Application Manage".into(),
        description: Some("Manage organization applications".into()),
        risk_level: ActionRiskLevel::Medium,
        visibility_granting: false,
        grantable_to_applications: true,
        consent_label: Some("애플리케이션 관리".into()),
        consent_description: Some("조직 애플리케이션을 관리합니다.".into()),
    };
    let application = Application {
        id: "application_1".into(),
        organization_id: organization.id.clone(),
        slug: "awesome-dashboard".into(),
        name: "Awesome Dashboard".into(),
        description: Some("외부 대시보드".into()),
        application_type: ApplicationType::Confidential,
        app_type: ApplicationAppType::Web,
        first_party: false,
        consent_policy: ApplicationConsentPolicy::Explicit,
        active: true,
        created_by_user_id: user.id.clone(),
    };
    let application_action = ApplicationAction {
        application_id: application.id.clone(),
        action_id: action.id.clone(),
    };
    let application_grant = ApplicationGrant {
        id: "application_grant_1".into(),
        user_id: user.id.clone(),
        application_id: application.id.clone(),
        organization_id: organization.id.clone(),
        status: ApplicationGrantStatus::Active,
        consented_at: "2026-03-31T00:00:00Z".into(),
        revoked_at: None,
        revoked_by_user_id: None,
        created_at: "2026-03-31T00:00:00Z".into(),
        updated_at: "2026-03-31T00:00:00Z".into(),
    };
    let application_grant_action = ApplicationGrantAction {
        application_grant_id: application_grant.id.clone(),
        action_id: action.id.clone(),
    };
    let role = Role {
        id: "role_1".into(),
        organization_id: Some(organization.id.clone()),
        code: "viewer".into(),
        name: "Viewer".into(),
        description: Some("Read-only access".into()),
    };
    let binding = RoleBinding {
        id: "binding_1".into(),
        subject_type: RoleBindingSubjectType::Group,
        subject_id: group.id.clone(),
        subject_organization_id: organization.id.clone(),
        provider_id: "account".into(),
        resource_type: "organization".into(),
        resource_id: organization.id.clone(),
        role_id: role.id.clone(),
        agreement_id: Some(agreement.id.clone()),
        state: RoleBindingState::Active,
        created_by_user_id: user.id.clone(),
        created_at: "2026-03-31T00:00:00Z".into(),
        updated_at: "2026-03-31T00:00:00Z".into(),
    };

    let value = serde_json::json!({
        "user": user,
        "organization": organization,
        "membership": membership,
        "group": group,
        "party": party,
        "partyLink": party_link,
        "agreement": agreement,
        "action": action,
        "role": role,
        "binding": binding,
        "application": application,
        "applicationAction": application_action,
        "applicationGrant": application_grant,
        "applicationGrantAction": application_grant_action,
    });

    assert_eq!(value["organization"]["partyId"], "party_1");
    assert_eq!(value["organization"]["businessNo"], "2768603291");
    assert_eq!(value["party"]["businessNo"], "2768603291");
    assert_eq!(value["party"]["legalName"], "주식회사 커널로그에너지");
    assert_eq!(value["partyLink"]["relationType"], "legal");
    assert_eq!(value["membership"]["membershipRole"], "owner");
    assert_eq!(value["group"]["name"], "현장팀");
    assert_eq!(value["agreement"]["agreementType"], "om");
    assert_eq!(value["agreement"]["referenceNo"], "AG-2026-001");
    assert_eq!(value["action"]["code"], "organization.application.manage");
    assert_eq!(value["action"]["grantableToApplications"], true);
    assert_eq!(value["action"]["consentLabel"], "애플리케이션 관리");
    assert_eq!(value["role"]["code"], "viewer");
    assert_eq!(value["binding"]["resourceType"], "organization");
    assert_eq!(value["binding"]["resourceId"], "2768603291");
    assert_eq!(value["application"]["slug"], "awesome-dashboard");
    assert_eq!(value["application"]["applicationType"], "confidential");
    assert_eq!(value["applicationAction"]["actionId"], "action_1");
    assert_eq!(value["applicationGrant"]["status"], "active");
    assert_eq!(
        value["applicationGrantAction"]["applicationGrantId"],
        "application_grant_1"
    );
}
